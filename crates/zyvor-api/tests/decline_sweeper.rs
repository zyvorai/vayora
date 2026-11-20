// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Decline classes, the health sweeper and the single source of truth for routing health, against the
//! simulators only.
//!
//! What is under test: a payer's decline (insufficient funds, the payer left) is not held against a provider
//! and not a reason to fail over; anything unrecognised still counts against it; a quiet business degrades and
//! recovers through the worker without a payment, exactly once per transition however many workers run; and
//! `GET /v1/routing/health` says what routing then does. Time is moved by changing stored timestamps, as in
//! `failover.rs`: the test clock (`clock.rs`) is a calendar date and does not drive the minutes-long health window.

mod common;

use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_api::{
    health::sweep_tenant,
    payments::{Incoming, apply_provider_state},
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
                "razorpay": { "card": { "bps": 100, "fixed_minor": 0 } },
                "cashfree": { "card": { "bps": 200, "fixed_minor": 0 } }
            } } }),
    )
    .await;
}

async fn open_intent(t: &T, tn: &Tenant, allowed: Value) -> String {
    t.intent(tn, 50_000, json!({ "allowed_providers": allowed }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn attempt(t: &T, tn: &Tenant, iid: &str, key: &str) -> Value {
    let (s, v) = t
        .call(
            tn,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some(key),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(s, axum::http::StatusCode::CREATED, "{v}");
    v
}

/// A fresh payment on one provider, left pending at the provider. Returns (intent id, attempt id).
async fn pending_on(t: &T, tn: &Tenant, provider: &str, key: &str) -> (String, String) {
    let iid = open_intent(t, tn, json!([provider])).await;
    let att = attempt(t, tn, &iid, key).await;
    assert_eq!(att["provider"], provider);
    (iid, att["id"].as_str().unwrap().to_string())
}

async fn class_of(t: &T, attempt_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT failure_class FROM payment_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_one(&t.root)
        .await
        .unwrap()
}

/// Record a failure the way every path (API, webhook, resolver) does.
async fn fail(t: &T, tn: &Tenant, attempt_id: &str, reason: Option<&str>, raw: Value, src: &str) {
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &tn.id)
        .await
        .unwrap();
    apply_provider_state(
        &mut tx,
        &tn.id,
        attempt_id,
        &Incoming {
            status: Attempt::Failed,
            failure_reason: reason.map(str::to_string),
            raw: Some(raw),
            ..Incoming::default()
        },
        src,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Insert resolved history for a provider's card payments, `age_mins` old, without a hundred API calls.
async fn seed(
    t: &T,
    tn: &Tenant,
    provider: &str,
    (status, class): (&str, Option<&str>),
    n: i32,
    tag: &str,
    age_mins: i32,
) {
    let tag = format!("{}{tag}", tn.id);
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method) SELECT $1 || g, $2, 10000, 'INR', 'requires_payment_method', 'automatic' FROM generate_series(1, $3) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(n)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, failure_class, created_at) SELECT 'a' || $1 || g, $2, $1 || g, $3, 'card', $4, 10000, 'INR', '{}', $5, now() - make_interval(mins => $7) FROM generate_series(1, $6) g")
        .bind(&tag)
        .bind(&tn.id)
        .bind(provider)
        .bind(status)
        .bind(class)
        .bind(n)
        .bind(age_mins)
        .execute(&t.root)
        .await
        .unwrap();
}

const OK: (&str, Option<&str>) = ("captured", None);
const PAYER: (&str, Option<&str>) = ("failed", Some("payer_declined"));
const UNKNOWN: (&str, Option<&str>) = ("failed", Some("declined"));
const FAULT: (&str, Option<&str>) = ("failed", Some("provider_fault"));

async fn health(t: &T, tn: &Tenant) -> Value {
    t.ok(tn, "GET", "/v1/routing/health", None, Value::Null)
        .await
}

fn card_row<'a>(h: &'a Value, provider: &str) -> &'a Value {
    h["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == provider)
        .and_then(|p| {
            p["methods"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["method"] == "card")
        })
        .unwrap_or_else(|| panic!("no card row for {provider}: {h}"))
}

async fn audit_count(t: &T, action: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM audit_events WHERE action = '{action}'"
    ))
    .await
}

