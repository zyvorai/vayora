//! End-to-end tests against real Postgres (each test gets a fresh database via sqlx::test).

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use vayora_api::{AppState, app};
use vayora_upi::SimulatedUpi;

const KEY: &str = "test_key";

fn router(pool: &PgPool) -> Router {
    app(AppState::new(pool.clone(), Arc::new(SimulatedUpi), KEY))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    idem: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    req = req.header("authorization", format!("Bearer {KEY}"));
    if let Some(k) = idem {
        req = req.header("idempotency-key", k);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn order(app: &Router, key: &str, paise: i64) -> String {
    let (s, b) = call(
        app,
        "POST",
        "/v1/orders",
        Some(key),
        json!({ "amount_paise": paise }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["id"].as_str().unwrap().to_owned()
}

async fn balance(app: &Router) -> (i64, i64) {
    let (_, b) = call(app, "GET", "/v1/balance", None, Value::Null).await;
    (
        b["merchant_payable_paise"].as_i64().unwrap(),
        b["upi_clearing_paise"].as_i64().unwrap(),
    )
}

async fn assert_ledger_balanced(pool: &PgPool) {
    let (d, c): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_paise) FILTER (WHERE side='debit'),0)::BIGINT,
                COALESCE(SUM(amount_paise) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(d, c, "ledger must always balance");
}

#[sqlx::test(migrations = "./migrations")]
async fn pay_then_partial_and_full_refund(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 50_000).await;

    let (s, pay) = call(
        &app,
        "POST",
        "/v1/payments",
        Some("p1"),
        json!({ "order_id": oid, "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{pay}");
    assert_eq!(pay["status"], "captured");
    assert_eq!(pay["upi_ref"].as_str().unwrap().len(), 12);
    let (_, o) = call(&app, "GET", &format!("/v1/orders/{oid}"), None, Value::Null).await;
    assert_eq!(o["status"], "paid");
    assert_eq!(balance(&app).await, (50_000, 50_000));

    let pid = pay["id"].as_str().unwrap();
    let (s, r1) = call(
        &app,
        "POST",
        &format!("/v1/payments/{pid}/refunds"),
        Some("r1"),
        json!({ "amount_paise": 20_000 }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{r1}");
    assert_eq!(balance(&app).await, (30_000, 30_000));

    // Over-refund is rejected and changes nothing.
    let (s, e) = call(
        &app,
        "POST",
        &format!("/v1/payments/{pid}/refunds"),
        Some("r2"),
        json!({ "amount_paise": 30_001 }),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "refund_exceeds_payment");

    // Omitted amount refunds the remainder.
    let (s, r3) = call(
        &app,
        "POST",
        &format!("/v1/payments/{pid}/refunds"),
        Some("r3"),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r3["amount_paise"], 30_000);
    assert_eq!(balance(&app).await, (0, 0));
    let (_, p) = call(
        &app,
        "GET",
        &format!("/v1/payments/{pid}"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(p["refunded_paise"], 50_000);
    assert_ledger_balanced(&pool).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn failed_payment_leaves_order_payable_and_ledger_untouched(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 9_900).await;

    let (s, p) = call(
        &app,
        "POST",
        "/v1/payments",
        Some("p1"),
        json!({ "order_id": oid, "vpa": "fail.me@okvayora" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(p["status"], "failed");
    assert_eq!(p["failure_reason"], "insufficient_funds");
    assert_eq!(balance(&app).await, (0, 0));

    // Retry with a good VPA succeeds.
    let (_, p2) = call(
        &app,
        "POST",
        "/v1/payments",
        Some("p2"),
        json!({ "order_id": oid, "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(p2["status"], "captured");
    assert_eq!(balance(&app).await, (9_900, 9_900));

    // A failed payment cannot be refunded.
    let (s, e) = call(
        &app,
        "POST",
        &format!("/v1/payments/{}/refunds", p["id"].as_str().unwrap()),
        Some("r1"),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(e["error"]["code"], "payment_not_captured");
}

#[sqlx::test(migrations = "./migrations")]
async fn idempotency_replays_and_detects_misuse(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 1_000).await;
    let body = json!({ "order_id": oid, "vpa": "asha@okvayora" });

    let (s1, b1) = call(&app, "POST", "/v1/payments", Some("same"), body.clone()).await;
    let (s2, b2) = call(&app, "POST", "/v1/payments", Some("same"), body.clone()).await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert_eq!(b1, b2, "replay returns the stored response");
    assert_eq!(balance(&app).await, (1_000, 1_000), "charged exactly once");

    let (s, e) = call(
        &app,
        "POST",
        "/v1/payments",
        Some("same"),
        json!({ "order_id": oid, "vpa": "other@okvayora" }),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "idempotency_key_reused");

    let (s, e) = call(&app, "POST", "/v1/payments", None, body).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(e["error"]["code"], "idempotency_key_required");
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_duplicate_requests_charge_once(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 7_000).await;
    let body = json!({ "order_id": oid, "vpa": "asha@okvayora" });

    let calls = (0..8).map(|_| {
        let (app, body) = (app.clone(), body.clone());
        tokio::spawn(async move { call(&app, "POST", "/v1/payments", Some("race"), body).await })
    });
    let results = futures_collect(calls).await;
    assert!(
        results.iter().all(|(s, _)| *s == StatusCode::CREATED),
        "{results:?}"
    );
    assert!(results.windows(2).all(|w| w[0].1 == w[1].1));
    assert_eq!(balance(&app).await, (7_000, 7_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_payments_with_different_keys_capture_once(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 7_000).await;
    let calls = (0..6).map(|i| {
        let (app, oid) = (app.clone(), oid.clone());
        tokio::spawn(async move {
            call(
                &app,
                "POST",
                "/v1/payments",
                Some(&format!("k{i}")),
                json!({ "order_id": oid, "vpa": "asha@okvayora" }),
            )
            .await
        })
    });
    let results = futures_collect(calls).await;
    assert_eq!(
        results
            .iter()
            .filter(|(s, _)| *s == StatusCode::CREATED)
            .count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .filter(|(s, _)| *s != StatusCode::CREATED)
            .all(|(s, _)| *s == StatusCode::CONFLICT)
    );
    assert_eq!(balance(&app).await, (7_000, 7_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn auth_and_validation(pool: PgPool) {
    let app = router(&pool);
    let res = app
        .clone()
        .oneshot(
            Request::post("/v1/orders")
                .header("content-type", "application/json")
                .header("idempotency-key", "k")
                .body(Body::from("{\"amount_paise\":1}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = app
        .clone()
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/orders",
            Some("a"),
            json!({ "amount_paise": 0 })
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let oid = order(&app, "o", 100).await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/payments",
            Some("b"),
            json!({ "order_id": oid, "vpa": "nope" })
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/v1/payments",
            Some("c"),
            json!({ "order_id": "order_missing", "vpa": "a1@upi" })
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn ledger_is_append_only(pool: PgPool) {
    let app = router(&pool);
    let oid = order(&app, "o1", 100).await;
    call(
        &app,
        "POST",
        "/v1/payments",
        Some("p"),
        json!({ "order_id": oid, "vpa": "asha@okvayora" }),
    )
    .await;
    assert!(
        sqlx::query("UPDATE ledger_entries SET amount_paise = 1")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM ledger_entries")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn webhooks_are_signed_retried_and_delivered(pool: PgPool) {
    // Receiver that rejects the first delivery, then accepts.
    type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
    let seen: Seen = Default::default();
    let hits = Arc::new(Mutex::new(0u32));
    let recv = {
        let (seen, hits) = (seen.clone(), hits.clone());
        Router::new().route(
            "/hook",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let (seen, hits) = (seen.clone(), hits.clone());
                    async move {
                        let mut n = hits.lock().unwrap();
                        *n += 1;
                        if *n == 1 {
                            return StatusCode::INTERNAL_SERVER_ERROR;
                        }
                        let sig = headers
                            .get("vayora-signature")
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_owned();
                        seen.lock().unwrap().push((sig, body.to_vec()));
                        StatusCode::OK
                    }
                },
            ),
        )
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, recv).await.unwrap() });

    let app = router(&pool);
    let oid = order(&app, "o1", 2_500).await;
    call(
        &app,
        "POST",
        "/v1/payments",
        Some("p"),
        json!({ "order_id": oid, "vpa": "asha@okvayora" }),
    )
    .await;

    let d = vayora_webhooks::Dispatcher::new(pool.clone(), url, b"whsec_test".to_vec());
    assert_eq!(d.run_once().await.unwrap(), 0, "first attempt fails");
    let (attempts, status): (i32, String) =
        sqlx::query_as("SELECT attempts, status FROM webhook_events")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((attempts, status.as_str()), (1, "pending"));

    // Backoff keeps it out of the queue until due...
    assert_eq!(d.run_once().await.unwrap(), 0);
    // ...so make it due and retry.
    sqlx::query("UPDATE webhook_events SET next_attempt_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(d.run_once().await.unwrap(), 1);

    let (sig, body) = seen.lock().unwrap()[0].clone();
    let now = chrono::Utc::now().timestamp();
    assert!(vayora_webhooks::verify(
        b"whsec_test",
        &sig,
        &body,
        now,
        300
    ));
    let event: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(event["type"], "payment.captured");
    assert_eq!(event["data"]["payment"]["order_id"], oid);
    let status: String = sqlx::query_scalar("SELECT status FROM webhook_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "delivered");
}

async fn futures_collect<T>(handles: impl Iterator<Item = tokio::task::JoinHandle<T>>) -> Vec<T> {
    let handles: Vec<_> = handles.collect();
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await.unwrap());
    }
    out
}
