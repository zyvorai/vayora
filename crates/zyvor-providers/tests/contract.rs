//! Adapter contract tests against the local mock (SIMULATED providers, not live sandboxes).

use std::time::Duration;

use serde_json::{Value, json};
use zyvor_core::Currency;
use zyvor_provider_mock::{MockConfig, spawn};
use zyvor_providers::{cashfree::Cashfree, razorpay::Razorpay, *};

const RZ_SECRET: &str = "rz_hook";
const CF_SECRET: &str = "cf_hook";

async fn mock() -> (String, reqwest::Client) {
    let url = spawn(MockConfig {
        razorpay_webhook_secret: RZ_SECRET.into(),
        cashfree_webhook_secret: CF_SECRET.into(),
    })
    .await;
    (url, reqwest::Client::new())
}

async fn sim(c: &reqwest::Client, url: &str, path: &str, body: Value) -> Value {
    c.post(format!("{url}/_sim/{path}"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn headers(v: &Value) -> Headers {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap().to_string()))
        .collect()
}

fn pay_req(id: &str, method: Method, vpa: Option<&str>) -> CreatePayment {
    CreatePayment {
        attempt_id: id.into(),
        amount_minor: 49_900,
        currency: Currency::Inr,
        method,
        vpa: vpa.map(Into::into),
        customer_id: None,
        customer_phone: None,
        capture_automatic: true,
        return_url: None,
        upi_intent: false,
        client: None,
    }
}

#[tokio::test]
async fn razorpay_checkout_webhook_refund_and_settlement() {
    let (url, c) = mock().await;
    let rz = Razorpay::new(&url, "rzp_test_key", "secret", Duration::from_secs(3));

    let init = rz
        .initiate(&pay_req("att_1", Method::Card, None))
        .await
        .unwrap();
    assert_eq!(init.state.status, PaymentStatus::Pending);
    let order = init.state.provider_order_ref.clone().unwrap();
    assert!(
        matches!(init.action, ClientAction::HostedCheckout { .. }),
        "card data stays with the provider"
    );

    // customer pays at hosted checkout; provider sends a signed webhook
    let paid = sim(
        &c,
        &url,
        "razorpay/pay",
        json!({ "order_id": order, "outcome": "captured" }),
    )
    .await;
    let hook = &paid["webhook"];
    let body = hook["body"].as_str().unwrap().as_bytes();
    let ev = rz
        .verify_webhook(RZ_SECRET.as_bytes(), &headers(&hook["headers"]), body, 0)
        .unwrap();
    assert_eq!(ev.kind, EventKind::PaymentCaptured);
    assert_eq!(ev.order_ref.as_deref(), Some(order.as_str()));
    assert_eq!(ev.amount_minor, Some(49_900));
    // tampered body / wrong secret are rejected
    assert_eq!(
        rz.verify_webhook(RZ_SECRET.as_bytes(), &headers(&hook["headers"]), b"{}", 0)
            .unwrap_err(),
        WebhookError::BadSignature
    );
    assert_eq!(
        rz.verify_webhook(b"nope", &headers(&hook["headers"]), body, 0)
            .unwrap_err(),
        WebhookError::BadSignature
    );

    // lookup by our own reference resolves the payment
    let st = rz.fetch_payment("att_1").await.unwrap();
    assert_eq!(st.status, PaymentStatus::Captured);
    let pref = st.provider_payment_ref.clone().unwrap();

    // partial refund, then an over-refund is a definitive rejection
    let r = rz
        .refund(&RefundRequest {
            refund_id: "rf_1".into(),
            attempt_id: "att_1".into(),
            provider_payment_ref: pref.clone(),
            amount_minor: 10_000,
            currency: Currency::Inr,
        })
        .await
        .unwrap();
    assert_eq!(r.status, RefundStatus::Processed);
    let over = rz
        .refund(&RefundRequest {
            refund_id: "rf_2".into(),
            attempt_id: "att_1".into(),
            provider_payment_ref: pref.clone(),
            amount_minor: 40_000,
            currency: Currency::Inr,
        })
        .await;
    assert!(matches!(over, Err(ProviderError::Rejected(_))), "{over:?}");
    assert_eq!(
        rz.fetch_refund("att_1", "rf_1", &pref)
            .await
            .unwrap()
            .status,
        RefundStatus::Processed
    );

    // settlement: gross 499.00, fee 2% = 9.98, tax 18% of fee = 1.80, refund -100.00
    sim(
        &c,
        &url,
        "razorpay/settle",
        json!({ "fee_bps": 200, "tax_bps": 1800 }),
    )
    .await;
    let s = rz
        .fetch_settlements(chrono::Utc::now().date_naive())
        .await
        .unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].net_minor, 49_900 - 998 - 180 - 10_000);
    assert!(
        s[0].items
            .iter()
            .any(|i| i.kind == SettlementItemKind::Fee && i.amount_minor == -1_178)
    );
}

