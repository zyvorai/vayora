//! Cashfree Subscriptions (UPI AutoPay) against a LOCAL MOCK HTTP server. The JSON shapes are written from
//! Cashfree's public docs (v2026-01-01); nothing here has touched Cashfree's sandbox. Every value is invented.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
};
use base64::Engine;
use serde_json::{Value, json};
use zyvor_core::Currency;
use zyvor_providers::{cashfree::Cashfree, *};

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Default)]
struct Inner {
    seen: Vec<Seen>,
    /// "METHOD /path" -> (status, body); an absent route answers 404.
    routes: HashMap<String, (u16, Value)>,
}
type Shared = Arc<Mutex<Inner>>;

async fn handle(
    State(s): State<Shared>,
    m: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let mut g = s.lock().unwrap();
    let key = format!("{m} {}", uri.path());
    g.seen.push(Seen {
        method: m.to_string(),
        path: uri.path().to_string(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    match g.routes.get(&key) {
        Some((st, b)) => (StatusCode::from_u16(*st).unwrap(), axum::Json(b.clone())),
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(
                json!({ "message": "Resource Not found.", "code": "x", "type": "invalid_request_error" }),
            ),
        ),
    }
}

async fn mock() -> (Cashfree, Shared) {
    let shared: Shared = Default::default();
    let app = Router::new().fallback(handle).with_state(shared.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (
        Cashfree::new(url, "test_client_id", "test_secret", Duration::from_secs(3)),
        shared,
    )
}

fn route(s: &Shared, key: &str, status: u16, body: Value) {
    s.lock().unwrap().routes.insert(key.into(), (status, body));
}
fn seen(s: &Shared) -> Vec<Seen> {
    s.lock().unwrap().seen.clone()
}

fn entity(status: &str, auth: &str) -> Value {
    json!({
        "cf_subscription_id": "9001", "subscription_id": "mnd_1", "subscription_status": status,
        "subscription_session_id": "sub_session_fixture", "next_schedule_date": null,
        "plan_details": { "plan_type": "ON_DEMAND", "plan_max_amount": 5000.0 },
        "authorisation_details": { "authorization_status": auth, "authorization_reference": "" }
    })
}

fn new_mandate() -> CreateMandate {
    CreateMandate {
        mandate_id: "mnd_1".into(),
        max_amount_minor: 500_000,
        currency: Currency::Inr,
        frequency: Frequency::Monthly,
        start_date: "2026-12-01".parse().unwrap(),
        end_date: "2027-12-01".parse().unwrap(),
        customer_id: None,
        customer_phone: Some("9000000000".into()),
    }
}

fn debit_req() -> DebitRequest {
    DebitRequest {
        attempt_id: "att_1".into(),
        mandate_id: "mnd_1".into(),
        provider_mandate_ref: "9001".into(),
        amount_minor: 149_900,
        currency: Currency::Inr,
    }
}

fn auth_response() -> Value {
    json!({
        "action": "custom", "channel": "link", "cf_payment_id": "7001", "payment_id": "mnd_1_auth",
        "payment_type": "AUTH", "payment_status": "PENDING", "subscription_id": "mnd_1",
        "data": { "url": "https://example.invalid/timer", "method": "post", "payload": { "paymentType": "UPI",
            "upiIntentData": {
                "androidAuthAppLinks": { "PHONEPE": "phonepe://mandate?x=android" },
                "iosAuthAppLinks": { "PHONEPE": "phonepe://mandate?x=ios", "GPAY": "gpay://mandate?x=ios" } } } }
    })
}

#[tokio::test]
async fn the_capability_is_on() {
    let (cf, _) = mock().await;
    assert!(cf.capabilities().mandates);
}

#[tokio::test]
async fn create_sends_an_on_demand_plan_and_returns_the_payers_links() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions",
        200,
        entity("INITIALIZED", "INITIALIZED"),
    );
    route(&s, "POST /subscriptions/pay", 200, auth_response());
    let st = cf.create_mandate(&new_mandate()).await.unwrap();
    assert_eq!(st.status, MandateStatus::Pending);
    assert_eq!(st.provider_mandate_ref.as_deref(), Some("9001"));
    let a = st.authorization.unwrap();
    assert_eq!(a["subscription_session_id"], "sub_session_fixture");
    assert_eq!(
        a["apps"]["phonepe"], "phonepe://mandate?x=android",
        "android wins"
    );
    assert_eq!(a["apps"]["gpay"], "gpay://mandate?x=ios");

    let calls = seen(&s);
    let create = &calls[0];
    assert_eq!(create.path, "/subscriptions");
    assert_eq!(create.headers["x-api-version"], "2026-01-01");
    assert_eq!(create.headers["x-client-id"], "test_client_id");
    let b = &create.body;
    assert_eq!(b["subscription_id"], "mnd_1");
    assert_eq!(b["plan_details"]["plan_type"], "ON_DEMAND");
    assert_eq!(b["plan_details"]["plan_max_amount"], json!(5000.0));
    assert_eq!(
        b["authorization_details"]["payment_methods"],
        json!(["upi"])
    );
    assert_eq!(b["subscription_expiry_time"], "2027-12-01T23:59:59+05:30");
    assert_eq!(b["customer_details"]["customer_phone"], "9000000000");
    // the idempotency key is a UUID shape and repeats for the same mandate
    let key = create.headers["x-idempotency-key"]
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(key.len(), 36);
    assert_eq!(key.matches('-').count(), 4);
    let again = cf.create_mandate(&new_mandate()).await.unwrap();
    assert_eq!(again.status, MandateStatus::Pending);
    assert_eq!(
        seen(&s)[2].headers["x-idempotency-key"].to_str().unwrap(),
        key
    );
    // the AUTH call carries the session and our own payment id
    let auth = &calls[1];
    assert_eq!(auth.body["payment_type"], "AUTH");
    assert_eq!(auth.body["subscription_session_id"], "sub_session_fixture");
    assert_eq!(auth.body["payment_id"], "mnd_1_auth");
}

#[tokio::test]
async fn a_failed_link_request_does_not_lose_the_created_subscription() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions",
        200,
        entity("INITIALIZED", "INITIALIZED"),
    );
    route(&s, "POST /subscriptions/pay", 500, json!({}));
    let st = cf.create_mandate(&new_mandate()).await.unwrap();
    assert_eq!(st.status, MandateStatus::Pending);
    assert_eq!(st.authorization.unwrap()["apps"], json!({}));
}

