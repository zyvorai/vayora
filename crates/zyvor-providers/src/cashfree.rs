//! Cashfree Payments adapter (PG API 2023-08-01). Built from Cashfree's public docs and tested
//! against `zyvor-provider-mock`; it has NOT been exercised against Cashfree's live sandbox.
//! Cashfree amounts are decimal rupees; this adapter converts at the boundary and keeps integer
//! minor units everywhere else. We use our attempt id as Cashfree's `order_id`, so any payment or
//! refund can be looked up even after a lost response.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use zyvor_core::{Currency, minor_to_decimal};

use crate::*;

pub struct Cashfree {
    base: String,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    /// Native UPI intent / QR through Order Pay. Off until the account has it enabled (Cashfree's S2S flag).
    upi_intent: bool,
}

/// The Order Pay API version the UPI intent / QR calls are made with (the order itself uses the version
/// below). Verified against Cashfree's sandbox on 31 October 2026.
const ORDER_PAY_VERSION: &str = "2026-01-01";

impl Cashfree {
    pub fn new(
        base_url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            http,
            upi_intent: false,
        }
    }

    /// Turn on native UPI intent / QR for this account. Only do so once Cashfree has enabled it.
    pub fn with_upi_intent(mut self, on: bool) -> Self {
        self.upi_intent = on;
        self
    }

    fn req_v(&self, m: reqwest::Method, path: &str, version: &str) -> reqwest::RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .header("x-client-id", &self.client_id)
            .header("x-client-secret", &self.client_secret)
            .header("x-api-version", version)
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.req_v(m, path, "2023-08-01")
    }
}

/// The partner's UPI intent response (`channel: link`): per-app links under `data.payload`.
/// `default` is the one to open when the payer has not chosen an app.
fn intent_action(p: &Value) -> Result<ClientAction, ProviderError> {
    let payload = p["data"]["payload"].as_object();
    let uri = payload
        .and_then(|m| m.get("default"))
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .ok_or_else(|| ProviderError::Uncertain("intent response had no default link".into()))?;
    let apps = payload
        .into_iter()
        .flatten()
        .filter(|(k, _)| !matches!(k.as_str(), "default" | "web"))
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();
    Ok(ClientAction::UpiIntent {
        uri: uri.to_string(),
        apps,
        expires_at: None,
    })
}

/// The partner's UPI QR response (`channel: qrcode`): a PNG as a data URL under `data.payload.qrcode`.
fn qr_action(p: &Value) -> Result<ClientAction, ProviderError> {
    let image = p["data"]["payload"]["qrcode"]
        .as_str()
        .filter(|i| i.starts_with("data:image/png;base64,"))
        .ok_or_else(|| ProviderError::Uncertain("QR response had no PNG image".into()))?;
    Ok(ClientAction::UpiQr {
        image: image.to_string(),
    })
}

/// Decimal rupees as a JSON number, built without going through float arithmetic.
fn rupees(minor: i64, c: Currency) -> Value {
    serde_json::from_str(&minor_to_decimal(minor, c)).expect("decimal is valid JSON")
}
fn to_minor(v: &Value) -> Option<i64> {
    v.as_f64().map(|f| (f * 100.0).round() as i64)
}

