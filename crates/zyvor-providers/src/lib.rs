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
    /// Provider can hold a UPI AutoPay mandate (the payer approves once, the merchant debits later). Default
    /// off: turn it on only for a flow verified with that partner. The simulator has it; Cashfree has it, built from docs and not verified against its sandbox.
    #[serde(default)]
    pub mandates: bool,
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

/// Where the payer is, in the words partners ask for (Cashfree wants these on every UPI call, and assumes
/// the strictest flow when they are missing). Derived from the payer's own request, never typed by them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClientEnv {
    /// `mobile`, `tablet` or `desktop`
    pub device: String,
    /// `android`, `ios`, `windows`, `macos`, `linux` or `others`
    pub os: String,
    /// `mweb`, `webview` or `native` (meaningful on mobile)
    pub rendering: String,
    /// `chrome`, `safari`, `firefox`, `edge` or `others`
    pub browser: String,
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
    /// Ask for a UPI QR / intent instead of a hosted or collect flow (only sent to a provider that has `upi_intent`).
    pub upi_intent: bool,
    pub client: Option<ClientEnv>,
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
        /// Per-app links when the partner supplies them (for example `gpay`, `phonepe`, `bhim`, `paytm`).
        #[serde(default)]
        apps: BTreeMap<String, String>,
        expires_at: Option<String>,
    },
    /// A partner-rendered QR image (a `data:image/png;base64,...` URL) for desktop and counter screens.
    UpiQr {
        image: String,
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

// ---- UPI AutoPay mandates -------------------------------------------------------------------------------

/// How often a mandate may be debited. `AsPresented` means the merchant picks the dates (each still needs
/// its pre-debit notice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    Daily,
    Weekly,
    Fortnightly,
    Monthly,
    Bimonthly,
    Quarterly,
    HalfYearly,
    Yearly,
    AsPresented,
}

impl Frequency {
    pub fn as_str(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Weekly => "weekly",
            Frequency::Fortnightly => "fortnightly",
            Frequency::Monthly => "monthly",
            Frequency::Bimonthly => "bimonthly",
            Frequency::Quarterly => "quarterly",
            Frequency::HalfYearly => "half_yearly",
            Frequency::Yearly => "yearly",
            Frequency::AsPresented => "as_presented",
        }
    }
}

impl std::str::FromStr for Frequency {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "daily" => Frequency::Daily,
            "weekly" => Frequency::Weekly,
            "fortnightly" => Frequency::Fortnightly,
            "monthly" => Frequency::Monthly,
            "bimonthly" => Frequency::Bimonthly,
            "quarterly" => Frequency::Quarterly,
            "half_yearly" => Frequency::HalfYearly,
            "yearly" => Frequency::Yearly,
            "as_presented" => Frequency::AsPresented,
            o => return Err(format!("unknown frequency {o}")),
        })
    }
}

