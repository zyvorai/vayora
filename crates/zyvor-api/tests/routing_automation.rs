//! Routing automation against the simulators only: shadow mode, soft-decline retry on another provider,
//! cost-aware routing, the kill switch and the provider scorecard.
//!
//! The rule under test above all: a soft retry may start the next attempt only after a DEFINITIVE provider-side
//! failure, and there is no path (API, webhook, resolver, worker, concurrent callers) that creates an attempt
//! while an earlier attempt on the same payment is unresolved. The tests count live attempts and orders at the
//! providers, not just responses.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_api::{
    payments::{Incoming, apply_provider_state},
    softretry,
    state_machine::Attempt,
};

async fn fees(t: &T, tn: &Tenant, rules: Value) {
    t.ok(
        tn,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": {
            "rules": rules,
            "fees": {
                "razorpay": { "card": { "bps": 100, "fixed_minor": 0 }, "upi": { "bps": 100, "fixed_minor": 0 } },
                "cashfree": { "card": { "bps": 200, "fixed_minor": 0 }, "upi": { "bps": 200, "fixed_minor": 0 } }
            } } }),
    )
    .await;
}

async fn auto(t: &T, tn: &Tenant, body: Value) -> Value {
    t.ok(tn, "PUT", "/v1/routing/automation", None, body).await
}

async fn open_intent(t: &T, tn: &Tenant, allowed: Value) -> Value {
    t.intent(tn, 50_000, json!({ "allowed_providers": allowed }))
        .await
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

async fn attempt_as(t: &T, tn: &Tenant, iid: &str, key: &str, body: Value) -> (StatusCode, Value) {
    t.call(
        tn,
        "POST",
        &format!("/v1/payment_intents/{iid}/attempts"),
        Some(key),
        body,
    )
    .await
}

async fn card_attempt(t: &T, tn: &Tenant, iid: &str, key: &str) -> (StatusCode, Value) {
    attempt_as(t, tn, iid, key, json!({ "method": "card" })).await
}

async fn attempts_of(t: &T, iid: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM payment_attempts WHERE intent_id = '{iid}'"
    ))
    .await
}

/// Attempts that are not finished: two of these on one payment would be a double-charge risk.
async fn live_of(t: &T, iid: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM payment_attempts WHERE intent_id = '{iid}' AND status IN ('created', 'pending', 'authorized', 'unknown', 'captured')"
    ))
    .await
}

async fn orders(t: &T) -> i64 {
    let s = t.mock_stats().await;
    s["razorpay_orders"].as_i64().unwrap() + s["cashfree_orders"].as_i64().unwrap()
}

async fn audit_count(t: &T, action: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM audit_events WHERE action = '{action}'"
    ))
    .await
}

async fn outage(t: &T, provider: &str, mode: &str) {
    t.mock_post("outage", json!({ "provider": provider, "mode": mode }))
        .await;
}

/// Put an attempt into a state the way every real path does, with an explicit failure class.
async fn finish(
    t: &T,
    tn: &Tenant,
    attempt_id: &str,
    status: Attempt,
    class: Option<&'static str>,
) {
    if status == Attempt::Unknown {
        // the state machine does not allow pending -> unknown (a real timeout happens while creating); put it there
        sqlx::query(
            "UPDATE payment_attempts SET status = 'unknown', updated_at = now() WHERE id = $1",
        )
        .bind(attempt_id)
        .execute(&t.root)
        .await
        .unwrap();
        return;
    }
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &tn.id)
        .await
        .unwrap();
    apply_provider_state(
        &mut tx,
        &tn.id,
        attempt_id,
        &Incoming {
            status,
            failure_reason: Some("test".into()),
            failure_class: class,
            ..Incoming::default()
        },
        "test",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// A payment whose first attempt is `pending` at the cheaper provider, with soft retry on (so the request is stored).
async fn pending_card(t: &T, tn: &Tenant, allowed: Value, key: &str) -> (String, String) {
    let iid = id_of(&open_intent(t, tn, allowed).await);
    let (s, att) = card_attempt(t, tn, &iid, key).await;
    assert_eq!(s, StatusCode::CREATED, "{att}");
    assert_eq!(att["status"], "pending", "{att}");
    (iid, id_of(&att))
}

async fn soft_rows(t: &T, iid: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT status, reason FROM routing_soft_retries WHERE intent_id = '{iid}' ORDER BY created_at"
    )))
    .fetch_all(&t.root)
    .await
    .unwrap()
}

async fn run_retry(t: &T, tn: &Tenant, iid: &str) {
    softretry::run(&t.state, &tn.id, iid).await;
}

// ---------- soft retry ----------

