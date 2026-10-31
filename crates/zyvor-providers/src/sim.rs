//! In-process provider for development and tests. Behaviour is chosen by the UPI handle:
//! `fail*` declines, `uncertain*` accepts the payment but answers with a timeout (outcome unknown),
//! `pending*` stays pending, anything else captures.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use serde_json::json;
use zyvor_core::{Currency, new_id};

use crate::*;

#[derive(Default)]
pub struct SimProvider {
    payments: Mutex<HashMap<String, PaymentState>>,
    refunds: Mutex<HashMap<String, RefundState>>,
}

impl SimProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Provider for SimProvider {
    fn name(&self) -> &'static str {
        "sim"
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
            upi_intent: true,
            refund_window_days: None,
        }
    }

    async fn initiate(&self, req: &CreatePayment) -> Result<Initiated, ProviderError> {
        // UPI with no payer-typed UPI ID: issue a QR / intent payload, as a partner would. The payee is a
        // sandbox-only handle that no real UPI app can pay. The payment stays pending until the payer
        // "approves"; a lookup after that returns the final state (see `fetch_payment`).
        if req.method == Method::Upi && req.vpa.is_none() {
            let state = PaymentState {
                status: PaymentStatus::Pending,
                provider_order_ref: Some(req.attempt_id.clone()),
                provider_payment_ref: Some(new_id("simpay")),
                failure_reason: None,
                raw: json!({ "sim": true, "flow": "upi_intent", "amount_minor": req.amount_minor }),
            };
            self.payments
                .lock()
                .unwrap()
                .insert(req.attempt_id.clone(), state.clone());
            let uri = format!(
                "upi://pay?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am={}&cu={}&tn={}",
                zyvor_core::minor_to_decimal(req.amount_minor, req.currency),
                req.currency.code(),
                req.attempt_id
            );
            return Ok(Initiated {
                state,
                action: ClientAction::UpiIntent {
                    uri,
                    expires_at: None,
                },
            });
        }
        let handle = req
            .vpa
            .as_deref()
            .and_then(|v| v.split('@').next())
            .unwrap_or("");
        let (status, reason) = if handle.starts_with("fail") {
            (
                PaymentStatus::Failed,
                Some("insufficient_funds".to_string()),
            )
        } else if handle.starts_with("pending") {
            (PaymentStatus::Pending, None)
        } else if req.capture_automatic {
            (PaymentStatus::Captured, None)
        } else {
            (PaymentStatus::Authorized, None)
        };
        let state = PaymentState {
            status,
            provider_order_ref: Some(req.attempt_id.clone()),
            provider_payment_ref: Some(new_id("simpay")),
            failure_reason: reason,
            raw: json!({ "sim": true, "amount_minor": req.amount_minor }),
        };
        self.payments
            .lock()
            .unwrap()
            .insert(req.attempt_id.clone(), state.clone());
        if handle.starts_with("uncertain") {
            return Err(ProviderError::Uncertain(
                "simulated timeout after acceptance".into(),
            ));
        }
        Ok(Initiated {
            state,
            action: ClientAction::None,
        })
    }

    async fn capture(
        &self,
        attempt_id: &str,
        _ref: &str,
        _amount: i64,
        _ccy: Currency,
    ) -> Result<PaymentState, ProviderError> {
        let mut m = self.payments.lock().unwrap();
        let s = m
            .get_mut(attempt_id)
            .ok_or_else(|| ProviderError::Rejected("unknown payment".into()))?;
        s.status = PaymentStatus::Captured;
        Ok(s.clone())
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let mut m = self.payments.lock().unwrap();
        let s = m
            .get_mut(attempt_id)
            .ok_or_else(|| ProviderError::Rejected("not found".into()))?;
        // A pending QR / intent payment is approved by the simulated payer on the first lookup, so a
        // sandbox run can complete without a real UPI app. A real adapter reports only what the partner says.
        if s.status == PaymentStatus::Pending && s.raw["flow"] == "upi_intent" {
            s.status = PaymentStatus::Captured;
        }
        Ok(s.clone())
    }

    async fn refund(&self, req: &RefundRequest) -> Result<RefundState, ProviderError> {
        let st = RefundState {
            status: RefundStatus::Processed,
            provider_refund_ref: Some(new_id("simrf")),
            failure_reason: None,
            raw: json!({ "sim": true }),
        };
        self.refunds
            .lock()
            .unwrap()
            .insert(req.refund_id.clone(), st.clone());
        Ok(st)
    }

    async fn fetch_refund(
        &self,
        _attempt: &str,
        refund_id: &str,
        _payment: &str,
    ) -> Result<RefundState, ProviderError> {
        self.refunds
            .lock()
            .unwrap()
            .get(refund_id)
            .cloned()
            .ok_or_else(|| ProviderError::Rejected("not found".into()))
    }

    async fn fetch_settlements(
        &self,
        _on: chrono::NaiveDate,
    ) -> Result<Vec<SettlementRecord>, ProviderError> {
        Ok(vec![])
    }

    fn verify_webhook(
        &self,
        _secret: &[u8],
        _h: &Headers,
        _body: &[u8],
        _now: i64,
    ) -> Result<WebhookEvent, WebhookError> {
        Err(WebhookError::Malformed(
            "the simulator does not send webhooks".into(),
        ))
    }
}