fn pay_status(s: &str) -> PaymentStatus {
    match s {
        "SUCCESS" => PaymentStatus::Captured,
        "FAILED" | "USER_DROPPED" | "CANCELLED" | "VOID" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

fn refund_state(r: &Value) -> RefundState {
    RefundState {
        status: match r["refund_status"].as_str() {
            Some("SUCCESS") => RefundStatus::Processed,
            Some("CANCELLED") | Some("FAILED") => RefundStatus::Failed,
            _ => RefundStatus::Pending,
        },
        provider_refund_ref: r["cf_refund_id"]
            .as_str()
            .map(str::to_string)
            .or_else(|| r["cf_refund_id"].as_i64().map(|i| i.to_string())),
        failure_reason: r["status_description"]
            .as_str()
            .filter(|_| r["refund_status"] == "CANCELLED")
            .map(str::to_string),
        raw: r.clone(),
    }
}

#[async_trait]
impl Provider for Cashfree {
    fn name(&self) -> &'static str {
        "cashfree"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            methods: vec![
                Method::Upi,
                Method::Card,
                Method::Netbanking,
                Method::Wallet,
            ],
            currencies: vec![Currency::Inr],
            partial_refund: true,
            manual_capture: true,
            server_initiated_upi: true,
            // Verified against the sandbox only, and needs the account to have it enabled: opt in per account.
            upi_intent: self.upi_intent,
            refund_window_days: Some(365),
        }
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        let body = json!({
            "order_id": req.attempt_id,
            "order_amount": rupees(req.amount_minor, req.currency),
            "order_currency": req.currency.code(),
            "customer_details": {
                "customer_id": req.customer_id.clone().unwrap_or_else(|| format!("guest_{}", req.attempt_id)),
                "customer_phone": req.customer_phone.clone().unwrap_or_else(|| "9999999999".into()),
            },
            "order_meta": { "return_url": req.return_url },
        });
        let order = read_json(
            self.req(reqwest::Method::POST, "/pg/orders")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let session = order["payment_session_id"]
            .as_str()
            .ok_or_else(|| ProviderError::Uncertain("order response had no session".into()))?
            .to_string();
        let mut state = PaymentState {
            status: PaymentStatus::Pending,
            provider_order_ref: Some(req.attempt_id.clone()),
            provider_payment_ref: None,
            failure_reason: None,
            raw: order,
        };

        if let (Method::Upi, Some(vpa)) = (req.method, &req.vpa) {
            // Server-initiated collect. If this call times out the collect may already be live.
            let body = json!({ "payment_session_id": session, "payment_method": { "upi": { "channel": "collect", "upi_id": vpa } } });
            let p = read_json(
                self.req(reqwest::Method::POST, "/pg/orders/sessions")
                    .json(&body)
                    .send()
                    .await
                    .map_err(classify)?,
            )
            .await?;
            state.provider_payment_ref = p["cf_payment_id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| p["cf_payment_id"].as_i64().map(|i| i.to_string()));
            state.raw = p;
            return Ok(Initiated {
                state,
                action: ClientAction::UpiCollect { vpa: vpa.clone() },
            });
        }
        // Both must hold: the request asked for it, and this account is opted in. The router already refuses to
        // send it elsewhere; the adapter does not rely on that.
        if req.method == Method::Upi && req.upi_intent && self.upi_intent {
            // Native intent (phones) or QR (desktop and counter screens). Cashfree wants the payer's device
            // on this call and assumes the strictest flow when it is missing.
            let env = req.client.clone().unwrap_or_default();
            let channel = if env.device == "desktop" {
                "qrcode"
            } else {
                "link"
            };
            let body = json!({ "payment_session_id": session, "payment_method": { "upi": { "channel": channel } } });
            let mut call = self
                .req_v(
                    reqwest::Method::POST,
                    "/pg/orders/sessions",
                    ORDER_PAY_VERSION,
                )
                .header("x-idempotency-key", req.attempt_id.as_str());
            for (name, value) in [
                ("x-client-device", env.device.as_str()),
                ("x-client-os", env.os.as_str()),
                ("x-client-rendering-type", env.rendering.as_str()),
                ("x-client-browser", env.browser.as_str()),
            ] {
                if !value.is_empty() {
                    call = call.header(name, value);
                }
            }
            let p = read_json(call.json(&body).send().await.map_err(classify)?).await?;
            let action = if channel == "qrcode" {
                qr_action(&p)?
            } else {
                intent_action(&p)?
            };
            state.provider_payment_ref = p["cf_payment_id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| p["cf_payment_id"].as_i64().map(|i| i.to_string()));
            state.raw = p;
            return Ok(Initiated { state, action });
        }
        Ok(Initiated {
            state,
            action: ClientAction::HostedCheckout {
                params: json!({ "provider": "cashfree", "payment_session_id": session }),
            },
        })
    }

    async fn capture(
        &self,
        attempt_id: &str,
        _ref: &str,
        amount_minor: i64,
        currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        let body = json!({ "action": "CAPTURE", "amount": rupees(amount_minor, currency) });
        let p = read_json(
            self.req(
                reqwest::Method::POST,
                &format!("/pg/orders/{attempt_id}/authorization"),
            )
            .json(&body)
            .send()
            .await
            .map_err(classify)?,
        )
        .await?;
        Ok(PaymentState {
            status: pay_status(p["payment_status"].as_str().unwrap_or("")),
            provider_order_ref: Some(attempt_id.into()),
            provider_payment_ref: p["cf_payment_id"].as_str().map(str::to_string),
            failure_reason: None,
            raw: p,
        })
    }

    async fn check(&self) -> Result<String, ProviderError> {
        // an order that cannot exist: a 404 means the credentials got us past authentication
        let res = self
            .req(reqwest::Method::GET, "/pg/orders/zyvor_connection_check")
            .send()
            .await
            .map_err(classify)?;
        check_status(res.status(), &[200, 404, 422])
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let res = self
            .req(
                reqwest::Method::GET,
                &format!("/pg/orders/{attempt_id}/payments"),
            )
            .send()
            .await
            .map_err(classify)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(PaymentState {
                status: PaymentStatus::Failed,
                provider_order_ref: None,
                provider_payment_ref: None,
                failure_reason: Some("not_found".into()),
                raw: Value::Null,
            });
        }
        let v = read_json(res).await?;
        let items = v.as_array().cloned().unwrap_or_default();
        let rank = |p: &Value| match pay_status(p["payment_status"].as_str().unwrap_or("")) {
            PaymentStatus::Captured => 3,
            PaymentStatus::Authorized => 2,
            PaymentStatus::Pending => 1,
            PaymentStatus::Failed => 0,
        };
        Ok(match items.iter().max_by_key(|p| rank(p)) {
            Some(p) => PaymentState {
                status: pay_status(p["payment_status"].as_str().unwrap_or("")),
                provider_order_ref: Some(attempt_id.into()),
                provider_payment_ref: p["cf_payment_id"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| p["cf_payment_id"].as_i64().map(|i| i.to_string())),
                failure_reason: p["error_details"]["error_description"]
                    .as_str()
                    .map(str::to_string),
                raw: p.clone(),
            },
            None => PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(attempt_id.into()),
                provider_payment_ref: None,
                failure_reason: None,
                raw: v,
            },
        })
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        let body = json!({ "refund_amount": rupees(req.amount_minor, req.currency), "refund_id": req.refund_id, "refund_speed": "STANDARD" });
        let r = read_json(
            self.req(
                reqwest::Method::POST,
                &format!("/pg/orders/{}/refunds", req.attempt_id),
            )
            .json(&body)
            .send()
            .await
            .map_err(classify)?,
        )
        .await?;
        Ok(refund_state(&r))
    }

    async fn fetch_refund(
        &self,
        attempt_id: &str,
        refund_id: &str,
        _payment: &str,
    ) -> Result<RefundState, ProviderError> {
        let res = self
            .req(
                reqwest::Method::GET,
                &format!("/pg/orders/{attempt_id}/refunds/{refund_id}"),
            )
            .send()
            .await
            .map_err(classify)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(RefundState {
                status: RefundStatus::Failed,
                provider_refund_ref: None,
                failure_reason: Some("not_found".into()),
                raw: Value::Null,
            });
        }
        Ok(refund_state(&read_json(res).await?))
    }

    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        let body = json!({ "pagination": { "limit": 100 }, "filters": { "start_date": format!("{on}T00:00:00Z"), "end_date": format!("{on}T23:59:59Z") } });
        let v = read_json(
            self.req(reqwest::Method::POST, "/pg/settlement/recon")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let mut by: std::collections::BTreeMap<String, SettlementRecord> = Default::default();
        for it in v["data"].as_array().cloned().unwrap_or_default() {
            let sid = it["cf_settlement_id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| it["cf_settlement_id"].as_i64().map(|i| i.to_string()))
                .unwrap_or_else(|| "unknown".into());
            let rec = by.entry(sid.clone()).or_insert_with(|| SettlementRecord {
                provider_ref: sid,
                utr: it["settlement_utr"].as_str().map(str::to_string),
                currency: Currency::Inr,
                settled_on: on,
                net_minor: 0,
                items: vec![],
            });
            let amount = to_minor(&it["event_amount"]).unwrap_or(0);
            let fees = to_minor(&it["service_charge"]).unwrap_or(0)
                + to_minor(&it["service_tax"]).unwrap_or(0);
            let id = it["order_id"].as_str().map(str::to_string);
            match it["event_type"].as_str() {
                Some("PAYMENT") => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Payment,
                    provider_ref: id.clone(),
                    amount_minor: amount,
                    description: "payment".into(),
                }),
                Some("REFUND") => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Refund,
                    provider_ref: id.clone(),
                    amount_minor: -amount.abs(),
                    description: "refund".into(),
                }),
                _ => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Adjustment,
                    provider_ref: id.clone(),
                    amount_minor: amount,
                    description: it["event_type"].as_str().unwrap_or("adjustment").into(),
                }),
            }
            if fees != 0 {
                rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Fee,
                    provider_ref: id,
                    amount_minor: -fees,
                    description: "service charge and tax".into(),
                });
            }
            rec.net_minor = rec.items.iter().map(|i| i.amount_minor).sum();
        }
        Ok(by.into_values().collect())
    }

    fn verify_webhook(
        &self,
        secret: &[u8],
        headers: &Headers,
        body: &[u8],
        now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        let sig = headers
            .get("x-webhook-signature")
            .ok_or(WebhookError::MissingSignature)?;
        let ts = headers
            .get("x-webhook-timestamp")
            .ok_or(WebhookError::MissingSignature)?;
        let given = base64::engine::general_purpose::STANDARD
            .decode(sig)
            .map_err(|_| WebhookError::BadSignature)?;
        if !ct_eq(&hmac_sha256(secret, &[ts.as_bytes(), body]), &given) {
            return Err(WebhookError::BadSignature);
        }
        // Cashfree's timestamp is milliseconds since epoch.
        let ts_secs = ts
            .parse::<i64>()
            .map_err(|_| WebhookError::Malformed("bad timestamp".into()))?
            / 1000;
        if (now_unix - ts_secs).abs() > 600 {
            return Err(WebhookError::Stale);
        }
        let v: Value =
            serde_json::from_slice(body).map_err(|e| WebhookError::Malformed(e.to_string()))?;
        let ty = v["type"].as_str().unwrap_or_default();
        let order = v["data"]["order"]["order_id"].as_str().map(str::to_string);
        let pay = &v["data"]["payment"];
        let refund = &v["data"]["refund"];
        let cf_id = |x: &Value| {
            x.as_str()
                .map(str::to_string)
                .or_else(|| x.as_i64().map(|i| i.to_string()))
        };
        let (kind, ident) = match ty {
            "PAYMENT_SUCCESS_WEBHOOK" => (EventKind::PaymentCaptured, cf_id(&pay["cf_payment_id"])),
            "PAYMENT_FAILED_WEBHOOK" | "PAYMENT_USER_DROPPED_WEBHOOK" => {
                (EventKind::PaymentFailed, cf_id(&pay["cf_payment_id"]))
            }
            "REFUND_STATUS_WEBHOOK" => (
                if refund["refund_status"] == "SUCCESS" {
                    EventKind::RefundProcessed
                } else if refund["refund_status"] == "CANCELLED" {
                    EventKind::RefundFailed
                } else {
                    EventKind::Other
                },
                refund["refund_id"].as_str().map(str::to_string),
            ),
            _ => (EventKind::Other, None),
        };
        let is_refund = ty == "REFUND_STATUS_WEBHOOK";
        Ok(WebhookEvent {
            // Cashfree sends no event id; the same payload redelivered must dedupe to the same key.
            event_id: format!(
                "{ty}:{}:{}",
                ident.clone().unwrap_or_default(),
                v["event_time"].as_str().unwrap_or_default()
            ),
            kind,
            order_ref: order,
            provider_payment_ref: if is_refund {
                cf_id(&refund["cf_payment_id"])
            } else {
                ident.clone()
            },
            provider_refund_ref: if is_refund {
                cf_id(&refund["cf_refund_id"])
            } else {
                None
            },
            refund_ref: if is_refund { ident } else { None },
            amount_minor: if is_refund {
                to_minor(&refund["refund_amount"])
            } else {
                to_minor(&pay["payment_amount"])
            },
            failure_reason: pay["payment_message"]
                .as_str()
                .filter(|_| kind == EventKind::PaymentFailed)
                .map(str::to_string),
            raw: v,
        })
    }
}

