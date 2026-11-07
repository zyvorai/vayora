//! Razorpay adapter (Orders + hosted Checkout). Built from Razorpay's public API docs and tested
//! against `zyvor-provider-mock`; it has NOT been exercised against Razorpay's live sandbox.
//! Amounts are already integer paise on Razorpay's side.
//!
//! Recurring / UPI AutoPay: see [`recurring`] — docs-only, local mock only.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zyvor_core::Currency;

use crate::*;

mod recurring;

pub struct Razorpay {
    base: String,
    key_id: String,
    key_secret: String,
    http: reqwest::Client,
    /// Native UPI intent / QR is opt-in per account: set `"upi_intent": true` in credentials once
    /// Razorpay has enabled S2S UPI for that account. Built from public docs; not sandbox-verified.
    upi_intent: bool,
}

impl Razorpay {
    pub fn new(
        base_url: impl Into<String>,
        key_id: impl Into<String>,
        key_secret: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            key_id: key_id.into(),
            key_secret: key_secret.into(),
            http,
            upi_intent: false,
        }
    }

    /// Turn on native UPI intent / QR for this account. Only after Razorpay has enabled it.
    pub fn with_upi_intent(mut self, on: bool) -> Self {
        self.upi_intent = on;
        self
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{}{path}", self.base))
            .basic_auth(&self.key_id, Some(&self.key_secret))
    }
    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.base))
            .basic_auth(&self.key_id, Some(&self.key_secret))
    }
}