#[tokio::test]
async fn create_errors_are_classified() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions",
        400,
        json!({ "message": "bad plan" }),
    );
    assert!(matches!(
        cf.create_mandate(&new_mandate()).await,
        Err(ProviderError::Rejected(m)) if m.contains("bad plan")
    ));
    route(&s, "POST /subscriptions", 502, json!({}));
    assert!(matches!(
        cf.create_mandate(&new_mandate()).await,
        Err(ProviderError::Uncertain(_))
    ));
    let mut usd = new_mandate();
    usd.currency = Currency::Usd;
    assert!(matches!(
        cf.create_mandate(&usd).await,
        Err(ProviderError::Rejected(_))
    ));
}

#[tokio::test]
async fn statuses_map_to_mandate_states() {
    let (cf, s) = mock().await;
    for (cashfree, auth, ours) in [
        ("INITIALIZED", "INITIALIZED", MandateStatus::Pending),
        ("BANK_APPROVAL_PENDING", "PENDING", MandateStatus::Pending),
        ("ACTIVE", "SUCCESS", MandateStatus::Active),
        ("ON_HOLD", "SUCCESS", MandateStatus::Paused),
        ("CUSTOMER_PAUSED", "SUCCESS", MandateStatus::Paused),
        ("CUSTOMER_CANCELLED", "SUCCESS", MandateStatus::Revoked),
        ("CANCELLED", "SUCCESS", MandateStatus::Revoked),
        ("COMPLETED", "SUCCESS", MandateStatus::Expired),
        ("EXPIRED", "SUCCESS", MandateStatus::Expired),
        ("LINK_EXPIRED", "INITIALIZED", MandateStatus::Failed),
        ("INITIALIZED", "FAILED", MandateStatus::Failed),
        ("SOMETHING_NEW", "", MandateStatus::Pending),
    ] {
        route(&s, "GET /subscriptions/mnd_1", 200, entity(cashfree, auth));
        let st = cf.fetch_mandate("mnd_1").await.unwrap();
        assert_eq!(st.status, ours, "{cashfree}/{auth}");
        assert!(
            st.authorization.is_none(),
            "a refresh must not overwrite the stored links"
        );
    }
}

#[tokio::test]
async fn an_unknown_mandate_is_rejected_and_a_server_error_is_unknown() {
    let (cf, s) = mock().await;
    assert!(matches!(
        cf.fetch_mandate("nope").await,
        Err(ProviderError::Rejected(_))
    ));
    route(&s, "GET /subscriptions/mnd_1", 503, json!({}));
    assert!(matches!(
        cf.fetch_mandate("mnd_1").await,
        Err(ProviderError::Uncertain(_))
    ));
}

