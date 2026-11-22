// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! One backend and one `Subject` constructor per adapter: the simulated provider's side of each conversation.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use zyvor_provider_mock::{MockConfig, spawn};
use zyvor_providers::{
    cashfree::Cashfree, custom::Custom, payu::PayU, phonepe::PhonePe, razorpay::Razorpay,
    sim::SimProvider, sponsor_upi::SponsorUpi, *,
};

use crate::gateway::{self, GW_KEY, GW_WEBHOOK_SECRET, PP_WEBHOOK_SECRET};
use crate::harness::*;

const RZ_HOOK: &str = "rz_test_webhook_secret";
const CF_HOOK: &str = "cf_test_webhook_secret";
const TIMEOUT: Duration = Duration::from_secs(1);
const NOWHERE: &str = "http://127.0.0.1:9";

/// POST JSON to a control endpoint and return the answer.
async fn control(base: &str, path: &str, body: Value) -> Value {
    let res = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "control call {path} failed: {}",
        res.status()
    );
    res.json().await.unwrap()
}

/// A `{ "headers": {..}, "body": ".." }` delivery from a control endpoint.
fn delivery(v: &Value) -> Delivery {
    let h = &v["webhook"];
    Delivery {
        headers: h["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap().to_string()))
            .collect(),
        body: h["body"].as_str().unwrap().as_bytes().to_vec(),
    }
}

async fn start_mock() -> String {
    spawn(MockConfig {
        razorpay_webhook_secret: RZ_HOOK.into(),
        cashfree_webhook_secret: CF_HOOK.into(),
    })
    .await
}

async fn settle(base: &str, path: &str, adjustment: i64) -> bool {
    control(
        base,
        path,
        json!({ "fee_bps": 200, "tax_bps": 1800, "adjustment_minor": adjustment }),
    )
    .await;
    true
}

// ---------------------------------------------------------------------------------------------- Razorpay

struct RazorpayBackend(String);

#[async_trait]
impl Backend for RazorpayBackend {
    async fn pay(&self, p: &Paid, outcome: Outcome) -> Option<Delivery> {
        let o = if outcome == Outcome::Captured {
            "captured"
        } else {
            "failed"
        };
        let v = control(
            &self.0,
            "/_sim/razorpay/pay",
            json!({ "order_id": p.order_ref, "outcome": o }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn refund_event(&self, r: &RefundCtx) -> Option<Delivery> {
        let v = control(
            &self.0,
            "/_sim/razorpay/refund_event",
            json!({ "refund_id": r.provider_refund_ref }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn settle(&self, adjustment: i64) -> bool {
        settle(&self.0, "/_sim/razorpay/settle", adjustment).await
    }
}

pub async fn razorpay() -> Subject {
    let mock = start_mock().await;
    let (url, faults) = gateway::proxy(mock.clone()).await;
    Subject {
        name: "razorpay",
        provider: Arc::new(Razorpay::new(
            &url,
            "rzp_test_key",
            "rzp_test_secret_value",
            TIMEOUT,
        )),
        unreachable: Some(Arc::new(Razorpay::new(
            NOWHERE,
            "rzp_test_key",
            "rzp_test_secret_value",
            TIMEOUT,
        ))),
        wrong_credentials: None,
        backend: Box::new(RazorpayBackend(mock)),
        faults: Some(faults),
        webhook_secret: RZ_HOOK.into(),
        secrets: vec!["rzp_test_secret_value".into()],
        expect: Expect {
            // Razorpay's webhook has no timestamp: the event id is what stops a replay.
            replay_window: false,
            // `receipt` is not unique at Razorpay, so a second initiate makes a second order; the lookup reads both.
            reinitiate_new_order: true,
            // Razorpay's refund call is not idempotent on our refund id: a lost response is resolved by lookup, never resent.
            refund_idempotent: false,
            ..Expect::default()
        },
    }
}

// ---------------------------------------------------------------------------------------------- Cashfree

struct CashfreeBackend(String);

#[async_trait]
impl Backend for CashfreeBackend {
    async fn pay(&self, p: &Paid, outcome: Outcome) -> Option<Delivery> {
        let o = if outcome == Outcome::Captured {
            "SUCCESS"
        } else {
            "FAILED"
        };
        let v = control(
            &self.0,
            "/_sim/cashfree/pay",
            json!({ "order_id": p.attempt_id, "outcome": o }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn refund_event(&self, r: &RefundCtx) -> Option<Delivery> {
        let v = control(
            &self.0,
            "/_sim/cashfree/refund_event",
            json!({ "refund_id": r.refund_id }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn settle(&self, adjustment: i64) -> bool {
        settle(&self.0, "/_sim/cashfree/settle", adjustment).await
    }
}

pub async fn cashfree() -> Subject {
    let mock = start_mock().await;
    let (url, faults) = gateway::proxy(mock.clone()).await;
    Subject {
        name: "cashfree",
        provider: Arc::new(Cashfree::new(
            &url,
            "cf_test_app_id",
            "cf_test_secret_value",
            TIMEOUT,
        )),
        unreachable: Some(Arc::new(Cashfree::new(
            NOWHERE,
            "cf_test_app_id",
            "cf_test_secret_value",
            TIMEOUT,
        ))),
        wrong_credentials: None,
        backend: Box::new(CashfreeBackend(mock)),
        faults: Some(faults),
        webhook_secret: CF_HOOK.into(),
        secrets: vec!["cf_test_secret_value".into()],
        expect: Expect::default(),
    }
}

// ---------------------------------------------------------------------------------------------- PayU

struct PayuBackend {
    mock: String,
    /// transactions the payer has already opened the checkout page for
    opened: Mutex<HashSet<String>>,
}

#[async_trait]
impl Backend for PayuBackend {
    async fn pay(&self, p: &Paid, outcome: Outcome) -> Option<Delivery> {
        let txnid = p.order_ref.clone().expect("payu order ref");
        if self.opened.lock().unwrap().insert(txnid.clone()) {
            // The payer's browser posts the form we handed over. The simulation checks its hash on its own.
            let action: Value = serde_json::from_str(&p.action).unwrap();
            let fields = action["params"]["fields"]
                .as_object()
                .expect("a hosted checkout form");
            let body = {
                let mut form = form_urlencoded::Serializer::new(String::new());
                for (k, v) in fields {
                    form.append_pair(k, v.as_str().unwrap());
                }
                form.finish()
            };
            let res = reqwest::Client::new()
                .post(format!("{}/_payment", self.mock))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .unwrap();
            assert!(
                res.status().is_success(),
                "the checkout form was refused: {:?}",
                res.text().await
            );
        }
        let o = if outcome == Outcome::Captured {
            "captured"
        } else {
            "failed"
        };
        let v = control(
            &self.mock,
            "/_sim/payu/pay",
            json!({ "txnid": txnid, "outcome": o }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn complete_refund(&self, r: &RefundCtx) {
        control(
            &self.mock,
            "/_sim/payu/refund_complete",
            json!({ "request_id": r.provider_refund_ref, "outcome": "success" }),
        )
        .await;
    }
    async fn refund_event(&self, r: &RefundCtx) -> Option<Delivery> {
        // PayU's refund callback is JSON and, in its documentation, unsigned.
        let body = json!({
            "action": "refund", "status": "success", "token": r.refund_id, "request_id": r.provider_refund_ref,
            "mihpayid": r.payment_ref, "amt": "100.00", "key": "payu_test_key",
        });
        Some(Delivery {
            headers: Headers::new(),
            body: body.to_string().into_bytes(),
        })
    }
    async fn settle(&self, adjustment: i64) -> bool {
        settle(&self.mock, "/_sim/payu/settle", adjustment).await
    }
    fn strip_signature(&self, d: &Delivery) -> Delivery {
        let kept: Vec<(String, String)> = form_urlencoded::parse(&d.body)
            .filter(|(k, _)| k != "hash")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let mut s = form_urlencoded::Serializer::new(String::new());
        for (k, v) in kept {
            s.append_pair(&k, &v);
        }
        Delivery {
            headers: d.headers.clone(),
            body: s.finish().into_bytes(),
        }
    }
}

pub async fn payu() -> Subject {
    let mock = start_mock().await;
    let (url, faults) = gateway::proxy(mock.clone()).await;
    let build = |base: &str, salt: &str| {
        PayU::new(base, "payu_test_key", salt, TIMEOUT)
            .with_merchant_secret("payu_test_merchant_secret")
    };
    Subject {
        name: "payu",
        provider: Arc::new(build(&url, "payu_test_salt")),
        unreachable: Some(Arc::new(build(NOWHERE, "payu_test_salt"))),
        wrong_credentials: Some(Arc::new(build(&url, "a_different_salt"))),
        backend: Box::new(PayuBackend {
            mock,
            opened: Default::default(),
        }),
        faults: Some(faults),
        // PayU's reverse hash is keyed by the salt held in the adapter, not by a separate webhook secret.
        webhook_secret: Vec::new(),
        secrets: vec!["payu_test_salt".into(), "payu_test_merchant_secret".into()],
        expect: Expect {
            // PayU's payment callback has no timestamp: the event id (txnid + status) is what stops a replay.
            replay_window: false,
            // PayU's refund callback is JSON with no documented signature: the adapter refuses it, and refund state
            // is read back with check_action_status.
            refund_events: false,
            unsigned_refund_events: true,
            // Nothing exists at PayU until the payer reaches its page, so "Not Found" is a payment not started yet.
            unknown_payment: Unknown::Pending,
            // A queued refund may not be visible yet; not found is not "failed", which would invite a second refund.
            unknown_refund: Unknown::Uncertain,
            initiate_is_local: true,
            ..Expect::default()
        },
    }
}

// ---------------------------------------------------------------------------------------------- custom gateway, sponsor UPI

struct GatewayBackend {
    base: String,
}

#[async_trait]
impl Backend for GatewayBackend {
    async fn pay(&self, p: &Paid, outcome: Outcome) -> Option<Delivery> {
        let o = if outcome == Outcome::Captured {
            "captured"
        } else {
            "failed"
        };
        let v = control(
            &self.base,
            "/_sim/pay",
            json!({ "attempt_id": p.attempt_id, "outcome": o }),
        )
        .await;
        Some(delivery(&v))
    }
    async fn refund_event(&self, r: &RefundCtx) -> Option<Delivery> {
        Some(delivery(
            &control(
                &self.base,
                "/_sim/refund_event",
                json!({ "refund_id": r.refund_id }),
            )
            .await,
        ))
    }
    async fn settle(&self, adjustment: i64) -> bool {
        settle(&self.base, "/_sim/settle", adjustment).await
    }
}

async fn gateway_subject(name: &'static str, sponsor: bool) -> Subject {
    let gw = gateway::serve(gateway::custom_router()).await;
    let (url, faults) = gateway::proxy(gw.clone()).await;
    let make = |base: &str| -> Arc<dyn Provider> {
        if sponsor {
            Arc::new(SponsorUpi::new(base, GW_KEY, TIMEOUT))
        } else {
            Arc::new(Custom::new(base, GW_KEY, TIMEOUT))
        }
    };
    Subject {
        name,
        provider: make(&url),
        unreachable: Some(make(NOWHERE)),
        wrong_credentials: None,
        backend: Box::new(GatewayBackend { base: gw }),
        faults: Some(faults),
        webhook_secret: GW_WEBHOOK_SECRET.to_vec(),
        secrets: vec![
            GW_KEY.into(),
            String::from_utf8_lossy(GW_WEBHOOK_SECRET).into_owned(),
        ],
        expect: Expect::default(),
    }
}

pub async fn custom() -> Subject {
    gateway_subject("custom", false).await
}
pub async fn sponsor_upi() -> Subject {
    gateway_subject("sponsor_upi", true).await
}

// ---------------------------------------------------------------------------------------------- PhonePe

pub async fn phonepe() -> Subject {
    let gw = gateway::serve(gateway::phonepe_router()).await;
    let (url, faults) = gateway::proxy(gw.clone()).await;
    Subject {
        name: "phonepe",
        provider: Arc::new(PhonePe::new(
            &url,
            "PP_TEST_MERCHANT",
            "pp_test_salt_key",
            TIMEOUT,
        )),
        unreachable: Some(Arc::new(PhonePe::new(
            NOWHERE,
            "PP_TEST_MERCHANT",
            "pp_test_salt_key",
            TIMEOUT,
        ))),
        wrong_credentials: None,
        backend: Box::new(GatewayBackend { base: gw }),
        faults: Some(faults),
        webhook_secret: PP_WEBHOOK_SECRET.to_vec(),
        secrets: vec![
            "pp_test_salt_key".into(),
            String::from_utf8_lossy(PP_WEBHOOK_SECRET).into_owned(),
        ],
        expect: Expect {
            // The adapter has no settlement sync: it says so instead of returning nothing.
            settlements: Settlements::Unsupported,
            ..Expect::default()
        },
    }
}

// ---------------------------------------------------------------------------------------------- the in-process simulator

struct SimBackend;

#[async_trait]
impl Backend for SimBackend {
    async fn pay(&self, _p: &Paid, _outcome: Outcome) -> Option<Delivery> {
        None // the simulator decides at initiation, from the UPI handle
    }
    fn vpa(&self, outcome: Outcome) -> String {
        match outcome {
            Outcome::Captured => "asha@okzyvor".into(),
            Outcome::Failed => "fail@okzyvor".into(),
        }
    }
    fn uncertain_vpa(&self) -> Option<String> {
        Some("uncertain@okzyvor".into())
    }
}

pub async fn sim() -> Subject {
    Subject {
        name: "sim",
        provider: Arc::new(SimProvider::new()),
        unreachable: None,
        wrong_credentials: None,
        backend: Box::new(SimBackend),
        faults: None,
        webhook_secret: Vec::new(),
        secrets: vec![],
        expect: Expect {
            webhooks: false,
            replay_window: false,
            refund_events: false,
            refund_idempotent: false,
            settlements: Settlements::Empty,
            // The simulator's maps only ever hold what it was asked to make, so "never seen" is a refusal.
            unknown_payment: Unknown::Rejected,
            unknown_refund: Unknown::Rejected,
            // The platform checks refund bounds before calling a provider (refunds.rs); the simulator does not repeat it.
            refund_bounds_enforced: false,
            immediate: true,
            ..Expect::default()
        },
    }
}