fn status_of(s: &str) -> PaymentStatus {
    match s {
        "authorized" => PaymentStatus::Authorized,
        "captured" | "refunded" => PaymentStatus::Captured,
        "failed" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

fn payment_state(p: &Value, order: Option<&str>) -> PaymentState {
    PaymentState {
        status: status_of(p["status"].as_str().unwrap_or("created")),
        provider_order_ref: order
            .map(str::to_string)
            .or_else(|| p["order_id"].as_str().map(str::to_string)),
        provider_payment_ref: p["id"]
            .as_str()
            .filter(|i| i.starts_with("pay_"))
            .map(str::to_string),
        failure_reason: p["error_description"].as_str().map(str::to_string),
        raw: p.clone(),
    }
}

fn refund_state(r: &Value) -> RefundState {
    RefundState {
        status: match r["status"].as_str() {
            Some("processed") => RefundStatus::Processed,
            Some("failed") => RefundStatus::Failed,
            _ => RefundStatus::Pending,
        },
        provider_refund_ref: r["id"].as_str().map(str::to_string),
        failure_reason: None,
        raw: r.clone(),
    }
}

#[async_trait]
impl Provider for Razorpay {
    fn name(&self) -> &'static str {
        "razorpay"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            methods: vec![
                Method::Upi,
                Method::Card,
                Method::Netbanking,
                Method::Wallet,
            ],
            currencies: vec![Currency::Inr, Currency::Usd],
            partial_refund: true,
            manual_capture: true,
            server_initiated_upi: false,
            // Opt-in: set `"upi_intent": true` in credentials. Docs-only; not sandbox-verified.
            upi_intent: self.upi_intent,
            // Recurring adapter built from public docs; NOT verified against Razorpay sandbox.
            mandates: true,
            refund_window_days: Some(180),
        }
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        let body = json!({ "amount": req.amount_minor, "currency": req.currency.code(), "receipt": req.attempt_id, "notes": { "attempt_id": req.attempt_id } });
        let order = read_json(
            self.post("/v1/orders")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let order_id = order["id"]
            .as_str()
            .ok_or_else(|| ProviderError::Uncertain("order response had no id".into()))?
            .to_string();

        if req.method == Method::Upi && req.upi_intent && self.upi_intent {
            // S2S UPI create (public docs). Desktop prefers QR; phones prefer intent.
            let desktop = req
                .client
                .as_ref()
                .map(|c| c.device == "desktop")
                .unwrap_or(false);
            let flow = if desktop { "qr" } else { "intent" };
            let body = json!({
                "amount": req.amount_minor,
                "currency": req.currency.code(),
                "order_id": order_id,
                "email": "payer@zyvor.invalid",
                "contact": req.customer_phone.clone().unwrap_or_else(|| "9999999999".into()),
                "method": "upi",
                "upi": { "flow": flow },
                "notes": { "attempt_id": req.attempt_id },
            });
            let p = read_json(
                self.post("/v1/payments/create/upi")
                    .json(&body)
                    .send()
                    .await
                    .map_err(classify)?,
            )
            .await?;
            let pay_id = p["razorpay_payment_id"]
                .as_str()
                .or_else(|| p["id"].as_str())
                .map(str::to_string);
            let action = if flow == "qr" {
                let image = p["qr_image"]
                    .as_str()
                    .or_else(|| p["image_url"].as_str())
                    .ok_or_else(|| ProviderError::Uncertain("upi qr response had no image".into()))?
                    .to_string();
                ClientAction::UpiQr { image }
            } else {
                let uri = p["link"]
                    .as_str()
                    .or_else(|| p["upi"]["link"].as_str())
                    .or_else(|| p["intent_url"].as_str())
                    .ok_or_else(|| {
                        ProviderError::Uncertain("upi intent response had no link".into())
                    })?
                    .to_string();
                ClientAction::UpiIntent {
                    uri,
                    apps: Default::default(),
                    expires_at: None,
                }
            };
            return Ok(Initiated {
                state: PaymentState {
                    status: PaymentStatus::Pending,
                    provider_order_ref: Some(order_id),
                    provider_payment_ref: pay_id,
                    failure_reason: None,
                    raw: p,
                },
                action,
            });
        }

        Ok(Initiated {
            state: PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(order_id.clone()),
                provider_payment_ref: None,
                failure_reason: None,
                raw: order,
            },
            // Hosted Checkout collects method details; card data never reaches Zyvor.
            action: ClientAction::HostedCheckout {
                params: json!({ "provider": "razorpay", "key": self.key_id, "order_id": order_id, "amount": req.amount_minor, "currency": req.currency.code() }),
            },
        })
    }

    async fn capture(
        &self,
        _attempt: &str,
        provider_payment_ref: &str,
        amount_minor: i64,
        currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        let body = json!({ "amount": amount_minor, "currency": currency.code() });
        let p = read_json(
            self.post(&format!("/v1/payments/{provider_payment_ref}/capture"))
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        Ok(payment_state(&p, None))
    }

    async fn check(&self) -> Result<String, ProviderError> {
        let res = self
            .get("/v1/payments?count=1")
            .send()
            .await
            .map_err(classify)?;
        check_status(res.status(), &[200])
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let orders = read_json(
            self.get(&format!("/v1/orders?receipt={attempt_id}"))
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let Some(order) = orders["items"].as_array().and_then(|a| a.first()) else {
            return Ok(PaymentState {
                status: PaymentStatus::Failed,
                provider_order_ref: None,
                provider_payment_ref: None,
                failure_reason: Some("not_found".into()),
                raw: orders,
            });
        };
        let order_id = order["id"].as_str().unwrap_or_default().to_string();
        let pays = read_json(
            self.get(&format!("/v1/orders/{order_id}/payments"))
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let items = pays["items"].as_array().cloned().unwrap_or_default();
        // A paid payment wins over everything; otherwise report the most advanced state.
        let rank = |p: &Value| match status_of(p["status"].as_str().unwrap_or("")) {
            PaymentStatus::Captured => 3,
            PaymentStatus::Authorized => 2,
            PaymentStatus::Pending => 1,
            PaymentStatus::Failed => 0,
        };
        Ok(match items.iter().max_by_key(|p| rank(p)) {
            Some(p) => payment_state(p, Some(&order_id)),
            None => PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(order_id),
                provider_payment_ref: None,
                failure_reason: None,
                raw: pays,
            },
        })
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        let body = json!({ "amount": req.amount_minor, "receipt": req.refund_id, "notes": { "refund_id": req.refund_id } });
        let r = read_json(
            self.post(&format!("/v1/payments/{}/refund", req.provider_payment_ref))
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
        _attempt: &str,
        refund_id: &str,
        provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError> {
        let rs = read_json(
            self.get(&format!("/v1/payments/{provider_payment_ref}/refunds"))
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let found = rs["items"].as_array().and_then(|a| {
            a.iter()
                .find(|r| r["notes"]["refund_id"] == refund_id || r["receipt"] == refund_id)
        });
        Ok(match found {
            Some(r) => refund_state(r),
            None => RefundState {
                status: RefundStatus::Failed,
                provider_refund_ref: None,
                failure_reason: Some("not_found".into()),
                raw: rs,
            },
        })
    }

    async fn create_mandate(&self, req: &CreateMandate) -> Result<MandateState, ProviderError> {
        self.rec_create_mandate(req).await
    }
    async fn fetch_mandate(&self, mandate_id: &str) -> Result<MandateState, ProviderError> {
        self.rec_fetch_mandate(mandate_id).await
    }
    async fn revoke_mandate(&self, mandate_id: &str) -> Result<MandateState, ProviderError> {
        self.rec_revoke_mandate(mandate_id).await
    }
    async fn debit(&self, req: &DebitRequest) -> Result<Initiated, ProviderError> {
        self.rec_debit(req).await
    }
    async fn fetch_debit(
        &self,
        attempt_id: &str,
        mandate_id: &str,
    ) -> Result<PaymentState, ProviderError> {
        self.rec_fetch_debit(attempt_id, mandate_id).await
    }

    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        use chrono::Datelike;
        let path = format!(
            "/v1/settlements/recon/combined?year={}&month={}&day={}",
            on.year(),
            on.month(),
            on.day()
        );
        let v = read_json(self.get(&path).send().await.map_err(classify)?).await?;
        let mut by: std::collections::BTreeMap<String, SettlementRecord> = Default::default();
        for it in v["items"].as_array().cloned().unwrap_or_default() {
            let sid = it["settlement_id"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let rec = by.entry(sid.clone()).or_insert_with(|| SettlementRecord {
                provider_ref: sid,
                utr: it["settlement_utr"].as_str().map(str::to_string),
                currency: Currency::Inr,
                settled_on: on,
                net_minor: 0,
                items: vec![],
            });
            let (credit, debit) = (
                it["credit"].as_i64().unwrap_or(0),
                it["debit"].as_i64().unwrap_or(0),
            );
            let fees = it["fee"].as_i64().unwrap_or(0) + it["tax"].as_i64().unwrap_or(0);
            let id = it["entity_id"].as_str().map(str::to_string);
            match it["type"].as_str() {
                Some("payment") => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Payment,
                    provider_ref: id.clone(),
                    amount_minor: credit,
                    description: "payment".into(),
                }),
                Some("refund") => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Refund,
                    provider_ref: id.clone(),
                    amount_minor: -debit,
                    description: "refund".into(),
                }),
                _ => rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Adjustment,
                    provider_ref: id.clone(),
                    amount_minor: credit - debit,
                    description: it["description"].as_str().unwrap_or("adjustment").into(),
                }),
            }
            if fees != 0 {
                rec.items.push(SettlementItem {
                    kind: SettlementItemKind::Fee,
                    provider_ref: id,
                    amount_minor: -fees,
                    description: "fee and tax".into(),
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
        _now: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        let sig = headers
            .get("x-razorpay-signature")
            .ok_or(WebhookError::MissingSignature)?;
        let given = hex::decode(sig).map_err(|_| WebhookError::BadSignature)?;
        if !ct_eq(&hmac_sha256(secret, &[body]), &given) {
            return Err(WebhookError::BadSignature);
        }
        let event_id = headers
            .get("x-razorpay-event-id")
            .cloned()
            .ok_or_else(|| WebhookError::Malformed("missing x-razorpay-event-id".into()))?;
        let v: Value =
            serde_json::from_slice(body).map_err(|e| WebhookError::Malformed(e.to_string()))?;
        let name = v["event"].as_str().unwrap_or_default();
        let pay = &v["payload"]["payment"]["entity"];
        let refund = &v["payload"]["refund"]["entity"];
        let kind = match name {
            "payment.authorized" => EventKind::PaymentAuthorized,
            "payment.captured" => EventKind::PaymentCaptured,
            "payment.failed" => EventKind::PaymentFailed,
            "refund.processed" => EventKind::RefundProcessed,
            "refund.failed" => EventKind::RefundFailed,
            _ => EventKind::Other,
        };
        let is_refund = matches!(kind, EventKind::RefundProcessed | EventKind::RefundFailed);
        Ok(WebhookEvent {
            event_id,
            kind,
            order_ref: pay["order_id"].as_str().map(str::to_string),
            provider_payment_ref: if is_refund {
                refund["payment_id"].as_str()
            } else {
                pay["id"].as_str()
            }
            .map(str::to_string),
            provider_refund_ref: if is_refund {
                refund["id"].as_str().map(str::to_string)
            } else {
                None
            },
            refund_ref: if is_refund {
                refund["notes"]["refund_id"].as_str().map(str::to_string)
            } else {
                None
            },
            amount_minor: if is_refund {
                refund["amount"].as_i64()
            } else {
                pay["amount"].as_i64()
            },
            failure_reason: pay["error_description"].as_str().map(str::to_string),
            raw: v,
        })
    }
}
