//! Success-rate-aware routing, the circuit breaker and failover, against the simulators only.
//!
//! The rule under test above all: failover may use another provider only after a definitive failure, and
//! never while an earlier attempt on the same payment is unresolved. There must be no path to two live
//! attempts, hence no path to a double charge.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn fees(t: &T, tn: &Tenant, extra_rules: Value) {
    t.ok(
        tn,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": {
            "rules": extra_rules,
            "fees": {
                "razorpay": { "card": { "bps": 100, "fixed_minor": 0 } },
                "cashfree": { "card": { "bps": 200, "fixed_minor": 0 } }
            } } }),
    )
    .await;
}

async fn outage(t: &T, provider: &str, mode: &str) {
    t.mock_post("outage", json!({ "provider": provider, "mode": mode }))
        .await;
}

/// An intent that may use either provider.
async fn open_intent(t: &T, tn: &Tenant) -> String {
    t.intent(tn, 50_000, json!({ "allowed_providers": null }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn attempt(t: &T, tn: &Tenant, iid: &str, key: &str) -> (StatusCode, Value) {
    t.call(
        tn,
        "POST",
        &format!("/v1/payment_intents/{iid}/attempts"),
        Some(key),
        json!({ "method": "card" }),
    )
    .await
}

async fn considered<'a>(att: &'a Value, provider: &str) -> &'a Value {
    att["route"]["considered"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["provider"] == provider)
        .unwrap_or_else(|| panic!("{provider} not considered in {}", att["route"]))
}

async fn health(t: &T, tn: &Tenant) -> Value {
    t.ok(tn, "GET", "/v1/routing/health", None, Value::Null)
        .await
}

fn method_row<'a>(h: &'a Value, provider: &str) -> &'a Value {
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