#[sqlx::test(migrations = "./migrations")]
async fn a_refused_payment_is_retried_once_on_the_other_provider_and_never_twice(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    outage(&t, "razorpay", "reject").await;

    // off by default: the payment just fails, as it always did
    let iid0 = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, first0) = card_attempt(&t, a, &iid0, "off-1").await;
    assert_eq!(first0["status"], "failed");
    assert_eq!(attempts_of(&t, &iid0).await, 1);

    auto(&t, a, json!({ "soft_retry_enabled": true })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let before = orders(&t).await;
    let (s, first) = card_attempt(&t, a, &iid, "on-1").await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    // the response is the attempt that was asked for: it failed at the cheaper provider
    assert_eq!(
        (first["provider"].as_str(), first["status"].as_str()),
        (Some("razorpay"), Some("failed"))
    );

    // ... and the payment now has exactly one more attempt, live, on the other provider
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(live_of(&t, &iid).await, 1);
    let second: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(a) FROM payment_attempts a WHERE intent_id = '{iid}' AND id <> '{}'",
        first["id"].as_str().unwrap()
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(second["provider"], "cashfree");
    assert_eq!(second["status"], "pending");
    assert_eq!(
        second["route"]["soft_retry"]["from_attempt_id"],
        first["id"]
    );
    assert_eq!(
        second["route"]["soft_retry"]["failure_class"],
        "provider_rejected"
    );
    assert_eq!(second["route"]["failover_from"], json!(["razorpay"]));
    assert!(
        second["route"]["soft_retry"]["text"]
            .as_str()
            .unwrap()
            .contains("tried on cashfree instead")
    );
    assert_eq!(
        orders(&t).await,
        before + 1,
        "one order at the other provider"
    );
    assert_eq!(
        soft_rows(&t, &iid).await,
        vec![("started".to_string(), None)]
    );
    assert_eq!(audit_count(&t, "routing.soft_retry_started").await, 1);
    // the payer's request was kept only for the retry
    let kept: Option<Value> =
        sqlx::query_scalar("SELECT retry_input FROM payment_attempts WHERE id = $1")
            .bind(first["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert!(kept.is_none());

    // idempotent: asking again, from anywhere, creates nothing
    for _ in 0..3 {
        run_retry(&t, a, &iid).await;
    }
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(orders(&t).await, before + 1);
    // replaying the original request returns the original attempt, and a manual one is refused while one is live
    let (_, again) = card_attempt(&t, a, &iid, "on-1").await;
    assert_eq!(again["id"], first["id"]);
    let (s, e) = card_attempt(&t, a, &iid, "on-2").await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("attempt_in_progress"))
    );
    assert_eq!(live_of(&t, &iid).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn only_provider_faults_and_refusals_are_retried_never_payer_declines_unknown_causes_or_unresolved_payments(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;

    // not retried: the payer's doing, a cause nobody recognised, a merchant cancellation
    for (i, class) in ["payer_declined", "declined", "cancelled"]
        .into_iter()
        .enumerate()
    {
        let (iid, att) = pending_card(&t, a, Value::Null, &format!("n{i}")).await;
        finish(&t, a, &att, Attempt::Failed, Some(class)).await;
        run_retry(&t, a, &iid).await;
        assert_eq!(
            attempts_of(&t, &iid).await,
            1,
            "{class} must not be retried"
        );
        assert!(soft_rows(&t, &iid).await.is_empty(), "{class}");
    }

    // an unknown outcome is not a failure at all: nothing may start while it is unresolved
    let (iid, att) = pending_card(&t, a, Value::Null, "u1").await;
    finish(&t, a, &att, Attempt::Unknown, None).await;
    let before = orders(&t).await;
    for _ in 0..3 {
        run_retry(&t, a, &iid).await;
    }
    softretry::sweep(&t.state, &a.id).await.unwrap();
    assert_eq!(attempts_of(&t, &iid).await, 1);
    let (s, e) = card_attempt(&t, a, &iid, "u2").await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("payment_outcome_unresolved"))
    );
    assert_eq!(orders(&t).await, before);
    // only when the provider has said "failed" (here: confirmed after the unknown) does another provider get it
    finish(&t, a, &att, Attempt::Failed, None).await;
    let class: Option<String> =
        sqlx::query_scalar("SELECT failure_class FROM payment_attempts WHERE id = $1")
            .bind(&att)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(class.as_deref(), Some("provider_fault"));
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(live_of(&t, &iid).await, 1);

    // a pending attempt blocks everything: not retried, however often it is looked at
    let (iid, _) = pending_card(&t, a, Value::Null, "p1").await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);

    // provider_fault and provider_rejected are the two that are retried
    let (iid, att) = pending_card(&t, a, Value::Null, "f1").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2);
    let (iid, att) = pending_card(&t, a, Value::Null, "f2").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_rejected")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_late_answer_from_the_resolver_starts_the_retry_after_the_unknown_is_settled(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    // UPI: cashfree is cheaper, so it goes first; razorpay is the alternative
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": { "rules": [], "fees": {
            "cashfree": { "upi": { "bps": 100, "fixed_minor": 0 } },
            "razorpay": { "upi": { "bps": 200, "fixed_minor": 0 } } } } }),
    )
    .await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    t.mock_post(
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let (_, unk) = attempt_as(
        &t,
        a,
        &iid,
        "k1",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(
        (unk["provider"].as_str(), unk["status"].as_str()),
        (Some("cashfree"), Some("unknown"))
    );
    let razorpay_orders = t.mock_stats().await["razorpay_orders"].as_i64().unwrap();
    // unresolved: no retry from any path
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);
    let (s, _) = attempt_as(
        &t,
        a,
        &iid,
        "k2",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        t.mock_stats().await["razorpay_orders"].as_i64().unwrap(),
        razorpay_orders
    );

    // the provider answers "failed"; asking it is what settles the attempt, and only then does the retry start
    let id = id_of(&unk);
    t.mock_post(
        "cashfree/pay",
        json!({ "order_id": id, "outcome": "FAILED" }),
    )
    .await;
    let asked = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_attempts/{id}/refresh"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(asked["status"], "failed");
    assert_eq!(attempts_of(&t, &iid).await, 2);
    let (prov, st, vpa_kept): (String, String, Option<Value>) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT provider, status, retry_input FROM payment_attempts WHERE intent_id = '{iid}' AND id <> '{id}'"
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((prov.as_str(), st.as_str()), ("razorpay", "pending"));
    assert!(
        vpa_kept.is_some_and(|v| v["vpa"] == "ravi@ybl"),
        "the retry repeated the payer's UPI ID"
    );
    assert_eq!(live_of(&t, &iid).await, 1);
    assert_eq!(
        t.mock_stats().await["razorpay_orders"].as_i64().unwrap(),
        razorpay_orders + 1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_provider_webhook_and_the_worker_start_retries_but_a_decline_with_no_known_cause_does_not(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": { "rules": [], "fees": {
            "cashfree": { "upi": { "bps": 100, "fixed_minor": 0 } },
            "razorpay": { "upi": { "bps": 200, "fixed_minor": 0 } } } } }),
    )
    .await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;
    let upi = json!({ "method": "upi", "vpa": "ravi@ybl" });

    // the provider says "failed" with no cause we recognise: nothing is retried
    let iid = id_of(&open_intent(&t, a, json!(["razorpay"])).await);
    let (_, att) = attempt_as(&t, a, &iid, "w0", upi.clone()).await;
    let failed = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "failed" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &a.id, &failed["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(attempts_of(&t, &iid).await, 1);

    // an unknown outcome that the provider's webhook later says failed: a provider fault, so it is retried
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    t.mock_post(
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let (_, unk) = attempt_as(&t, a, &iid, "w1", upi.clone()).await;
    assert_eq!(unk["status"], "unknown", "{unk}");
    let hook = t
        .mock_post(
            "cashfree/pay",
            json!({ "order_id": unk["id"], "outcome": "FAILED" }),
        )
        .await;
    let (s, _) = t.deliver("cashfree", &a.id, &hook["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    // the retry runs in the background of the webhook; wait for it
    for _ in 0..50 {
        if attempts_of(&t, &iid).await == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(live_of(&t, &iid).await, 1);
    let to: String =
        sqlx::query_scalar("SELECT to_provider FROM routing_soft_retries WHERE intent_id = $1")
            .bind(&iid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(to, "razorpay");

    // the worker picks up a failure nobody handled (here: recorded while no process was looking)
    let (iid, att) = pending_card(&t, a, Value::Null, "w2").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);
    let st = zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let _ = st;
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(live_of(&t, &iid).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_retry_that_itself_goes_unknown_blocks_every_further_attempt(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": { "rules": [], "fees": {
            "razorpay": { "upi": { "bps": 100, "fixed_minor": 0 } },
            "cashfree": { "upi": { "bps": 200, "fixed_minor": 0 } } } } }),
    )
    .await;
    auto(
        &t,
        a,
        json!({ "soft_retry_enabled": true, "soft_retry_max_attempts": 5 }),
    )
    .await;
    outage(&t, "razorpay", "reject").await;
    t.mock_post(
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, first) = attempt_as(
        &t,
        a,
        &iid,
        "k1",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(first["status"], "failed");
    // the retry went to cashfree and its outcome is unknown
    assert_eq!(attempts_of(&t, &iid).await, 2);
    let retry_status: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT status FROM payment_attempts WHERE intent_id = '{iid}' AND id <> '{}'",
        first["id"].as_str().unwrap()
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(retry_status, "unknown");
    let before = orders(&t).await;
    for _ in 0..5 {
        run_retry(&t, a, &iid).await;
    }
    softretry::sweep(&t.state, &a.id).await.unwrap();
    let (s, e) = attempt_as(
        &t,
        a,
        &iid,
        "k2",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("payment_outcome_unresolved"))
    );
    assert_eq!(
        attempts_of(&t, &iid).await,
        2,
        "no third attempt exists anywhere"
    );
    assert_eq!(orders(&t).await, before);
    assert_eq!(live_of(&t, &iid).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn limits_alternates_degraded_providers_and_the_window_stop_a_retry_and_the_decision_is_recorded_once(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;

    // no other eligible provider (the payment allows only one)
    let (iid, att) = pending_card(&t, a, json!(["razorpay"]), "o1").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);
    let rows = soft_rows(&t, &iid).await;
    assert_eq!(rows.len(), 1, "decided once");
    assert_eq!(rows[0].0, "skipped");
    assert!(
        rows[0]
            .1
            .as_deref()
            .unwrap()
            .contains("no other eligible provider"),
        "{rows:?}"
    );
    assert_eq!(audit_count(&t, "routing.soft_retry_skipped").await, 1);

    // the attempt limit counts the first attempt: two by default
    let (iid, att) = pending_card(&t, a, Value::Null, "l1").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2);
    let second: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM payment_attempts WHERE intent_id = '{iid}' AND id <> '{att}'"
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    // the retry itself needs its request stored; it was, because soft retry was on
    finish(&t, a, &second, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2, "limit of 2 attempts");
    let rows = soft_rows(&t, &iid).await;
    assert_eq!(rows.len(), 2);
    assert!(
        rows[1].1.as_deref().unwrap().contains("limit 2"),
        "{rows:?}"
    );

    // a higher limit still cannot use a provider already tried for this payment
    auto(&t, a, json!({ "soft_retry_max_attempts": 4 })).await;
    let (iid, att) = pending_card(&t, a, Value::Null, "l2").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    let second: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT id FROM payment_attempts WHERE intent_id = '{iid}' AND id <> '{att}'"
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    finish(&t, a, &second, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 2, "both providers were tried");
    assert!(
        soft_rows(&t, &iid).await[1]
            .1
            .as_deref()
            .unwrap()
            .contains("no other eligible provider")
    );

    // the window runs from the first attempt
    auto(&t, a, json!({ "soft_retry_window_seconds": 30 })).await;
    let (iid, att) = pending_card(&t, a, Value::Null, "w1").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    sqlx::query("UPDATE payment_attempts SET created_at = now() - interval '1 hour' WHERE id = $1")
        .bind(&att)
        .execute(&t.root)
        .await
        .unwrap();
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);
    assert!(
        soft_rows(&t, &iid).await[0]
            .1
            .as_deref()
            .unwrap()
            .contains("window")
    );

    // an alternate that the circuit breaker has degraded is not a retry target
    sqlx::query("INSERT INTO provider_health_state (tenant_id, provider, method, state, reason, degraded_since) VALUES ($1, 'cashfree', 'card', 'degraded', 'test', now())")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    let (iid, att) = pending_card(&t, a, Value::Null, "d1").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    assert_eq!(attempts_of(&t, &iid).await, 1);
    assert!(
        soft_rows(&t, &iid).await[0]
            .1
            .as_deref()
            .unwrap()
            .contains("degraded")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn many_callers_racing_to_retry_one_failure_create_exactly_one_attempt_and_never_two_live(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;

    for round in 0..3 {
        let (iid, att) = pending_card(&t, a, Value::Null, &format!("r{round}")).await;
        finish(&t, a, &att, Attempt::Failed, Some("provider_fault")).await;
        let before = orders(&t).await;
        let mut tasks = Vec::new();
        // eight retriers (the worker, webhooks and requests all land here) ...
        for _ in 0..8 {
            let (st, tid, iid) = (t.state.clone(), a.id.clone(), iid.clone());
            tasks.push(tokio::spawn(async move {
                softretry::run(&st, &tid, &iid).await
            }));
        }
        // ... and four people retrying by hand at the same moment
        let mut manual = Vec::new();
        for m in 0..4 {
            let (app, key, iid) = (t.app.clone(), a.key.clone(), iid.clone());
            manual.push(tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    None,
                    "POST",
                    &format!("/v1/payment_intents/{iid}/attempts"),
                    Some(&format!("manual-{round}-{m}")),
                    json!({ "method": "card" }),
                )
                .await
                .0
            }));
        }
        for h in tasks {
            h.await.unwrap();
        }
        let mut created = 0;
        for h in manual {
            let s = h.await.unwrap();
            assert!(s == StatusCode::CREATED || s == StatusCode::CONFLICT, "{s}");
            created += (s == StatusCode::CREATED) as i32;
        }
        assert_eq!(
            attempts_of(&t, &iid).await,
            2,
            "round {round}: the failed attempt plus exactly one"
        );
        assert_eq!(live_of(&t, &iid).await, 1, "round {round}");
        assert_eq!(
            orders(&t).await,
            before + 1,
            "round {round}: one order at one provider"
        );
        assert!(
            created <= 1,
            "round {round}: at most one manual attempt got through"
        );
    }

    // the same failure retried only by retriers: still exactly one, and one soft-retry row
    let (iid, att) = pending_card(&t, a, Value::Null, "only").await;
    finish(&t, a, &att, Attempt::Failed, Some("provider_rejected")).await;
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let (st, tid, iid) = (t.state.clone(), a.id.clone(), iid.clone());
        tasks.push(tokio::spawn(async move {
            softretry::run(&st, &tid, &iid).await
        }));
    }
    for h in tasks {
        h.await.unwrap();
    }
    assert_eq!(attempts_of(&t, &iid).await, 2);
    assert_eq!(soft_rows(&t, &iid).await.len(), 1);
    assert_eq!(live_of(&t, &iid).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_checkout_says_trying_another_way_while_a_retry_is_due_and_never_offers_a_second_payment(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    auto(&t, a, json!({ "soft_retry_enabled": true })).await;
    let intent = open_intent(&t, a, Value::Null).await;
    let (iid, secret) = (
        id_of(&intent),
        intent["client_secret"].as_str().unwrap().to_string(),
    );
    let path = format!("/v1/checkout/{}/{iid}?secret={secret}", a.id);
    let session = || async { t.ok_public("GET", &path, Value::Null).await };

    let (_, att) = card_attempt(&t, a, &iid, "c1").await;
    assert_eq!(session().await["phase"], "awaiting_payment");
    // the provider says the payment failed; the retry has not run yet
    finish(&t, a, &id_of(&att), Attempt::Failed, Some("provider_fault")).await;
    let s = session().await;
    assert_eq!(s["phase"], "trying_another_way", "{s}");
    assert_eq!(s["retried_on_another_provider"], false);
    // ... then it has: one new attempt, awaiting the payer, and the page says it was a second try
    run_retry(&t, a, &iid).await;
    let s = session().await;
    assert_eq!(s["phase"], "awaiting_payment", "{s}");
    assert_eq!(s["retried_on_another_provider"], true);
    assert_eq!(live_of(&t, &iid).await, 1);

    // a payment with nowhere else to go is "failed" at once, not "trying another way" for ever
    let intent = open_intent(&t, a, json!(["razorpay"])).await;
    let (iid, secret) = (
        id_of(&intent),
        intent["client_secret"].as_str().unwrap().to_string(),
    );
    let path = format!("/v1/checkout/{}/{iid}?secret={secret}", a.id);
    let (_, att) = card_attempt(&t, a, &iid, "c2").await;
    finish(&t, a, &id_of(&att), Attempt::Failed, Some("provider_fault")).await;
    assert_eq!(
        t.ok_public("GET", &path, Value::Null).await["phase"],
        "trying_another_way"
    );
    run_retry(&t, a, &iid).await;
    assert_eq!(
        t.ok_public("GET", &path, Value::Null).await["phase"],
        "failed"
    );
    // and a payer decline is never "trying another way"
    let intent = open_intent(&t, a, Value::Null).await;
    let (iid, secret) = (
        id_of(&intent),
        intent["client_secret"].as_str().unwrap().to_string(),
    );
    let path = format!("/v1/checkout/{}/{iid}?secret={secret}", a.id);
    let (_, att) = card_attempt(&t, a, &iid, "c3").await;
    finish(&t, a, &id_of(&att), Attempt::Failed, Some("payer_declined")).await;
    assert_eq!(
        t.ok_public("GET", &path, Value::Null).await["phase"],
        "failed"
    );
}

// ---------- shadow mode ----------

#[sqlx::test(migrations = "./migrations")]
async fn shadow_records_what_a_candidate_policy_would_do_next_to_what_was_applied_and_changes_nothing(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    fees(&t, a, json!([])).await;
    fees(&t, b, json!([])).await;

    // off by default: nothing recorded
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    card_attempt(&t, a, &iid, "off0").await;
    assert_eq!(
        t.count("SELECT count(*) FROM routing_shadow_decisions")
            .await,
        0
    );

    // candidate policy: prefer the dearer provider (cashfree). Live routing keeps choosing the cheaper one.
    auto(
        &t,
        a,
        json!({ "shadow_enabled": true, "shadow_policy": { "rules": [ { "method": "card", "prefer": ["cashfree"] } ], "cost_aware": false } }),
    )
    .await;
    let cashfree_before = t.mock_stats().await["cashfree_orders"].as_i64().unwrap();
    let mut ids = Vec::new();
    for i in 0..4 {
        let iid = id_of(&open_intent(&t, a, Value::Null).await);
        let (st, att) = card_attempt(&t, a, &iid, &format!("s{i}")).await;
        assert_eq!(st, StatusCode::CREATED, "{att}");
        assert_eq!(
            att["provider"], "razorpay",
            "shadow never changes routing: {}",
            att["route"]
        );
        assert!(
            att["route"]["explanation"]
                .as_str()
                .unwrap()
                .contains("lowest estimated fee")
        );
        ids.push(id_of(&att));
    }
    assert_eq!(
        t.mock_stats().await["cashfree_orders"].as_i64().unwrap(),
        cashfree_before,
        "shadow never calls a provider"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payment_attempts WHERE provider = 'cashfree'")
            .await,
        0
    );
    let (agree, sp, sx): (bool, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT agree, shadow_provider, shadow_explanation FROM routing_shadow_decisions WHERE attempt_id = $1",
    )
    .bind(&ids[0])
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert!(!agree);
    assert_eq!(sp.as_deref(), Some("cashfree"));
    assert!(sx.unwrap().contains("preferred by your routing rule"));

    // a policy that agrees with the live rules records agreement
    auto(&t, a, json!({ "shadow_policy": { "cost_aware": false } })).await;
    for i in 0..2 {
        let iid = id_of(&open_intent(&t, a, Value::Null).await);
        card_attempt(&t, a, &iid, &format!("g{i}")).await;
    }

    let rep = t
        .ok(
            a,
            "GET",
            "/v1/routing/shadow/report?days=7",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(rep["decisions"], 6, "{rep}");
    assert_eq!(rep["agreed"], 2);
    assert_eq!(rep["differed"], 4);
    assert_eq!(rep["agreement_rate_bps"], 3333);
    // 50_000 at 100 bps vs 200 bps: applied 500, shadow 1000, four times; shadow would have cost 2000 more
    assert_eq!(rep["estimated_fee"]["comparable_decisions"], 6);
    assert_eq!(rep["estimated_fee"]["difference_minor"], -2000, "{rep}");
    let d = &rep["differences"][0];
    assert_eq!(
        (
            d["applied_provider"].as_str(),
            d["shadow_provider"].as_str(),
            d["decisions"].as_i64()
        ),
        (Some("razorpay"), Some("cashfree"), Some(4))
    );
    assert_eq!(d["estimated_fee_difference_minor"], -2000);
    let out = &rep["outcome_by_provider"][0];
    assert_eq!(
        (
            out["provider"].as_str(),
            out["payments_sent"].as_i64(),
            out["of_which_shadow_would_have_chosen_another"].as_i64()
        ),
        (Some("razorpay"), Some(6), Some(4))
    );
    assert_eq!(rep["recent_differences"].as_array().unwrap().len(), 4);
    assert!(
        rep["limits"]
            .as_str()
            .unwrap()
            .contains("never sends a payment")
    );
    let only_card = t
        .ok(
            a,
            "GET",
            "/v1/routing/shadow/report?method=card",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(only_card["decisions"], 6);
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/routing/shadow/report?method=upi",
            None,
            Value::Null
        )
        .await["decisions"],
        0
    );

    // another business sees none of it, in the API or the database
    let rb = t
        .ok(b, "GET", "/v1/routing/shadow/report", None, Value::Null)
        .await;
    assert_eq!(rb["decisions"], 0);
    assert_eq!(rb["agreement_rate_bps"], Value::Null);
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM routing_shadow_decisions")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0);
    tx.rollback().await.unwrap();

    // validation, scopes
    let (s, e) = t
        .call(
            a,
            "GET",
            "/v1/routing/shadow/report?days=0",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_days"))
    );
    let (s, _) = t
        .call(
            a,
            "PUT",
            "/v1/routing/automation",
            None,
            json!({ "shadow_policy": { "rules": "x" } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

// ---------- cost-aware routing ----------

#[sqlx::test(migrations = "./migrations")]
async fn cost_aware_routing_uses_fee_rules_and_observed_fees_and_never_picks_a_provider_below_the_floor_for_being_cheaper(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    // configured table says razorpay is the cheaper one (50 vs 200 bps) ...
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": { "rules": [], "fees": {
            "razorpay": { "upi": { "bps": 50, "fixed_minor": 0 } },
            "cashfree": { "upi": { "bps": 200, "fixed_minor": 0 } } } } }),
    )
    .await;
    // ... but the contract rules the merchant typed in say cashfree is
    for (p, bps) in [("razorpay", 300), ("cashfree", 100)] {
        t.ok(
            a,
            "POST",
            "/v1/fee_rules",
            None,
            json!({ "provider": p, "method": "upi", "bps": bps, "effective_from": "2020-01-01" }),
        )
        .await;
    }
    let upi = |key: &'static str, iid: String| {
        let t = &t;
        async move {
            attempt_as(
                t,
                a,
                &iid,
                key,
                json!({ "method": "upi", "vpa": "ravi@ybl" }),
            )
            .await
            .1
        }
    };

    // off: the configured fee table decides, exactly as before
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let att = upi("c0", iid).await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    // on, no success-rate data yet: the fee rules decide, and the sentence says where the number is from
    auto(&t, a, json!({ "cost_aware_enabled": true })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let att = upi("c1", iid).await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    let expl = att["route"]["explanation"].as_str().unwrap();
    assert!(
        expl.contains("from your fee rule")
            && expl.contains("no provider has enough recent attempts"),
        "{expl}"
    );
    let rz = att["route"]["considered"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["provider"] == "razorpay")
        .unwrap();
    assert_eq!(
        rz["estimated_fee_minor"], 1500,
        "300 bps of 50_000: the fee rule, not the configured 50 bps"
    );

    // enough history: razorpay succeeds, cashfree keeps refusing
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "min_samples": 5, "breaker_enabled": false }),
    )
    .await;
    for _ in 0..5 {
        t.paid_razorpay(a, 50_000, json!({})).await;
    }
    outage(&t, "cashfree", "reject").await;
    for i in 0..5 {
        let iid = id_of(&open_intent(&t, a, json!(["cashfree"])).await);
        let att = upi(Box::leak(format!("cf{i}").into_boxed_str()), iid).await;
        assert_eq!(att["status"], "failed");
    }
    outage(&t, "cashfree", "off").await;
    // cashfree is far cheaper but its success rate (0%) is below the 80% floor: it is never chosen for the price
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let att = upi("c2", iid).await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    let expl = att["route"]["explanation"].as_str().unwrap();
    assert!(
        expl.contains("left out of the cost comparison") && expl.contains("below your 80.0% floor"),
        "{expl}"
    );
    // the merchant can lower the floor, and then cost decides
    auto(&t, a, json!({ "cost_success_floor_bps": 0 })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let att = upi("c3", iid).await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    // switched off again: back to the configured table
    auto(&t, a, json!({ "cost_aware_enabled": false })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    assert_eq!(upi("c4", iid).await["provider"], "razorpay");
    assert_eq!(
        audit_count(&t, "routing_automation.cost_aware_enabled").await,
        1
    );
    assert_eq!(
        audit_count(&t, "routing_automation.cost_aware_disabled").await,
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn observed_fees_from_settlements_beat_a_fee_rule_once_there_are_enough_samples(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(a, "POST", "/v1/fee_rules", None, json!({ "provider": "razorpay", "method": "upi", "bps": 50, "effective_from": "2020-01-01" })).await;
    t.ok(a, "POST", "/v1/fee_rules", None, json!({ "provider": "cashfree", "method": "upi", "bps": 100, "effective_from": "2020-01-01" })).await;
    // razorpay's contract says 0.5%, but its settlements show 3.0% deducted on two payments
    for n in 0..2 {
        let (_, _, pref) = t.paid_razorpay(a, 100_000, json!({})).await;
        t.ok(a, "POST", "/v1/settlements/import", None, json!({
            "provider": "razorpay", "provider_ref": format!("stl_{n}"), "utr": format!("UTR{n}"), "settled_on": zyvor_api::ist_today().to_string(),
            "items": [
                { "kind": "payment", "provider_ref": pref, "amount_minor": 100_000 },
                { "kind": "fee", "provider_ref": pref, "amount_minor": -3_000 }
            ] })).await;
    }
    auto(
        &t,
        a,
        json!({ "cost_aware_enabled": true, "cost_min_fee_samples": 3 }),
    )
    .await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, att) = attempt_as(
        &t,
        a,
        &iid,
        "o1",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    let rz = att["route"]["considered"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["provider"] == "razorpay")
        .unwrap();
    assert_eq!(
        rz["estimated_fee_minor"], 250,
        "2 samples is below the minimum of 3: the fee rule (50 bps of 50_000)"
    );
    // with the minimum met the observed rate (3.0%) replaces the rule: razorpay now costs 1500, cashfree 500
    auto(&t, a, json!({ "cost_min_fee_samples": 2 })).await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, att) = attempt_as(
        &t,
        a,
        &iid,
        "o2",
        json!({ "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    let rz = att["route"]["considered"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["provider"] == "razorpay")
        .unwrap();
    assert_eq!(rz["estimated_fee_minor"], 1500);
    // the scorecard shows the same observed fee
    let sc = t
        .ok(
            a,
            "GET",
            "/v1/routing/scorecard?method=upi",
            None,
            Value::Null,
        )
        .await;
    let row = sc["scorecard"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["provider"] == "razorpay")
        .unwrap();
    assert_eq!(row["observed_fee"]["effective_rate_bps"], 300);
    assert_eq!(row["observed_fee"]["payments"], 2);
    assert_eq!(row["observed_fee"]["used_by_cost_routing"], true);
}

// ---------- kill switch ----------

#[sqlx::test(migrations = "./migrations")]
async fn the_kill_switch_stops_every_automation_at_once_is_audited_and_is_per_business(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    fees(&t, a, json!([])).await;
    fees(&t, b, json!([])).await;
    for tn in [a, b] {
        auto(
            &t,
            tn,
            json!({ "shadow_enabled": true, "soft_retry_enabled": true, "cost_aware_enabled": true }),
        )
        .await;
    }
    // a degraded cheaper provider: the breaker skips it ...
    for tn in [a, b] {
        sqlx::query("INSERT INTO provider_health_state (tenant_id, provider, method, state, reason, degraded_since) VALUES ($1, 'razorpay', 'card', 'degraded', 'test', now())")
            .bind(&tn.id)
            .execute(&t.root)
            .await
            .unwrap();
    }
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, att) = card_attempt(&t, a, &iid, "k1").await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    assert_eq!(
        t.count("SELECT count(*) FROM routing_shadow_decisions")
            .await,
        1
    );
    let eff = t
        .ok(a, "GET", "/v1/routing/automation", None, Value::Null)
        .await;
    assert_eq!(eff["effective"]["shadow"], true);
    assert_eq!(eff["effective"]["circuit_breaker"], true);

    // ... pull the switch
    let p = t
        .ok(
            a,
            "POST",
            "/v1/routing/automation/pause",
            None,
            json!({ "reason": "provider talks" }),
        )
        .await;
    assert_eq!(p["settings"]["paused"], true);
    assert_eq!(p["settings"]["paused_reason"], "provider talks");
    for k in [
        "shadow",
        "soft_retry",
        "cost_aware",
        "failover",
        "circuit_breaker",
    ] {
        assert_eq!(p["effective"][k], false, "{k}");
    }
    // each switch keeps its own setting
    assert_eq!(p["settings"]["shadow_enabled"], true);
    // pausing again changes nothing and writes no second entry
    t.ok(
        a,
        "POST",
        "/v1/routing/automation/pause",
        None,
        json!({ "reason": "again" }),
    )
    .await;
    assert_eq!(audit_count(&t, "routing_automation.paused").await, 1);

    // routing is the merchant's own rules and the lowest configured fee: the degraded cheaper provider is used
    let before_shadow = t
        .count("SELECT count(*) FROM routing_shadow_decisions")
        .await;
    let iid = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, att) = card_attempt(&t, a, &iid, "k2").await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    assert_eq!(att["route"]["failover_from"], Value::Null);
    assert_eq!(
        t.count("SELECT count(*) FROM routing_shadow_decisions")
            .await,
        before_shadow,
        "shadow recorded nothing"
    );
    let h = t
        .ok(a, "GET", "/v1/routing/health", None, Value::Null)
        .await;
    assert_eq!(h["automation_paused"], true);
    // no soft retry while paused, whatever fails; no failover either
    finish(&t, a, &id_of(&att), Attempt::Failed, Some("provider_fault")).await;
    run_retry(&t, a, &iid).await;
    softretry::sweep(&t.state, &a.id).await.unwrap();
    assert_eq!(attempts_of(&t, &iid).await, 1);
    let (_, again) = card_attempt(&t, a, &iid, "k3").await;
    assert_eq!(
        again["provider"], "razorpay",
        "no failover while paused: {}",
        again["route"]
    );
    // the sweeper leaves the breaker alone while paused
    assert_eq!(
        zyvor_api::health::sweep_tenant(&t.state.pool, &a.id)
            .await
            .unwrap(),
        Default::default()
    );

    // another business is unaffected
    let ib = id_of(&open_intent(&t, b, Value::Null).await);
    let (_, atb) = card_attempt(&t, b, &ib, "k4").await;
    assert_eq!(atb["provider"], "cashfree", "{}", atb["route"]);
    assert_eq!(
        t.ok(b, "GET", "/v1/routing/automation", None, Value::Null)
            .await["settings"]["paused"],
        false
    );

    // resume: each automation acts again
    t.ok(
        a,
        "POST",
        "/v1/routing/automation/resume",
        None,
        Value::Null,
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/routing/automation/resume",
        None,
        Value::Null,
    )
    .await;
    assert_eq!(audit_count(&t, "routing_automation.resumed").await, 1);
    finish(
        &t,
        a,
        &id_of(&again),
        Attempt::Failed,
        Some("provider_fault"),
    )
    .await;
    let iid3 = id_of(&open_intent(&t, a, Value::Null).await);
    let (_, att) = card_attempt(&t, a, &iid3, "k5").await;
    assert_eq!(
        att["provider"], "cashfree",
        "the breaker applies again: {}",
        att["route"]
    );
    assert_eq!(
        t.ok(a, "GET", "/v1/routing/automation", None, Value::Null)
            .await["effective"]["shadow"],
        true
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn automation_settings_are_validated_scoped_audited_and_isolated(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let d = t
        .ok(a, "GET", "/v1/routing/automation", None, Value::Null)
        .await;
    // everything off by default, the kill switch not pulled
    for k in [
        "shadow_enabled",
        "soft_retry_enabled",
        "cost_aware_enabled",
        "paused",
    ] {
        assert_eq!(d["settings"][k], false, "{k}");
    }
    assert_eq!(d["settings"]["cost_success_floor_bps"], 8000);
    for body in [
        json!({ "soft_retry_max_attempts": 1 }),
        json!({ "soft_retry_max_attempts": 9 }),
        json!({ "soft_retry_window_seconds": 1 }),
        json!({ "cost_success_floor_bps": 20000 }),
        json!({ "cost_premium_bps": -1 }),
        json!({ "cost_min_fee_samples": 0 }),
    ] {
        let (s, e) = t
            .call(a, "PUT", "/v1/routing/automation", None, body.clone())
            .await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("invalid_automation_settings")
            ),
            "{body}"
        );
    }
    auto(
        &t,
        a,
        json!({ "soft_retry_enabled": true, "soft_retry_max_attempts": 3 }),
    )
    .await;
    assert_eq!(
        audit_count(&t, "routing_automation.soft_retry_enabled").await,
        1
    );
    assert_eq!(
        audit_count(&t, "routing_automation.settings_updated").await,
        1
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/routing/automation", None, Value::Null)
            .await["settings"]["soft_retry_enabled"],
        false
    );
    // read needs payments:read, change and the kill switch need admin
    let key = |scopes: Value| {
        let t = &t;
        async move {
            let k = t
                .ok(
                    a,
                    "POST",
                    "/v1/api_keys",
                    None,
                    json!({ "name": "k", "scopes": scopes }),
                )
                .await;
            Tenant {
                id: a.id.clone(),
                key: k["key"].as_str().unwrap().to_string(),
            }
        }
    };
    let ro = key(json!(["payments:read"])).await;
    for path in [
        "/v1/routing/automation",
        "/v1/routing/shadow/report",
        "/v1/routing/scorecard",
    ] {
        assert_eq!(
            t.call(&ro, "GET", path, None, Value::Null).await.0,
            StatusCode::OK,
            "{path}"
        );
    }
    assert_eq!(
        t.call(
            &ro,
            "PUT",
            "/v1/routing/automation",
            None,
            json!({ "shadow_enabled": true })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&ro, "POST", "/v1/routing/automation/pause", None, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(
            &ro,
            "POST",
            "/v1/routing/automation/resume",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let none = key(json!(["refunds:write"])).await;
    for path in [
        "/v1/routing/automation",
        "/v1/routing/shadow/report",
        "/v1/routing/scorecard",
    ] {
        assert_eq!(
            t.call(&none, "GET", path, None, Value::Null).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    // the database keeps the businesses apart
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM routing_automation")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0, "business B sees none of A's settings");
    tx.rollback().await.unwrap();
}

// ---------- scorecard ----------

#[sqlx::test(migrations = "./migrations")]
async fn the_scorecard_excludes_payer_declines_and_shows_classes_unresolved_and_the_last_degrade(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    fees(&t, a, json!([])).await;
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "min_samples": 5 }),
    )
    .await;

    // razorpay card: 6 captured, 2 provider faults, 3 payer declines, 1 unknown, 1 cancelled
    let mut mk = Vec::new();
    for i in 0..13 {
        let iid = id_of(&open_intent(&t, a, json!(["razorpay"])).await);
        let (_, att) = card_attempt(&t, a, &iid, &format!("m{i}")).await;
        mk.push(id_of(&att));
    }
    let plan: [(Attempt, Option<&'static str>); 13] = [
        (Attempt::Captured, None),
        (Attempt::Captured, None),
        (Attempt::Captured, None),
        (Attempt::Captured, None),
        (Attempt::Captured, None),
        (Attempt::Captured, None),
        (Attempt::Failed, Some("provider_fault")),
        (Attempt::Failed, Some("provider_fault")),
        (Attempt::Failed, Some("payer_declined")),
        (Attempt::Failed, Some("payer_declined")),
        (Attempt::Failed, Some("payer_declined")),
        (Attempt::Unknown, None),
        (Attempt::Failed, Some("cancelled")),
    ];
    for (id, (st, class)) in mk.iter().zip(plan) {
        finish(&t, a, id, st, class).await;
    }
    // a degrade and a recovery are written the way the breaker writes them
    sqlx::query("INSERT INTO audit_events (tenant_id, actor, action, subject, detail) VALUES ($1, 'routing', 'provider.degraded', 'razorpay', '{\"method\": \"card\"}')")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    let sc = t
        .ok(
            a,
            "GET",
            "/v1/routing/scorecard?days=7&method=card",
            None,
            Value::Null,
        )
        .await;
    let row = sc["scorecard"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["provider"] == "razorpay")
        .unwrap();
    assert_eq!(row["method"], "card");
    assert_eq!(row["successes"], 6);
    assert_eq!(row["provider_failures"], 2);
    assert_eq!(row["payer_declines"], 3);
    assert_eq!(row["unresolved_in_window"], 1);
    assert_eq!(row["unresolved_now"], 1);
    assert_eq!(row["cancelled_by_merchant"], 1);
    // 6 / (6 + 2 + 1): payer declines and the cancellation are left out
    assert_eq!(row["samples"], 9);
    assert_eq!(row["success_rate_bps"], 6666);
    assert_eq!(row["failure_classes"]["provider_fault"], 2);
    assert_eq!(row["failure_classes"]["payer_declined"], 3);
    assert_eq!(row["failure_classes"]["cancelled"], 1);
    assert!(row["median_seconds_to_resolve"].as_f64().is_some(), "{row}");
    assert!(row["last_degraded_at"].is_string());
    assert_eq!(row["last_recovered_at"], Value::Null);
    assert_eq!(row["configured_fee"]["bps"], 100);
    // below the minimum sample the rate is withheld, not guessed
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "min_samples": 50 }),
    )
    .await;
    let sc = t
        .ok(
            a,
            "GET",
            "/v1/routing/scorecard?method=card",
            None,
            Value::Null,
        )
        .await;
    let row = sc["scorecard"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["provider"] == "razorpay")
        .unwrap();
    assert_eq!(row["success_rate_bps"], Value::Null);

    // another business has an empty scorecard
    let sb = t
        .ok(b, "GET", "/v1/routing/scorecard", None, Value::Null)
        .await;
    assert_eq!(sb["scorecard"].as_array().unwrap().len(), 0);
    let (s, e) = t
        .call(
            a,
            "GET",
            "/v1/routing/scorecard?method=cheque",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_method"))
    );
}
