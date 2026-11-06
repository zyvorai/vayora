//! Sponsor-bank UPI adapter for acquire mode / TPAP path.
//!
//! Speaks the same HTTP contract as [`crate::custom::Custom`] against a sponsor PSP
//! (or a thin wrapper in front of NPCI). Capabilities default to UPI collect + intent/QR
//! and mandates. Verified against local stubs until a sponsor sandbox is connected.

use std::time::Duration;

use async_trait::async_trait;
use zyvor_core::Currency;

use crate::{
    Capabilities, CreatePayment, Headers, Initiated, Method, PaymentState, Provider, ProviderError,
    RefundRequest, RefundState, SettlementRecord, WebhookError, WebhookEvent, custom::Custom,
};

pub struct SponsorUpi {
    inner: Custom,
}

impl SponsorUpi {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, timeout: Duration) -> Self {
        let caps = Capabilities {
            methods: vec![Method::Upi],
            currencies: vec![Currency::Inr],
            partial_refund: true,
            manual_capture: false,
            server_initiated_upi: true,
            upi_intent: true,
            mandates: true,
            refund_window_days: Some(90),
        };
        Self {
            inner: Custom::new(base_url, api_key, timeout).with_capabilities(caps),
        }
    }
}

#[async_trait]
impl Provider for SponsorUpi {
    fn name(&self) -> &'static str {
        "sponsor_upi"
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        if req.method != Method::Upi {
            return Err(ProviderError::Unsupported(
                "sponsor_upi only accepts UPI".into(),
            ));
        }
        self.inner.initiate(req).await
    }

    async fn capture(
        &self,
        attempt_id: &str,
        provider_payment_ref: &str,
        amount_minor: i64,
        currency: Currency,
    ) -> Result<PaymentState, ProviderError> {
        self.inner
            .capture(attempt_id, provider_payment_ref, amount_minor, currency)
            .await
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        self.inner.fetch_payment(attempt_id).await
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        self.inner.refund(req).await
    }

    async fn fetch_refund(
        &self,
        attempt_id: &str,
        refund_id: &str,
        provider_payment_ref: &str,
    ) -> Result<RefundState, ProviderError> {
        self.inner
            .fetch_refund(attempt_id, refund_id, provider_payment_ref)
            .await
    }

    async fn fetch_settlements(
        &self,
        on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        self.inner.fetch_settlements(on).await
    }

    fn verify_webhook(
        &self,
        secret: &[u8],
        headers: &Headers,
        body: &[u8],
        now_unix: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        self.inner.verify_webhook(secret, headers, body, now_unix)
    }
}