async fn event_count(t: &T, kind: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM outbox_events WHERE type = '{kind}'"
    ))
    .await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_attempt_is_classified_by_who_caused_it(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;

    // the payer's doing: the plain words the simulator and the subscription tests use
    let (_, p1) = pending_on(&t, a, "razorpay", "c1").await;
    fail(&t, a, &p1, Some("insufficient_funds"), json!({}), "webhook").await;
    assert_eq!(class_of(&t, &p1).await.as_deref(), Some("payer_declined"));

    // Cashfree: the payer leaving is the payer's doing; a plain FAILED says nothing about whose fault
    let (_, p2) = pending_on(&t, a, "cashfree", "c2").await;
    fail(
        &t,
        a,
        &p2,
        None,
        json!({ "payment_status": "USER_DROPPED" }),
        "webhook",
    )
    .await;
    assert_eq!(class_of(&t, &p2).await.as_deref(), Some("payer_declined"));
    let (_, p3) = pending_on(&t, a, "cashfree", "c3").await;
    fail(
        &t,
        a,
        &p3,
        Some("Payment declined by payer bank"),
        json!({ "payment_status": "FAILED" }),
        "webhook",
    )
    .await;
    assert_eq!(
        class_of(&t, &p3).await.as_deref(),
        Some("declined"),
        "unknown"
    );

    // the provider's own failure
    let (_, p4) = pending_on(&t, a, "razorpay", "c4").await;
    fail(&t, a, &p4, Some("failed_at_provider"), json!({}), "webhook").await;
    assert_eq!(class_of(&t, &p4).await.as_deref(), Some("provider_fault"));

    // a reason nobody recognises is unknown, and unknown counts against the provider
    let (_, p5) = pending_on(&t, a, "razorpay", "c5").await;
    fail(
        &t,
        a,
        &p5,
        Some("Payment failed at bank"),
        json!({}),
        "webhook",
    )
    .await;
    assert_eq!(class_of(&t, &p5).await.as_deref(), Some("declined"));

    // a timeout that the provider later confirms as failed is the provider side, with no reason given
    t.mock_post("outage", json!({ "provider": "razorpay", "mode": "hang" }))
        .await;
    let iid = open_intent(&t, a, json!(["razorpay"])).await;
    let unk = attempt(&t, a, &iid, "c6").await;
    assert_eq!(unk["status"], "unknown");
    t.mock_post("outage", json!({ "provider": "razorpay", "mode": "off" }))
        .await;
    let id6 = unk["id"].as_str().unwrap();
    fail(&t, a, id6, None, json!({}), "resolver").await;
    assert_eq!(class_of(&t, id6).await.as_deref(), Some("provider_fault"));

    // a merchant cancellation is neither
    let (_, p7) = pending_on(&t, a, "razorpay", "c7").await;
    fail(&t, a, &p7, None, json!({}), "cancel").await;
    assert_eq!(class_of(&t, &p7).await.as_deref(), Some("cancelled"));

    // the column accepts exactly the known classes
    let bad = sqlx::query("UPDATE payment_attempts SET failure_class = 'made_up' WHERE id = $1")
        .bind(&p1)
        .execute(&t.root)
        .await;
    assert!(bad.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn payer_declines_are_shown_but_never_degrade_a_provider_or_sway_success_rate_routing(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    fees(&t, a, json!([])).await;
    fees(&t, b, json!([])).await;

    // A: razorpay has 20 declines by payers and 12 good payments. B: the same 20 declines are unrecognised.
    seed(&t, a, "razorpay", OK, 12, "ok", 1).await;
    seed(&t, a, "razorpay", PAYER, 20, "py", 1).await;
    seed(&t, b, "razorpay", OK, 12, "ok", 1).await;
    seed(&t, b, "razorpay", UNKNOWN, 20, "un", 1).await;

    let h = health(&t, a).await;
    let row = card_row(&h, "razorpay");
    assert_eq!(row["payer_declines"], 20, "visible");
    assert_eq!(row["signal"]["payer_declines"], 20);
    assert_eq!(row["signal"]["samples"], 12, "not judged");
    assert_eq!(row["signal"]["failures"], 0);
    assert_eq!(row["signal"]["success_rate_bps"], 10_000);
    assert_eq!(row["state"], "healthy");
    // the same numbers with the cause unknown: that is a failing provider
    let hb = health(&t, b).await;
    let rowb = card_row(&hb, "razorpay");
    assert_eq!(rowb["payer_declines"], 0);
    assert_eq!(rowb["signal"]["failures"], 20);
    assert_eq!(rowb["state"], "degraded");

    // routing agrees: A keeps using the cheaper provider and records nothing; B routes away and records why
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "n1").await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    assert_eq!(audit_count(&t, "provider.degraded").await, 0);
    let iid = open_intent(&t, b, Value::Null).await;
    let att = attempt(&t, b, &iid, "n2").await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn success_rate_routing_ignores_payer_declines(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(
        &t,
        a,
        json!([{ "method": "card", "prefer_success_rate": true }]),
    )
    .await;
    // identical provider quality (19 of 20); razorpay also had 80 payers short of money
    seed(&t, a, "razorpay", OK, 19, "rok", 1).await;
    seed(&t, a, "razorpay", UNKNOWN, 1, "rbad", 1).await;
    seed(&t, a, "razorpay", PAYER, 80, "rpay", 1).await;
    seed(&t, a, "cashfree", OK, 19, "cok", 1).await;
    seed(&t, a, "cashfree", UNKNOWN, 1, "cbad", 1).await;
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "s1").await;
    // within the margin, so fee decides (razorpay is cheaper). Counting the payer declines it would have lost.
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    assert!(
        att["route"]["reasons"]
            .to_string()
            .contains("within 200 bps")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payer_decline_does_not_move_the_payment_to_another_provider_but_a_provider_fault_does(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;

    // the payer had no money: retrying elsewhere is pointless, so the retry stays with the provider
    let iid = open_intent(&t, a, Value::Null).await;
    let first = attempt(&t, a, &iid, "f1").await;
    assert_eq!(first["provider"], "razorpay");
    fail(
        &t,
        a,
        first["id"].as_str().unwrap(),
        Some("insufficient_funds"),
        json!({}),
        "webhook",
    )
    .await;
    let retry = attempt(&t, a, &iid, "f2").await;
    assert_eq!(retry["provider"], "razorpay", "{}", retry["route"]);
    assert!(retry["route"].get("failover_from").is_none());

    // the provider's own failure does move it
    let iid = open_intent(&t, a, Value::Null).await;
    let first = attempt(&t, a, &iid, "g1").await;
    fail(
        &t,
        a,
        first["id"].as_str().unwrap(),
        Some("failed_at_provider"),
        json!({}),
        "webhook",
    )
    .await;
    let retry = attempt(&t, a, &iid, "g2").await;
    assert_eq!(retry["provider"], "cashfree", "{}", retry["route"]);
    assert_eq!(retry["route"]["failover_from"], json!(["razorpay"]));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_sweeper_degrades_and_recovers_a_quiet_business_once_per_transition(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    let pool = &t.state.pool;

    // nothing recent: nothing to do, nothing written
    let s = sweep_tenant(pool, &a.id).await.unwrap();
    assert_eq!((s.checked, s.degraded, s.recovered), (0, 0, 0));

    // ten failed payments and no new payment: routing never ran, so nothing is stored yet
    seed(&t, a, "razorpay", UNKNOWN, 10, "bad", 1).await;
    assert_eq!(
        t.count("SELECT count(*) FROM provider_health_state").await,
        0
    );
    assert_eq!(
        card_row(&health(&t, a).await, "razorpay")["stored_state"],
        "healthy"
    );

    // the sweep records the degradation, once, with the same audit entry and event a routed payment writes
    let s = sweep_tenant(pool, &a.id).await.unwrap();
    assert_eq!((s.checked, s.degraded, s.recovered), (1, 1, 0));
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
    assert_eq!(event_count(&t, "provider.degraded").await, 1);
    let h = health(&t, a).await;
    assert_eq!(card_row(&h, "razorpay")["stored_state"], "degraded");
    assert_eq!(card_row(&h, "razorpay")["routing_effect"], "skipped");
    assert_eq!(h["recent_changes"][0]["event"], "provider.degraded");
    let detail = &h["recent_changes"][0]["detail"];
    assert_eq!(detail["method"], "card");
    assert!(
        detail["reason"]
            .as_str()
            .unwrap()
            .contains("10 of the last 10")
    );

    // sweeping again, and a routed payment evaluating the same state, add nothing
    for _ in 0..3 {
        let s = sweep_tenant(pool, &a.id).await.unwrap();
        assert_eq!((s.degraded, s.recovered), (0, 0));
    }
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "q1").await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
    assert_eq!(event_count(&t, "provider.degraded").await, 1);

    // time passes with nobody paying: the cool-down ends. The sweeper never takes the probe slot or starts a payment
    let attempts_before = t.count("SELECT count(*) FROM payment_attempts").await;
    sqlx::query("UPDATE provider_health_state SET degraded_since = now() - interval '1 hour'")
        .execute(&t.root)
        .await
        .unwrap();
    let s = sweep_tenant(pool, &a.id).await.unwrap();
    assert_eq!((s.degraded, s.recovered), (0, 0));
    let h = health(&t, a).await;
    assert_eq!(card_row(&h, "razorpay")["state"], "probe_due");
    assert_eq!(card_row(&h, "razorpay")["routing_effect"], "probe_next");
    assert_eq!(
        t.count("SELECT count(*) FROM provider_health_state WHERE last_probe_at IS NOT NULL")
            .await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payment_attempts").await,
        attempts_before
    );

    // late outcomes arrive (the resolver / webhooks settle the old attempts as paid): recovered without a payment
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL WHERE provider = 'razorpay'")
        .execute(&t.root)
        .await
        .unwrap();
    let s = sweep_tenant(pool, &a.id).await.unwrap();
    assert_eq!((s.degraded, s.recovered), (0, 1));
    assert_eq!(audit_count(&t, "provider.recovered").await, 1);
    assert_eq!(event_count(&t, "provider.recovered").await, 1);
    assert_eq!(
        card_row(&health(&t, a).await, "razorpay")["stored_state"],
        "healthy"
    );
    for _ in 0..3 {
        let s = sweep_tenant(pool, &a.id).await.unwrap();
        assert_eq!((s.degraded, s.recovered), (0, 0));
    }
    assert_eq!(audit_count(&t, "provider.recovered").await, 1);
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn evidence_older_than_the_window_is_not_held_against_a_provider(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // the default window is 30 minutes: these failures are two hours old
    seed(&t, a, "razorpay", UNKNOWN, 10, "old", 120).await;
    let s = sweep_tenant(&t.state.pool, &a.id).await.unwrap();
    assert_eq!((s.checked, s.degraded), (0, 0), "nothing in the window");
    // the same failures, now recent
    sqlx::query("UPDATE payment_attempts SET created_at = now()")
        .execute(&t.root)
        .await
        .unwrap();
    let s = sweep_tenant(&t.state.pool, &a.id).await.unwrap();
    assert_eq!((s.checked, s.degraded), (1, 1));
}

#[sqlx::test(migrations = "./migrations")]
async fn several_workers_sweeping_at_once_record_each_transition_exactly_once(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    seed(&t, a, "razorpay", FAULT, 10, "bad", 1).await;

    // while another worker holds this business's sweep, this one steps aside instead of waiting or duplicating
    let mut held = t.root.begin().await.unwrap();
    let got: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtext('zyvor.health_sweep'), hashtext($1))",
    )
    .bind(&a.id)
    .fetch_one(&mut *held)
    .await
    .unwrap();
    assert!(got);
    let s = sweep_tenant(&t.state.pool, &a.id).await.unwrap();
    assert!(s.skipped_busy);
    assert_eq!((s.checked, s.degraded), (0, 0));
    assert_eq!(audit_count(&t, "provider.degraded").await, 0);
    held.rollback().await.unwrap();

    // ten at once: exactly one writes
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let pool = t.state.pool.clone();
        let id = a.id.clone();
        tasks.push(tokio::spawn(async move {
            sweep_tenant(&pool, &id).await.unwrap()
        }));
    }
    let mut degraded = 0;
    for task in tasks {
        degraded += task.await.unwrap().degraded;
    }
    assert_eq!(degraded, 1);
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
    assert_eq!(event_count(&t, "provider.degraded").await, 1);

    // and a routed payment racing a sweep cannot add a second record either
    sqlx::query("UPDATE payment_attempts SET status = 'captured', failure_class = NULL")
        .execute(&t.root)
        .await
        .unwrap();
    fees(&t, a, json!([])).await;
    let iid = open_intent(&t, a, Value::Null).await;
    let route = attempt(&t, a, &iid, "r1");
    let (_, att) = tokio::join!(sweep_tenant(&t.state.pool, &a.id), route);
    assert_eq!(att["provider"], "razorpay");
    assert_eq!(audit_count(&t, "provider.recovered").await, 1);
    assert_eq!(event_count(&t, "provider.recovered").await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_worker_runs_the_sweep_for_every_business_and_keeps_them_apart(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    seed(&t, a, "razorpay", UNKNOWN, 10, "bad", 1).await;
    seed(&t, b, "razorpay", OK, 10, "ok", 1).await;
    let client = zyvor_api::outbox::http_client();
    let st = zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(st.health_changes, 1);
    let rows: Vec<(String,)> = sqlx::query_as("SELECT tenant_id FROM provider_health_state")
        .fetch_all(&t.root)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![(a.id.clone(),)],
        "only the failing business changed"
    );
    // a second cycle changes nothing
    let st = zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(st.health_changes, 0);
    assert_eq!(audit_count(&t, "provider.degraded").await, 1);
    // turning the breaker off releases the provider on the next sweep, again without any payment
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "breaker_enabled": false }),
    )
    .await;
    let st = zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(st.health_changes, 1);
    assert_eq!(audit_count(&t, "provider.recovered").await, 1);
}

