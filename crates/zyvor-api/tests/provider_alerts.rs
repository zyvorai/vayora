// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Provider, bank and UPI-handle downtime alerts, against seeded history only (no real provider, bank or money):
//! one alert per group however often it is checked, a case, an outbox event and an optional email, hysteresis and
//! flap control, and isolation between businesses.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const OK: (&str, Option<&str>) = ("captured", None);
const FAIL: (&str, Option<&str>) = ("failed", Some("declined"));
const PAYER: (&str, Option<&str>) = ("failed", Some("payer_declined"));

/// Insert `n` resolved attempts of one provider and method, `age_mins` old, carrying `instrument`.
#[allow(clippy::too_many_arguments)]
async fn seed(
    t: &T,
    tn: &Tenant,
    provider: &str,
    method: &str,
    instrument: Value,
    (status, class): (&str, Option<&str>),
    n: i32,
    age_mins: i32,
) {
    let tag = format!("{}{}", tn.id, zyvor_core::new_id("s"));
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method) SELECT $1 || g, $2, 10000, 'INR', 'requires_payment_method', 'automatic' FROM generate_series(1, $3) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(n)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, failure_class, instrument, created_at) SELECT 'a' || $1 || g, $2, $1 || g, $3, $4, $5, 10000, 'INR', '{}', $6, $7, now() - make_interval(mins => $9) FROM generate_series(1, $8) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(provider)
        .bind(method)
        .bind(status)
        .bind(class)
        .bind(instrument)
        .bind(n)
        .bind(age_mins)
        .execute(&t.root)
        .await
        .unwrap();
}

async fn settings(t: &T, tn: &Tenant, body: Value) -> Value {
    t.ok(tn, "PUT", "/v1/routing/alerts/settings", None, body)
        .await
}

async fn evaluate(t: &T, tn: &Tenant) -> Value {
    t.ok(tn, "POST", "/v1/routing/alerts/evaluate", None, json!({}))
        .await
}

async fn alerts(t: &T, tn: &Tenant, status: &str) -> Vec<Value> {
    t.ok(
        tn,
        "GET",
        &format!("/v1/routing/alerts?status={status}"),
        None,
        Value::Null,
    )
    .await["data"]
        .as_array()
        .unwrap()
        .clone()
}

fn fast() -> Value {
    json!({ "enabled": true, "min_samples": 5, "hold_seconds": 0, "flap_window_seconds": 900, "email_to": "ops@example.com" })
}