impl Frequency {
    /// The first date of the *next* period after `d`, or `None` for `AsPresented` (no fixed periods). Two
    /// debits may not fall in the same period: the period of `d` runs from the previous boundary up to,
    /// and not including, this date.
    pub fn period_end(self, d: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
        use chrono::{Datelike, Days, Months};
        Some(match self {
            Frequency::Daily => d.checked_add_days(Days::new(1))?,
            Frequency::Weekly => d.checked_add_days(Days::new(7))?,
            Frequency::Fortnightly => d.checked_add_days(Days::new(14))?,
            Frequency::Monthly => d.with_day(1)?.checked_add_months(Months::new(1))?,
            Frequency::Bimonthly => d.with_day(1)?.checked_add_months(Months::new(2))?,
            Frequency::Quarterly => {
                let m0 = (d.month0() / 3) * 3;
                d.with_day(1)?
                    .with_month0(m0)?
                    .checked_add_months(Months::new(3))?
            }
            Frequency::HalfYearly => {
                let m0 = (d.month0() / 6) * 6;
                d.with_day(1)?
                    .with_month0(m0)?
                    .checked_add_months(Months::new(6))?
            }
            Frequency::Yearly => d
                .with_day(1)?
                .with_month0(0)?
                .checked_add_months(Months::new(12))?,
            Frequency::AsPresented => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateStatus {
    /// Created; the payer has not approved it yet.
    Pending,
    Active,
    Paused,
    Revoked,
    Expired,
    Failed,
}

impl MandateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MandateStatus::Pending => "pending_authorization",
            MandateStatus::Active => "active",
            MandateStatus::Paused => "paused",
            MandateStatus::Revoked => "revoked",
            MandateStatus::Expired => "expired",
            MandateStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateMandate {
    /// Our own reference, so a lost response can be resolved by asking.
    pub mandate_id: String,
    /// The most a single debit may take, in minor units.
    pub max_amount_minor: i64,
    pub currency: Currency,
    pub frequency: Frequency,
    pub start_date: chrono::NaiveDate,
    pub end_date: chrono::NaiveDate,
    pub customer_id: Option<String>,
    pub customer_phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MandateState {
    pub status: MandateStatus,
    pub provider_mandate_ref: Option<String>,
    /// What the payer must do to approve it (for example a `upi://mandate?...` link). Shown, never edited.
    pub authorization: Option<Value>,
    pub failure_reason: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebitRequest {
    /// Our reference for this debit, which is also the payment attempt id.
    pub attempt_id: String,
    pub mandate_id: String,
    pub provider_mandate_ref: String,
    pub amount_minor: i64,
    pub currency: Currency,
}

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

    /// Create a UPI AutoPay mandate for the payer to approve. Providers without `mandates` refuse it.
    async fn create_mandate(&self, _req: &CreateMandate) -> Result<MandateState, ProviderError> {
        Err(ProviderError::Unsupported("UPI AutoPay mandates".into()))
    }
    /// Look a mandate up by our own reference (its id), so a lost response can be resolved.
    async fn fetch_mandate(&self, _mandate_id: &str) -> Result<MandateState, ProviderError> {
        Err(ProviderError::Unsupported("UPI AutoPay mandates".into()))
    }
    async fn revoke_mandate(&self, _mandate_id: &str) -> Result<MandateState, ProviderError> {
        Err(ProviderError::Unsupported("UPI AutoPay mandates".into()))
    }
    /// Debit an active mandate. The result is an ordinary payment state, so the same handling (including an
    /// uncertain outcome that is resolved by asking, never by re-sending) applies.
    async fn debit(&self, _req: &DebitRequest) -> Result<Initiated, ProviderError> {
        Err(ProviderError::Unsupported("UPI AutoPay debits".into()))
    }

    /// Look a mandate debit up by our attempt id and its mandate. A provider that can only find a debit
    /// through its mandate (Cashfree) needs the mandate id; the default ignores it.
    async fn fetch_debit(
        &self,
        attempt_id: &str,
        _mandate_id: &str,
    ) -> Result<PaymentState, ProviderError> {
        self.fetch_payment(attempt_id).await
    }

    /// Is the account reachable, and are its credentials accepted? Read-only: nothing is created. The
    /// string says what was found, for a person to read.
    async fn check(&self) -> Result<String, ProviderError> {
        Err(ProviderError::Unsupported("a connection check".into()))
    }

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

/// The outcome of a connection check from the status the provider answered with. `authenticated` lists the
/// statuses that mean "we got past authentication" (an expected "not found" counts).
pub(crate) fn check_status(
    status: reqwest::StatusCode,
    authenticated: &[u16],
) -> Result<String, ProviderError> {
    if authenticated.contains(&status.as_u16()) {
        Ok("the provider accepted the credentials".into())
    } else if status == reqwest::StatusCode::UNAUTHORIZED
        || status == reqwest::StatusCode::FORBIDDEN
    {
        Err(ProviderError::Rejected(format!(
            "the provider did not accept the credentials (HTTP {status})"
        )))
    } else {
        Err(ProviderError::Uncertain(format!(
            "unexpected answer from the provider (HTTP {status})"
        )))
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
