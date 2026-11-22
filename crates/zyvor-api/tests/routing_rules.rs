// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! The no-code routing rule builder, the dry run, versioned rule sets with rollback, and routing experiments with
//! their guardrails, against the simulators only. Rules only ever narrow what the router already allows: the hard
//! filters, the circuit breaker, failover and the "unknown payment blocks retries" rule are untouched.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

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

/// An intent that either provider may take.
async fn intent(t: &T, tn: &Tenant, extra: Value) -> String {
    let mut b = json!({ "allowed_providers": null });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.intent(tn, 50_000, b).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn attempt(t: &T, tn: &Tenant, iid: &str, body: Value) -> Value {
    let (s, b) = t
        .call(
            tn,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some(&format!("att-{}", zyvor_core::new_id("k"))),
            body,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

fn rule(name: &str, when: Value, then: Value) -> Value {
    json!({ "name": name, "when": when, "then": then })
}

async fn new_version(t: &T, tn: &Tenant, rules: Value, activate: bool) -> Value {
    let (s, b) = t
        .call(
            tn,
            "POST",
            "/v1/routing/rule_sets",
            None,
            json!({ "rules": rules, "activate": activate, "note": "test" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

async fn dry(t: &T, tn: &Tenant, body: Value) -> Value {
    let mut b = json!({ "method": "card", "amount_minor": 50_000 });
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.ok(tn, "POST", "/v1/routing/dry_run", None, b).await
}

async fn outage(t: &T, provider: &str, mode: &str) {
    t.mock_post("outage", json!({ "provider": provider, "mode": mode }))
        .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn rules_are_validated_versioned_activated_and_rolled_back(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;

    // nothing active: the settings rules (none) apply and the cheaper provider goes first
    let first = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(first["provider"], "razorpay");
    assert_eq!(first["route"]["rule_set"]["source"], "settings");

    // the validator names every mistake with the rule it is in
    let bad = json!([
        rule(
            "a",
            json!({ "methods": ["crypto"] }),
            json!({ "prefer": ["razorpay"] })
        ),
        rule("b", json!({}), json!({})),
        rule("c", json!({}), json!({ "prefer": ["nosuch"] })),
        rule(
            "d",
            json!({}),
            json!({ "exclude": ["razorpay", "cashfree"] })
        ),
        rule(
            "e",
            json!({ "time_window": { "from": "9:00", "to": "17:00" } }),
            json!({ "prefer": ["razorpay"] })
        ),
    ]);
    let rep = t
        .ok(
            a,
            "POST",
            "/v1/routing/rule_sets/validate",
            None,
            json!({ "rules": bad }),
        )
        .await;
    assert_eq!(rep["valid"], false);
    let rules_with_errors: Vec<i64> = rep["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["rule"].as_i64())
        .collect();
    for i in 0..5 {
        assert!(rules_with_errors.contains(&i), "rule {i}: {rep}");
    }
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/routing/rule_sets",
            None,
            json!({ "rules": bad }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(e["error"]["code"], "invalid_rules");
    // a typo in a condition name is refused, not silently ignored
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/routing/rule_sets",
            None,
            json!({ "rules": [{ "when": { "card_network": ["visa"] }, "then": { "prefer": ["razorpay"] } }] }),
        )
        .await;
    assert!(s.is_client_error());
    assert_eq!(
        t.count("SELECT COUNT(*) FROM routing_rule_versions").await,
        0
    );

    // version 1: cards go to cashfree. version 2: cards go to razorpay
    let v1 = new_version(
        &t,
        a,
        json!([rule(
            "cards to cashfree",
            json!({ "methods": ["card"] }),
            json!({ "prefer": ["cashfree"] })
        )]),
        true,
    )
    .await;
    assert_eq!(v1["version"], 1);
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "cashfree", "{}", att["route"]);
    assert_eq!(att["route"]["rule_set"]["version"], 1);
    assert_eq!(
        att["route"]["rule_set"]["matched_rules"][0]["name"],
        "cards to cashfree"
    );
    // a UPI payment does not match a card rule and is routed as before
    let upi = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(upi["provider"], "razorpay");
    assert_eq!(upi["route"]["rule_set"]["matched_rules"], json!([]));

    let v2 = new_version(
        &t,
        a,
        json!([rule(
            "cards to razorpay",
            json!({ "methods": ["card"] }),
            json!({ "prefer": ["razorpay"] })
        )]),
        false,
    )
    .await;
    assert_eq!(v2["version"], 2);
    assert_eq!(v2["active"], false);
    let still = dry(&t, a, json!({})).await;
    assert_eq!(still["provider"], "cashfree", "saved is not active");
    t.ok(
        a,
        "POST",
        "/v1/routing/rule_sets/2/activate",
        None,
        json!({}),
    )
    .await;
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "razorpay");
    assert_eq!(att["route"]["rule_set"]["version"], 2);

    let list = t
        .ok(a, "GET", "/v1/routing/rule_sets", None, Value::Null)
        .await;
    assert_eq!(list["active_version"], 2);
    assert_eq!(list["previous_version"], 1);
    assert_eq!(list["data"].as_array().unwrap().len(), 2);
    // a version never changes once saved
    let got = t
        .ok(a, "GET", "/v1/routing/rule_sets/1", None, Value::Null)
        .await;
    assert_eq!(got["rules"][0]["then"]["prefer"], json!(["cashfree"]));

    // roll back to what was active before
    let rb = t
        .ok(a, "POST", "/v1/routing/rule_sets/rollback", None, json!({}))
        .await;
    assert_eq!(rb["active_version"], 1);
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "cashfree");
    // and out of rule sets altogether: the settings rules apply again
    let d = t
        .ok(
            a,
            "POST",
            "/v1/routing/rule_sets/deactivate",
            None,
            json!({}),
        )
        .await;
    assert_eq!(d["active_version"], Value::Null);
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "razorpay");
    assert_eq!(att["route"]["rule_set"]["source"], "settings");
    // activating something that does not exist
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/routing/rule_sets/99/activate",
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // every change is in the audit trail
    for action in [
        "routing_rules.version_created",
        "routing_rules.activated",
        "routing_rules.rolled_back",
        "routing_rules.deactivated",
    ] {
        assert!(
            t.count(&format!(
                "SELECT COUNT(*) FROM audit_events WHERE action = '{action}'"
            ))
            .await
                >= 1,
            "{action}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn conditions_method_amount_network_time_of_day_and_segment(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    new_version(
        &t,
        a,
        json!([
            rule("rupay to cashfree", json!({ "methods": ["card"], "card_networks": ["rupay"] }), json!({ "prefer": ["cashfree"] })),
            rule("big to cashfree", json!({ "min_amount_minor": 200_000 }), json!({ "prefer": ["cashfree"] })),
            rule("night to cashfree", json!({ "methods": ["upi"], "time_window": { "from": "22:00", "to": "06:00", "utc_offset_minutes": 330 } }), json!({ "prefer": ["cashfree"] })),
            rule("new customers to cashfree", json!({ "methods": ["netbanking"], "segments": ["new"] }), json!({ "prefer": ["cashfree"] })),
        ]),
        true,
    )
    .await;
    // card network, from a network or from a BIN
    assert_eq!(
        dry(&t, a, json!({ "card_network": "visa" })).await["provider"],
        "razorpay"
    );
    assert_eq!(
        dry(&t, a, json!({ "card_network": "rupay" })).await["provider"],
        "cashfree"
    );
    assert_eq!(
        dry(&t, a, json!({ "card_bin": "606000" })).await["provider"],
        "cashfree"
    );
    assert_eq!(
        dry(&t, a, json!({})).await["provider"],
        "razorpay",
        "an unknown network matches no network rule"
    );
    // amount band
    assert_eq!(
        dry(&t, a, json!({ "amount_minor": 250_000 })).await["provider"],
        "cashfree"
    );
    // time of day: 17:00 UTC is 22:30 in India (inside), 06:00 UTC is 11:30 (outside)
    assert_eq!(
        dry(
            &t,
            a,
            json!({ "method": "upi", "at": "2026-11-18T17:00:00Z" })
        )
        .await["provider"],
        "cashfree"
    );
    assert_eq!(
        dry(
            &t,
            a,
            json!({ "method": "upi", "at": "2026-11-18T06:00:00Z" })
        )
        .await["provider"],
        "razorpay"
    );
    // new or returning
    let new = dry(&t, a, json!({ "method": "netbanking", "segment": "new" })).await;
    let ret = dry(
        &t,
        a,
        json!({ "method": "netbanking", "segment": "returning" }),
    )
    .await;
    // the simulated providers do not all offer netbanking: what matters is the rule that matched
    assert_eq!(new["matched_rules"][0]["name"], "new customers to cashfree");
    assert_eq!(ret["matched_rules"], json!([]));
    assert_eq!(ret["unmatched_rules"].as_array().unwrap().len(), 4);
    assert!(ret["unmatched_rules"].to_string().contains("returning"));
    // a real attempt uses the same facts: the card network the payer chose
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card", "instrument": { "card_network": "rupay" } }),
    )
    .await;
    assert_eq!(att["provider"], "cashfree");
    assert_eq!(
        att["route"]["rule_set"]["matched_rules"][0]["name"],
        "rupay to cashfree"
    );
    // the stored facts hold the network and no card number
    assert_eq!(att["amount_minor"], 50_000);
    let stored: Value = sqlx::query_scalar("SELECT instrument FROM payment_attempts WHERE id = $1")
        .bind(att["id"].as_str().unwrap())
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(stored["card_network"], "rupay");
    assert_eq!(stored["source"], "caller_hint");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dry_run_creates_nothing_and_can_try_unsaved_rules_and_old_versions(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    new_version(
        &t,
        a,
        json!([rule(
            "to cashfree",
            json!({}),
            json!({ "prefer": ["cashfree"] })
        )]),
        true,
    )
    .await;
    new_version(
        &t,
        a,
        json!([rule(
            "to razorpay",
            json!({}),
            json!({ "prefer": ["razorpay"] })
        )]),
        false,
    )
    .await;
    let attempts = t.count("SELECT COUNT(*) FROM payment_attempts").await;
    let audits = t.count("SELECT COUNT(*) FROM audit_events").await;
    let d = dry(&t, a, json!({})).await;
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["provider"], "cashfree");
    assert_eq!(d["rule_source"]["kind"], "rule_set");
    assert!(
        d["explanation"]
            .as_str()
            .unwrap()
            .starts_with("Chosen because")
    );
    assert!(d["decision"]["considered"].as_array().unwrap().len() >= 2);
    // another saved version
    let d = dry(&t, a, json!({ "version": 2 })).await;
    assert_eq!(d["provider"], "razorpay");
    assert_eq!(d["rule_source"]["version"], 2);
    // rules that are not saved
    let d = dry(
        &t,
        a,
        json!({ "rules": [rule("x", json!({}), json!({ "exclude": ["razorpay"] }))] }),
    )
    .await;
    assert_eq!(d["provider"], "cashfree");
    assert_eq!(d["rule_source"]["kind"], "inline");
    // unsaved rules are validated too
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/routing/dry_run",
            None,
            json!({ "method": "card", "amount_minor": 1_000, "rules": [rule("x", json!({}), json!({}))] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM payment_attempts").await,
        attempts
    );
    assert_eq!(t.count("SELECT COUNT(*) FROM audit_events").await, audits);
    // and the answer is the one a real payment gets
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "cashfree");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rule_cannot_send_a_payment_where_the_router_would_refuse(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    new_version(
        &t,
        a,
        json!([rule(
            "to cashfree",
            json!({}),
            json!({ "prefer": ["cashfree"] })
        )]),
        true,
    )
    .await;
    // this payment may only use razorpay: the preference for cashfree cannot override that
    let iid = intent(&t, a, json!({ "allowed_providers": ["razorpay"] })).await;
    let att = attempt(&t, a, &iid, json!({ "method": "card" })).await;
    assert_eq!(att["provider"], "razorpay");
    // an unknown outcome still blocks every retry, whatever the rules say
    outage(&t, "razorpay", "hang").await;
    let iid = intent(&t, a, json!({ "allowed_providers": ["razorpay"] })).await;
    let att = attempt(&t, a, &iid, json!({ "method": "card" })).await;
    assert_eq!(att["status"], "unknown");
    let (s, e) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("retry"),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert_eq!(e["error"]["code"], "payment_outcome_unresolved");
}

/// Settle a created Razorpay attempt as captured.
async fn capture(t: &T, tn: &Tenant, att: &Value) {
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "captured" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &tn.id, &paid["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn an_experiment_splits_traffic_and_stops_a_clearly_worse_variant(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    // the breaker would hide the very difference we want to measure
    t.ok(
        a,
        "PUT",
        "/v1/routing/health/settings",
        None,
        json!({ "breaker_enabled": false }),
    )
    .await;
    new_version(
        &t,
        a,
        json!([rule(
            "cards to razorpay",
            json!({ "methods": ["card"] }),
            json!({ "prefer": ["razorpay"] })
        )]),
        true,
    )
    .await;
    let v2 = new_version(
        &t,
        a,
        json!([rule(
            "cards to cashfree",
            json!({ "methods": ["card"] }),
            json!({ "prefer": ["cashfree"] })
        )]),
        false,
    )
    .await;
    assert_eq!(v2["version"], 2);

    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "bad", "variant_version": 2, "variant_percent": 90 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "bad", "variant_version": 1, "variant_percent": 10 }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the variant cannot be the active set"
    );
    let exp = t
        .ok(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "cashfree for cards", "variant_version": 2, "variant_percent": 50, "min_sample": 20, "method": "card" }),
        )
        .await;
    let eid = exp["id"].as_str().unwrap().to_string();
    assert_eq!(exp["status"], "running");
    assert_eq!(exp["control_version"], 1);
    assert_eq!(exp["verdict"]["state"], "not_enough_data");
    // only one at a time, and the active rule set cannot change under it
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "again", "variant_version": 2, "variant_percent": 10 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/routing/rule_sets/2/activate",
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert_eq!(e["error"]["code"], "experiment_running");

    // cashfree is down; razorpay works. Drive payments until both arms have enough.
    outage(&t, "cashfree", "reject").await;
    let (mut control, mut variant) = (0, 0);
    for _ in 0..300 {
        if control >= 24 && variant >= 24 {
            break;
        }
        let iid = intent(&t, a, json!({})).await;
        let att = attempt(&t, a, &iid, json!({ "method": "card" })).await;
        let arm = att["route"]["experiment"]["arm"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(att["route"]["experiment"]["id"], exp["id"]);
        if arm == "variant" {
            variant += 1;
            assert_eq!(att["provider"], "cashfree");
            assert_eq!(att["status"], "failed");
        } else {
            control += 1;
            assert_eq!(att["provider"], "razorpay");
            capture(&t, a, &att).await;
        }
        // the same payment is always in the same arm
        if control + variant == 1 {
            let again = t
                .ok(
                    a,
                    "GET",
                    &format!("/v1/routing/experiments/{eid}"),
                    None,
                    Value::Null,
                )
                .await;
            assert_eq!(again["status"], "running");
        }
    }
    let res = t
        .ok(
            a,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(res["control"]["stats"]["success_rate_bps"], 10_000);
    assert_eq!(res["variant"]["stats"]["success_rate_bps"], 0);
    assert_eq!(res["verdict"]["state"], "variant_worse", "{res}");
    assert_eq!(res["would_auto_stop"], true);
    assert_eq!(res["status"], "running", "a read never stops anything");
    assert_eq!(res["control"]["providers"][0]["provider"], "razorpay");
    assert_eq!(res["variant"]["providers"][0]["provider"], "cashfree");

    // the worker applies the guardrail
    let client = zyvor_api::outbox::http_client();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    let res = t
        .ok(
            a,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(res["status"], "auto_stopped", "{res}");
    assert!(
        res["stop_reason"]
            .as_str()
            .unwrap()
            .contains("variant success rate 0 bps")
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM recon_cases WHERE kind = 'routing_experiment_stopped'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'routing.experiment_stopped'")
            .await,
        1
    );
    assert!(
        t.count(
            "SELECT COUNT(*) FROM audit_events WHERE action = 'routing_experiment.auto_stopped'"
        )
        .await
            >= 1
    );
    // running it again changes nothing
    let again = t
        .ok(
            a,
            "POST",
            &format!("/v1/routing/experiments/{eid}/evaluate"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(again["auto_stopped_now"], false);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'routing.experiment_stopped'")
            .await,
        1
    );
    // the control rules are in force for everyone again, and a new payment is in no experiment
    outage(&t, "cashfree", "off").await;
    let att = attempt(
        &t,
        a,
        &intent(&t, a, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert_eq!(att["provider"], "razorpay");
    assert!(att["route"].get("experiment").is_none());
    // and the active rule set can be changed again
    t.ok(
        a,
        "POST",
        "/v1/routing/rule_sets/2/activate",
        None,
        json!({}),
    )
    .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn a_variant_that_cannot_route_falls_back_and_a_small_sample_stops_nothing(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    fees(&t, a).await;
    new_version(
        &t,
        a,
        json!([rule(
            "cards to razorpay",
            json!({}),
            json!({ "prefer": ["razorpay"] })
        )]),
        true,
    )
    .await;
    // the variant keeps razorpay out; cashfree is switched off, so it leaves nothing for a card payment
    new_version(
        &t,
        a,
        json!([rule(
            "no razorpay",
            json!({}),
            json!({ "exclude": ["razorpay"] })
        )]),
        false,
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/provider_accounts/cashfree/enabled",
        None,
        json!({ "enabled": false }),
    )
    .await;
    let exp = t
        .ok(a, "POST", "/v1/routing/experiments", None, json!({ "name": "no razorpay", "variant_version": 2, "variant_percent": 50, "min_sample": 20 }))
        .await;
    let eid = exp["id"].as_str().unwrap();
    let mut fell_back = 0;
    for _ in 0..40 {
        let iid = intent(&t, a, json!({})).await;
        let att = attempt(&t, a, &iid, json!({ "method": "card" })).await;
        // no payment failed to route because of the experiment
        assert_eq!(att["provider"], "razorpay");
        if att["route"]["experiment"]["fell_back_to_control"] == true {
            fell_back += 1;
            assert_eq!(att["route"]["experiment"]["arm"], "variant");
        }
    }
    assert!(fell_back > 0, "about half should have been in the variant");
    let res = t
        .ok(
            a,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(res["left_out_fell_back_to_control"], fell_back);
    assert_eq!(
        res["variant"]["stats"]["samples"], 0,
        "fallbacks are left out of the variant's numbers"
    );
    assert_eq!(res["verdict"]["state"], "not_enough_data");
    // a person can stop it
    let stopped = t
        .ok(
            a,
            "POST",
            &format!("/v1/routing/experiments/{eid}/stop"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(stopped["status"], "stopped");
    let (s, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/routing/experiments/{eid}/stop"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn rule_sets_and_experiments_are_private_to_their_business(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    fees(&t, a).await;
    new_version(
        &t,
        a,
        json!([rule("x", json!({}), json!({ "prefer": ["cashfree"] }))]),
        true,
    )
    .await;
    new_version(
        &t,
        a,
        json!([rule("y", json!({}), json!({ "prefer": ["razorpay"] }))]),
        false,
    )
    .await;
    let exp = t
        .ok(
            a,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "e", "variant_version": 2, "variant_percent": 10 }),
        )
        .await;
    let eid = exp["id"].as_str().unwrap();

    let list = t
        .ok(b, "GET", "/v1/routing/rule_sets", None, Value::Null)
        .await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    assert_eq!(list["active_version"], Value::Null);
    let (s, _) = t
        .call(b, "GET", "/v1/routing/rule_sets/1", None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "POST",
            "/v1/routing/rule_sets/1/activate",
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "GET",
            &format!("/v1/routing/experiments/{eid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "POST",
            &format!("/v1/routing/experiments/{eid}/stop"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // B is not in A's experiment and not routed by A's rules
    let att = attempt(
        &t,
        b,
        &intent(&t, b, json!({})).await,
        json!({ "method": "card" }),
    )
    .await;
    assert!(att["route"].get("experiment").is_none());
    assert_eq!(att["route"]["rule_set"]["version"], Value::Null);
    // the database, as the app role with no business set, sees nothing of it
    for table in [
        "routing_rule_versions",
        "routing_rule_active",
        "routing_experiments",
    ] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.state.pool)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
        assert!(
            t.count(&format!("SELECT count(*) FROM {table}")).await > 0,
            "{table}"
        );
    }
    // a key without the write scope can read but not change
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
    t.ok(&ro, "GET", "/v1/routing/rule_sets", None, Value::Null)
        .await;
    let (s, _) = t
        .call(
            &ro,
            "POST",
            "/v1/routing/rule_sets/rollback",
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call(
            &ro,
            "POST",
            "/v1/routing/experiments",
            None,
            json!({ "name": "e", "variant_version": 2, "variant_percent": 10 }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}
