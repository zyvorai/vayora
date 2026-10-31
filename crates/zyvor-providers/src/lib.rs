//! Provider abstraction: what a payment provider can do, and how Zyvor talks to it.
//!
//! Money-moving calls return [`ProviderError::Uncertain`] when the outcome cannot be known
//! (timeout, 5xx, dropped connection after the request was sent). Callers must then treat the
//! operation as *unresolved* and never retry it through another provider.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zyvor_core::Currency;

pub mod cashfree;
pub mod custom;
pub mod razorpay;
pub mod router;
pub mod sim;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Upi,
    Card,
    Netbanking,
    Wallet,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Upi => "upi",
            Method::Card => "card",
            Method::Netbanking => "netbanking",
            Method::Wallet => "wallet",
        }
    }
}

impl std::str::FromStr for Method {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "upi" => Ok(Method::Upi),
            "card" => Ok(Method::Card),
            "netbanking" => Ok(Method::Netbanking),
            "wallet" => Ok(Method::Wallet),
            o => Err(format!("unknown method {o}")),
        }
    }
}

/// What a provider supports. Anything not listed here cannot be selected by the router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub methods: Vec<Method>,
    pub currencies: Vec<Currency>,
    pub partial_refund: bool,
    pub manual_capture: bool,
    /// Provider can start a UPI collect from the server (no hosted checkout needed).
    pub server_initiated_upi: bool,
    /// Provider issues a partner-generated UPI QR / intent payload for a payment, so the payer pays from
    /// any UPI app (BHIM, PhonePe, Google Pay...) without typing a UPI ID. Default off: turn it on only
    /// for a flow verified with that partner.
    #[serde(default)]
    pub upi_intent: bool,
    /// Refunds are refused after this many days.
    pub refund_window_days: Option<u32>,
}

impl Capabilities {
    pub fn supports(&self, method: Method, currency: Currency) -> Result<(), String> {
        if !self.methods.contains(&method) {
            return Err(format!("does not support {}", method.as_str()));
        }
        if !self.currencies.contains(&currency) {
            return Err(format!("does not support {currency}"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Pending,
    Authorized,
    Captured,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatePayment {
    pub attempt_id: String,
    pub amount_minor: i64,
    pub currency: Currency,
    pub method: Method,
    pub vpa: Option<String>,
    pub customer_id: Option<String>,
    pub customer_phone: Option<String>,
    pub capture_automatic: bool,
    pub return_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentState {
    pub status: PaymentStatus,
    pub provider_order_ref: Option<String>,
    pub provider_payment_ref: Option<String>,
    pub failure_reason: Option<String>,
    /// Provider's own response, kept so every normalised field can be traced back.
    pub raw: Value,
}

/// What the client must do next to complete the payment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientAction {
    None,
    /// Open the provider's hosted fields/checkout with these parameters. No card data touches Zyvor.
    HostedCheckout {
        params: Value,
    },
    /// Approve the collect request in the payer's UPI app.
    UpiCollect {
        vpa: String,
    },
    /// A partner-issued `upi://pay?...` payload. Show it as a QR on a desktop or counter screen and open
    /// it as an intent on a phone. Never edit it: partners may sign fields inside it. It is not proof of
    /// payment; only a verified provider result is.
    UpiIntent {
        uri: String,
        expires_at: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Initiated {
    pub state: PaymentState,
    pub action: ClientAction,
}

#[derive(Debug, Clone)]
pub struct RefundRequest {
    pub refund_id: String,
    pub attempt_id: String,
    pub provider_payment_ref: String,
    pub amount_minor: i64,
    pub currency: Currency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    Pending,
    Processed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundState {
    pub status: RefundStatus,
    pub provider_refund_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub raw: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// Definitively not done (4xx, validation). Safe to try elsewhere.
    #[error("provider rejected the request: {0}")]
    Rejected(String),
    /// Outcome unknown. Must be resolved by querying the provider, never by retrying elsewhere.
    #[error("provider outcome unknown: {0}")]
    Uncertain(String),
    #[error("not supported by this provider: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    PaymentAuthorized,
    PaymentCaptured,
    PaymentFailed,
    RefundProcessed,
    RefundFailed,
    Other,
}

/// Provider webhook, verified and normalised.
#[derive(Debug, Clone)]
pub struct WebhookEvent {
    /// Unique per provider; used for deduplication.
    pub event_id: String,
    pub kind: EventKind,
    /// Our attempt reference (provider order ref or our own id, depending on provider).
    pub order_ref: Option<String>,
    pub provider_payment_ref: Option<String>,
    pub provider_refund_ref: Option<String>,
    /// Our refund id, if the provider echoes it.
    pub refund_ref: Option<String>,
    pub amount_minor: Option<i64>,
    pub failure_reason: Option<String>,
    pub raw: Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebhookError {
    #[error("missing signature header")]
    MissingSignature,
    #[error("signature mismatch")]
    BadSignature,
    #[error("timestamp outside tolerance (possible replay)")]
    Stale,
    #[error("malformed payload: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementItemKind {
    Payment,
    Refund,
    Fee,
    Adjustment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementItem {
    pub kind: SettlementItemKind,
    pub provider_ref: Option<String>,
    /// Signed from the merchant's view: payments +, refunds -, fees -, adjustments +/-.
    pub amount_minor: i64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementRecord {
    pub provider_ref: String,
    pub utr: Option<String>,
    pub currency: Currency,
    pub settled_on: chrono::NaiveDate,
    pub net_minor: i64,
    pub items: Vec<SettlementItem>,
}

pub type Headers = BTreeMap<String, String>;

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError>;
    async fn capture(
        &self,
        attempt_id: &str,
        provider_payment_ref: &str,
        amount_minor: i64,
        currency: Currency,
    ) -> Result<PaymentState, ProviderError>;
    /// Look a payment up by our own reference (the attempt id), so even a lost response can be resolved.
    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError>;
    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError>;
    async fn fetch_refund(
        &self,
        attempt_id: &str,
        refund_id: &str,
        provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError>;
    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError>;

    /// Verify the signature and normalise. `headers` keys are lower-cased.
    fn verify_webhook(
        &self,
        secret: &[u8],
        headers: &Headers,
        body: &[u8],
        now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError>;
}

// ---------- shared HTTP helpers ----------

pub(crate) fn classify(e: reqwest::Error) -> ProviderError {
    if e.is_connect() {
        // The request never left: definitively not done.
        ProviderError::Rejected(format!("connection failed: {e}"))
    } else {
        ProviderError::Uncertain(e.to_string())
    }
}

pub(crate) async fn read_json(res: reqwest::Response) -> Result<Value, ProviderError> {
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(body)
    } else if status.is_server_error() {
        Err(ProviderError::Uncertain(format!("HTTP {status}")))
    } else {
        let msg = body
            .pointer("/error/description")
            .or_else(|| body.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("request rejected");
        Err(ProviderError::Rejected(format!("HTTP {status}: {msg}")))
    }
}

pub(crate) fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(key).expect("any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().to_vec()
}

pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