#[sqlx::test(migrations = "./migrations")]
async fn a_definitive_failure_fails_over_but_cancel_and_timeout_never_do(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;

    // ---- provider refuses the request before anything is created: the next attempt may use the other provider
    outage(&t, "razorpay", "reject").await;
    let i1 = open_intent(&t, a).await;
    let (s, first) = attempt(&t, a, &i1, "k1").await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    assert_eq!(
        first["provider"], "razorpay",
        "the cheaper provider goes first"
    );
    assert_eq!(first["status"], "failed");
    let class: Option<String> =
        sqlx::query_scalar("SELECT failure_class FROM payment_attempts WHERE id = $1")
            .bind(first["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(class.as_deref(), Some("provider_rejected"));

    let (s, second) = attempt(&t, a, &i1, "k2").await;
    assert_eq!(s, StatusCode::CREATED, "{second}");
    assert_eq!(
        second["provider"], "cashfree",
        "failover: {}",
        second["route"]
    );
    assert_eq!(second["route"]["failover_from"], json!(["razorpay"]));
    assert_eq!(
        considered(&second, "razorpay").await["outcome"],
        "skipped_failover"
    );
    assert!(
        second["route"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("failover"),
        "{}",
        second["route"]
    );

    // ---- idempotency: replaying either key returns the same attempt, and creates nothing
    let orders_before = t.mock_stats().await["cashfree_orders"].as_i64().unwrap();
    let (s, again) = attempt(&t, a, &i1, "k2").await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(again["id"], second["id"]);
    let (_, again1) = attempt(&t, a, &i1, "k1").await;
    assert_eq!(again1["id"], first["id"]);
    assert_eq!(again1["provider"], "razorpay", "a replay is not re-routed");
    assert_eq!(
        t.mock_stats().await["cashfree_orders"].as_i64().unwrap(),
        orders_before
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE intent_id = '{i1}'"
        ))
        .await,
        2
    );
    // and while the failover attempt is live, no third one can be started
    let (s, e) = attempt(&t, a, &i1, "k3").await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("attempt_in_progress"))
    );

    // ---- the provider answered "failed" (a declined payment) after creating it: failover is allowed too
    outage(&t, "razorpay", "off").await;
    let i2 = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &i2, "d1").await;
    assert_eq!(att["provider"], "razorpay");
    assert_eq!(att["status"], "pending");
    let failed = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "failed" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &a.id, &failed["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    let (_, retry) = attempt(&t, a, &i2, "d2").await;
    assert_eq!(retry["provider"], "cashfree", "{}", retry["route"]);
    assert_eq!(retry["route"]["failover_from"], json!(["razorpay"]));

    // ---- a merchant cancellation proves nothing about the money (a late capture is possible): no switch
    let i3 = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &i3, "c1").await;
    assert_eq!(att["provider"], "razorpay");
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/payment_attempts/{}/cancel",
            att["id"].as_str().unwrap()
        ),
        None,
        Value::Null,
    )
    .await;
    let (_, retry) = attempt(&t, a, &i3, "c2").await;
    assert_eq!(retry["provider"], "razorpay", "{}", retry["route"]);
    assert!(retry["route"].get("failover_from").is_none());

    // ---- an unknown outcome (timeout) never fails over, however the other provider looks
    outage(&t, "razorpay", "hang").await;
    let i4 = open_intent(&t, a).await;
    let cashfree_orders = t.mock_stats().await["cashfree_orders"].as_i64().unwrap();
    let (s, unk) = attempt(&t, a, &i4, "u1").await;
    assert_eq!(s, StatusCode::CREATED, "{unk}");
    assert_eq!(unk["provider"], "razorpay");
    assert_eq!(unk["status"], "unknown");
    for (k, mode) in [("u2", "hang"), ("u3", "off")] {
        outage(&t, "razorpay", mode).await;
        let (s, e) = attempt(&t, a, &i4, k).await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::CONFLICT, Some("payment_outcome_unresolved")),
            "{e}"
        );
    }
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE intent_id = '{i4}'"
        ))
        .await,
        1,
        "no second attempt exists anywhere"
    );
    assert_eq!(
        t.mock_stats().await["cashfree_orders"].as_i64().unwrap(),
        cashfree_orders,
        "the other provider was never contacted"
    );
    // a 503 is no more definitive than a timeout
    outage(&t, "razorpay", "unavailable").await;
    let i5 = t.intent(a, 50_000, json!({})).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, unk) = attempt(&t, a, &i5, "v1").await;
    assert_eq!(unk["status"], "unknown");
    let (s, _) = attempt(&t, a, &i5, "v2").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        t.mock_stats().await["cashfree_orders"].as_i64().unwrap(),
        cashfree_orders
    );
    assert!(
        t.count(&format!("SELECT count(*) FROM recon_cases WHERE kind = 'unresolved_payment' AND subject_id = '{}'", unk["id"].as_str().unwrap())).await >= 1
    );
}

