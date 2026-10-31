//! Generic HTTP provider: bring your own gateway.
//!
//! Point this adapter at a gateway you run (or a bank or acquirer API behind a thin wrapper) that speaks
//! the small JSON contract in `docs/zyvor/PROVIDER_CONTRACT.md`, and it behaves like Razorpay or Cashfree
//! to the rest of Zyvor: routing, the state machine, the ledger and reconciliation need no changes.
//!
//! It is also the template to copy for a native adapter. The rules every adapter must keep:
//! * a failure that left no trace is [`ProviderError::Rejected`]; one whose outcome cannot be known
//!   (timeout, 5xx, dropped connection) is [`ProviderError::Uncertain`] and is never retried elsewhere;
//! * our attempt id is the gateway's reference, so a lost response is resolved by `fetch_payment`;
//! * amounts are integer minor units on the wire and everywhere else, never floats;
//! * webhooks are verified (signature and age) before anything in them is read.
//!
//! Verified only against a local stub in `tests/custom.rs`, never against a live gateway.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zyvor_core::Currency;

use crate::*;

/// Webhooks older or newer than this many seconds are rejected as possible replays.
const WEBHOOK_TOLERANCE_SECS: i64 = 300;

pub struct Custom {
    base: String,
    api_key: String,
    http: reqwest::Client,
    caps: Capabilities,
}

impl Custom {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http,
            // Conservative defaults. The router trusts these, so override them to match your gateway.
            caps: Capabilities {
                methods: vec![Method::Upi, Method::Card],
                currencies: vec![Currency::Inr],
                partial_refund: true,
                manual_capture: false,
                server_initiated_upi: true,
                upi_intent: false,
                mandates: false,
                refund_window_days: None,
            },
        }
    }

    /// Declare what your gateway really supports; the router will never send it anything else.
    pub fn with_capabilities(mut self, caps: Capabilities) -> Self {
        self.caps = caps;
        self
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.api_key)
    }
}

/// Percent-encode one path segment. Our own ids are already safe; a gateway's references may not be.
fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn pay_status(v: &Value) -> Result<PaymentStatus, ProviderError> {
    match v["status"].as_str() {
        Some("pending") => Ok(PaymentStatus::Pending),
        Some("authorized") => Ok(PaymentStatus::Authorized),
        Some("captured") => Ok(PaymentStatus::Captured),
        Some("failed") => Ok(PaymentStatus::Failed),
        // An answer we cannot read is an unknown outcome, not a failure.
        other => Err(ProviderError::Uncertain(format!(
            "gateway returned an unrecognised payment status {other:?}"
        ))),
    }
}

fn pay_state(v: Value, attempt_id: &str) -> Result<PaymentState, ProviderError> {
    Ok(PaymentState {
        status: pay_status(&v)?,
        provider_order_ref: Some(attempt_id.to_string()),
        provider_payment_ref: v["provider_payment_ref"].as_str().map(str::to_string),
        failure_reason: v["failure_reason"].as_str().map(str::to_string),
        raw: v,
    })
}

fn refund_state(v: Value) -> Result<RefundState, ProviderError> {
    let status = match v["status"].as_str() {
        Some("pending") => RefundStatus::Pending,
        Some("processed") => RefundStatus::Processed,
        Some("failed") => RefundStatus::Failed,
        other => {
            return Err(ProviderError::Uncertain(format!(
                "gateway returned an unrecognised refund status {other:?}"
            )));
        }
    };
    Ok(RefundState {
        status,
        provider_refund_ref: v["provider_refund_ref"].as_str().map(str::to_string),
        failure_reason: v["failure_reason"].as_str().map(str::to_string),
        raw: v,
    })
}

#[async_trait]
impl Provider for Custom {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn capabilities(&self) -> Capabilities {
        self.caps.clone()
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        let body = json!({
            "attempt_id": req.attempt_id,
            "amount_minor": req.amount_minor,
            "currency": req.currency.code(),
            "method": req.method.as_str(),
            "vpa": req.vpa,
            "customer_id": req.customer_id,
            "customer_phone": req.customer_phone,
            "capture_automatic": req.capture_automatic,
            "return_url": req.return_url,
        });
        let v = read_json(
            self.req(reqwest::Method::POST, "/payments")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let action = serde_json::from_value(v["action"].clone()).unwrap_or(ClientAction::None);
        Ok(Initiated {
            state: pay_state(v, &req.attempt_id)?,
            action,
        })
    }

