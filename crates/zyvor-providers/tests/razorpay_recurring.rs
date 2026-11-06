//! Razorpay Recurring (UPI AutoPay) against a LOCAL MOCK HTTP server. Docs-shaped only; not sandbox-verified.

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
use serde_json::{Value, json};
use zyvor_core::Currency;
use zyvor_providers::{razorpay::Razorpay, *};

#[derive(Default)]
struct Inner {
    routes: HashMap<String, (u16, Value)>,
}
type Shared = Arc<Mutex<Inner>>;

async fn handle(
    State(s): State<Shared>,
    m: Method,
    uri: Uri,
    _headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let key = format!("{m} {}", uri.path());
    let g = s.lock().unwrap();
    match g.routes.get(&key) {
        Some((st, b)) => (StatusCode::from_u16(*st).unwrap(), axum::Json(b.clone())),
        None => {
            let _ = body;
            (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": { "description": "not found" } })),
            )
        }
    }
}

async fn mock() -> (Razorpay, Shared) {
    let shared: Shared = Default::default();
    let app = Router::new().fallback(handle).with_state(shared.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (
        Razorpay::new(url, "rzp_test", "secret", Duration::from_secs(3)),
        shared,
    )
}

fn route(s: &Shared, key: &str, status: u16, body: Value) {
    s.lock().unwrap().routes.insert(key.into(), (status, body));
}

#[tokio::test]
async fn create_fetch_debit_and_revoke_against_local_mock() {
    let (rz, s) = mock().await;
    assert!(rz.capabilities().mandates);

    route(
        &s,
        "POST /v1/orders",
        200,
        json!({ "id": "order_reg1", "status": "created", "short_url": "https://rzp.io/i/x" }),
    );
    let mnd = rz
        .create_mandate(&CreateMandate {
            mandate_id: "mnd_1".into(),
            max_amount_minor: 15_000_00,
            currency: Currency::Inr,
            frequency: Frequency::Monthly,
            start_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            end_date: chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            customer_id: None,
            customer_phone: Some("9999999999".into()),
        })
        .await
        .unwrap();
    assert_eq!(mnd.status, MandateStatus::Pending);
    assert_eq!(mnd.provider_mandate_ref.as_deref(), Some("order_reg1"));

    route(
        &s,
        "GET /v1/orders",
        200,
        json!({
            "items": [{
                "id": "order_reg1",
                "status": "paid",
                "token": { "id": "token_abc", "status": "confirmed" }
            }]
        }),
    );
    // query string is ignored by our path-only mock key — register exact path used by client
    // fetch uses `/v1/orders?receipt=mnd_1` which our fallback keys as GET /v1/orders
    let fetched = rz.fetch_mandate("mnd_1").await.unwrap();
    assert_eq!(fetched.status, MandateStatus::Active);
    assert_eq!(fetched.provider_mandate_ref.as_deref(), Some("token_abc"));

    route(
        &s,
        "POST /v1/orders",
        200,
        json!({ "id": "order_debit1", "status": "created" }),
    );
    route(
        &s,
        "POST /v1/payments/create/recurring",
        200,
        json!({ "razorpay_payment_id": "pay_1", "status": "captured" }),
    );
    let debit = rz
        .debit(&DebitRequest {
            attempt_id: "att_d1".into(),
            mandate_id: "mnd_1".into(),
            provider_mandate_ref: "token_abc".into(),
            amount_minor: 49900,
            currency: Currency::Inr,
        })
        .await
        .unwrap();
    assert_eq!(debit.state.status, PaymentStatus::Captured);

    route(&s, "DELETE /v1/tokens/token_abc", 200, json!({ "deleted": true }));
    // revoke fetches first — keep GET /v1/orders returning the token
    let rev = rz.revoke_mandate("mnd_1").await.unwrap();
    assert_eq!(rev.status, MandateStatus::Revoked);
}