#[tokio::test]
async fn revoke_cancels_the_subscription() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions/mnd_1/manage",
        200,
        entity("CANCELLED", "SUCCESS"),
    );
    let st = cf.revoke_mandate("mnd_1").await.unwrap();
    assert_eq!(st.status, MandateStatus::Revoked);
    let c = &seen(&s)[0];
    assert_eq!(c.body["action"], "CANCEL");
    assert!(c.headers.contains_key("x-idempotency-key"));
}

#[tokio::test]
async fn a_debit_is_one_idempotent_charge_keyed_by_our_attempt() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions/pay",
        200,
        json!({ "cf_payment_id": 7002, "payment_id": "att_1", "payment_type": "CHARGE",
                "payment_status": "PENDING", "subscription_id": "mnd_1", "payment_amount": 1499.0 }),
    );
    let init = cf.debit(&debit_req()).await.unwrap();
    assert_eq!(init.state.status, PaymentStatus::Pending);
    assert_eq!(init.state.provider_order_ref.as_deref(), Some("att_1"));
    assert_eq!(init.state.provider_payment_ref.as_deref(), Some("7002"));
    assert!(matches!(init.action, ClientAction::None));
    let c = &seen(&s)[0];
    assert_eq!(c.body["payment_type"], "CHARGE");
    assert_eq!(c.body["payment_id"], "att_1");
    assert_eq!(c.body["subscription_id"], "mnd_1");
    assert_eq!(c.body["payment_amount"], json!(1499.0));
    let key = c.headers["x-idempotency-key"].to_str().unwrap().to_string();
    cf.debit(&debit_req()).await.unwrap();
    assert_eq!(
        seen(&s)[1].headers["x-idempotency-key"].to_str().unwrap(),
        key
    );
    let mut other = debit_req();
    other.attempt_id = "att_2".into();
    cf.debit(&other).await.unwrap();
    assert_ne!(
        seen(&s)[2].headers["x-idempotency-key"].to_str().unwrap(),
        key
    );
}

#[tokio::test]
async fn a_debit_answered_success_or_failed_maps_straight_through() {
    let (cf, s) = mock().await;
    route(
        &s,
        "POST /subscriptions/pay",
        200,
        json!({ "cf_payment_id": "1", "payment_status": "SUCCESS" }),
    );
    assert_eq!(
        cf.debit(&debit_req()).await.unwrap().state.status,
        PaymentStatus::Captured
    );
    route(
        &s,
        "POST /subscriptions/pay",
        200,
        json!({ "cf_payment_id": "1", "payment_status": "FAILED", "failure_details": { "failure_reason": "insufficient funds" } }),
    );
    let st = cf.debit(&debit_req()).await.unwrap().state;
    assert_eq!(st.status, PaymentStatus::Failed);
    assert_eq!(st.failure_reason.as_deref(), Some("insufficient funds"));
}

#[tokio::test]
async fn a_lost_debit_answer_is_unknown_and_is_resolved_by_asking_not_resending() {
    let (cf, s) = mock().await;
    route(&s, "POST /subscriptions/pay", 500, json!({}));
    assert!(matches!(
        cf.debit(&debit_req()).await,
        Err(ProviderError::Uncertain(_))
    ));
    // A dropped connection after sending is also unknown.
    let dead = Cashfree::new("http://127.0.0.1:9", "i", "s", Duration::from_millis(300));
    assert!(matches!(
        dead.debit(&debit_req()).await,
        Err(ProviderError::Rejected(_)) // connection refused: the request never left
    ));

    let before = seen(&s).iter().filter(|c| c.method == "POST").count();
    // the resolver asks by (mandate, attempt)
    route(
        &s,
        "GET /subscriptions/mnd_1/payments/att_1",
        200,
        json!({ "cf_payment_id": "7002", "payment_id": "att_1", "payment_status": "SUCCESS", "payment_type": "CHARGE" }),
    );
    let st = cf.fetch_debit("att_1", "mnd_1").await.unwrap();
    assert_eq!(st.status, PaymentStatus::Captured);
    assert_eq!(st.provider_payment_ref.as_deref(), Some("7002"));
    assert_eq!(
        seen(&s).iter().filter(|c| c.method == "POST").count(),
        before,
        "asking never sends the debit again"
    );
}

#[tokio::test]
async fn an_unfindable_charge_stays_unknown_not_failed() {
    let (cf, _) = mock().await;
    assert!(matches!(
        cf.fetch_debit("att_9", "mnd_1").await,
        Err(ProviderError::Uncertain(_))
    ));
}

