// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! The reusable conformance harness. A [`Subject`] is one adapter wired to a simulated provider; every check
//! below takes a subject and asserts one rule of the provider contract. `main.rs` runs every check against every
//! adapter. Behaviour an adapter does differently on purpose is declared in its [`Expect`], where it shows up in
//! review and in `docs/zyvor/ADD_A_PROVIDER.md`, never as a silently skipped assertion.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use zyvor_core::Currency;
use zyvor_providers::*;

use crate::gateway::{Fault, Faults};

pub const RETURN_URL: &str = "https://merchant.example/return";

/// How the simulated payer finishes a payment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Captured,
    Failed,
}

/// One webhook delivery as the provider would send it.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub headers: Headers,
    pub body: Vec<u8>,
}

/// What the backend needs to know to complete a payment.
#[derive(Clone, Debug)]
pub struct Paid {
    pub attempt_id: String,
    pub order_ref: Option<String>,
    pub action: String, // the ClientAction as JSON, for backends that must read the checkout form
}

#[derive(Clone, Debug)]
pub struct RefundCtx {
    pub refund_id: String,
    pub provider_refund_ref: Option<String>,
    pub payment_ref: String,
}

/// The simulated provider's side of the conversation.
#[async_trait]
pub trait Backend: Send + Sync {
    /// The payer finishes the payment. Returns the webhook the provider would send, when it sends one.
    async fn pay(&self, p: &Paid, outcome: Outcome) -> Option<Delivery>;
    /// The UPI ID that makes an in-process provider decide `outcome` at initiation time.
    fn vpa(&self, _outcome: Outcome) -> String {
        "asha@okzyvor".into()
    }
    /// An in-process provider that can accept a payment and then answer with a timeout.
    fn uncertain_vpa(&self) -> Option<String> {
        None
    }
    /// The provider finishes a refund it had queued.
    async fn complete_refund(&self, _r: &RefundCtx) {}
    /// The provider's refund webhook, when it sends one.
    async fn refund_event(&self, _r: &RefundCtx) -> Option<Delivery> {
        None
    }
    /// The provider settles what is captured and refunded (fee 200 bps, tax 1800 bps on the fee) with a signed
    /// `adjustment_minor`. `false` when this provider has no settlement simulation.
    async fn settle(&self, _adjustment_minor: i64) -> bool {
        false
    }
    /// The same delivery with its signature removed.
    fn strip_signature(&self, d: &Delivery) -> Delivery {
        let headers = d
            .headers
            .iter()
            .filter(|(k, _)| !(k.contains("signature") || k.as_str() == "x-verify"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Delivery {
            headers,
            body: d.body.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlements {
    /// Reports parseable settlements.
    Parsed,
    /// Says plainly that it cannot (`Unsupported`).
    Unsupported,
    /// Answers with no settlements, always (the in-process simulator).
    Empty,
}

/// What an adapter reports for a payment or refund reference the provider has never seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// `Failed` with reason `not_found`: a definite "never happened".
    FailedNotFound,
    /// `Pending`: not a failure, because the reference may simply not exist yet (PayU hosted checkout).
    Pending,
    /// `Err(Uncertain)`: not found yet, and not safe to call a failure.
    Uncertain,
    /// `Err(Rejected)`.
    Rejected,
}

/// Documented, deliberate behaviour. Anything an adapter does differently from the default rule is stated here.
#[derive(Clone, Debug)]
pub struct Expect {
    /// The adapter verifies webhooks at all.
    pub webhooks: bool,
    /// The webhook carries a timestamp and an old one is refused (`Stale`). Otherwise replays are stopped by
    /// the event id alone.
    pub replay_window: bool,
    /// The provider's refund webhook is signed and can be verified.
    pub refund_events: bool,
    /// The provider's refund callback carries no documented signature and the adapter refuses it.
    pub unsigned_refund_events: bool,
    /// Sending the same refund id twice returns the first refund and does not refund twice.
    pub refund_idempotent: bool,
    pub settlements: Settlements,
    pub unknown_payment: Unknown,
    pub unknown_refund: Unknown,
    /// `initiate` builds a form for the payer's browser and makes no call to the provider.
    pub initiate_is_local: bool,
    /// The provider refuses a refund above what was captured, or of a payment that was not captured.
    pub refund_bounds_enforced: bool,
    /// The outcome is known as soon as `initiate` returns (an in-process provider).
    pub immediate: bool,
    /// Initiating the same attempt twice creates a second provider order (Razorpay).
    pub reinitiate_new_order: bool,
}

impl Default for Expect {
    fn default() -> Self {
        Self {
            webhooks: true,
            replay_window: true,
            refund_events: true,
            unsigned_refund_events: false,
            refund_idempotent: true,
            settlements: Settlements::Parsed,
            unknown_payment: Unknown::FailedNotFound,
            unknown_refund: Unknown::FailedNotFound,
            initiate_is_local: false,
            refund_bounds_enforced: true,
            immediate: false,
            reinitiate_new_order: false,
        }
    }
}

pub struct Subject {
    pub name: &'static str,
    pub provider: Arc<dyn Provider>,
    /// The same adapter pointed at an address nothing listens on (HTTP adapters only).
    pub unreachable: Option<Arc<dyn Provider>>,
    /// The same adapter with credentials that are wrong, for providers whose webhook secret is in the adapter.
    pub wrong_credentials: Option<Arc<dyn Provider>>,
    pub backend: Box<dyn Backend>,
    pub faults: Option<Faults>,
    pub webhook_secret: Vec<u8>,
    /// Values that must never appear in anything handed to a payer's device.
    pub secrets: Vec<String>,
    pub expect: Expect,
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn unique(prefix: &str) -> String {
    format!(
        "{prefix}_{:08}",
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Subject {
    pub fn caps(&self) -> Capabilities {
        self.provider.capabilities()
    }

    /// The ordinary payment request for this adapter: a server-sent UPI collect when it can do one, otherwise the
    /// first method it declares (a hosted checkout).
    pub fn request(&self, amount: i64, outcome: Outcome) -> CreatePayment {
        let caps = self.caps();
        let (method, vpa) = if caps.server_initiated_upi && caps.methods.contains(&Method::Upi) {
            (Method::Upi, Some(self.backend.vpa(outcome)))
        } else {
            (caps.methods[0], None)
        };
        CreatePayment {
            attempt_id: unique("att"),
            amount_minor: amount,
            currency: Currency::Inr,
            method,
            vpa,
            customer_id: None,
            customer_phone: None,
            capture_automatic: true,
            return_url: Some(RETURN_URL.into()),
            upi_intent: false,
            client: None,
            network_token_ref: None,
        }
    }

    pub fn paid(&self, req: &CreatePayment, init: &Initiated) -> Paid {
        Paid {
            attempt_id: req.attempt_id.clone(),
            order_ref: init.state.provider_order_ref.clone(),
            action: serde_json::to_string(&init.action).unwrap(),
        }
    }

    /// Initiate and have the payer complete it. Returns the request, the result and the provider's webhook.
    pub async fn pay(
        &self,
        amount: i64,
        outcome: Outcome,
    ) -> (CreatePayment, Initiated, Option<Delivery>) {
        let req = self.request(amount, outcome);
        let init = self
            .provider
            .initiate(&req)
            .await
            .unwrap_or_else(|e| panic!("{}: initiate: {e}", self.name));
        let hook = self.backend.pay(&self.paid(&req, &init), outcome).await;
        (req, init, hook)
    }

    fn verify(&self, d: &Delivery, at: i64) -> Result<WebhookEvent, WebhookError> {
        self.provider
            .verify_webhook(&self.webhook_secret, &d.headers, &d.body, at)
    }

    /// Run `f` with `fault` switched on, then switch it off again.
    async fn with_fault<T>(&self, fault: Fault, f: impl std::future::Future<Output = T>) -> T {
        let faults = self
            .faults
            .as_ref()
            .expect("this subject has no fault proxy");
        faults.set(fault);
        let out = f.await;
        faults.clear();
        out
    }
}

fn is_rejected<T: std::fmt::Debug>(r: &Result<T, ProviderError>) -> bool {
    matches!(r, Err(ProviderError::Rejected(_)))
}
fn is_uncertain<T: std::fmt::Debug>(r: &Result<T, ProviderError>) -> bool {
    matches!(r, Err(ProviderError::Uncertain(_)))
}

fn paid_state(s: &PaymentStatus) -> bool {
    matches!(s, PaymentStatus::Captured)
}

// ====================================================================================================== checks

/// An adapter must not claim a method, currency or flow it cannot do, and must refuse what it did not claim.
pub async fn capabilities_are_honest(s: &Subject) {
    let caps = s.caps();
    assert!(!caps.methods.is_empty(), "{}: declares no method", s.name);
    assert!(
        caps.currencies.contains(&Currency::Inr),
        "{}: no INR",
        s.name
    );

    // refuses, without calling the provider, a method it did not declare
    for m in [
        Method::Upi,
        Method::Card,
        Method::Netbanking,
        Method::Wallet,
    ] {
        if !caps.methods.contains(&m) {
            let mut r = s.request(10_000, Outcome::Captured);
            r.method = m;
            let out = s.provider.initiate(&r).await;
            assert!(
                matches!(
                    out,
                    Err(ProviderError::Unsupported(_) | ProviderError::Rejected(_))
                ),
                "{}: accepted undeclared method {m:?}: {out:?}",
                s.name
            );
        }
    }
    // ... and a currency it did not declare
    for c in [Currency::Inr, Currency::Usd] {
        if !caps.currencies.contains(&c) {
            let mut r = s.request(10_000, Outcome::Captured);
            r.currency = c;
            let out = s.provider.initiate(&r).await;
            assert!(
                matches!(
                    out,
                    Err(ProviderError::Unsupported(_) | ProviderError::Rejected(_))
                ),
                "{}: accepted undeclared currency {c}: {out:?}",
                s.name
            );
        }
    }
    // ... and an amount that is not money
    for bad in [0, -100] {
        let out = s
            .provider
            .initiate(&s.request(bad, Outcome::Captured))
            .await;
        assert!(
            matches!(
                out,
                Err(ProviderError::Unsupported(_) | ProviderError::Rejected(_))
            ),
            "{}: accepted an amount of {bad}: {out:?}",
            s.name
        );
    }

    // flags it did not claim must really be off
    if !caps.mandates {
        let m = CreateMandate {
            mandate_id: unique("mnd"),
            max_amount_minor: 10_000,
            currency: Currency::Inr,
            frequency: Frequency::Monthly,
            start_date: chrono::Utc::now().date_naive(),
            end_date: chrono::Utc::now().date_naive() + chrono::Days::new(365),
            customer_id: None,
            customer_phone: None,
        };
        let out = s.provider.create_mandate(&m).await;
        assert!(
            matches!(out, Err(ProviderError::Unsupported(_))),
            "{}: mandates not declared but {out:?}",
            s.name
        );
    }

    // a UPI request: the flow it gets back must match what it declared
    if caps.methods.contains(&Method::Upi) {
        let mut r = s.request(10_000, Outcome::Captured);
        r.method = Method::Upi;
        r.vpa = Some(s.backend.vpa(Outcome::Captured));
        let init = s
            .provider
            .initiate(&r)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", s.name));
        match (&init.action, caps.server_initiated_upi) {
            (ClientAction::HostedCheckout { .. }, true) => {
                panic!(
                    "{}: declares server-initiated UPI but sent the payer to a hosted page",
                    s.name
                )
            }
            (ClientAction::UpiCollect { .. }, false) => {
                panic!(
                    "{}: does not declare server-initiated UPI but reports a collect request",
                    s.name
                )
            }
            _ => {}
        }
        // intent / QR, when asked for and not declared, must not be pretended
        let mut r = s.request(10_000, Outcome::Captured);
        r.method = Method::Upi;
        r.vpa = None;
        r.upi_intent = true;
        let out = s
            .provider
            .initiate(&r)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", s.name));
        let intent = matches!(
            out.action,
            ClientAction::UpiIntent { .. } | ClientAction::UpiQr { .. }
        );
        assert_eq!(
            intent, caps.upi_intent,
            "{}: upi_intent is declared as {} but the intent request answered {:?}",
            s.name, caps.upi_intent, out.action
        );
    }

    // manual capture not declared: capture must not move any money
    if !caps.manual_capture {
        let (req, _init, _) = s.pay(20_000, Outcome::Captured).await;
        let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
        let out = s
            .provider
            .capture(
                &req.attempt_id,
                st.provider_payment_ref.as_deref().unwrap_or(""),
                20_000,
                Currency::Inr,
            )
            .await;
        match out {
            Err(ProviderError::Unsupported(_)) => {}
            Ok(c) => assert_eq!(
                c.status, st.status,
                "{}: manual capture not declared but capture changed the payment",
                s.name
            ),
            other => panic!("{}: unexpected capture answer {other:?}", s.name),
        }
    }
}

/// Create a charge, follow it to the end and read it back: lookup, refs, amount and the webhook all agree.
pub async fn captured_payment_round_trip(s: &Subject) {
    let req = s.request(49_900, Outcome::Captured);
    let init = s
        .provider
        .initiate(&req)
        .await
        .unwrap_or_else(|e| panic!("{}: initiate: {e}", s.name));
    assert!(
        init.state
            .provider_order_ref
            .as_deref()
            .is_some_and(|r| !r.is_empty()),
        "{}: no order reference",
        s.name
    );
    assert_ne!(
        init.state.status,
        PaymentStatus::Failed,
        "{}: happy path failed at initiate",
        s.name
    );
    assert!(
        !init.state.raw.is_null(),
        "{}: the provider's own answer is not kept",
        s.name
    );
    if !s.expect.immediate {
        let before = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
        assert_eq!(
            before.status,
            PaymentStatus::Pending,
            "{}: unpaid payment is not pending",
            s.name
        );
    }
    let hook = s.backend.pay(&s.paid(&req, &init), Outcome::Captured).await;
    let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
    assert_eq!(
        st.status,
        PaymentStatus::Captured,
        "{}: lookup after payment",
        s.name
    );
    assert!(
        st.provider_payment_ref
            .as_deref()
            .is_some_and(|r| !r.is_empty()),
        "{}: no payment reference",
        s.name
    );
    if let (true, Some(d)) = (s.expect.webhooks, hook) {
        let ev = s
            .verify(&d, now())
            .unwrap_or_else(|e| panic!("{}: webhook: {e}", s.name));
        assert_eq!(ev.kind, EventKind::PaymentCaptured, "{}", s.name);
        assert!(!ev.event_id.is_empty(), "{}: no event id", s.name);
        assert_eq!(ev.amount_minor, Some(49_900), "{}: webhook amount", s.name);
        let order = init.state.provider_order_ref.clone().unwrap();
        assert!(
            ev.order_ref.as_deref() == Some(order.as_str())
                || ev.order_ref.as_deref() == Some(req.attempt_id.as_str()),
            "{}: webhook order ref {:?} is neither the order {order} nor the attempt",
            s.name,
            ev.order_ref
        );
        if let (Some(a), Some(b)) = (&ev.provider_payment_ref, &st.provider_payment_ref) {
            assert_eq!(
                a, b,
                "{}: webhook and lookup disagree on the payment reference",
                s.name
            );
        }
    }
}

pub async fn declined_payment_is_failed(s: &Subject) {
    let (req, _init, hook) = s.pay(30_000, Outcome::Failed).await;
    let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
    assert_eq!(
        st.status,
        PaymentStatus::Failed,
        "{}: a declined payment must read as failed",
        s.name
    );
    if let (true, Some(d)) = (s.expect.webhooks, hook) {
        let ev = s
            .verify(&d, now())
            .unwrap_or_else(|e| panic!("{}: webhook: {e}", s.name));
        assert_eq!(ev.kind, EventKind::PaymentFailed, "{}", s.name);
    }
}

/// A definite refusal is `Rejected`; anything that may have happened is `Uncertain`; a refused connection never
/// reached the provider so it is a definite no; an unreadable answer is never read as success or failure.
pub async fn definite_failure_vs_uncertain_outcome(s: &Subject) {
    // the connection is refused: nothing was sent
    if let Some(nowhere) = &s.unreachable {
        let r = nowhere.fetch_payment("att_x").await;
        assert!(
            is_rejected(&r),
            "{}: a refused connection must be Rejected, got {r:?}",
            s.name
        );
        if !s.expect.initiate_is_local {
            let out = nowhere
                .initiate(&s.request(10_000, Outcome::Captured))
                .await;
            assert!(
                is_rejected(&out),
                "{}: initiate over a refused connection: {out:?}",
                s.name
            );
        }
    }
    let Some(_) = &s.faults else {
        // in-process provider: the one way to be unsure is accepting a payment and timing out
        let vpa = s
            .backend
            .uncertain_vpa()
            .expect("an in-process subject needs an uncertain handle");
        let mut r = s.request(10_000, Outcome::Captured);
        r.vpa = Some(vpa);
        let out = s.provider.initiate(&r).await;
        assert!(
            is_uncertain(&out),
            "{}: timeout after acceptance must be Uncertain, got {out:?}",
            s.name
        );
        return;
    };

    // the operations to try: creating a payment (when it calls the provider) and looking one up
    let probe = s.request(10_000, Outcome::Captured);
    for (fault, want) in [
        (Fault::Status(400), "rejected"),
        (Fault::Status(422), "rejected"),
        (Fault::Status(503), "uncertain"),
        (Fault::Status(500), "uncertain"),
        (Fault::Hang, "uncertain"),
        (Fault::Garbled, "uncertain"),
    ] {
        if !s.expect.initiate_is_local {
            let out = s.with_fault(fault, s.provider.initiate(&probe)).await;
            check_mapping(s, "initiate", fault, want, &out);
        }
        let out = s
            .with_fault(fault, s.provider.fetch_payment(&probe.attempt_id))
            .await;
        check_mapping(s, "fetch_payment", fault, want, &out);
    }
}

fn check_mapping<T: std::fmt::Debug>(
    s: &Subject,
    op: &str,
    fault: Fault,
    want: &str,
    out: &Result<T, ProviderError>,
) {
    let ok = match want {
        "rejected" => is_rejected(out),
        _ => is_uncertain(out),
    };
    assert!(
        ok,
        "{}: {op} under {fault:?} must be {want}, got {out:?}",
        s.name
    );
}

/// The response to a call that the provider acted on is lost. The adapter reports Uncertain, and a lookup by our
/// own reference then finds exactly the payment that exists (never "not found", never a second one).
pub async fn lost_response_is_resolved_by_lookup(s: &Subject) {
    if let Some(vpa) = s.backend.uncertain_vpa() {
        let mut r = s.request(10_000, Outcome::Captured);
        r.vpa = Some(vpa);
        let out = s.provider.initiate(&r).await;
        assert!(is_uncertain(&out), "{}: {out:?}", s.name);
        let st = s.provider.fetch_payment(&r.attempt_id).await.unwrap();
        assert_ne!(
            st.failure_reason.as_deref(),
            Some("not_found"),
            "{}: the payment exists",
            s.name
        );
        return;
    }
    if s.expect.initiate_is_local {
        return; // nothing is sent, so no response can be lost
    }
    let req = s.request(10_000, Outcome::Captured);
    let out = s
        .with_fault(Fault::LoseResponse, s.provider.initiate(&req))
        .await;
    assert!(
        is_uncertain(&out),
        "{}: a lost response must be Uncertain, got {out:?}",
        s.name
    );
    let st = s
        .provider
        .fetch_payment(&req.attempt_id)
        .await
        .unwrap_or_else(|e| panic!("{}: lookup after a lost response: {e}", s.name));
    assert_ne!(
        st.failure_reason.as_deref(),
        Some("not_found"),
        "{}: the provider acted, so the lookup must find the payment",
        s.name
    );
    assert!(
        matches!(
            st.status,
            PaymentStatus::Pending | PaymentStatus::Authorized | PaymentStatus::Captured
        ),
        "{}: lookup after a lost response gave {:?}",
        s.name,
        st.status
    );
}

/// Initiating the same attempt twice is either answered with the same payment or refused as a duplicate. It never
/// leaves a payment the lookup cannot find.
pub async fn initiate_twice_is_safe(s: &Subject) {
    let req = s.request(25_000, Outcome::Captured);
    let first = s
        .provider
        .initiate(&req)
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", s.name));
    let second = s.provider.initiate(&req).await;
    let latest = match second {
        Ok(second) => {
            if !s.expect.reinitiate_new_order {
                assert_eq!(
                    second.state.provider_order_ref, first.state.provider_order_ref,
                    "{}: a second initiate for the same attempt made a different order",
                    s.name
                );
            }
            second
        }
        Err(ProviderError::Rejected(_)) => first,
        Err(e) => panic!(
            "{}: a duplicate initiate must be answered or Rejected, got {e:?}",
            s.name
        ),
    };
    // money paid on whichever order the payer was handed is found by the attempt id
    s.backend
        .pay(&s.paid(&req, &latest), Outcome::Captured)
        .await;
    let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
    assert!(
        paid_state(&st.status),
        "{}: duplicate initiate hid a payment: {:?}",
        s.name,
        st.status
    );
}

/// Several amounts through the whole path: what we send is what the provider records and what it reports back.
pub async fn amounts_round_trip_in_paise(s: &Subject) {
    for amount in [100, 101, 12_345, 123_456_789, 999_999_999_999] {
        let (req, _init, hook) = s.pay(amount, Outcome::Captured).await;
        assert_eq!(
            s.provider
                .fetch_payment(&req.attempt_id)
                .await
                .unwrap()
                .status,
            PaymentStatus::Captured,
            "{}: {amount}",
            s.name
        );
        if let (true, Some(d)) = (s.expect.webhooks, hook) {
            let ev = s.verify(&d, now()).unwrap();
            assert_eq!(
                ev.amount_minor,
                Some(amount),
                "{}: {amount} paise did not round-trip",
                s.name
            );
        }
    }
    // every currency it declares is accepted
    for c in s.caps().currencies {
        let mut r = s.request(10_000, Outcome::Captured);
        r.currency = c;
        s.provider
            .initiate(&r)
            .await
            .unwrap_or_else(|e| panic!("{}: declared {c} but refused it: {e}", s.name));
    }
}

/// Refunds: partial then partial then full, an over-refund is a definite no, a repeat of a refund id refunds once,
/// and the refund can be looked up by our id.
pub async fn refunds_full_and_partial(s: &Subject) {
    let (req, _init, _) = s.pay(50_000, Outcome::Captured).await;
    let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
    let pref = st.provider_payment_ref.clone().unwrap();
    let rq = |id: &str, amount: i64| RefundRequest {
        refund_id: id.into(),
        attempt_id: req.attempt_id.clone(),
        provider_payment_ref: pref.clone(),
        amount_minor: amount,
        currency: Currency::Inr,
    };
    let mut made = Vec::new();
    for (amount, label) in [(10_000, "partial"), (25_000, "partial")] {
        let id = unique("rfd");
        let r = s
            .provider
            .refund(&rq(&id, amount))
            .await
            .unwrap_or_else(|e| panic!("{}: {label} refund: {e}", s.name));
        assert!(
            matches!(r.status, RefundStatus::Pending | RefundStatus::Processed),
            "{}: {label} refund {:?}",
            s.name,
            r.status
        );
        assert!(
            r.provider_refund_ref
                .as_deref()
                .is_some_and(|x| !x.is_empty()),
            "{}: no refund reference",
            s.name
        );
        made.push((id, r));
    }
    if s.expect.refund_bounds_enforced {
        let over = s.provider.refund(&rq(&unique("rfd"), 15_001)).await;
        assert!(
            is_rejected(&over),
            "{}: refunding more than what is left must be Rejected, got {over:?}",
            s.name
        );
    }
    let id = unique("rfd");
    let last = s
        .provider
        .refund(&rq(&id, 15_000))
        .await
        .unwrap_or_else(|e| panic!("{}: full refund of the rest: {e}", s.name));
    made.push((id, last));
    // a repeat of a refund id
    if s.expect.refund_idempotent {
        let (id, first) = &made[0];
        let again = s
            .provider
            .refund(&rq(id, 10_000))
            .await
            .unwrap_or_else(|e| panic!("{}: repeated refund id: {e}", s.name));
        assert_eq!(
            again.provider_refund_ref, first.provider_refund_ref,
            "{}: a repeated refund id made a new refund",
            s.name
        );
        if s.expect.refund_bounds_enforced {
            // ... and nothing was double counted: there is nothing left to refund
            let out = s.provider.refund(&rq(&unique("rfd"), 100)).await;
            assert!(
                is_rejected(&out),
                "{}: everything is refunded yet a further refund was accepted: {out:?}",
                s.name
            );
        }
    }
    // each can be looked up by our own id, and ends processed
    for (id, r) in &made {
        s.backend
            .complete_refund(&RefundCtx {
                refund_id: id.clone(),
                provider_refund_ref: r.provider_refund_ref.clone(),
                payment_ref: pref.clone(),
            })
            .await;
        let f = s
            .provider
            .fetch_refund(&req.attempt_id, id, &pref)
            .await
            .unwrap_or_else(|e| panic!("{}: fetch_refund: {e}", s.name));
        assert_eq!(
            f.status,
            RefundStatus::Processed,
            "{}: refund {id} after the provider finished it",
            s.name
        );
        assert_eq!(
            f.provider_refund_ref, r.provider_refund_ref,
            "{}: refund reference differs between create and lookup",
            s.name
        );
    }
    // a refund of a payment that never succeeded is never processed
    if s.expect.refund_bounds_enforced {
        let unpaid = s.request(10_000, Outcome::Captured);
        let init = s.provider.initiate(&unpaid).await;
        let out = s
            .provider
            .refund(&RefundRequest {
                refund_id: unique("rfd"),
                attempt_id: unpaid.attempt_id.clone(),
                provider_payment_ref: init
                    .ok()
                    .and_then(|i| i.state.provider_payment_ref)
                    .unwrap_or_else(|| "never_paid".into()),
                amount_minor: 5_000,
                currency: Currency::Inr,
            })
            .await;
        assert!(
            !matches!(
                out,
                Ok(RefundState {
                    status: RefundStatus::Processed,
                    ..
                })
            ),
            "{}: refunded a payment nobody paid",
            s.name
        );
    }
}

/// A refund whose response is lost is found again by its id, once, and the provider holds exactly one refund.
pub async fn lost_refund_response_is_resolved_by_lookup(s: &Subject) {
    let Some(_) = &s.faults else { return };
    let (req, _init, _) = s.pay(50_000, Outcome::Captured).await;
    let pref = s
        .provider
        .fetch_payment(&req.attempt_id)
        .await
        .unwrap()
        .provider_payment_ref
        .unwrap();
    let rq = |id: &str, amount: i64| RefundRequest {
        refund_id: id.into(),
        attempt_id: req.attempt_id.clone(),
        provider_payment_ref: pref.clone(),
        amount_minor: amount,
        currency: Currency::Inr,
    };
    let id = unique("rfd");
    let out = s
        .with_fault(Fault::Status(503), s.provider.refund(&rq(&id, 10_000)))
        .await;
    assert!(
        is_uncertain(&out),
        "{}: refund under a 5xx must be Uncertain: {out:?}",
        s.name
    );
    let out = s
        .with_fault(Fault::Status(400), s.provider.refund(&rq(&id, 10_000)))
        .await;
    assert!(
        is_rejected(&out),
        "{}: refund under a 4xx must be Rejected: {out:?}",
        s.name
    );

    // never seen: reported as the adapter declared
    let ghost = s
        .provider
        .fetch_refund(&req.attempt_id, &unique("rfd"), &pref)
        .await;
    match (s.expect.unknown_refund, &ghost) {
        (Unknown::FailedNotFound, Ok(f)) => {
            assert_eq!(
                (f.status.clone(), f.failure_reason.as_deref()),
                (RefundStatus::Failed, Some("not_found")),
                "{}",
                s.name
            )
        }
        (Unknown::Uncertain, Err(ProviderError::Uncertain(_)))
        | (Unknown::Rejected, Err(ProviderError::Rejected(_))) => {}
        other => panic!("{}: unknown refund id gave {other:?}", s.name),
    }

    // the response is lost after the provider acted
    let id = unique("rfd");
    let out = s
        .with_fault(Fault::LoseResponse, s.provider.refund(&rq(&id, 10_000)))
        .await;
    assert!(is_uncertain(&out), "{}: {out:?}", s.name);
    let f = s
        .provider
        .fetch_refund(&req.attempt_id, &id, &pref)
        .await
        .unwrap_or_else(|e| panic!("{}: lookup of a lost refund: {e}", s.name));
    assert!(
        matches!(f.status, RefundStatus::Pending | RefundStatus::Processed),
        "{}: the refund exists, lookup said {:?}",
        s.name,
        f.status
    );
    if s.expect.refund_bounds_enforced {
        // exactly one refund of 10_000 exists: 40_000 is left, so 40_001 is refused and 40_000 is not
        let over = s.provider.refund(&rq(&unique("rfd"), 40_001)).await;
        assert!(
            is_rejected(&over),
            "{}: more than one refund seems to exist: {over:?}",
            s.name
        );
    }
}

/// What the adapter says about a reference the provider has never seen is exactly what it declared.
pub async fn unknown_payment_reference(s: &Subject) {
    let out = s.provider.fetch_payment(&unique("never_sent")).await;
    match (s.expect.unknown_payment, &out) {
        (Unknown::FailedNotFound, Ok(st)) => {
            assert_eq!(st.status, PaymentStatus::Failed, "{}", s.name);
            assert_eq!(
                st.failure_reason.as_deref(),
                Some("not_found"),
                "{}",
                s.name
            );
        }
        (Unknown::Pending, Ok(st)) => assert_eq!(st.status, PaymentStatus::Pending, "{}", s.name),
        (Unknown::Rejected, Err(ProviderError::Rejected(_))) => {}
        other => panic!("{}: unknown payment gave {other:?}", s.name),
    }
}

/// Settlement reports: parsed into items that add up to the net, in signed paise, tied back to our payments.
pub async fn settlement_report_parsing(s: &Subject) {
    let today = chrono::Utc::now().date_naive();
    match s.expect.settlements {
        Settlements::Unsupported => {
            let out = s.provider.fetch_settlements(today).await;
            assert!(
                matches!(out, Err(ProviderError::Unsupported(_))),
                "{}: {out:?}",
                s.name
            );
            return;
        }
        Settlements::Empty => {
            assert!(
                s.provider
                    .fetch_settlements(today)
                    .await
                    .unwrap()
                    .is_empty(),
                "{}",
                s.name
            );
            return;
        }
        Settlements::Parsed => {}
    }
    assert!(
        s.provider
            .fetch_settlements(today)
            .await
            .unwrap()
            .is_empty(),
        "{}: nothing settled yet",
        s.name
    );

    let (a_req, a_init, _) = s.pay(49_900, Outcome::Captured).await;
    let (b_req, b_init, _) = s.pay(12_345, Outcome::Captured).await;
    let mut refs = Vec::new();
    for (req, init) in [(&a_req, &a_init), (&b_req, &b_init)] {
        let st = s.provider.fetch_payment(&req.attempt_id).await.unwrap();
        refs.push((
            req.attempt_id.clone(),
            init.state.provider_order_ref.clone(),
            st.provider_payment_ref.unwrap(),
        ));
    }
    let rid = unique("rfd");
    let rf = s
        .provider
        .refund(&RefundRequest {
            refund_id: rid.clone(),
            attempt_id: a_req.attempt_id.clone(),
            provider_payment_ref: refs[0].2.clone(),
            amount_minor: 10_000,
            currency: Currency::Inr,
        })
        .await
        .unwrap();
    s.backend
        .complete_refund(&RefundCtx {
            refund_id: rid.clone(),
            provider_refund_ref: rf.provider_refund_ref.clone(),
            payment_ref: refs[0].2.clone(),
        })
        .await;
    assert!(
        s.backend.settle(-500).await,
        "{}: no settlement simulation",
        s.name
    );

    let recs = s
        .provider
        .fetch_settlements(today)
        .await
        .unwrap_or_else(|e| panic!("{}: fetch_settlements: {e}", s.name));
    assert_eq!(recs.len(), 1, "{}: one settlement was made", s.name);
    let r = &recs[0];
    assert!(!r.provider_ref.is_empty(), "{}", s.name);
    assert!(
        r.utr.as_deref().is_some_and(|u| !u.is_empty()),
        "{}: no UTR",
        s.name
    );
    assert_eq!(r.currency, Currency::Inr, "{}", s.name);
    assert_eq!(
        r.items.iter().map(|i| i.amount_minor).sum::<i64>(),
        r.net_minor,
        "{}: the items must add up to the net",
        s.name
    );

    let fee_of = |amt: i64| {
        let fee = (amt * 200 + 5_000) / 10_000;
        fee + (fee * 1_800 + 5_000) / 10_000
    };
    let kind = |k: SettlementItemKind| r.items.iter().filter(move |i| i.kind == k);
    let payments: Vec<i64> = kind(SettlementItemKind::Payment)
        .map(|i| i.amount_minor)
        .collect();
    assert_eq!(
        payments.iter().sum::<i64>(),
        62_245,
        "{}: gross, in paise",
        s.name
    );
    assert!(
        payments.iter().all(|a| *a > 0),
        "{}: payments are positive",
        s.name
    );
    for i in kind(SettlementItemKind::Payment) {
        let id = i.provider_ref.as_deref().unwrap_or_default();
        assert!(
            refs.iter()
                .any(|(att, order, pay)| id == att || id == pay || Some(id) == order.as_deref()),
            "{}: a payment item points at {id}, which is none of our references",
            s.name
        );
    }
    assert_eq!(
        kind(SettlementItemKind::Fee)
            .map(|i| i.amount_minor)
            .sum::<i64>(),
        -(fee_of(49_900) + fee_of(12_345)),
        "{}: fees",
        s.name
    );
    assert!(
        kind(SettlementItemKind::Fee).all(|i| i.amount_minor < 0),
        "{}: fees are negative",
        s.name
    );
    assert_eq!(
        kind(SettlementItemKind::Refund)
            .map(|i| i.amount_minor)
            .sum::<i64>(),
        -10_000,
        "{}: refunds are negative",
        s.name
    );
    assert_eq!(
        kind(SettlementItemKind::Adjustment)
            .map(|i| i.amount_minor)
            .sum::<i64>(),
        -500,
        "{}: the adjustment",
        s.name
    );
    assert_eq!(
        r.net_minor,
        62_245 - 10_000 - fee_of(49_900) - fee_of(12_345) - 500,
        "{}: net",
        s.name
    );
}

/// Webhook signatures: valid, tampered, wrong secret, missing, replayed and out of order.
pub async fn webhook_verification(s: &Subject) {
    if !s.expect.webhooks {
        let out = s
            .provider
            .verify_webhook(b"x", &Headers::new(), b"{}", now());
        assert!(
            out.is_err(),
            "{}: declares no webhooks but accepted one",
            s.name
        );
        return;
    }
    let (req, init, first) = s.pay(40_000, Outcome::Captured).await;
    let captured = first.unwrap_or_else(|| panic!("{}: expected a webhook delivery", s.name));
    // a second, conflicting event for the same attempt, as when deliveries arrive out of order
    let failed = s
        .backend
        .pay(&s.paid(&req, &init), Outcome::Failed)
        .await
        .expect("a second delivery");

    let ev = s
        .verify(&captured, now())
        .unwrap_or_else(|e| panic!("{}: a valid webhook was refused: {e}", s.name));
    assert_eq!(ev.kind, EventKind::PaymentCaptured, "{}", s.name);

    // tampered: one digit of the amount changed
    let mut t = captured.clone();
    if let Some(at) = t.body.windows(6).position(|w| w == b"amount")
        && let Some(d) = t.body[at..].iter().position(|b| b.is_ascii_digit())
    {
        let i = at + d;
        t.body[i] = if t.body[i] == b'9' {
            b'1'
        } else {
            t.body[i] + 1
        };
    }
    assert_ne!(
        t.body, captured.body,
        "{}: the harness could not tamper with this body",
        s.name
    );
    assert_eq!(
        s.verify(&t, now()).unwrap_err(),
        WebhookError::BadSignature,
        "{}: tampered body",
        s.name
    );

    // wrong secret
    let wrong = match &s.wrong_credentials {
        Some(p) => p.verify_webhook(&s.webhook_secret, &captured.headers, &captured.body, now()),
        None => {
            s.provider
                .verify_webhook(b"not-the-secret", &captured.headers, &captured.body, now())
        }
    };
    assert_eq!(
        wrong.unwrap_err(),
        WebhookError::BadSignature,
        "{}: wrong secret",
        s.name
    );

    // missing signature
    assert_eq!(
        s.verify(&s.backend.strip_signature(&captured), now())
            .unwrap_err(),
        WebhookError::MissingSignature,
        "{}: no signature",
        s.name
    );

    // replay: the same delivery twice gives the same event id (that is what de-duplication keys on)...
    let again = s.verify(&captured, now()).unwrap();
    assert_eq!(
        ev.event_id, again.event_id,
        "{}: a redelivery must keep its event id",
        s.name
    );
    // ... and an old one is refused where the provider timestamps its deliveries
    let late = s.verify(&captured, now() + 86_400);
    if s.expect.replay_window {
        assert_eq!(
            late.unwrap_err(),
            WebhookError::Stale,
            "{}: an old delivery must be Stale",
            s.name
        );
    } else {
        assert!(
            late.is_ok(),
            "{}: declares no replay window but refused a late delivery",
            s.name
        );
    }

    // out of order: the failure arrives before the capture. Each is read on its own, as itself, and they do
    // not share an event id, so de-duplication cannot swallow the later one.
    let f = s
        .verify(&failed, now())
        .unwrap_or_else(|e| panic!("{}: second delivery: {e}", s.name));
    assert_eq!(f.kind, EventKind::PaymentFailed, "{}", s.name);
    assert_ne!(
        f.event_id, ev.event_id,
        "{}: two different events share an id",
        s.name
    );
    assert_eq!(
        f.order_ref, ev.order_ref,
        "{}: both events must name the same order",
        s.name
    );
    assert_eq!(
        s.verify(&captured, now()).unwrap().kind,
        EventKind::PaymentCaptured,
        "{}: order must not matter",
        s.name
    );
}

/// The provider's refund webhook, where it has one: signed and read, or refused on purpose.
pub async fn refund_webhooks(s: &Subject) {
    if !(s.expect.refund_events || s.expect.unsigned_refund_events) {
        return;
    }
    let (req, _init, _) = s.pay(50_000, Outcome::Captured).await;
    let pref = s
        .provider
        .fetch_payment(&req.attempt_id)
        .await
        .unwrap()
        .provider_payment_ref
        .unwrap();
    let id = unique("rfd");
    let r = s
        .provider
        .refund(&RefundRequest {
            refund_id: id.clone(),
            attempt_id: req.attempt_id.clone(),
            provider_payment_ref: pref.clone(),
            amount_minor: 10_000,
            currency: Currency::Inr,
        })
        .await
        .unwrap();
    let ctx = RefundCtx {
        refund_id: id.clone(),
        provider_refund_ref: r.provider_refund_ref.clone(),
        payment_ref: pref,
    };
    s.backend.complete_refund(&ctx).await;
    let d = s
        .backend
        .refund_event(&ctx)
        .await
        .unwrap_or_else(|| panic!("{}: no refund delivery", s.name));
    let out = s.verify(&d, now());
    if s.expect.unsigned_refund_events {
        assert_eq!(
            out.unwrap_err(),
            WebhookError::MissingSignature,
            "{}: an unsigned refund callback must be refused",
            s.name
        );
        return;
    }
    let ev = out.unwrap_or_else(|e| panic!("{}: refund webhook: {e}", s.name));
    assert_eq!(ev.kind, EventKind::RefundProcessed, "{}", s.name);
    assert!(
        ev.refund_ref.as_deref() == Some(id.as_str())
            || (ev.provider_refund_ref.is_some()
                && ev.provider_refund_ref == r.provider_refund_ref),
        "{}: the refund webhook names neither our refund id nor the provider's reference: {ev:?}",
        s.name
    );
    assert_eq!(ev.amount_minor, Some(10_000), "{}", s.name);
    assert_eq!(
        s.verify(&s.backend.strip_signature(&d), now()).unwrap_err(),
        WebhookError::MissingSignature,
        "{}",
        s.name
    );
}

/// Nothing secret reaches the payer's device or the stored answer.
pub async fn secrets_never_leave(s: &Subject) {
    let req = s.request(10_000, Outcome::Captured);
    let init = s.provider.initiate(&req).await.unwrap();
    let shown = format!(
        "{}{}",
        serde_json::to_string(&init.action).unwrap(),
        init.state.raw
    );
    for secret in &s.secrets {
        assert!(
            !shown.contains(secret.as_str()),
            "{}: a secret leaked into what is handed to the payer",
            s.name
        );
    }
    let _: Value = serde_json::from_str(&serde_json::to_string(&init.action).unwrap()).unwrap();
}