/// The old heuristic (any unresolved payment, or a majority of the last 50 failing, in the router) is gone: what the
/// health endpoint says is what routing does, and routing's reasons and explanations tell the same story.
#[sqlx::test(migrations = "./migrations")]
async fn the_health_endpoint_tells_the_truth_about_routing_and_every_decision_explains_itself(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;

    // one unresolved payment used to push razorpay down the list; now it is only a number
    seed(&t, a, "razorpay", ("unknown", None), 1, "stuck", 1).await;
    seed(&t, a, "cashfree", OK, 3, "cok", 1).await; // so cashfree has a row to read
    let h = health(&t, a).await;
    assert_eq!(card_row(&h, "razorpay")["state"], "healthy");
    assert_eq!(card_row(&h, "razorpay")["routing_effect"], "normal");
    assert_eq!(card_row(&h, "razorpay")["signal"]["unresolved"], 1);
    assert!(
        h["routing_health_rule"]
            .as_str()
            .unwrap()
            .contains("only health rule")
    );
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "e1").await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    // the sentence on every considered entry, and on the decision
    let considered = att["route"]["considered"].as_array().unwrap();
    assert_eq!(considered.len(), 2);
    for c in considered {
        assert!(
            c["explanation"].as_str().is_some_and(|e| e.len() > 10),
            "{c}"
        );
    }
    assert!(
        att["route"]["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Chosen because it has the lowest estimated fee"),
        "{}",
        att["route"]
    );
    let other = considered
        .iter()
        .find(|c| c["provider"] == "cashfree")
        .unwrap();
    assert!(
        other["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Not chosen: razorpay was used instead")
    );

    // when it does degrade, health said so before routing acted, and routing then did exactly that
    seed(&t, a, "razorpay", UNKNOWN, 10, "bad", 1).await;
    let h = health(&t, a).await;
    assert_eq!(card_row(&h, "razorpay")["state"], "degraded");
    assert_eq!(card_row(&h, "razorpay")["routing_effect"], "skipped");
    assert_eq!(card_row(&h, "cashfree")["routing_effect"], "normal");
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "e2").await;
    assert_eq!(att["provider"], "cashfree");
    let reasons = att["route"]["reasons"].to_string();
    assert!(!reasons.contains("only eligible provider"), "{reasons}");
    assert!(
        reasons.contains(
            "only provider left after skipping razorpay (degraded by the circuit breaker)"
        ),
        "{reasons}"
    );
    let skipped = att["route"]["considered"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["provider"] == "razorpay")
        .unwrap();
    assert_eq!(skipped["outcome"], "skipped_degraded");
    assert!(
        skipped["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Skipped because the circuit breaker has it degraded")
    );
    assert!(
        att["route"]["explanation"]
            .as_str()
            .unwrap()
            .contains("only provider left after skipping razorpay")
    );

    // an hour passes: the old attempts (including the stuck and the pending one, which would block a probe) are
    // an hour old, and the cool-down is over. Health says the next payment probes it, and the next payment does
    sqlx::query("UPDATE payment_attempts SET created_at = now() - interval '3 hours' WHERE provider = 'razorpay'")
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE provider_health_state SET degraded_since = now() - interval '1 hour'")
        .execute(&t.root)
        .await
        .unwrap();
    let h = health(&t, a).await;
    assert_eq!(card_row(&h, "razorpay")["routing_effect"], "probe_next");
    let iid = open_intent(&t, a, Value::Null).await;
    let att = attempt(&t, a, &iid, "e3").await;
    assert_eq!(att["provider"], "razorpay");
    assert!(
        att["route"]["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Chosen as a test")
    );

    // the breaker off: health and routing both ignore provider health
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "breaker_enabled": false }),
    )
    .await;
    let h = health(&t, a).await;
    assert!(
        h["routing_health_rule"]
            .as_str()
            .unwrap()
            .contains("switched off")
    );
}