#[cfg(test)]
mod upi_tests {
    use super::*;

    // Shapes captured from Cashfree's sandbox on 31 October 2026 (values shortened).
    #[test]
    fn reads_the_intent_response_and_keeps_per_app_links() {
        let p = json!({
            "action": "custom", "channel": "link", "cf_payment_id": "1469871650921752576",
            "data": { "url": null, "payload": {
                "default": "https://payments-test.cashfree.com/sim/1", "bhim": "https://payments-test.cashfree.com/sim/1?a=bhim",
                "gpay": "https://payments-test.cashfree.com/sim/1?a=gpay", "paytm": "https://payments-test.cashfree.com/sim/1?a=paytm",
                "phonepe": "https://payments-test.cashfree.com/sim/1?a=phonepe", "web": "https://sandbox.cashfree.com/pg/view/upi/x" } },
        });
        match intent_action(&p).unwrap() {
            ClientAction::UpiIntent { uri, apps, .. } => {
                assert_eq!(uri, "https://payments-test.cashfree.com/sim/1");
                assert_eq!(
                    apps.keys().map(String::as_str).collect::<Vec<_>>(),
                    ["bhim", "gpay", "paytm", "phonepe"],
                    "per-app links are kept; default and the web page are not apps"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_the_qr_response_and_refuses_anything_that_is_not_a_png() {
        let ok =
            json!({ "data": { "payload": { "qrcode": "data:image/png;base64,iVBORw0KGgo=" } } });
        assert!(
            matches!(qr_action(&ok).unwrap(), ClientAction::UpiQr { image } if image.starts_with("data:image/png;base64,"))
        );
        for bad in [
            json!({ "data": { "payload": {} } }),
            json!({ "data": { "payload": { "qrcode": "https://evil.example/q.png" } } }),
            json!({ "data": { "payload": { "qrcode": "javascript:alert(1)" } } }),
        ] {
            assert!(
                matches!(qr_action(&bad), Err(ProviderError::Uncertain(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_intent_answer_without_a_link_is_unknown_not_a_failure() {
        for bad in [
            json!({ "data": { "payload": {} } }),
            json!({ "data": { "payload": { "default": "" } } }),
            json!({ "data": null }),
        ] {
            assert!(
                matches!(intent_action(&bad), Err(ProviderError::Uncertain(_))),
                "{bad}"
            );
        }
    }
}
