//! PhonePe Payment Gateway adapter (orchestrate mode — merchant's own PhonePe account).
//!
//! Speaks PhonePe's PG HTTP surface via a normalised thin client. Until live credentials are
//! verified, point `base_url` at a local mock that mirrors the paths below. Built from public
//! PhonePe PG docs; treat sandbox verification as a follow-up before production.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zyvor_core::Currency;

use crate::*;

const WEBHOOK_TOLERANCE_SECS: i64 = 300;

pub struct PhonePe {
    base: String,
    merchant_id: String,
    salt_key: String,
    http: reqwest::Client,
    caps: Capabilities,
}

impl PhonePe {
    pub fn new(
        base_url: impl Into<String>,
        merchant_id: impl Into<String>,
        salt_key: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            merchant_id: merchant_id.into(),
            salt_key: salt_key.into(),
            http,
            caps: Capabilities {
                methods: vec![Method::Upi, Method::Card, Method::Wallet, Method::Netbanking],
                currencies: vec![Currency::Inr],
                partial_refund: true,
                manual_capture: false,
                server_initiated_upi: true,
                upi_intent: true,
                mandates: false,
                refund_window_days: Some(60),
            },
        }
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(m, format!("{}{path}", self.base))
            .header("X-MERCHANT-ID", &self.merchant_id)
            .header("X-SALT-KEY", &self.salt_key)
    }
}

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
    match v["status"].as_str().or_else(|| v["code"].as_str()) {
        Some("pending" | "PAYMENT_PENDING") => Ok(PaymentStatus::Pending),
        Some("authorized" | "PAYMENT_AUTHORIZED") => Ok(PaymentStatus::Authorized),
        Some("captured" | "PAYMENT_SUCCESS" | "COMPLETED") => Ok(PaymentStatus::Captured),
        Some("failed" | "PAYMENT_ERROR" | "PAYMENT_DECLINED") => Ok(PaymentStatus::Failed),
        other => Err(ProviderError::Uncertain(format!(
            "phonepe unrecognised status {other:?}"
        ))),
    }
}

fn pay_state(v: Value, attempt_id: &str) -> Result<PaymentState, ProviderError> {
    Ok(PaymentState {
        status: pay_status(&v)?,
        provider_order_ref: Some(attempt_id.to_string()),
        provider_payment_ref: v["provider_payment_ref"]
            .as_str()
            .or_else(|| v["transactionId"].as_str())
            .map(str::to_string),
        failure_reason: v["failure_reason"]
            .as_str()
            .or_else(|| v["message"].as_str())
            .map(str::to_string),
        raw: v,
    })
}