#[tokio::test]
async fn debit_inputs_are_checked_before_any_request() {
    let (cf, s) = mock().await;
    let mut bad = debit_req();
    bad.amount_minor = 0;
    assert!(matches!(
        cf.debit(&bad).await,
        Err(ProviderError::Rejected(_))
    ));
    bad = debit_req();
    bad.currency = Currency::Usd;
    assert!(matches!(
        cf.debit(&bad).await,
        Err(ProviderError::Rejected(_))
    ));
    assert!(seen(&s).is_empty());
}

// ---- webhooks: same signature path as every other Cashfree webhook ----

fn signed(secret: &str, ts_ms: i64, body: &Value) -> (Headers, Vec<u8>) {
    use hmac::{Hmac, KeyInit, Mac};
    let raw = serde_json::to_vec(body).unwrap();
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    m.update(ts_ms.to_string().as_bytes());
    m.update(&raw);
    let sig = base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes());
    let h: Headers = [
        ("x-webhook-signature".to_string(), sig),
        ("x-webhook-timestamp".to_string(), ts_ms.to_string()),
    ]
    .into();
    (h, raw)
}

fn payment_event(ty: &str, payment_type: &str, status: &str) -> Value {
    json!({ "type": ty, "event_time": "2026-12-02T10:00:00+05:30", "data": {
        "payment_id": "att_1", "cf_payment_id": "7002", "subscription_id": "mnd_1",
        "payment_type": payment_type, "payment_amount": 1499.0, "payment_status": status,
        "failure_details": { "failure_reason": if status == "FAILED" { "insufficient funds" } else { "" } } } })
}

#[tokio::test]
async fn charge_webhooks_become_payment_events_for_our_attempt() {
    let (cf, _) = mock().await;
    let now = 1_800_000_000i64;
    let (h, body) = signed(
        "whsec_fixture",
        now * 1000,
        &payment_event("SUBSCRIPTION_PAYMENT_SUCCESS", "CHARGE", "SUCCESS"),
    );
    let ev = cf.verify_webhook(b"whsec_fixture", &h, &body, now).unwrap();
    assert_eq!(ev.kind, EventKind::PaymentCaptured);
    assert_eq!(ev.order_ref.as_deref(), Some("att_1"));
    assert_eq!(ev.provider_payment_ref.as_deref(), Some("7002"));
    assert_eq!(ev.amount_minor, Some(149_900));

    let (h, body) = signed(
        "whsec_fixture",
        now * 1000,
        &payment_event("SUBSCRIPTION_PAYMENT_FAILED", "CHARGE", "FAILED"),
    );
    let ev = cf.verify_webhook(b"whsec_fixture", &h, &body, now).unwrap();
    assert_eq!(ev.kind, EventKind::PaymentFailed);
    assert_eq!(ev.failure_reason.as_deref(), Some("insufficient funds"));

    // redelivery dedupes to the same key
    let again = cf.verify_webhook(b"whsec_fixture", &h, &body, now).unwrap();
    assert_eq!(again.event_id, ev.event_id);
}

#[tokio::test]
async fn authorization_and_status_webhooks_never_touch_a_payment_attempt() {
    let (cf, _) = mock().await;
    let now = 1_800_000_000i64;
    for v in [
        payment_event("SUBSCRIPTION_PAYMENT_SUCCESS", "AUTH", "SUCCESS"),
        payment_event("SUBSCRIPTION_AUTH_STATUS", "AUTH", "SUCCESS"),
        payment_event(
            "SUBSCRIPTION_PAYMENT_NOTIFICATION_INITIATED",
            "CHARGE",
            "INITIALIZED",
        ),
        json!({ "type": "SUBSCRIPTION_STATUS_CHANGED", "event_time": "t", "data": { "subscription_details": { "subscription_id": "mnd_1", "subscription_status": "ACTIVE" } } }),
    ] {
        let (h, body) = signed("whsec_fixture", now * 1000, &v);
        let ev = cf.verify_webhook(b"whsec_fixture", &h, &body, now).unwrap();
        assert_eq!(ev.kind, EventKind::Other, "{v}");
    }
}

#[tokio::test]
async fn a_forged_or_stale_subscription_webhook_is_refused() {
    let (cf, _) = mock().await;
    let now = 1_800_000_000i64;
    let ev = payment_event("SUBSCRIPTION_PAYMENT_SUCCESS", "CHARGE", "SUCCESS");
    let (h, body) = signed("someone_else", now * 1000, &ev);
    assert_eq!(
        cf.verify_webhook(b"whsec_fixture", &h, &body, now)
            .unwrap_err(),
        WebhookError::BadSignature
    );
    let (h, body) = signed("whsec_fixture", (now - 3600) * 1000, &ev);
    assert_eq!(
        cf.verify_webhook(b"whsec_fixture", &h, &body, now)
            .unwrap_err(),
        WebhookError::Stale
    );
}
