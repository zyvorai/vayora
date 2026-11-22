// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! PayU through the whole API against the local PayU simulation (SIMULATED: built from PayU's public documentation,
//! never run against PayU's sandbox or a live account). Covers what the adapter tests cannot: the provider account
//! (and its migration), the attempt record, the webhook route with a form-encoded body, de-duplication, and a refund
//! that is queued first and finished later.

mod common;

use axum::{body::Body, http::Request};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const RETURN_URL: &str = "https://merchant.example/done";

async fn connect(t: &T, tn: &Tenant) {
    let (s, b) = t
        .call(
            tn,
            "PUT",
            "/v1/provider_accounts/payu",
            None,
            json!({
                // obviously fake: the simulation's own defaults
                "credentials": { "key": "payu_test_key", "salt": "payu_test_salt", "merchant_secret": "payu_test_merchant_secret" },
                "webhook_secret": "unused-payu-signs-with-the-salt",
                "base_url": t.mock,
            }),
        )
        .await;
    assert!(s.is_success(), "{b}");
}

fn form(fields: &Value) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in fields.as_object().unwrap() {
        s.append_pair(k, v.as_str().unwrap());
    }
    s.finish()
}

async fn webhook(t: &T, tenant_id: &str, body: String) -> (axum::http::StatusCode, Value) {
    let res = t
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/webhooks/payu/{tenant_id}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payu_payment_is_paid_by_form_webhook_and_partly_refunded(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    connect(&t, a).await;

    let intent = t
        .intent(a, 50_000, json!({ "allowed_providers": ["payu"] }))
        .await;
    let iid = intent["id"].as_str().unwrap();
    let (s, att) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("payu-att-1"),
            json!({ "method": "card", "return_url": RETURN_URL }),
        )
        .await;
    assert_eq!(s, axum::http::StatusCode::CREATED, "{att}");
    assert_eq!(
        (att["provider"].as_str(), att["status"].as_str()),
        (Some("payu"), Some("pending"))
    );
    let txnid = att["provider_order_ref"].as_str().unwrap();
    assert_eq!(txnid.len(), 25, "PayU's own limit on a transaction id");
    // the payer's device gets a form with a hash; the salt and the merchant secret never leave the server
    assert_eq!(att["client_action"]["type"], "hosted_checkout");
    let shown = att.to_string();
    assert!(
        !shown.contains("payu_test_salt") && !shown.contains("payu_test_merchant_secret"),
        "{shown}"
    );

    // the payer's browser posts the form to PayU (the simulation checks the hash on its own), then pays
    let fields = &att["client_action"]["params"]["fields"];
    let res = t
        .http
        .post(format!("{}/_payment", t.mock))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(fields))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.text().await.unwrap());
    let paid = t
        .mock_post("payu/pay", json!({ "txnid": txnid, "outcome": "captured" }))
        .await;
    let body = paid["webhook"]["body"].as_str().unwrap().to_string();

    // a body changed in transit is refused, and nothing is applied
    let (s, _) = webhook(&t, &a.id, body.replacen("amount=500.00", "amount=5.00", 1)).await;
    assert_eq!(s, axum::http::StatusCode::UNAUTHORIZED);
    // the real one is applied, once
    let (s, out) = webhook(&t, &a.id, body.clone()).await;
    assert_eq!(s, axum::http::StatusCode::OK, "{out}");
    assert_eq!(out["outcome"], "Applied(Captured)", "{out}");
    let (s, again) = webhook(&t, &a.id, body).await;
    assert_eq!(
        (s, again["status"].as_str()),
        (axum::http::StatusCode::OK, Some("duplicate"))
    );
    let (status, pref): (String, Option<String>) =
        sqlx::query_as("SELECT status, provider_payment_ref FROM payment_attempts WHERE id = $1")
            .bind(att["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(status, "captured");
    assert!(
        pref.is_some_and(|p| p.starts_with("4039")),
        "the PayU id (mihpayid) is kept"
    );

    // a partial refund is queued at PayU, then finished
    let rf = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("payu-rf-1"),
            json!({ "amount_minor": 20_000, "reason": "returned" }),
        )
        .await;
    assert_ne!(rf["status"], "processed", "PayU queues refunds: {rf}");
    let rid = rf["id"].as_str().unwrap();
    let ref_id: Option<String> =
        sqlx::query_scalar("SELECT provider_refund_ref FROM refunds WHERE id = $1")
            .bind(rid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    t.mock_post(
        "payu/refund_complete",
        json!({ "request_id": ref_id, "outcome": "success" }),
    )
    .await;
    let done = t
        .ok(
            a,
            "POST",
            &format!("/v1/refunds/{rid}/refresh"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(done["status"], "processed", "{done}");

    // the tenant boundary holds: another business has no PayU account, so the same webhook finds nothing
    let (s, _) = webhook(
        &t,
        &t.b.id,
        paid["webhook"]["body"].as_str().unwrap().to_string(),
    )
    .await;
    assert_eq!(s, axum::http::StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn payu_needs_its_credentials_and_a_return_url(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (s, b) = t
        .call(
            a,
            "PUT",
            "/v1/provider_accounts/payu",
            None,
            json!({ "credentials": { "key": "k" }, "webhook_secret": "x", "base_url": t.mock }),
        )
        .await;
    assert!(
        s.is_client_error(),
        "a PayU account without its salt is refused: {s} {b}"
    );

    connect(&t, a).await;
    let intent = t
        .intent(a, 50_000, json!({ "allowed_providers": ["payu"] }))
        .await;
    let iid = intent["id"].as_str().unwrap();
    let (s, att) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("payu-att-2"),
            json!({ "method": "card" }),
        )
        .await;
    // PayU makes surl and furl mandatory: with no return_url nothing is started, definitively
    assert!(
        att["status"] == "failed" || s.is_client_error(),
        "an attempt PayU cannot start must not stay open: {s} {att}"
    );
}