#[async_trait]
impl Provider for PhonePe {
    fn name(&self) -> &'static str {
        "phonepe"
    }

    fn capabilities(&self) -> Capabilities {
        self.caps.clone()
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        let body = json!({
            "merchantId": self.merchant_id,
            "merchantTransactionId": req.attempt_id,
            "amount": req.amount_minor,
            "currency": req.currency.code(),
            "method": req.method.as_str(),
            "vpa": req.vpa,
            "upiIntent": req.upi_intent,
            "callbackUrl": req.return_url,
        });
        let v = read_json(
            self.req(reqwest::Method::POST, "/pg/v1/pay")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let action = if let Some(uri) = v["data"]["instrumentResponse"]["intentUrl"]
            .as_str()
            .or_else(|| v["action"]["uri"].as_str())
        {
            ClientAction::UpiIntent {
                uri: uri.to_string(),
                apps: Default::default(),
                expires_at: None,
            }
        } else {
            serde_json::from_value(v["action"].clone()).unwrap_or(ClientAction::None)
        };
        Ok(Initiated {
            state: pay_state(v, &req.attempt_id)?,
            action,
        })
    }

    async fn capture(
        &self,
        attempt_id: &str,
        _provider_payment_ref: &str,
        _amount_minor: i64,
        _currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        // PhonePe PG auto-captures for most methods; treat as fetch.
        self.fetch_payment(attempt_id).await
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let res = self
            .req(
                reqwest::Method::GET,
                &format!(
                    "/pg/v1/status/{}/{}",
                    seg(&self.merchant_id),
                    seg(attempt_id)
                ),
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
            "merchantId": self.merchant_id,
            "merchantTransactionId": req.attempt_id,
            "merchantRefundId": req.refund_id,
            "originalTransactionId": req.provider_payment_ref,
            "amount": req.amount_minor,
        });
        let v = read_json(
            self.req(reqwest::Method::POST, "/pg/v1/refund")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let status = match v["status"].as_str().or_else(|| v["code"].as_str()) {
            Some("pending" | "PENDING") => RefundStatus::Pending,
            Some("processed" | "PAYMENT_SUCCESS" | "COMPLETED") => RefundStatus::Processed,
            Some("failed" | "PAYMENT_ERROR") => RefundStatus::Failed,
            other => {
                return Err(ProviderError::Uncertain(format!(
                    "phonepe refund status {other:?}"
                )));
            }
        };
        Ok(RefundState {
            status,
            provider_refund_ref: v["provider_refund_ref"]
                .as_str()
                .or_else(|| v["refundId"].as_str())
                .map(str::to_string),
            failure_reason: v["failure_reason"].as_str().map(str::to_string),
            raw: v,
        })
    }

    async fn fetch_settlements(
        &self,
        _on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        Err(ProviderError::Unsupported(
            "phonepe settlement sync not wired yet; import settlements manually".into(),
        ))
    }

    async fn fetch_refund(
        &self,
        _attempt_id: &str,
        refund_id: &str,
        _provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError> {
        let v = read_json(
            self.req(
                reqwest::Method::GET,
                &format!("/pg/v1/refund/{}/{}", seg(&self.merchant_id), seg(refund_id)),
            )
            .send()
            .await
            .map_err(classify)?,
        )
        .await?;
        let status = match v["status"].as_str().or_else(|| v["code"].as_str()) {
            Some("pending" | "PENDING") => RefundStatus::Pending,
            Some("processed" | "PAYMENT_SUCCESS" | "COMPLETED") => RefundStatus::Processed,
            Some("failed" | "PAYMENT_ERROR") => RefundStatus::Failed,
            other => {
                return Err(ProviderError::Uncertain(format!(
                    "phonepe refund status {other:?}"
                )));
            }
        };
        Ok(RefundState {
            status,
            provider_refund_ref: v["refundId"].as_str().map(str::to_string),
            failure_reason: v["failure_reason"].as_str().map(str::to_string),
            raw: v,
        })
    }

    fn verify_webhook(
        &self,
        secret: &[u8],
        headers: &Headers,
        body: &[u8],
        now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        // Prefer PhonePe-style headers; fall back to Zyvor custom HMAC for mocks.
        let sig = headers
            .get("x-verify")
            .or_else(|| headers.get("x-zyvor-signature"))
            .ok_or(WebhookError::MissingSignature)?;
        let ts = headers
            .get("x-zyvor-timestamp")
            .cloned()
            .unwrap_or_else(|| now_unix.to_string());
        let key = if secret.is_empty() {
            self.salt_key.as_bytes()
        } else {
            secret
        };
        let given = hex::decode(sig).map_err(|_| WebhookError::BadSignature)?;
        if !ct_eq(&hmac_sha256(key, &[ts.as_bytes(), b".", body]), &given) {
            return Err(WebhookError::BadSignature);
        }
        if let Ok(ts_secs) = ts.parse::<i64>()
            && (now_unix - ts_secs).abs() > WEBHOOK_TOLERANCE_SECS
        {
            return Err(WebhookError::Stale);
        }
        let v: Value =
            serde_json::from_slice(body).map_err(|e| WebhookError::Malformed(e.to_string()))?;
        let event_id = v["event_id"]
            .as_str()
            .or_else(|| v["data"]["transactionId"].as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| WebhookError::Malformed("missing event_id".into()))?
            .to_string();
        let kind = match v["type"]
            .as_str()
            .or_else(|| v["event"].as_str())
            .unwrap_or_default()
        {
            "payment.authorized" => EventKind::PaymentAuthorized,
            "payment.captured" | "PAYMENT_SUCCESS" => EventKind::PaymentCaptured,
            "payment.failed" | "PAYMENT_ERROR" => EventKind::PaymentFailed,
            "refund.processed" => EventKind::RefundProcessed,
            "refund.failed" => EventKind::RefundFailed,
            _ => EventKind::Other,
        };
        let text = |k: &str| v[k].as_str().map(str::to_string);
        Ok(WebhookEvent {
            event_id,
            kind,
            order_ref: text("attempt_id")
                .or_else(|| v["data"]["merchantTransactionId"].as_str().map(str::to_string)),
            provider_payment_ref: text("provider_payment_ref")
                .or_else(|| v["data"]["transactionId"].as_str().map(str::to_string)),
            provider_refund_ref: text("provider_refund_ref"),
            refund_ref: text("refund_id"),
            amount_minor: v["amount_minor"].as_i64().or_else(|| v["data"]["amount"].as_i64()),
            failure_reason: text("failure_reason"),
            raw: v,
        })
    }
}

async fn read_json(res: reqwest::Response) -> Result<Value, ProviderError> {
    let status = res.status();
    let v: Value = res
        .json()
        .await
        .map_err(|e| ProviderError::Uncertain(e.to_string()))?;
    if status.is_server_error() {
        return Err(ProviderError::Uncertain(format!("phonepe {status}")));
    }
    if !status.is_success() {
        return Err(ProviderError::Rejected(
            v["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .unwrap_or("rejected")
                .into(),
        ));
    }
    Ok(v)
}

fn classify(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() || e.is_connect() {
        ProviderError::Uncertain(e.to_string())
    } else {
        ProviderError::Rejected(e.to_string())
    }
}
