//! In-process provider for development and tests. Behaviour is chosen by the UPI handle:
//! `fail*` declines, `uncertain*` accepts the payment but answers with a timeout (outcome unknown),
//! `pending*` stays pending, anything else captures.
//!
//! Mandate debits are chosen by the amount's last two digits (paise): `...13` declines
//! (`insufficient_funds`), `...07` is accepted but answers with a timeout (outcome unknown), anything
//! else captures. A mandate must be approved first (by the payer; see `mandate_decide`).

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;
use zyvor_core::{Currency, new_id};

use crate::*;

#[derive(Default)]
pub struct SimProvider {
    mandates: Mutex<HashMap<String, SimMandate>>,
    payments: Mutex<HashMap<String, PaymentState>>,
    refunds: Mutex<HashMap<String, RefundState>>,
    /// When set, a pending QR / intent payment waits for `payer_decide` instead of being approved by the
    /// first lookup. This is what lets a payer app (or a test) play the payer.
    manual_payer: AtomicBool,
}

struct SimMandate {
    state: MandateState,
    max_amount_minor: i64,
    currency: Currency,
    /// Debits this simulated bank actually took: what the payer's own bank would show them.
    debits: Vec<PayerDebit>,
}

/// One debit the payer's bank took under a mandate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PayerDebit {
    pub attempt_id: String,
    pub amount_minor: i64,
    /// RFC 3339.
    pub at: String,
}

/// What a payer's UPI app can see of a mandate it approved. Note what is **not** here: upcoming debits. Zyvor
/// emits the pre-debit notice to the merchant; delivering it to the payer is the merchant's or provider's job.
#[derive(Debug, Clone, Serialize)]
pub struct PayerMandateView {
    pub mandate_id: String,
    pub status: String,
    pub max_amount_minor: i64,
    pub debits: Vec<PayerDebit>,
}