/// The first attempt times out after the provider accepted it. Only asking the provider can settle it.
#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_attempt_is_settled_by_asking_the_provider_and_only_then_can_another_provider_be_used(
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
    let upi = |key: &'static str, iid: String| {
        let t = &t;
        async move {
            t.call(
                a,
                "POST",
                &format!("/v1/payment_intents/{iid}/attempts"),
                Some(key),
                json!({ "method": "upi", "vpa": "ravi@ybl" }),
            )
            .await
        }
    };
    // two payments: one the provider later says failed, one it says succeeded
    for (outcome, tag) in [("FAILED", "x"), ("SUCCESS", "y")] {
        let iid = open_intent(&t, a).await;
        t.mock_post(
            "behaviour",
            json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
        )
        .await;
        let (_, unk) = upi(if tag == "x" { "x1" } else { "y1" }, iid.clone()).await;
        assert_eq!(unk["provider"], "cashfree");
        assert_eq!(unk["status"], "unknown", "{unk}");
        let id = unk["id"].as_str().unwrap();
        let razorpay_orders = t.mock_stats().await["razorpay_orders"].as_i64().unwrap();

        // unresolved: no other provider, no matter how often it is tried
        for k in ["a", "b"] {
            let (s, e) = upi(
                if tag == "x" {
                    ["x2", "x3"][(k == "b") as usize]
                } else {
                    ["y2", "y3"][(k == "b") as usize]
                },
                iid.clone(),
            )
            .await;
            assert_eq!(
                (s, e["error"]["code"].as_str()),
                (StatusCode::CONFLICT, Some("payment_outcome_unresolved"))
            );
        }
        // asking while the provider still shows it pending: now the state is known (pending), and a live
        // attempt blocks every new one just the same
        let asked = t
            .ok(
                a,
                "POST",
                &format!("/v1/payment_attempts/{id}/refresh"),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(asked["status"], "pending", "{asked}");
        let (s, e) = upi(if tag == "x" { "x4" } else { "y4" }, iid.clone()).await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::CONFLICT, Some("attempt_in_progress"))
        );
        assert_eq!(
            t.mock_stats().await["razorpay_orders"].as_i64().unwrap(),
            razorpay_orders
        );

        // the provider's answer arrives
        t.mock_post(
            "cashfree/pay",
            json!({ "order_id": id, "outcome": outcome }),
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
        if outcome == "FAILED" {
            assert_eq!(asked["status"], "failed", "{asked}");
            // now, and only now, another provider may take over
            let (s, next) = upi("x5", iid.clone()).await;
            assert_eq!(s, StatusCode::CREATED, "{next}");
            assert_eq!(next["provider"], "razorpay", "{}", next["route"]);
            assert_eq!(next["route"]["failover_from"], json!(["cashfree"]));
        } else {
            // it had taken the money: a second attempt would be a double charge, and is refused
            assert_eq!(asked["status"], "captured", "{asked}");
            let (s, e) = upi("y5", iid.clone()).await;
            assert_eq!(
                (s, e["error"]["code"].as_str()),
                (StatusCode::CONFLICT, Some("attempt_in_progress")),
                "{e}"
            );
            assert_eq!(
                t.mock_stats().await["razorpay_orders"].as_i64().unwrap(),
                razorpay_orders
            );
            assert_eq!(
                t.count(&format!(
                    "SELECT count(*) FROM payment_attempts WHERE intent_id = '{iid}'"
                ))
                .await,
                1
            );
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn two_simultaneous_attempts_on_one_payment_create_exactly_one(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    let iid = open_intent(&t, a).await;
    let before = t.mock_stats().await;
    let total =
        |s: &Value| s["razorpay_orders"].as_i64().unwrap() + s["cashfree_orders"].as_i64().unwrap();
    let (x, y) = tokio::join!(
        attempt(&t, a, &iid, "race-1"),
        attempt(&t, a, &iid, "race-2")
    );
    let mut codes = [x.0.as_u16(), y.0.as_u16()];
    codes.sort_unstable();
    assert_eq!(codes, [201, 409], "{x:?} {y:?}");
    assert_eq!(
        total(&t.mock_stats().await),
        total(&before) + 1,
        "one order at one provider"
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE intent_id = '{iid}'"
        ))
        .await,
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failing_provider_is_degraded_skipped_probed_and_recovers_with_an_audit_trail(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a, json!([])).await;
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "min_samples": 5, "window_attempts": 5, "cooldown_seconds": 10 }),
    )
    .await;
    outage(&t, "razorpay", "reject").await;

    // five refused payments (each its own payment): the sample is big enough only at the fifth
    for i in 0..5 {
        let iid = open_intent(&t, a).await;
        let (_, att) = attempt(&t, a, &iid, &format!("f{i}")).await;
        assert_eq!(att["provider"], "razorpay", "attempt {i}: {}", att["route"]);
        assert_eq!(att["status"], "failed");
    }
    let h = health(&t, a).await;
    assert_eq!(method_row(&h, "razorpay")["signal"]["samples"], 5);
    assert_eq!(method_row(&h, "razorpay")["signal"]["success_rate_bps"], 0);
    // reading health is read-only: it shows what routing will do, and records nothing
    assert_eq!(method_row(&h, "razorpay")["stored_state"], "healthy");
    assert_eq!(method_row(&h, "razorpay")["state"], "degraded");
    assert_eq!(
        t.count("SELECT count(*) FROM provider_health_state").await,
        0
    );

    // the next payment: routing sees the signal, degrades razorpay, skips it, and records why
    let iid = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &iid, "s1").await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    let c = considered(&att, "razorpay").await;
    assert_eq!(c["outcome"], "skipped_degraded");
    assert_eq!(c["signal"]["samples"], 5);
    assert_eq!(c["signal"]["failures"], 5);
    assert_eq!(c["signal"]["breaker"], "degraded");
    assert!(considered(&att, "cashfree").await["signal"].is_object());
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'provider.degraded'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'provider.degraded'")
            .await,
        1
    );
    let h = health(&t, a).await;
    assert_eq!(method_row(&h, "razorpay")["stored_state"], "degraded");
    assert!(
        method_row(&h, "razorpay")["reason"]
            .as_str()
            .unwrap()
            .contains("5 of the last 5")
    );
    assert_eq!(h["recent_changes"][0]["event"], "provider.degraded");

    // inside the cool-down: skipped again, and it is not degraded a second time
    let iid = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &iid, "s2").await;
    assert_eq!(att["provider"], "cashfree");
    assert_eq!(
        t.count("SELECT count(*) FROM payment_attempts WHERE provider = 'razorpay'")
            .await,
        5,
        "no new payment went to the degraded provider"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'provider.degraded'")
            .await,
        1
    );

    // cool-down over (the clock is moved, not waited for): exactly one payment probes it
    let age = "UPDATE provider_health_state SET degraded_since = now() - interval '1 hour', last_probe_at = NULL";
    sqlx::query(age).execute(&t.root).await.unwrap();
    assert_eq!(
        method_row(&health(&t, a).await, "razorpay")["state"],
        "probe_due"
    );
    let iid = open_intent(&t, a).await;
    let (_, probe) = attempt(&t, a, &iid, "p1").await;
    assert_eq!(probe["provider"], "razorpay");
    assert_eq!(considered(&probe, "razorpay").await["outcome"], "probe");
    assert_eq!(probe["status"], "failed", "still down: the probe failed");
    // the very next payment is not a second probe, and the provider stays degraded
    let iid = open_intent(&t, a).await;
    let (_, next) = attempt(&t, a, &iid, "p2").await;
    assert_eq!(next["provider"], "cashfree");
    assert_eq!(
        method_row(&health(&t, a).await, "razorpay")["stored_state"],
        "degraded"
    );

    // the outage ends. After another cool-down the probe goes through and is left pending at the provider...
    outage(&t, "razorpay", "off").await;
    sqlx::query(age).execute(&t.root).await.unwrap();
    let iid_probe = open_intent(&t, a).await;
    let (_, probe) = attempt(&t, a, &iid_probe, "p3").await;
    assert_eq!(probe["provider"], "razorpay");
    assert_eq!(probe["status"], "pending");
    // ...while that probe is out, nothing else is sent to it
    sqlx::query(age).execute(&t.root).await.unwrap();
    let iid = open_intent(&t, a).await;
    let (_, other) = attempt(&t, a, &iid, "p4").await;
    assert_eq!(
        other["provider"], "cashfree",
        "a probe is already out: {}",
        other["route"]
    );
    // the payer pays: the probe succeeded
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": probe["provider_order_ref"], "outcome": "captured" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &a.id, &paid["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    // the next payment notices, records the recovery and goes to the cheaper provider again
    let iid = open_intent(&t, a).await;
    let (_, back) = attempt(&t, a, &iid, "r1").await;
    assert_eq!(back["provider"], "razorpay", "{}", back["route"]);
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'provider.recovered'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'provider.recovered'")
            .await,
        1
    );
    let h = health(&t, a).await;
    assert_eq!(method_row(&h, "razorpay")["stored_state"], "healthy");
    assert_eq!(h["recent_changes"][0]["event"], "provider.recovered");
    assert_eq!(h["recent_changes"][1]["event"], "provider.degraded");
    // the old failures are not held against it again: the window started afresh at the recovery
    assert_eq!(method_row(&h, "razorpay")["signal"]["failures"], 0);
    let iid = open_intent(&t, a).await;
    let (_, again) = attempt(&t, a, &iid, "r2").await;
    assert_eq!(again["provider"], "razorpay");
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'provider.degraded'")
            .await,
        1
    );
}

