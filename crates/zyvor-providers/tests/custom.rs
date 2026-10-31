//! The generic HTTP provider against a local stub gateway (SIMULATED: never a live gateway).
//! The stub is the reference implementation of `docs/zyvor/PROVIDER_CONTRACT.md`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use zyvor_core::Currency;
use zyvor_providers::{custom::Custom, *};

const KEY: &str = "gw_key";
const SECRET: &[u8] = b"whsec";

#[derive(Default)]
struct Gw {
    payments: HashMap<String, Value>,
    /// Answer every payment call with a 503, as a gateway that fell over mid-request would.
    down: bool,
    /// Answer with a status the contract does not define.
    garbled: bool,
}
type Shared = Arc<Mutex<Gw>>;

fn authed(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {KEY}"))
}

async fn create(
    State(g): State<Shared>,
    h: HeaderMap,
    Json(b): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if !authed(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut g = g.lock().unwrap();
    if g.down {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let id = b["attempt_id"].as_str().unwrap().to_string();
    let status = if g.garbled { "weird" } else { "pending" };
    let p = json!({
        "status": status,
        "provider_payment_ref": format!("gw_{id}"),
        "action": { "type": "upi_collect", "vpa": b["vpa"] },
    });
    g.payments.insert(id, p.clone());
    Ok(Json(p))
}

async fn lookup(
    State(g): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    g.lock()
        .unwrap()
        .payments
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn capture(Path(r): Path<String>) -> Json<Value> {
    Json(json!({ "status": "captured", "provider_payment_ref": r }))
}

async fn refund(Json(b): Json<Value>) -> Json<Value> {
    Json(
        json!({ "status": "processed", "provider_refund_ref": format!("rf_{}", b["refund_id"].as_str().unwrap()) }),
    )
}

async fn refund_get(Path(id): Path<String>) -> Json<Value> {
    Json(json!({ "status": "pending", "provider_refund_ref": format!("rf_{id}") }))
}

async fn settlements() -> Json<Value> {
    Json(json!({ "settlements": [{
        "provider_ref": "st_1", "utr": "UTR1", "currency": "INR", "settled_on": "2026-10-30", "net_minor": 49000,
        "items": [
            { "kind": "payment", "provider_ref": "gw_a", "amount_minor": 49900, "description": "payment" },
            { "kind": "fee", "provider_ref": null, "amount_minor": -900, "description": "gateway fee" },
        ],
    }]}))
}

async fn stub() -> (Custom, Shared, String) {
    let g: Shared = Default::default();
    let app = Router::new()
        .route("/payments", post(create))
        .route("/payments/{id}", get(lookup))
        .route("/payments/{id}/capture", post(capture))
        .route("/refunds", post(refund))
        .route("/refunds/{id}", get(refund_get))
        .route("/settlements", get(settlements))
        .with_state(g.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (
        Custom::new(url.clone(), KEY, Duration::from_secs(2)),
        g,
        url,
    )
}

fn payment(attempt: &str) -> CreatePayment {
    CreatePayment {
        attempt_id: attempt.into(),
        amount_minor: 49900,
        currency: Currency::Inr,
        method: Method::Upi,
        vpa: Some("asha@okzyvor".into()),
        customer_id: None,
        customer_phone: None,
        capture_automatic: true,
        return_url: None,
        upi_intent: false,
        client: None,
    }
}

fn sign(ts: &str, body: &[u8]) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(SECRET).unwrap();
    m.update(ts.as_bytes());
    m.update(b".");
    m.update(body);
    hex::encode(m.finalize().into_bytes())
}

fn hook_headers(ts: i64, sig: &str) -> Headers {
    [
        ("x-zyvor-timestamp".to_string(), ts.to_string()),
        ("x-zyvor-signature".to_string(), sig.to_string()),
    ]
    .into()
}

const NOW: i64 = 1_793_000_000;

#[tokio::test]
async fn initiate_starts_the_payment_and_returns_what_the_payer_must_do() {
    let (c, _g, _url) = stub().await;
    let r = c.initiate(&payment("att_1")).await.unwrap();
    assert_eq!(r.state.status, PaymentStatus::Pending);
    assert_eq!(r.state.provider_order_ref.as_deref(), Some("att_1"));
    assert_eq!(r.state.provider_payment_ref.as_deref(), Some("gw_att_1"));
    assert!(
        matches!(r.action, ClientAction::UpiCollect { vpa } if vpa == "asha@okzyvor"),
        "the gateway's action is passed through"
    );
}

#[tokio::test]
async fn a_gateway_error_is_unknown_and_a_refused_connection_is_a_definite_no() {
    let (c, g, _url) = stub().await;
    g.lock().unwrap().down = true;
    assert!(
        matches!(
            c.initiate(&payment("att_2")).await,
            Err(ProviderError::Uncertain(_))
        ),
        "a 5xx may have happened: never a failure, never retried elsewhere"
    );

    let nowhere = Custom::new("http://127.0.0.1:9", KEY, Duration::from_secs(1));
    assert!(
        matches!(
            nowhere.initiate(&payment("att_3")).await,
            Err(ProviderError::Rejected(_))
        ),
        "a refused connection never reached the gateway"
    );
}

#[tokio::test]
async fn bad_credentials_are_rejected_and_an_unreadable_status_is_unknown() {
    let (_c, _g, url) = stub().await;
    let bad = Custom::new(url, "not-the-key", Duration::from_secs(2));
    assert!(matches!(
        bad.initiate(&payment("att_4")).await,
        Err(ProviderError::Rejected(_))
    ));

    let (c, g, _url) = stub().await;
    g.lock().unwrap().garbled = true;
    assert!(
        matches!(
            c.initiate(&payment("att_5")).await,
            Err(ProviderError::Uncertain(_))
        ),
        "a status the contract does not define must not be read as success or failure"
    );
}

#[tokio::test]
async fn a_lost_response_is_resolved_by_asking_with_our_attempt_id() {
    let (c, _g, _url) = stub().await;
    // The gateway never saw this attempt: a definite failed, so the worker can close it out.
    let none = c.fetch_payment("att_never").await.unwrap();
    assert_eq!(none.status, PaymentStatus::Failed);
    assert_eq!(none.failure_reason.as_deref(), Some("not_found"));

    // The gateway did see it, but our response was lost: asking returns the truth.
    c.initiate(&payment("att_6")).await.unwrap();
    let found = c.fetch_payment("att_6").await.unwrap();
    assert_eq!(found.status, PaymentStatus::Pending);
    assert_eq!(found.provider_payment_ref.as_deref(), Some("gw_att_6"));
}

#[tokio::test]
async fn capture_refund_and_settlements_round_trip() {
    let (c, _g, _url) = stub().await;
    let cap = c
        .capture("att_7", "gw/odd ref", 49900, Currency::Inr)
        .await
        .unwrap();
    assert_eq!(cap.status, PaymentStatus::Captured);
    assert_eq!(
        cap.provider_payment_ref.as_deref(),
        Some("gw/odd ref"),
        "a reference with unsafe characters survives the path"
    );

    let rf = c
        .refund(&RefundRequest {
            refund_id: "rfd_1".into(),
            attempt_id: "att_7".into(),
            provider_payment_ref: "gw_att_7".into(),
            amount_minor: 10000,
            currency: Currency::Inr,
        })
        .await
        .unwrap();
    assert_eq!(rf.status, RefundStatus::Processed);
    assert_eq!(rf.provider_refund_ref.as_deref(), Some("rf_rfd_1"));
    let again = c.fetch_refund("att_7", "rfd_1", "gw_att_7").await.unwrap();
    assert_eq!(again.status, RefundStatus::Pending);

    let s = c
        .fetch_settlements(chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap())
        .await
        .unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].net_minor, 49000);
    assert_eq!(
        s[0].items.iter().map(|i| i.amount_minor).sum::<i64>(),
        49000,
        "the items explain the net"
    );
}

#[tokio::test]
async fn webhooks_are_verified_before_they_are_read() {
    let (c, _g, _url) = stub().await;
    let body = serde_json::to_vec(&json!({
        "event_id": "evt_1", "type": "payment.captured", "attempt_id": "att_8",
        "provider_payment_ref": "gw_att_8", "amount_minor": 49900,
    }))
    .unwrap();
    let ts = NOW.to_string();

    let ev = c
        .verify_webhook(
            SECRET,
            &hook_headers(NOW, &sign(&ts, &body)),
            &body,
            NOW + 5,
        )
        .unwrap();
    assert_eq!(ev.kind, EventKind::PaymentCaptured);
    assert_eq!(ev.event_id, "evt_1");
    assert_eq!(ev.order_ref.as_deref(), Some("att_8"));
    assert_eq!(ev.amount_minor, Some(49900));

    // Wrong secret, tampered body, missing headers and an old timestamp are all refused.
    let wrong = c.verify_webhook(b"other", &hook_headers(NOW, &sign(&ts, &body)), &body, NOW);
    assert_eq!(wrong.unwrap_err(), WebhookError::BadSignature);
    let tampered = c.verify_webhook(
        SECRET,
        &hook_headers(NOW, &sign(&ts, &body)),
        b"{\"event_id\":\"evt_1\",\"type\":\"payment.failed\"}",
        NOW,
    );
    assert_eq!(tampered.unwrap_err(), WebhookError::BadSignature);
    assert_eq!(
        c.verify_webhook(SECRET, &Headers::new(), &body, NOW)
            .unwrap_err(),
        WebhookError::MissingSignature
    );
    let stale = c.verify_webhook(
        SECRET,
        &hook_headers(NOW, &sign(&ts, &body)),
        &body,
        NOW + 3600,
    );
    assert_eq!(stale.unwrap_err(), WebhookError::Stale);
}

#[tokio::test]
async fn a_signed_webhook_without_an_event_id_cannot_be_deduplicated_so_it_is_refused() {
    let (c, _g, _url) = stub().await;
    let body = br#"{"type":"payment.captured","attempt_id":"att_9"}"#;
    let ts = NOW.to_string();
    let r = c.verify_webhook(SECRET, &hook_headers(NOW, &sign(&ts, body)), body, NOW);
    assert!(matches!(r, Err(WebhookError::Malformed(_))));
}