impl SimProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// The simulated payer approves or declines a pending mandate. `None` when there is no such mandate
    /// waiting (unknown, or already decided).
    pub fn mandate_decide(&self, mandate_id: &str, approve: bool) -> Option<MandateStatus> {
        let mut m = self.mandates.lock().unwrap();
        let s = m.get_mut(mandate_id)?;
        if s.state.status != MandateStatus::Pending {
            return None;
        }
        if approve {
            s.state.status = MandateStatus::Active;
        } else {
            s.state.status = MandateStatus::Failed;
            s.state.failure_reason = Some("declined_by_payer".into());
        }
        Some(s.state.status.clone())
    }

    /// What the payer's own bank would show about a mandate: its state and the debits it took. `None` when the
    /// simulator has never heard of it.
    pub fn payer_view(&self, mandate_id: &str) -> Option<PayerMandateView> {
        let m = self.mandates.lock().unwrap();
        let s = m.get(mandate_id)?;
        Some(PayerMandateView {
            mandate_id: mandate_id.to_string(),
            status: s.state.status.as_str().to_string(),
            max_amount_minor: s.max_amount_minor,
            debits: s.debits.clone(),
        })
    }

    /// The payer revokes a mandate from their own UPI app. `None` when there is nothing live to revoke.
    pub fn payer_revoke(&self, mandate_id: &str) -> Option<MandateStatus> {
        let mut m = self.mandates.lock().unwrap();
        let s = m.get_mut(mandate_id)?;
        if !matches!(
            s.state.status,
            MandateStatus::Pending | MandateStatus::Active | MandateStatus::Paused
        ) {
            return None;
        }
        s.state.status = MandateStatus::Revoked;
        Some(MandateStatus::Revoked)
    }

    pub fn set_manual_payer(&self, on: bool) {
        self.manual_payer.store(on, Ordering::Relaxed);
    }

    /// The simulated payer approves or declines a pending QR / intent payment. `None` when there is no
    /// such payment waiting (unknown, already decided, or not an intent flow).
    pub fn payer_decide(&self, attempt_id: &str, approve: bool) -> Option<PaymentStatus> {
        let mut m = self.payments.lock().unwrap();
        let s = m.get_mut(attempt_id)?;
        if s.status != PaymentStatus::Pending || s.raw["flow"] != "upi_intent" {
            return None;
        }
        if approve {
            s.status = PaymentStatus::Captured;
        } else {
            s.status = PaymentStatus::Failed;
            s.failure_reason = Some("declined_by_payer".into());
        }
        Some(s.status.clone())
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
            mandates: true,
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
                    apps: Default::default(),
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

    async fn create_mandate(&self, req: &CreateMandate) -> Result<MandateState, ProviderError> {
        let state = MandateState {
            status: MandateStatus::Pending,
            provider_mandate_ref: Some(new_id("simmnd")),
            authorization: Some(json!({
                "type": "upi_mandate",
                "uri": format!(
                    "upi://mandate?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am={}&cu={}&tn={}&recur={}&start={}&end={}",
                    zyvor_core::minor_to_decimal(req.max_amount_minor, req.currency),
                    req.currency.code(),
                    req.mandate_id,
                    req.frequency.as_str(),
                    req.start_date,
                    req.end_date
                ),
                "expires_at": null,
            })),
            failure_reason: None,
            raw: json!({ "sim": true, "max_amount_minor": req.max_amount_minor }),
        };
        self.mandates.lock().unwrap().insert(
            req.mandate_id.clone(),
            SimMandate {
                state: state.clone(),
                max_amount_minor: req.max_amount_minor,
                currency: req.currency,
                debits: Vec::new(),
            },
        );
        Ok(state)
    }

    async fn fetch_mandate(&self, mandate_id: &str) -> Result<MandateState, ProviderError> {
        let mut m = self.mandates.lock().unwrap();
        let s = m
            .get_mut(mandate_id)
            .ok_or_else(|| ProviderError::Rejected("not found".into()))?;
        // As with payments: without a manual payer the first lookup approves, so a sandbox run completes.
        if s.state.status == MandateStatus::Pending && !self.manual_payer.load(Ordering::Relaxed) {
            s.state.status = MandateStatus::Active;
        }
        Ok(s.state.clone())
    }

    async fn revoke_mandate(&self, mandate_id: &str) -> Result<MandateState, ProviderError> {
        let mut m = self.mandates.lock().unwrap();
        let s = m
            .get_mut(mandate_id)
            .ok_or_else(|| ProviderError::Rejected("not found".into()))?;
        s.state.status = MandateStatus::Revoked;
        Ok(s.state.clone())
    }

    async fn debit(&self, req: &DebitRequest) -> Result<Initiated, ProviderError> {
        {
            let m = self.mandates.lock().unwrap();
            let s = m
                .get(&req.mandate_id)
                .ok_or_else(|| ProviderError::Rejected("unknown mandate".into()))?;
            if s.state.status != MandateStatus::Active {
                return Err(ProviderError::Rejected("mandate_not_active".into()));
            }
            if req.currency != s.currency {
                return Err(ProviderError::Rejected("currency_mismatch".into()));
            }
            if req.amount_minor <= 0 || req.amount_minor > s.max_amount_minor {
                return Err(ProviderError::Rejected("amount_exceeds_mandate".into()));
            }
        }
        let last = req.amount_minor % 100;
        let (status, reason) = if last == 13 {
            (
                PaymentStatus::Failed,
                Some("insufficient_funds".to_string()),
            )
        } else {
            (PaymentStatus::Captured, None)
        };
        let state = PaymentState {
            status,
            provider_order_ref: Some(req.attempt_id.clone()),
            provider_payment_ref: Some(new_id("simdeb")),
            failure_reason: reason,
            raw: json!({ "sim": true, "flow": "mandate_debit", "mandate": req.mandate_id, "amount_minor": req.amount_minor }),
        };
        self.payments
            .lock()
            .unwrap()
            .insert(req.attempt_id.clone(), state.clone());
        if state.status == PaymentStatus::Captured
            && let Some(m) = self.mandates.lock().unwrap().get_mut(&req.mandate_id)
        {
            // the bank took it, whether or not the answer reaches the merchant (an unknown outcome included)
            m.debits.push(PayerDebit {
                attempt_id: req.attempt_id.clone(),
                amount_minor: req.amount_minor,
                at: chrono::Utc::now().to_rfc3339(),
            });
        }
        if last == 7 {
            return Err(ProviderError::Uncertain(
                "simulated timeout after acceptance".into(),
            ));
        }
        Ok(Initiated {
            state,
            action: ClientAction::None,
        })
    }

    async fn fetch_payment(&self, attempt_id: &str) -> Result<PaymentState, ProviderError> {
        let mut m = self.payments.lock().unwrap();
        let s = m
            .get_mut(attempt_id)
            .ok_or_else(|| ProviderError::Rejected("not found".into()))?;
        // A pending QR / intent payment is approved by the simulated payer on the first lookup, so a
        // sandbox run can complete without a real UPI app. A real adapter reports only what the partner says.
        if s.status == PaymentStatus::Pending
            && s.raw["flow"] == "upi_intent"
            && !self.manual_payer.load(Ordering::Relaxed)
        {
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

#[cfg(test)]
mod payer_tests {
    use super::*;

    fn intent_payment(id: &str) -> CreatePayment {
        CreatePayment {
            attempt_id: id.into(),
            amount_minor: 49900,
            currency: Currency::Inr,
            method: Method::Upi,
            vpa: None,
            customer_id: None,
            customer_phone: None,
            capture_automatic: true,
            return_url: None,
            upi_intent: true,
            client: None,
        }
    }

    #[tokio::test]
    async fn a_manual_payer_decides_and_a_lookup_does_not() {
        let sim = SimProvider::new();
        sim.set_manual_payer(true);
        sim.initiate(&intent_payment("att_a")).await.unwrap();
        assert_eq!(
            sim.fetch_payment("att_a").await.unwrap().status,
            PaymentStatus::Pending,
            "waiting for the payer, not approved by looking"
        );
        assert_eq!(
            sim.payer_decide("att_a", true),
            Some(PaymentStatus::Captured)
        );
        assert_eq!(sim.payer_decide("att_a", true), None, "decided once");
        assert_eq!(
            sim.fetch_payment("att_a").await.unwrap().status,
            PaymentStatus::Captured
        );

        sim.initiate(&intent_payment("att_b")).await.unwrap();
        assert_eq!(
            sim.payer_decide("att_b", false),
            Some(PaymentStatus::Failed)
        );
        assert_eq!(
            sim.fetch_payment("att_b")
                .await
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("declined_by_payer")
        );
        assert_eq!(sim.payer_decide("att_nope", true), None);
    }

    #[tokio::test]
    async fn without_a_manual_payer_the_first_lookup_approves_as_before() {
        let sim = SimProvider::new();
        sim.initiate(&intent_payment("att_c")).await.unwrap();
        assert_eq!(
            sim.fetch_payment("att_c").await.unwrap().status,
            PaymentStatus::Captured
        );
    }
}

#[cfg(test)]
mod mandate_tests {
    use super::*;
    use chrono::NaiveDate;

    fn new_mandate(id: &str, max: i64) -> CreateMandate {
        CreateMandate {
            mandate_id: id.into(),
            max_amount_minor: max,
            currency: Currency::Inr,
            frequency: Frequency::Monthly,
            start_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2027, 11, 1).unwrap(),
            customer_id: None,
            customer_phone: None,
        }
    }
    fn debit(att: &str, mandate: &str, pref: &str, amount: i64) -> DebitRequest {
        DebitRequest {
            attempt_id: att.into(),
            mandate_id: mandate.into(),
            provider_mandate_ref: pref.into(),
            amount_minor: amount,
            currency: Currency::Inr,
        }
    }

    #[tokio::test]
    async fn a_mandate_waits_for_the_payer_and_debits_only_once_active() {
        let sim = SimProvider::new();
        sim.set_manual_payer(true);
        let m = sim
            .create_mandate(&new_mandate("mnd_1", 100_000))
            .await
            .unwrap();
        assert_eq!(m.status, MandateStatus::Pending);
        let pref = m.provider_mandate_ref.unwrap();
        assert!(
            m.authorization.unwrap()["uri"]
                .as_str()
                .unwrap()
                .starts_with("upi://mandate?")
        );
        assert_eq!(
            sim.fetch_mandate("mnd_1").await.unwrap().status,
            MandateStatus::Pending,
            "looking does not approve"
        );
        assert!(
            matches!(sim.debit(&debit("att_1", "mnd_1", &pref, 50_000)).await, Err(ProviderError::Rejected(r)) if r == "mandate_not_active")
        );
        assert_eq!(
            sim.mandate_decide("mnd_1", true),
            Some(MandateStatus::Active)
        );
        assert_eq!(sim.mandate_decide("mnd_1", true), None, "decided once");
        let d = sim
            .debit(&debit("att_1", "mnd_1", &pref, 50_000))
            .await
            .unwrap();
        assert_eq!(d.state.status, PaymentStatus::Captured);
        assert_eq!(
            sim.fetch_payment("att_1").await.unwrap().status,
            PaymentStatus::Captured,
            "a lost reply can be resolved by asking"
        );
    }

    #[tokio::test]
    async fn limits_and_payer_revoke_are_enforced_by_the_provider_too() {
        let sim = SimProvider::new();
        sim.set_manual_payer(true);
        let pref = sim
            .create_mandate(&new_mandate("mnd_2", 100_000))
            .await
            .unwrap()
            .provider_mandate_ref
            .unwrap();
        sim.mandate_decide("mnd_2", true);
        assert!(
            matches!(sim.debit(&debit("a1", "mnd_2", &pref, 100_001)).await, Err(ProviderError::Rejected(r)) if r == "amount_exceeds_mandate")
        );
        assert!(matches!(
            sim.debit(&debit("a2", "mnd_2", &pref, 0)).await,
            Err(ProviderError::Rejected(_))
        ));
        let mut usd = debit("a3", "mnd_2", &pref, 100);
        usd.currency = Currency::Usd;
        assert!(
            matches!(sim.debit(&usd).await, Err(ProviderError::Rejected(r)) if r == "currency_mismatch")
        );
        assert!(matches!(
            sim.debit(&debit("a4", "nope", &pref, 100)).await,
            Err(ProviderError::Rejected(_))
        ));
        assert_eq!(sim.payer_revoke("mnd_2"), Some(MandateStatus::Revoked));
        assert_eq!(sim.payer_revoke("mnd_2"), None, "nothing live to revoke");
        assert!(
            matches!(sim.debit(&debit("a5", "mnd_2", &pref, 100)).await, Err(ProviderError::Rejected(r)) if r == "mandate_not_active")
        );
    }

    #[tokio::test]
    async fn the_amount_picks_a_decline_or_an_unknown_outcome() {
        let sim = SimProvider::new();
        sim.set_manual_payer(true);
        let pref = sim
            .create_mandate(&new_mandate("mnd_3", 1_000_000))
            .await
            .unwrap()
            .provider_mandate_ref
            .unwrap();
        sim.mandate_decide("mnd_3", true);
        let declined = sim
            .debit(&debit("b1", "mnd_3", &pref, 50_013))
            .await
            .unwrap();
        assert_eq!(declined.state.status, PaymentStatus::Failed);
        assert_eq!(
            declined.state.failure_reason.as_deref(),
            Some("insufficient_funds")
        );
        assert!(matches!(
            sim.debit(&debit("b2", "mnd_3", &pref, 50_007)).await,
            Err(ProviderError::Uncertain(_))
        ));
        assert_eq!(
            sim.fetch_payment("b2").await.unwrap().status,
            PaymentStatus::Captured,
            "it did happen; asking finds out"
        );
    }

    #[tokio::test]
    async fn without_a_manual_payer_the_first_lookup_activates_and_other_providers_refuse() {
        let sim = SimProvider::new();
        sim.create_mandate(&new_mandate("mnd_4", 1000))
            .await
            .unwrap();
        assert_eq!(
            sim.fetch_mandate("mnd_4").await.unwrap().status,
            MandateStatus::Active
        );
        assert_eq!(sim.mandate_decide("nope", true), None);
    }

    #[test]
    fn periods_end_on_calendar_boundaries() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(
            Frequency::Monthly.period_end(d(2026, 11, 17)),
            Some(d(2026, 12, 1))
        );
        assert_eq!(
            Frequency::Monthly.period_end(d(2026, 12, 31)),
            Some(d(2027, 1, 1))
        );
        assert_eq!(
            Frequency::Quarterly.period_end(d(2026, 11, 17)),
            Some(d(2027, 1, 1))
        );
        assert_eq!(
            Frequency::HalfYearly.period_end(d(2026, 7, 1)),
            Some(d(2027, 1, 1))
        );
        assert_eq!(
            Frequency::Yearly.period_end(d(2026, 12, 31)),
            Some(d(2027, 1, 1))
        );
        assert_eq!(
            Frequency::Bimonthly.period_end(d(2026, 1, 20)),
            Some(d(2026, 3, 1))
        );
        assert_eq!(
            Frequency::Weekly.period_end(d(2026, 11, 17)),
            Some(d(2026, 11, 24))
        );
        assert_eq!(Frequency::AsPresented.period_end(d(2026, 11, 17)), None);
        assert_eq!(
            "half_yearly".parse::<Frequency>(),
            Ok(Frequency::HalfYearly)
        );
        assert!("hourly".parse::<Frequency>().is_err());
    }

    #[tokio::test]
    async fn the_payers_view_lists_only_the_debits_the_bank_took() {
        let sim = SimProvider::new();
        sim.set_manual_payer(true);
        let pref = sim
            .create_mandate(&new_mandate("mnd_5", 1_000_000))
            .await
            .unwrap()
            .provider_mandate_ref
            .unwrap();
        assert_eq!(sim.payer_view("nope").map(|v| v.mandate_id), None);
        assert_eq!(
            sim.payer_view("mnd_5").unwrap().status,
            "pending_authorization"
        );
        assert!(sim.payer_view("mnd_5").unwrap().debits.is_empty());
        sim.mandate_decide("mnd_5", true);
        sim.debit(&debit("c1", "mnd_5", &pref, 50_000))
            .await
            .unwrap();
        let _ = sim.debit(&debit("c2", "mnd_5", &pref, 50_013)).await; // declined: the bank took nothing
        let _ = sim.debit(&debit("c3", "mnd_5", &pref, 50_007)).await; // timed out for the merchant, but the bank took it
        let v = sim.payer_view("mnd_5").unwrap();
        assert_eq!(v.status, "active");
        assert_eq!(
            v.debits
                .iter()
                .map(|d| (d.attempt_id.as_str(), d.amount_minor))
                .collect::<Vec<_>>(),
            vec![("c1", 50_000), ("c3", 50_007)]
        );
        sim.payer_revoke("mnd_5");
        assert_eq!(sim.payer_view("mnd_5").unwrap().status, "revoked");
        assert_eq!(
            sim.payer_view("mnd_5").unwrap().debits.len(),
            2,
            "history stays after a revoke"
        );
    }
}
