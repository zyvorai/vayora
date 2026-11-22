// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! How offers, rule sets, experiments and downtime alerts sit with routing automation (main's soft retry and kill
//! switch), against the simulators only.

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

async fn fees(t: &T, tn: &Tenant) {
    t.ok(
        tn,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": { "rules": [], "fees": {
            "razorpay": { "card": { "bps": 100, "fixed_minor": 0 }, "upi": { "bps": 100, "fixed_minor": 0 } },
            "cashfree": { "card": { "bps": 200, "fixed_minor": 0 }, "upi": { "bps": 200, "fixed_minor": 0 } }
        } } }),
    )
    .await;
}

async fn intent(t: &T, tn: &Tenant) -> String {
    t.intent(tn, 50_000, json!({ "allowed_providers": null }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn card(t: &T, tn: &Tenant, iid: &str, key: &str, extra: Value) -> (StatusCode, Value) {
    let mut b = json!({ "method": "card" });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.call(
        tn,
        "POST",
        &format!("/v1/payment_intents/{iid}/attempts"),
        Some(key),
        b,
    )
    .await
}

async fn offer(t: &T, tn: &Tenant, body: Value) -> String {
    let mut b =
        json!({ "name": "Ten percent", "kind": "percent", "value": 1_000, "methods": ["card"] });
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.ok(tn, "POST", "/v1/offers", None, b).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn offer_row(t: &T, id: &str) -> (i32, i64) {
    sqlx::query_as("SELECT redemptions, spent_minor FROM offers WHERE id = $1")
        .bind(id)
        .fetch_one(&t.root)
        .await
        .unwrap()
}

async fn outage(t: &T, provider: &str, mode: &str) {
    t.mock_post("outage", json!({ "provider": provider, "mode": mode }))
        .await;
}

async fn redemptions(t: &T, iid: &str) -> Vec<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT status FROM offer_redemptions WHERE intent_id = '{iid}' ORDER BY created_at, id"
    )))
    .fetch_all(&t.root)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn a_soft_retry_keeps_the_offer_and_never_counts_a_capped_offer_twice(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    // only Visa cards, one use in total: the retry has to be priced on the same instrument to keep it
    let oid = offer(
        &t,
        a,
        json!({ "card_networks": ["visa"], "max_redemptions": 1 }),
    )
    .await;
    t.ok(
        a,
        "PUT",
        "/v1/routing/automation",
        None,
        json!({ "soft_retry_enabled": true }),
    )
    .await;
    outage(&t, "razorpay", "reject").await;

    let iid = intent(&t, a).await;
    let (s, first) = card(
        &t,
        a,
        &iid,
        "k1",
        json!({ "instrument": { "card_network": "visa" } }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    assert_eq!(first["provider"], "razorpay");
    assert_eq!(first["status"], "failed");
    assert_eq!(first["amount_minor"], 45_000);

    // the retry went to the other provider, for the same discounted amount
    let second: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(a) FROM payment_attempts a WHERE intent_id = '{iid}' AND id <> '{}'",
        first["id"].as_str().unwrap()
    )))
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(second["provider"], "cashfree");
    assert_eq!(second["amount_minor"], 45_000, "{second}");
    assert_eq!(second["route"]["offers"]["discount_minor"], 5_000);
    // the failed attempt gave its use back and the retry took it: one use, not two
    assert_eq!(redemptions(&t, &iid).await, vec!["released", "reserved"]);
    assert_eq!(offer_row(&t, &oid).await, (1, 5_000));
    // the stored request (hint and codes included) is gone once the retry is made
    let kept: Option<Value> =
        sqlx::query_scalar("SELECT retry_input FROM payment_attempts WHERE id = $1")
            .bind(first["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert!(kept.is_none());
    // asking again from anywhere changes nothing
    for _ in 0..3 {
        softretry::run(&t.state, &a.id, &iid).await;
    }
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(redemptions(&t, &iid).await, vec!["released", "reserved"]);
    assert_eq!(offer_row(&t, &oid).await, (1, 5_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_retry_is_not_made_when_the_offers_would_now_price_it_differently(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    let oid = offer(&t, a, json!({ "max_redemptions": 1 })).await;
    t.ok(
        a,
        "PUT",
        "/v1/routing/automation",
        None,
        json!({ "soft_retry_enabled": true }),
    )
    .await;

    // payer one holds the only use on a pending payment at the cheaper provider
    let iid1 = intent(&t, a).await;
    let (_, att1) = card(&t, a, &iid1, "p1", json!({})).await;
    assert_eq!(
        (att1["status"].as_str(), att1["amount_minor"].as_i64()),
        (Some("pending"), Some(45_000))
    );
    // the provider then says it failed: the use is given back ...
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &a.id)
        .await
        .unwrap();
    apply_provider_state(
        &mut tx,
        &a.id,
        att1["id"].as_str().unwrap(),
        &Incoming {
            status: Attempt::Failed,
            failure_reason: Some("test".into()),
            failure_class: Some("provider_fault"),
            ..Incoming::default()
        },
        "test",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(offer_row(&t, &oid).await, (0, 0));
    // ... and payer two takes it before the retry is made
    let iid2 = intent(&t, a).await;
    let (_, att2) = card(&t, a, &iid2, "p2", json!({})).await;
    assert_eq!(att2["amount_minor"], 45_000);
    assert_eq!(offer_row(&t, &oid).await, (1, 5_000));

    // the retry would now cost payer one 50,000 instead of 45,000: no automatic retry, and nothing is double-counted
    softretry::run(&t.state, &a.id, &iid1).await;
    let n: i64 = t
        .count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE intent_id = '{iid1}'"
        ))
        .await;
    assert_eq!(n, 1, "no second attempt was started");
    let skipped: Vec<(String, Option<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT status, reason FROM routing_soft_retries WHERE intent_id = '{iid1}'"
    )))
    .fetch_all(&t.root)
    .await
    .unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].0, "skipped");
    assert!(
        skipped[0]
            .1
            .as_deref()
            .unwrap()
            .contains("offers now apply differently"),
        "{skipped:?}"
    );
    assert_eq!(offer_row(&t, &oid).await, (1, 5_000));
    assert_eq!(redemptions(&t, &iid1).await, vec!["released"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_kill_switch_also_stops_rule_sets_experiments_and_alert_notifications(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    let rule = |to: &str| json!([{ "name": "cards", "when": { "methods": ["card"] }, "then": { "prefer": [to] } }]);
    t.ok(
        a,
        "POST",
        "/v1/routing/rule_sets",
        None,
        json!({ "rules": rule("cashfree"), "activate": true }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/routing/rule_sets",
        None,
        json!({ "rules": rule("razorpay") }),
    )
    .await;
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "breaker_enabled": false }),
    )
    .await;
    let exp = t
        .ok(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "e", "variant_version": 2, "variant_percent": 50 }),
        )
        .await;
    let eid = exp["id"].as_str().unwrap().to_string();

    // running: the rule set routes cards to cashfree and every payment is in an arm
    let iid = intent(&t, a).await;
    let (_, att) = card(&t, a, &iid, "on", json!({})).await;
    assert!(att["route"]["experiment"].is_object(), "{}", att["route"]);
    let eff = t
        .ok(a, "GET", "/v1/routing/automation", None, Value::Null)
        .await;
    assert_eq!(eff["effective"]["rule_sets"], true);
    assert_eq!(eff["effective"]["experiments"], true);
    assert_eq!(eff["effective"]["alert_notifications"], true);

    // pull the switch
    t.ok(
        a,
        "POST",
        "/v1/routing/automation/pause",
        None,
        json!({ "reason": "provider talks" }),
    )
    .await;
    let eff = t
        .ok(a, "GET", "/v1/routing/automation", None, Value::Null)
        .await;
    for k in ["rule_sets", "experiments", "alert_notifications"] {
        assert_eq!(eff["effective"][k], false, "{k}");
    }
    let assigned_before = t
        .count("SELECT count(*) FROM routing_experiment_assignments")
        .await;
    for i in 0..12 {
        let iid = intent(&t, a).await;
        let (s, att) = card(&t, a, &iid, &format!("off{i}"), json!({})).await;
        assert_eq!(s, StatusCode::CREATED, "{att}");
        // the older settings rules and the lowest fee route it: no rule set, no experiment arm
        assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
        assert!(att["route"].get("experiment").is_none(), "{}", att["route"]);
        assert_eq!(att["route"]["rule_set"]["source"], "settings");
    }
    assert_eq!(
        t.count("SELECT count(*) FROM routing_experiment_assignments")
            .await,
        assigned_before,
        "nothing was split"
    );
    let d = t
        .ok(
            a,
            "POST",
            "/v1/routing/dry_run",
            None,
            json!({ "method": "card", "amount_minor": 50_000 }),
        )
        .await;
    assert_eq!(d["automation_paused"], true);
    assert_eq!(d["provider"], "razorpay");
    assert!(d["notes"][0].as_str().unwrap().contains("kill switch"));
    // a named version can still be tried
    let d = t
        .ok(
            a,
            "POST",
            "/v1/routing/dry_run",
            None,
            json!({ "method": "card", "amount_minor": 50_000, "version": 1 }),
        )
        .await;
    assert_eq!(d["provider"], "cashfree");
    let res = t
        .ok(
            a,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(res["automation_paused"], true);
    assert_eq!(res["status"], "running");
    // the guardrail does not act while it is on
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(
        t.ok(
            a,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null
        )
        .await["status"],
        "running"
    );

    // alerts: recorded, but no case, no event, no email
    t.ok(a, "PUT", "/v1/routing/alerts/settings", None, json!({ "enabled": true, "min_samples": 5, "hold_seconds": 0, "email_to": "ops@example.com" }))
        .await;
    seed_failures(&t, a, 10).await;
    let r = t
        .ok(a, "POST", "/v1/routing/alerts/evaluate", None, json!({}))
        .await;
    assert_eq!(r["opened"], 1, "{r}");
    assert_eq!(
        t.count("SELECT count(*) FROM provider_alerts WHERE status = 'open'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'provider_alert.opened'")
            .await,
        1
    );
    let quiet: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_events WHERE action = 'provider_alert.opened'",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(quiet["notifications_suppressed"], true);
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'provider_alert'")
            .await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type LIKE 'provider.alert_%'")
            .await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM mail_queue WHERE to_email = 'ops@example.com'")
            .await,
        0
    );

    // resumed: rule sets and experiments act again
    t.ok(a, "POST", "/v1/routing/automation/resume", None, json!({}))
        .await;
    let iid = intent(&t, a).await;
    let (_, att) = card(&t, a, &iid, "again", json!({})).await;
    assert!(att["route"]["experiment"].is_object());
    assert_eq!(
        att["route"]["rule_set"]["source"]
            .as_str()
            .map(|s| s != "settings"),
        Some(true)
    );
}