    async fn capture(
        &self,
        attempt_id: &str,
        provider_payment_ref: &str,
        amount_minor: i64,
        currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        let body = json!({
            "attempt_id": attempt_id,
            "amount_minor": amount_minor,
            "currency": currency.code(),
        });
        let v = read_json(
            self.req(
                reqwest::Method::POST,
                &format!("/payments/{}/capture", seg(provider_payment_ref)),
            )
            .json(&body)
            .send()
            .await
            .map_err(classify)?,
        )
        .await?;
        pay_state(v, attempt_id)
    }

    /// Looked up by our attempt id. A 404 means the gateway never saw the request, which is a definite
    /// "failed", so the worker can close out an attempt whose response was lost before it arrived.
    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let res = self
            .req(
                reqwest::Method::GET,
                &format!("/payments/{}", seg(attempt_id)),
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
        pay_state(read_json(res).await?, attempt_id)
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        let body = json!({
            "refund_id": req.refund_id,
            "attempt_id": req.attempt_id,
            "provider_payment_ref": req.provider_payment_ref,
            "amount_minor": req.amount_minor,
            "currency": req.currency.code(),
        });
        refund_state(
            read_json(
                self.req(reqwest::Method::POST, "/refunds")
                    .json(&body)
                    .send()
                    .await
                    .map_err(classify)?,
            )
            .await?,
        )
    }

    async fn fetch_refund(
        &self,
        _attempt_id: &str,
        refund_id: &str,
        _provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError> {
        refund_state(
            read_json(
                self.req(
                    reqwest::Method::GET,
                    &format!("/refunds/{}", seg(refund_id)),
                )
                .send()
                .await
                .map_err(classify)?,
            )
            .await?,
        )
    }

    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        let v = read_json(
            self.req(reqwest::Method::GET, &format!("/settlements?date={on}"))
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        serde_json::from_value(v["settlements"].clone())
            .map_err(|e| ProviderError::Rejected(format!("malformed settlements: {e}")))
    }

    /// Signature: hex HMAC-SHA256 over `"{timestamp}.{body}"` in `x-zyvor-signature`, with the Unix
    /// timestamp (seconds) in `x-zyvor-timestamp`.
    fn verify_webhook(
        &self,
        secret: &[u8],
        headers: &Headers,
        body: &[u8],
        now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        let sig = headers
            .get("x-zyvor-signature")
            .ok_or(WebhookError::MissingSignature)?;
        let ts = headers
            .get("x-zyvor-timestamp")
            .ok_or(WebhookError::MissingSignature)?;
        let given = hex::decode(sig).map_err(|_| WebhookError::BadSignature)?;
        if !ct_eq(&hmac_sha256(secret, &[ts.as_bytes(), b".", body]), &given) {
            return Err(WebhookError::BadSignature);
        }
        let ts_secs = ts
            .parse::<i64>()
            .map_err(|_| WebhookError::Malformed("bad timestamp".into()))?;
        if (now_unix - ts_secs).abs() > WEBHOOK_TOLERANCE_SECS {
            return Err(WebhookError::Stale);
        }
        let v: Value =
            serde_json::from_slice(body).map_err(|e| WebhookError::Malformed(e.to_string()))?;
        let event_id = v["event_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WebhookError::Malformed("missing event_id".into()))?
            .to_string();
        let kind = match v["type"].as_str().unwrap_or_default() {
            "payment.authorized" => EventKind::PaymentAuthorized,
            "payment.captured" => EventKind::PaymentCaptured,
            "payment.failed" => EventKind::PaymentFailed,
            "refund.processed" => EventKind::RefundProcessed,
            "refund.failed" => EventKind::RefundFailed,
            _ => EventKind::Other,
        };
        let text = |k: &str| v[k].as_str().map(str::to_string);
        Ok(WebhookEvent {
            event_id,
            kind,
            order_ref: text("attempt_id"),
            provider_payment_ref: text("provider_payment_ref"),
            provider_refund_ref: text("provider_refund_ref"),
            refund_ref: text("refund_id"),
            amount_minor: v["amount_minor"].as_i64(),
            failure_reason: text("failure_reason"),
            raw: v,
        })
    }
}