#[sqlx::test(migrations = "./migrations")]
async fn an_outage_raises_one_alert_with_a_case_an_event_and_an_email_and_resolves_itself(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    // settings are validated
    let (s, _) = t
        .call(
            a,
            "PUT",
            "/v1/routing/alerts/settings",
            None,
            json!({ "open_failure_pct": 30, "resolve_failure_pct": 40 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = t
        .call(
            a,
            "PUT",
            "/v1/routing/alerts/settings",
            None,
            json!({ "email_to": "not an address" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let cfg = settings(&t, a, fast()).await;
    assert_eq!(cfg["email_to"], "ops@example.com");
    assert_eq!(
        t.ok(a, "GET", "/v1/routing/alerts/settings", None, Value::Null)
            .await["min_samples"],
        5
    );

    seed(&t, a, "razorpay", "card", json!({}), FAIL, 8, 2).await;
    seed(&t, a, "razorpay", "card", json!({}), OK, 2, 2).await;
    let first = evaluate(&t, a).await;
    assert_eq!(first["opened"], 1, "{first}");
    let open = alerts(&t, a, "open").await;
    assert_eq!(open.len(), 1);
    let al = &open[0];
    assert_eq!(
        (
            al["provider"].as_str(),
            al["method"].as_str(),
            al["dimension"].as_str()
        ),
        (Some("razorpay"), Some("card"), Some("provider_method"))
    );
    assert_eq!(
        (al["samples"].as_i64(), al["failure_pct"].as_i64()),
        (Some(10), Some(80))
    );
    let id = al["id"].as_str().unwrap().to_string();
    let detail = t
        .ok(
            a,
            "GET",
            &format!("/v1/routing/alerts/{id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(detail["case"]["status"], "open");

    // all three: a case, an outbox event, an email
    assert_eq!(
        t.count("SELECT COUNT(*) FROM recon_cases WHERE kind = 'provider_alert'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM audit_events WHERE action = 'provider_alert.opened'")
            .await,
        1
    );

    // de-duplication: checking again, by hand or by the worker, raises nothing new
    for _ in 0..3 {
        assert_eq!(evaluate(&t, a).await["opened"], 0);
    }
    let client = zyvor_api::outbox::http_client();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(alerts(&t, a, "open").await.len(), 1);
    assert_eq!(t.count("SELECT COUNT(*) FROM provider_alerts").await, 1);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        1
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/routing/alerts/{id}/ack"),
        None,
        json!({}),
    )
    .await;
    assert!(alerts(&t, a, "open").await[0]["acknowledged_at"].is_string());

    // the failures turn out fine (late webhooks): it resolves itself, once, and says so
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL WHERE status = 'failed'")
        .execute(&t.root)
        .await
        .unwrap();
    let r = evaluate(&t, a).await;
    assert_eq!(r["resolved"], 1, "{r}");
    assert_eq!(alerts(&t, a, "open").await.len(), 0);
    assert_eq!(alerts(&t, a, "resolved").await.len(), 1);
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'provider_alert' AND status = 'resolved'"
        )
        .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_resolved'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        2
    );
    assert_eq!(evaluate(&t, a).await["resolved"], 0);
    // the alert and its events are in the catalogue merchants subscribe to
    let cat = t
        .ok(a, "GET", "/v1/events/catalogue", None, Value::Null)
        .await
        .to_string();
    assert!(cat.contains("provider.alert_opened") && cat.contains("provider.alert_resolved"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failing_bank_or_upi_handle_alerts_on_its_own_and_payer_declines_do_not_count(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    settings(
        &t,
        a,
        json!({ "enabled": true, "min_samples": 5, "hold_seconds": 0 }),
    )
    .await;
    // UPI: one handle is failing, another is fine, and overall the provider is mostly fine
    seed(
        &t,
        a,
        "razorpay",
        "upi",
        json!({ "upi_handle": "@badpsp" }),
        FAIL,
        10,
        1,
    )
    .await;
    seed(
        &t,
        a,
        "razorpay",
        "upi",
        json!({ "upi_handle": "@okaxis" }),
        OK,
        40,
        1,
    )
    .await;
    // a handle whose payers are simply short of money: not a provider problem
    seed(
        &t,
        a,
        "razorpay",
        "upi",
        json!({ "upi_handle": "@poorpsp" }),
        PAYER,
        30,
        1,
    )
    .await;
    seed(
        &t,
        a,
        "razorpay",
        "upi",
        json!({ "upi_handle": "@poorpsp" }),
        OK,
        3,
        1,
    )
    .await;
    // netbanking: one bank is failing
    seed(
        &t,
        a,
        "cashfree",
        "netbanking",
        json!({ "bank": "hdfc" }),
        FAIL,
        8,
        1,
    )
    .await;
    seed(
        &t,
        a,
        "cashfree",
        "netbanking",
        json!({ "bank": "sbi" }),
        OK,
        30,
        1,
    )
    .await;
    let r = evaluate(&t, a).await;
    assert_eq!(r["opened"], 2, "{r}");
    let open = alerts(&t, a, "open").await;
    let mut seen: Vec<(String, String, String)> = open
        .iter()
        .map(|x| {
            (
                x["dimension"].as_str().unwrap().into(),
                x["dimension_value"].as_str().unwrap().into(),
                x["provider"].as_str().unwrap().into(),
            )
        })
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (
                "bank".to_string(),
                "hdfc".to_string(),
                "cashfree".to_string()
            ),
            (
                "upi_handle".to_string(),
                "@badpsp".to_string(),
                "razorpay".to_string()
            ),
        ]
    );
    assert!(
        open.iter()
            .any(|x| x["reason"].as_str().unwrap().contains("bank hdfc"))
    );
    // no email address was set, so none was queued; the event and the case were
    assert_eq!(t.count("SELECT COUNT(*) FROM mail_queue").await, 0);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        2
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM recon_cases WHERE kind = 'provider_alert'")
            .await,
        2
    );
    // the numbers in the group are what was seeded, not the whole provider's
    let bad = open
        .iter()
        .find(|x| x["dimension_value"] == "@badpsp")
        .unwrap();
    assert_eq!(
        (bad["samples"].as_i64(), bad["failure_pct"].as_i64()),
        (Some(10), Some(100))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn flap_control_reopens_quietly_and_does_not_count_old_failures_twice(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    settings(&t, a, fast()).await;
    seed(&t, a, "razorpay", "card", json!({}), FAIL, 10, 3).await;
    assert_eq!(evaluate(&t, a).await["opened"], 1);
    let id = alerts(&t, a, "open").await[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // the circuit breaker recovers the provider: the alert resolves though the old failures are still in the window
    sqlx::query("INSERT INTO provider_health_state (tenant_id, provider, method, state, changed_at) VALUES ($1, 'razorpay', 'card', 'healthy', now()) ON CONFLICT (tenant_id, provider, method) DO UPDATE SET state = 'healthy', changed_at = now()")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    let r = evaluate(&t, a).await;
    assert_eq!(r["resolved"], 1, "{r}");
    let resolved = alerts(&t, a, "resolved").await;
    assert!(
        resolved[0]["resolve_reason"]
            .as_str()
            .unwrap()
            .contains("circuit breaker")
    );
    // ... and those same failures do not raise it again on the next check
    let r = evaluate(&t, a).await;
    assert_eq!(
        (r["opened"].as_i64(), r["reopened"].as_i64()),
        (Some(0), Some(0)),
        "{r}"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM provider_alerts WHERE status = 'open'")
            .await,
        0
    );

    // a real recurrence soon after: the SAME alert reopens, quietly
    seed(&t, a, "razorpay", "card", json!({}), FAIL, 30, 0).await;
    let r = evaluate(&t, a).await;
    assert_eq!(
        (r["opened"].as_i64(), r["reopened"].as_i64()),
        (Some(0), Some(1)),
        "{r}"
    );
    let open = alerts(&t, a, "open").await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["id"], id.as_str());
    assert_eq!(open[0]["flap_count"], 1);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        1,
        "no second event"
    );
    assert_eq!(t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com' AND subject LIKE 'Possible downtime%'").await, 1, "no second email");
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'provider_alert' AND status = 'open'"
        )
        .await,
        1,
        "the case is open again"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM audit_events WHERE action = 'provider_alert.reopened'")
            .await,
        1
    );

    // hysteresis: 40% failing is neither bad enough to open nor good enough to resolve
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL WHERE id IN (SELECT id FROM payment_attempts WHERE status = 'failed' ORDER BY id LIMIT 33)")
        .execute(&t.root)
        .await
        .unwrap();
    // 40 attempts, 7 failed + 33 captured... make exactly 40% failing instead
    sqlx::query("UPDATE payment_attempts SET status = 'failed', failure_class = 'declined' WHERE id IN (SELECT id FROM payment_attempts WHERE status = 'captured' ORDER BY id LIMIT 9)")
        .execute(&t.root)
        .await
        .unwrap();
    let (s, f): (i64, i64) = sqlx::query_as("SELECT COUNT(*) FILTER (WHERE status = 'captured'), COUNT(*) FILTER (WHERE status = 'failed') FROM payment_attempts")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(f * 100 / (s + f), 40, "{s} {f}");
    let r = evaluate(&t, a).await;
    assert_eq!(r["resolved"], 0, "40% is in the gap, so it stays open: {r}");
    assert_eq!(alerts(&t, a, "open").await.len(), 1);

    // it recovers properly, then fails again long after the flap window: a NEW alert
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL")
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(evaluate(&t, a).await["resolved"], 1);
    sqlx::query("UPDATE provider_alerts SET resolved_at = now() - interval '2 hours'")
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE payment_attempts SET created_at = now() - interval '3 hours'")
        .execute(&t.root)
        .await
        .unwrap();
    seed(&t, a, "razorpay", "card", json!({}), FAIL, 12, 0).await;
    let r = evaluate(&t, a).await;
    assert_eq!(
        (r["opened"].as_i64(), r["reopened"].as_i64()),
        (Some(1), Some(0)),
        "{r}"
    );
    let open = alerts(&t, a, "open").await;
    assert_ne!(open[0]["id"], id.as_str());
    assert_eq!(t.count("SELECT COUNT(*) FROM provider_alerts").await, 2);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        2
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_alert_with_no_traffic_for_a_day_resolves_and_a_hold_time_is_respected(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    settings(
        &t,
        a,
        json!({ "enabled": true, "min_samples": 5, "hold_seconds": 3600 }),
    )
    .await;
    seed(&t, a, "cashfree", "card", json!({}), FAIL, 10, 1).await;
    assert_eq!(evaluate(&t, a).await["opened"], 1);
    // good numbers arrive but the alert has not been open for its hold time
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL")
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(evaluate(&t, a).await["resolved"], 0);
    sqlx::query("UPDATE provider_alerts SET opened_at = now() - interval '2 hours'")
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(evaluate(&t, a).await["resolved"], 1);

    // a second one that just goes quiet
    sqlx::query("UPDATE payment_attempts SET created_at = now() - interval '2 days'")
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE provider_alerts SET resolved_at = now() - interval '2 days'")
        .execute(&t.root)
        .await
        .unwrap();
    seed(&t, a, "cashfree", "card", json!({}), FAIL, 10, 1).await;
    assert_eq!(evaluate(&t, a).await["opened"], 1);
    sqlx::query("UPDATE payment_attempts SET created_at = now() - interval '5 hours' WHERE status = 'failed'").execute(&t.root).await.unwrap();
    sqlx::query("UPDATE provider_alerts SET opened_at = now() - interval '30 hours', last_seen_at = now() - interval '25 hours' WHERE status = 'open'").execute(&t.root).await.unwrap();
    let r = evaluate(&t, a).await;
    assert_eq!(r["resolved"], 1, "{r}");
    let last = alerts(&t, a, "resolved").await;
    assert!(last.iter().any(|x| {
        x["resolve_reason"]
            .as_str()
            .unwrap()
            .contains("no attempts for 24 hours")
    }));

    // switched off: nothing is raised
    settings(&t, a, json!({ "enabled": false })).await;
    seed(&t, a, "razorpay", "upi", json!({}), FAIL, 30, 0).await;
    assert_eq!(evaluate(&t, a).await["opened"], 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn alerts_belong_to_one_business_and_need_the_routing_scope(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    settings(&t, a, fast()).await;
    seed(&t, a, "razorpay", "card", json!({}), FAIL, 10, 1).await;
    assert_eq!(evaluate(&t, a).await["opened"], 1);

    // B sees nothing of A's, and A's attempts do not make B alert
    assert_eq!(alerts(&t, b, "open").await.len(), 0);
    let r = evaluate(&t, b).await;
    assert_eq!(r["opened"], 0);
    let id: String = sqlx::query_scalar("SELECT id FROM provider_alerts")
        .fetch_one(&t.root)
        .await
        .unwrap();
    let (s, _) = t
        .call(
            b,
            "GET",
            &format!("/v1/routing/alerts/{id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "POST",
            &format!("/v1/routing/alerts/{id}/ack"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // B's settings are its own
    assert_eq!(
        t.ok(b, "GET", "/v1/routing/alerts/settings", None, Value::Null)
            .await["email_to"],
        Value::Null
    );
    // B has its own outage, with its own (empty) thresholds: one alert there, none for A's email
    settings(
        &t,
        b,
        json!({ "enabled": true, "min_samples": 5, "hold_seconds": 0 }),
    )
    .await;
    seed(&t, b, "cashfree", "upi", json!({}), FAIL, 10, 1).await;
    assert_eq!(evaluate(&t, b).await["opened"], 1);
    assert_eq!(alerts(&t, a, "open").await.len(), 1);
    assert_eq!(alerts(&t, a, "open").await[0]["provider"], "razorpay");
    assert_eq!(
        t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        1
    );
    // the database, as the app role with no business set, sees none of it
    for table in [
        "provider_alerts",
        "provider_alert_config",
        "offer_redemptions",
        "offer_refund_adjustments",
        "routing_experiment_assignments",
    ] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.state.pool)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    // a read-only key can read alerts but not change settings or run a check
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "ro", "scopes": ["routing:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: ro["key"].as_str().unwrap().to_string(),
    };
    t.ok(&ro, "GET", "/v1/routing/alerts", None, Value::Null)
        .await;
    let (s, _) = t
        .call(
            &ro,
            "PUT",
            "/v1/routing/alerts/settings",
            None,
            json!({ "enabled": false }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call(&ro, "POST", "/v1/routing/alerts/evaluate", None, json!({}))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "./migrations")]
async fn several_workers_checking_at_once_raise_one_alert(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    settings(&t, a, fast()).await;
    seed(&t, a, "razorpay", "card", json!({}), FAIL, 12, 1).await;
    let results = futures_util::future::join_all(
        (0..6).map(|_| zyvor_api::alerts::sweep_tenant(&t.state.pool, &a.id)),
    )
    .await;
    let opened: usize = results.iter().map(|r| r.as_ref().unwrap().opened).sum();
    assert_eq!(
        opened, 1,
        "an advisory lock per business: one replica raises it, the others skip or find it open"
    );
    assert_eq!(t.count("SELECT COUNT(*) FROM provider_alerts").await, 1);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'provider.alert_opened'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        1
    );
}