async fn seed_failures(t: &T, tn: &Tenant, n: i32) {
    let tag = format!("{}sf", tn.id);
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method) SELECT $1 || g, $2, 10000, 'INR', 'requires_payment_method', 'automatic' FROM generate_series(1, $3) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(n)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, failure_class, created_at) SELECT 'a' || $1 || g, $2, $1 || g, 'razorpay', 'upi', 'failed', 10000, 'INR', '{}', 'declined', now() - interval '1 minute' FROM generate_series(1, $3) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(n)
        .execute(&t.root)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn downtime_alerts_are_opt_in_so_existing_subscribers_see_no_new_events(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    // a business that subscribes to every event
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["*"] }),
    )
    .await;
    let cfg = t
        .ok(a, "GET", "/v1/routing/alerts/settings", None, Value::Null)
        .await;
    assert_eq!(
        cfg["enabled"], false,
        "off until a business asks for alerts"
    );
    seed_failures(&t, a, 30).await;
    seed_failures(&t, b, 30).await;
    // neither the endpoint nor the worker raises anything
    let r = t
        .ok(a, "POST", "/v1/routing/alerts/evaluate", None, json!({}))
        .await;
    assert_eq!(r["opened"], 0, "{r}");
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(t.count("SELECT count(*) FROM provider_alerts").await, 0);
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type LIKE 'provider.alert_%'")
            .await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'provider_alert'")
            .await,
        0
    );

    // opting in is per business: A asks, B did not
    t.ok(
        a,
        "PUT",
        "/v1/routing/alerts/settings",
        None,
        json!({ "enabled": true, "min_samples": 5 }),
    )
    .await;
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM provider_alerts WHERE tenant_id = '{}'",
            a.id
        ))
        .await,
        1
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM provider_alerts WHERE tenant_id = '{}'",
            b.id
        ))
        .await,
        0
    );
    assert_eq!(t.count(&format!("SELECT count(*) FROM outbox_events WHERE type = 'provider.alert_opened' AND tenant_id = '{}'", a.id)).await, 1);
    assert_eq!(t.count(&format!("SELECT count(*) FROM outbox_events WHERE type LIKE 'provider.alert_%' AND tenant_id = '{}'", b.id)).await, 0);
    // and it can be turned off again
    t.ok(
        a,
        "PUT",
        "/v1/routing/alerts/settings",
        None,
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/routing/alerts/settings", None, Value::Null)
            .await["enabled"],
        false
    );
}