/// Insert resolved history for a provider so a signal exists without a hundred API calls.
async fn seed(t: &T, tn: &Tenant, provider: &str, ok: i32, bad: i32) {
    let tag = format!("{}{provider}{ok}x{bad}", tn.id);
    for (n, status, class) in [(ok, "captured", None), (bad, "failed", Some("declined"))] {
        if n == 0 {
            continue;
        }
        sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method) SELECT $1 || $4 || g, $2, 10000, 'INR', 'requires_payment_method', 'automatic' FROM generate_series(1, $3) g")
            .bind(&tag)
            .bind(&tn.id)
            .bind(n)
            .bind(status)
            .execute(&t.root)
            .await
            .unwrap();
        sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, failure_class) SELECT 'a' || $1 || $6 || g, $2, $1 || $6 || g, $3, 'card', $4, 10000, 'INR', '{}', $5 FROM generate_series(1, $7) g")
            .bind(&tag)
            .bind(&tn.id)
            .bind(provider)
            .bind(status)
            .bind(class)
            .bind(status)
            .bind(n)
            .execute(&t.root)
            .await
            .unwrap();
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rule_can_prefer_the_better_success_rate_but_only_on_enough_data(pool: PgPool) {
    let t = setup(pool).await;
    let rule = json!([{ "method": "card", "prefer_success_rate": true }]);

    // A: both providers have a real history, cashfree is clearly better although dearer
    let a = &t.a;
    fees(&t, a, rule.clone()).await;
    seed(&t, a, "razorpay", 15, 5).await; // 75%
    seed(&t, a, "cashfree", 19, 1).await; // 95%
    let iid = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &iid, "q1").await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    assert!(
        att["route"]["reasons"]
            .to_string()
            .contains("best recent success rate")
    );
    let rz = considered(&att, "razorpay").await;
    assert_eq!(rz["signal"]["success_rate_bps"], 7500);
    assert_eq!(rz["outcome"], "not_chosen");

    // without the option, fee decides as it always did
    fees(&t, a, json!([])).await;
    let iid = open_intent(&t, a).await;
    let (_, att) = attempt(&t, a, &iid, "q2").await;
    assert_eq!(att["provider"], "razorpay");

    // B: the better provider has only 3 attempts: a tiny sample never flips routing
    let b = &t.b;
    fees(&t, b, rule).await;
    seed(&t, b, "razorpay", 15, 5).await;
    seed(&t, b, "cashfree", 3, 0).await;
    let iid = open_intent(&t, b).await;
    let (_, att) = attempt(&t, b, &iid, "q3").await;
    assert_eq!(att["provider"], "razorpay", "{}", att["route"]);
    assert!(
        att["route"]["reasons"]
            .to_string()
            .contains("not enough recent attempts")
    );
    assert!(considered(&att, "cashfree").await["signal"]["success_rate_bps"].is_null());
}