#[tokio::test]
async fn cashfree_collect_webhook_refund_and_timeout_resolution() {
    let (url, c) = mock().await;
    let cf = Cashfree::new(&url, "cf_id", "cf_secret", Duration::from_secs(1));

    // happy path: server-initiated UPI collect, payer approves
    let init = cf
        .initiate(&pay_req("att_a", Method::Upi, Some("asha@okvayora")))
        .await
        .unwrap();
    assert!(matches!(init.action, ClientAction::UpiCollect { .. }));
    assert_eq!(
        cf.fetch_payment("att_a").await.unwrap().status,
        PaymentStatus::Pending
    );
    let paid = sim(
        &c,
        &url,
        "cashfree/pay",
        json!({ "order_id": "att_a", "outcome": "SUCCESS" }),
    )
    .await;
    let hook = &paid["webhook"];
    let body = hook["body"].as_str().unwrap().as_bytes();
    let ev = cf
        .verify_webhook(
            CF_SECRET.as_bytes(),
            &headers(&hook["headers"]),
            body,
            chrono::Utc::now().timestamp(),
        )
        .unwrap();
    assert_eq!(ev.kind, EventKind::PaymentCaptured);
    assert_eq!(ev.order_ref.as_deref(), Some("att_a"));
    assert_eq!(ev.amount_minor, Some(49_900));
    // replay protection: the same delivery is stale ten minutes later
    let late = chrono::Utc::now().timestamp() + 700;
    assert_eq!(
        cf.verify_webhook(CF_SECRET.as_bytes(), &headers(&hook["headers"]), body, late)
            .unwrap_err(),
        WebhookError::Stale
    );
    assert_eq!(
        cf.verify_webhook(b"wrong", &headers(&hook["headers"]), body, 0)
            .unwrap_err(),
        WebhookError::BadSignature
    );

    let st = cf.fetch_payment("att_a").await.unwrap();
    assert_eq!(st.status, PaymentStatus::Captured);
    let rq = RefundRequest {
        refund_id: "rf_a".into(),
        attempt_id: "att_a".into(),
        provider_payment_ref: st.provider_payment_ref.clone().unwrap(),
        amount_minor: 12_550,
        currency: Currency::Inr,
    };
    let r = cf.refund(&rq).await.unwrap();
    assert_eq!(r.status, RefundStatus::Processed);
    // refund ids are idempotent at the provider
    assert_eq!(
        cf.refund(&rq).await.unwrap().provider_refund_ref,
        r.provider_refund_ref
    );

    // timeout AFTER the provider accepted the collect: outcome unknown, but exactly one payment exists
    sim(
        &c,
        &url,
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let lost = cf
        .initiate(&pay_req("att_b", Method::Upi, Some("ravi@ybl")))
        .await;
    assert!(matches!(lost, Err(ProviderError::Uncertain(_))), "{lost:?}");
    let found = cf.fetch_payment("att_b").await.unwrap();
    assert_eq!(
        found.status,
        PaymentStatus::Pending,
        "the collect exists at the provider"
    );
    let stats: Value = c
        .get(format!("{url}/_sim/stats"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stats["cashfree_payments"], 2,
        "no duplicate was created by the lost response"
    );

    // an order the provider never saw is reported as not found, i.e. definitively not accepted
    assert_eq!(
        cf.fetch_payment("never_sent")
            .await
            .unwrap()
            .failure_reason
            .as_deref(),
        Some("not_found")
    );
}

#[test]
fn capability_matrix_describes_each_provider() {
    let rz = Razorpay::new("http://x", "k", "s", Duration::from_secs(1));
    let cf = Cashfree::new("http://x", "k", "s", Duration::from_secs(1));
    assert!(
        rz.capabilities()
            .supports(Method::Card, Currency::Usd)
            .is_ok()
    );
    assert!(
        cf.capabilities()
            .supports(Method::Card, Currency::Usd)
            .is_err(),
        "cashfree adapter is INR only"
    );
    assert!(!rz.capabilities().server_initiated_upi && cf.capabilities().server_initiated_upi);
}