#[sqlx::test(migrations = "./migrations")]
async fn thresholds_are_per_tenant_validated_scoped_and_audited(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let defaults = t
        .ok(a, "GET", "/v1/routing/health/settings", None, Value::Null)
        .await;
    assert_eq!(defaults["window_attempts"], 100);
    assert_eq!(defaults["min_samples"], 10);
    assert_eq!(defaults["failover_enabled"], true);

    let (s, e) = t
        .call(
            a,
            "PUT",
            "/v1/routing/health/settings",
            None,
            json!({ "min_samples": 1 }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("invalid_health_settings")
        )
    );
    let (s, _) = t
        .call(
            a,
            "PUT",
            "/v1/routing/health/settings",
            None,
            json!({ "recover_failure_pct": 80 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    let set = t
        .ok(
            a,
            "PUT",
            "/v1/routing/health/settings",
            None,
            json!({ "degrade_failure_pct": 70, "failover_enabled": false }),
        )
        .await;
    assert_eq!(set["degrade_failure_pct"], 70);
    assert_eq!(
        set["window_minutes"], 30,
        "fields not sent keep their value"
    );
    // another tenant is untouched
    assert_eq!(
        t.ok(b, "GET", "/v1/routing/health/settings", None, Value::Null)
            .await["degrade_failure_pct"],
        50
    );
    assert_eq!(
        t.count(
            "SELECT count(*) FROM audit_events WHERE action = 'routing_health.settings_updated'"
        )
        .await,
        1
    );

    // read needs payments:read, change needs admin
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "reader", "scopes": ["payments:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&ro, "GET", "/v1/routing/health", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/routing/health/settings", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(
            &ro,
            "PUT",
            "/v1/routing/health/settings",
            None,
            json!({ "min_samples": 8 })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "nobody", "scopes": ["refunds:write"] }),
        )
        .await;
    let none = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&none, "GET", "/v1/routing/health", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // failover off: after a definitive failure the same (cheaper) provider is simply tried again
    fees(&t, a, json!([])).await;
    outage(&t, "razorpay", "reject").await;
    let iid = open_intent(&t, a).await;
    attempt(&t, a, &iid, "z1").await;
    let (_, second) = attempt(&t, a, &iid, "z2").await;
    assert_eq!(second["provider"], "razorpay", "{}", second["route"]);
    assert!(second["route"].get("failover_from").is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn health_state_and_thresholds_stay_inside_their_tenant(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "min_samples": 3, "window_attempts": 5 }),
    )
    .await;
    seed(&t, a, "razorpay", 0, 6).await;
    fees(&t, a, json!([])).await;
    let iid = open_intent(&t, a).await;
    attempt(&t, a, &iid, "t1").await; // routing degrades razorpay for A
    assert_eq!(
        t.count("SELECT count(*) FROM provider_health_state").await,
        1
    );

    // as the application role, scoped to B: nothing of A's is visible, and A's rows cannot be written
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
        .await
        .unwrap();
    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_health_state")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 0);
    let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM routing_health_config")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 0);
    let forged = sqlx::query("INSERT INTO provider_health_state (tenant_id, provider, method, state) VALUES ($1, 'razorpay', 'card', 'degraded')")
        .bind(&a.id)
        .execute(&mut *tx)
        .await;
    assert!(
        forged.is_err(),
        "row-level security refuses another tenant's row"
    );
    drop(tx);
    let hb = health(&t, b).await;
    assert!(method_row_opt(&hb, "razorpay").is_none());
    assert_eq!(hb["recent_changes"].as_array().unwrap().len(), 0);
    assert_eq!(
        health(&t, a).await["recent_changes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn method_row_opt<'a>(h: &'a Value, provider: &str) -> Option<&'a Value> {
    h["providers"]
        .as_array()?
        .iter()
        .find(|p| p["provider"] == provider)?["methods"]
        .as_array()?
        .iter()
        .find(|m| m["method"] == "card")
}
