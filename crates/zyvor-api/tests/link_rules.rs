// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Payment-link amount modes and lifecycle, payer-instrument rules, VPA and bank-account verification, the risk
//! rules engine, and payout links. Everything runs against the simulators and mocks in this repository: the
//! verification provider is the simulator (its answers verify nothing real), and Razorpay is the in-process mock.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use zyvor_api::{instrument, link_lifecycle, risk_engine};

async fn start(t: &T, tid: &str, token: &str, body: Value) -> (StatusCode, Value) {
    t.public(
        "POST",
        &format!("/v1/checkout/links/{tid}/{token}/start"),
        None,
        body,
    )
    .await
}

/// Pay a link's intent through the Razorpay mock; `extra` is merged into what the payer's bank reports.
async fn pay(t: &T, tn: &Tenant, iid: &str, outcome: &str, extra: Value) {
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(iid)
        .execute(&t.root)
        .await
        .unwrap();
    let att = t
        .ok(
            tn,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some(&format!("att-{iid}-{outcome}")),
            json!({ "method": "upi" }),
        )
        .await;
    let mut body = json!({ "order_id": att["provider_order_ref"], "outcome": outcome });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        body[k] = v;
    }
    let paid = t.mock_post("razorpay/pay", body).await;
    let (s, _) = t.deliver("razorpay", &tn.id, &paid["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
}

async fn lifecycle(t: &T, tn: &Tenant, id: &str) -> Value {
    t.ok(
        tn,
        "GET",
        &format!("/v1/payment_links/{id}/lifecycle"),
        None,
        Value::Null,
    )
    .await
}

async fn sweep_links(t: &T, tn: &Tenant) -> usize {
    link_lifecycle::sweep_tenant(&t.state, &tn.id)
        .await
        .unwrap()
}

/// Another business with its own key (and so its own rate-limit buckets).
async fn fresh_tenant(t: &T, name: &str) -> Tenant {
    let b = zyvor_api::admin::bootstrap_org(&t.state.pool, name)
        .await
        .unwrap();
    Tenant {
        id: b.test_tenant,
        key: b.test_key,
    }
}

fn limited(scopes: Value) -> Value {
    json!({ "name": "limited", "scopes": scopes })
}

// ───────────────────────────── amount modes ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn amount_modes_are_enforced_on_the_server_and_shown_to_the_payer(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // each mode demands its own fields
    for body in [
        json!({ "amount_mode": "minimum" }),
        json!({ "amount_mode": "minimum", "min_amount_minor": 500, "max_amount_minor": 900 }),
        json!({ "amount_mode": "up_to" }),
        json!({ "amount_mode": "up_to", "min_amount_minor": 1, "max_amount_minor": 900 }),
        json!({ "amount_mode": "pay_what_you_want", "min_amount_minor": 900, "max_amount_minor": 100 }),
        json!({ "amount_mode": "exact", "min_amount_minor": 100, "amount_minor": 500 }),
        json!({ "amount_mode": "minimum", "min_amount_minor": 500, "amount_minor": 500 }),
        json!({ "amount_mode": "sometimes", "min_amount_minor": 500 }),
        json!({ "amount_mode": "minimum", "min_amount_minor": 500, "open_amount": false }),
        json!({ "amount_mode": "pay_what_you_want", "suggested_amount_minor": 50, "min_amount_minor": 100 }),
    ] {
        let (s, e) = t
            .call(a, "POST", "/v1/payment_links", None, body.clone())
            .await;
        assert_eq!(s.as_u16(), 422, "{body} -> {e}");
        assert_eq!(e["error"]["code"], "invalid_amount_mode", "{body}");
    }
    assert_eq!(
        t.count("SELECT count(*) FROM payment_links").await,
        0,
        "nothing half-made"
    );

    // minimum: at least the floor, no cap
    let min = t
        .ok(a, "POST", "/v1/payment_links", None, json!({ "amount_mode": "minimum", "min_amount_minor": 50_000, "description": "Donation" }))
        .await;
    assert_eq!(min["amount_mode"], "minimum");
    let (tid, tok) = (&a.id, min["token"].as_str().unwrap());
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{tid}/{tok}"),
            Value::Null,
        )
        .await;
    assert_eq!(info["amount_mode"], "minimum");
    assert_eq!(
        info["amount_rule"],
        "Pay ₹500.00 or more. You choose the amount."
    );
    assert_eq!(info["min_amount_minor"], 50_000);
    assert_eq!(info["max_amount_minor"], Value::Null);
    for (amount, ok) in [(49_999, false), (50_000, true), (7_500_000, true)] {
        let (s, v) = start(&t, tid, tok, json!({ "amount_minor": amount })).await;
        assert_eq!(s == StatusCode::CREATED, ok, "{amount}: {v}");
        if ok {
            assert_eq!(
                v["amount_minor"], amount,
                "the intent carries what the payer chose"
            );
        } else {
            assert_eq!(v["error"]["code"], "amount_out_of_range");
        }
    }
    let (s, v) = start(&t, tid, tok, json!({})).await;
    assert_eq!(
        (s.as_u16(), v["error"]["code"].clone()),
        (422, json!("amount_required"))
    );
    let (s, _) = start(
        &t,
        tid,
        tok,
        json!({ "amount_minor": zyvor_core::MAX_AMOUNT_MINOR + 1 }),
    )
    .await;
    assert_eq!(s.as_u16(), 422, "never above the platform ceiling");
    let (s, _) = start(&t, tid, tok, json!({ "amount_minor": -5 })).await;
    assert_eq!(s.as_u16(), 422);

    // up to: 1 .. cap
    let up = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_mode": "up_to", "max_amount_minor": 90_000 }),
        )
        .await;
    let tok = up["token"].as_str().unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{tid}/{tok}"),
            Value::Null,
        )
        .await;
    assert_eq!(info["amount_rule"], "Pay any amount up to ₹900.00.");
    for (amount, ok) in [(0, false), (1, true), (90_000, true), (90_001, false)] {
        let (s, v) = start(&t, tid, tok, json!({ "amount_minor": amount })).await;
        assert_eq!(s == StatusCode::CREATED, ok, "{amount}: {v}");
    }

    // pay what you want, bounded, with a suggested amount
    let pw = t
        .ok(a, "POST", "/v1/payment_links", None, json!({ "amount_mode": "pay_what_you_want", "min_amount_minor": 10_000, "max_amount_minor": 100_000, "suggested_amount_minor": 25_000 }))
        .await;
    let tok = pw["token"].as_str().unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{tid}/{tok}"),
            Value::Null,
        )
        .await;
    assert_eq!(
        info["amount_rule"],
        "Pay what you want, between ₹100.00 and ₹1000.00."
    );
    assert_eq!(info["suggested_amount_minor"], 25_000);
    assert_eq!(
        start(&t, tid, tok, json!({ "amount_minor": 9_999 }))
            .await
            .0
            .as_u16(),
        422
    );
    assert_eq!(
        start(&t, tid, tok, json!({ "amount_minor": 100_001 }))
            .await
            .0
            .as_u16(),
        422
    );
    assert_eq!(
        start(&t, tid, tok, json!({ "amount_minor": 100_000 }))
            .await
            .0,
        StatusCode::CREATED
    );

    // an exact link takes no amount from the payer, and the older open_amount way still works as before
    let ex = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_mode": "exact", "amount_minor": 4_900 }),
        )
        .await;
    let tok = ex["token"].as_str().unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{tid}/{tok}"),
            Value::Null,
        )
        .await;
    assert_eq!(info["amount_rule"], "Pay exactly ₹49.00.");
    let (s, v) = start(&t, tid, tok, json!({ "amount_minor": 1 })).await;
    assert_eq!(
        (s.as_u16(), v["error"]["code"].clone()),
        (422, json!("amount_not_allowed"))
    );
    let old = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "open_amount": true, "min_amount_minor": 100 }),
        )
        .await;
    assert_eq!(old["amount_mode"], "pay_what_you_want");
    let plain = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100 }),
        )
        .await;
    assert_eq!(plain["amount_mode"], "exact");
    // links made before the migration have no stored mode and read as before
    sqlx::query("UPDATE payment_links SET amount_mode = NULL")
        .execute(&t.root)
        .await
        .unwrap();
    let listed = t.ok(a, "GET", "/v1/payment_links", None, Value::Null).await;
    let modes: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["amount_mode"].as_str().unwrap())
        .collect();
    assert!(modes.contains(&"pay_what_you_want") && modes.contains(&"exact"));
    let info = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/links/{tid}/{}",
                old["token"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(info["amount_mode"], "pay_what_you_want");

    // another business sees none of this
    assert!(
        t.ok(b, "GET", "/v1/payment_links", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (s, _) = t
        .public(
            "GET",
            &format!("/v1/checkout/links/{}/{tok}", b.id),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s.as_u16(), 404);
}

// ───────────────────────────── lifecycle ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn the_link_lifecycle_is_one_vocabulary_with_recorded_transitions_and_events(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // the documented model
    let model = t
        .ok(a, "GET", "/v1/payment_links/lifecycle", None, Value::Null)
        .await;
    let names: Vec<&str> = model["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "created",
            "active",
            "partially_paid",
            "paid",
            "expired",
            "cancelled",
            "failed"
        ]
    );
    let paid = model["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["status"] == "paid")
        .unwrap();
    assert_eq!(paid["terminal"], true);

    // a single-use link: created -> active -> paid
    let l = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 50_000, "max_uses": 1 }),
        )
        .await;
    let (id, tok) = (
        l["id"].as_str().unwrap().to_string(),
        l["token"].as_str().unwrap().to_string(),
    );
    assert_eq!(l["lifecycle_status"], "created");
    let lc = lifecycle(&t, a, &id).await;
    assert_eq!(
        (lc["status"].clone(), lc["payable"].clone()),
        (json!("created"), json!(true))
    );
    let s = start(&t, &a.id, &tok, json!({})).await.1;
    assert_eq!(
        lifecycle(&t, a, &id).await["status"],
        "active",
        "computed live, before the worker looks"
    );
    assert_eq!(lifecycle(&t, a, &id).await["recorded_status"], "created");
    assert_eq!(sweep_links(&t, a).await, 1);
    assert_eq!(lifecycle(&t, a, &id).await["recorded_status"], "active");
    assert_eq!(
        sweep_links(&t, a).await,
        0,
        "nothing changed, nothing recorded"
    );
    pay(
        &t,
        a,
        s["intent_id"].as_str().unwrap(),
        "captured",
        json!({}),
    )
    .await;
    assert_eq!(sweep_links(&t, a).await, 1);
    let lc = lifecycle(&t, a, &id).await;
    assert_eq!(
        (
            lc["status"].clone(),
            lc["payable"].clone(),
            lc["paid_minor"].clone()
        ),
        (json!("paid"), json!(false), json!(50_000))
    );
    let hist: Vec<(String, String)> = lc["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["from"].as_str().unwrap_or("-").to_string(),
                h["to"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        hist,
        [
            ("-".into(), "created".into()),
            ("created".into(), "active".into()),
            ("active".into(), "paid".into())
        ]
    );
    for ev in [
        "payment_link.created",
        "payment_link.activated",
        "payment_link.paid",
    ] {
        assert_eq!(
            t.count(&format!(
                "SELECT count(*) FROM outbox_events WHERE type = '{ev}'"
            ))
            .await,
            1,
            "{ev}"
        );
    }
    // paid is final, whatever is done to the link afterwards
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_links/{id}/cancel",),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(lifecycle(&t, a, &id).await["status"], "paid");

    // the old status field is untouched; the new one sits beside it
    let listed = t.ok(a, "GET", "/v1/payment_links", None, Value::Null).await;
    let row = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap();
    assert_eq!(row["lifecycle_status"], "paid");
    assert!(row["status"].is_string());

    // capped at two uses, one paid: partially paid
    let l = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 20_000, "max_uses": 2 }),
        )
        .await;
    let (id2, tok2) = (
        l["id"].as_str().unwrap().to_string(),
        l["token"].as_str().unwrap().to_string(),
    );
    let s1 = start(&t, &a.id, &tok2, json!({})).await.1;
    pay(
        &t,
        a,
        s1["intent_id"].as_str().unwrap(),
        "captured",
        json!({}),
    )
    .await;
    sweep_links(&t, a).await;
    assert_eq!(lifecycle(&t, a, &id2).await["status"], "partially_paid");
    let s2 = start(&t, &a.id, &tok2, json!({})).await.1;
    pay(
        &t,
        a,
        s2["intent_id"].as_str().unwrap(),
        "captured",
        json!({}),
    )
    .await;
    sweep_links(&t, a).await;
    assert_eq!(lifecycle(&t, a, &id2).await["status"], "paid");

    // every allowed checkout failed: failed
    let l = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 20_000, "max_uses": 1 }),
        )
        .await;
    let (id3, tok3) = (
        l["id"].as_str().unwrap().to_string(),
        l["token"].as_str().unwrap().to_string(),
    );
    let s3 = start(&t, &a.id, &tok3, json!({})).await.1;
    assert_eq!(
        lifecycle(&t, a, &id3).await["status"],
        "active",
        "the payer is still filling it in"
    );
    pay(
        &t,
        a,
        s3["intent_id"].as_str().unwrap(),
        "failed",
        json!({}),
    )
    .await;
    assert_eq!(lifecycle(&t, a, &id3).await["status"], "failed");
    sweep_links(&t, a).await;
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment_link.failed'")
            .await,
        1
    );

    // expiry and cancel
    let e = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 100, "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339() })).await;
    let eid = e["id"].as_str().unwrap();
    sqlx::query("UPDATE payment_links SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(eid)
        .execute(&t.root)
        .await
        .unwrap();
    sweep_links(&t, a).await;
    let lc = lifecycle(&t, a, eid).await;
    assert_eq!(
        (lc["status"].clone(), lc["payable"].clone()),
        (json!("expired"), json!(false))
    );
    let c = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100 }),
        )
        .await;
    let cancelled = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_links/{}/cancel", c["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cancelled["lifecycle_status"], "cancelled");
    assert_eq!(
        cancelled["status"], "canceled",
        "the older field keeps its spelling"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment_link.cancelled'")
            .await,
        1
    );

    // a change the table does not allow is refused and audited, not applied
    sqlx::query("UPDATE payment_links SET lifecycle_status = 'paid' WHERE id = $1")
        .bind(c["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &a.id)
        .await
        .unwrap();
    assert!(
        link_lifecycle::sync(&mut tx, &a.id, c["id"].as_str().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    tx.commit().await.unwrap();
    assert_eq!(
        t.count(
            "SELECT count(*) FROM audit_events WHERE action = 'payment_link.lifecycle_refused'"
        )
        .await,
        1
    );

    // another business can neither read nor move this link
    let (s, _) = t
        .call(
            b,
            "GET",
            &format!("/v1/payment_links/{id}/lifecycle"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s.as_u16(), 404);
    assert_eq!(sweep_links(&t, b).await, 0);
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            limited(json!(["payments:write"])),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: ro["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(
            &ro,
            "GET",
            &format!("/v1/payment_links/{id}/lifecycle"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

// ───────────────────────────── payer-instrument rules ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn a_payment_from_a_disallowed_payer_instrument_is_refunded_and_what_cannot_be_checked_says_so(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // rules are validated, and normalised on the way in
    let l = t
        .ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 30_000, "instrument_rules": { "allowed_vpa_handles": ["@OkHdfcBank"] } }))
        .await;
    let (id, tok) = (
        l["id"].as_str().unwrap().to_string(),
        l["token"].as_str().unwrap().to_string(),
    );
    let stored: Value =
        sqlx::query_scalar("SELECT instrument_rules FROM payment_links WHERE id = $1")
            .bind(&id)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(
        stored,
        json!({ "allowed_banks": [], "allowed_vpa_handles": ["okhdfcbank"] })
    );
    for bad in [
        json!({ "allowed_vpa_handles": ["x"] }),
        json!({ "allowed_banks": ["a;b"] }),
    ] {
        let (s, e) = t
            .call(
                a,
                "POST",
                "/v1/payment_links",
                None,
                json!({ "amount_minor": 100, "instrument_rules": bad }),
            )
            .await;
        assert_eq!(
            (s.as_u16(), e["error"]["code"].clone()),
            (422, json!("invalid_instrument_rules"))
        );
    }

    // three payers: an allowed handle, a disallowed one, and one the provider does not describe
    let mut intents = Vec::new();
    for vpa in [Some("ravi@okhdfcbank"), Some("amit@ybl"), None] {
        let s = start(&t, &a.id, &tok, json!({})).await.1;
        let iid = s["intent_id"].as_str().unwrap().to_string();
        pay(
            &t,
            a,
            &iid,
            "captured",
            vpa.map(|v| json!({ "vpa": v })).unwrap_or(json!({})),
        )
        .await;
        intents.push(iid);
    }
    let swept = instrument::sweep_tenant(&t.state, &a.id, "test")
        .await
        .unwrap();
    assert_eq!(
        (swept.checked, swept.refused, swept.unverifiable),
        (3, 1, 1)
    );
    let checks = t
        .ok(a, "GET", "/v1/instrument_checks", None, Value::Null)
        .await;
    let by_intent = |i: &str| {
        checks["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["intent_id"] == i)
            .unwrap()
            .clone()
    };
    assert_eq!(by_intent(&intents[0])["result"], "allowed");
    let bad = by_intent(&intents[1]);
    assert_eq!(
        (bad["result"].clone(), bad["payer_vpa_handle"].clone()),
        (json!("disallowed"), json!("ybl"))
    );
    assert!(bad["refund_id"].is_string(), "{bad}");
    assert!(
        !checks.to_string().contains("amit"),
        "only the handle is kept, never the name part"
    );
    let un = by_intent(&intents[2]);
    assert_eq!(un["result"], "unverifiable");
    assert!(un["detail"].as_str().unwrap().contains("did not report"));
    assert!(un["refund_id"].is_null());
    // the disallowed payment was refunded through the normal refund flow; the others were not
    let refunds = t.ok(a, "GET", "/v1/refunds", None, Value::Null).await;
    let rf = refunds["data"].as_array().unwrap();
    assert_eq!(rf.len(), 1, "{refunds}");
    assert_eq!(
        (
            rf[0]["intent_id"].clone(),
            rf[0]["status"].clone(),
            rf[0]["requested_by"].clone()
        ),
        (
            json!(intents[1]),
            json!("processed"),
            json!("system:instrument_rules")
        )
    );
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'disallowed_instrument'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment.instrument_refused'")
            .await,
        1
    );
    // a second pass changes nothing and refunds nothing again
    let again = instrument::sweep_tenant(&t.state, &a.id, "test")
        .await
        .unwrap();
    assert_eq!(again.checked, 0);
    assert_eq!(
        t.ok(a, "GET", "/v1/refunds", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // a rule on the intent itself wins over the link's, can be set before it succeeds, not after
    let s = start(&t, &a.id, &tok, json!({})).await.1;
    let iid = s["intent_id"].as_str().unwrap();
    let r = t
        .ok(
            a,
            "PUT",
            &format!("/v1/payment_intents/{iid}/instrument_rules"),
            None,
            json!({ "allowed_banks": ["SBI"] }),
        )
        .await;
    assert_eq!(r["instrument_rules"]["allowed_banks"], json!(["sbi"]));
    pay(&t, a, iid, "captured", json!({ "bank": "HDFC" })).await;
    let swept = instrument::sweep_tenant(&t.state, &a.id, "test")
        .await
        .unwrap();
    assert_eq!(
        swept.refused, 1,
        "HDFC is not on this payment's own bank list"
    );
    let (s, e) = t
        .call(
            a,
            "PUT",
            &format!("/v1/payment_intents/{iid}/instrument_rules"),
            None,
            json!({ "allowed_banks": ["HDFC"] }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("intent_not_open"))
    );

    // another business cannot set rules on this link or read these checks
    let (s, _) = t
        .call(
            b,
            "PUT",
            &format!("/v1/payment_links/{id}/instrument_rules"),
            None,
            json!({ "allowed_banks": ["X"] }),
        )
        .await;
    assert_eq!(s.as_u16(), 404);
    assert!(
        t.ok(b, "GET", "/v1/instrument_checks", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // an empty list removes the rule
    t.ok(
        a,
        "PUT",
        &format!("/v1/payment_links/{id}/instrument_rules"),
        None,
        json!({}),
    )
    .await;
    let stored: Option<Value> =
        sqlx::query_scalar("SELECT instrument_rules FROM payment_links WHERE id = $1")
            .bind(&id)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert!(stored.is_none());
}

// ───────────────────────────── verification ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn vpa_and_bank_checks_are_labelled_simulated_masked_conservative_and_rate_limited(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let b = &t.b;
    // each business has its own rate-limit bucket, so the phases use separate ones
    let a = &fresh_tenant(&t, "Verify VPA").await;
    let bank_t = &fresh_tenant(&t, "Verify Bank").await;
    let capped = &fresh_tenant(&t, "Verify Cap").await;

    // UPI ids
    let v = t
        .ok(
            a,
            "POST",
            "/v1/verify/vpa",
            None,
            json!({ "vpa": "ravi.kumar@okhdfcbank", "name": "Ravi Kumar" }),
        )
        .await;
    assert_eq!(
        (
            v["outcome"].clone(),
            v["name_match"].clone(),
            v["safe_to_pay"].clone()
        ),
        (json!("valid"), json!("match"), json!(true))
    );
    assert_eq!(
        (v["simulated"].clone(), v["provider"].clone()),
        (json!(true), json!("simulator"))
    );
    assert!(v["note"].as_str().unwrap().starts_with("SIMULATED"));
    assert_eq!(v["subject"], "ra***@okhdfcbank");
    for (vpa, outcome, nm, safe) in [
        ("nobody.invalid@ybl", "invalid", "not_checked", false),
        ("slow.timeout@ybl", "inconclusive", "not_checked", false),
        ("other.person@ybl", "valid", "mismatch", false),
        ("close.review@ybl", "valid", "review", false),
    ] {
        let v = t
            .ok(
                a,
                "POST",
                "/v1/verify/vpa",
                None,
                json!({ "vpa": vpa, "name": "Ravi Kumar" }),
            )
            .await;
        assert_eq!(
            (
                v["outcome"].as_str().unwrap(),
                v["name_match"].as_str().unwrap(),
                v["safe_to_pay"].as_bool().unwrap()
            ),
            (outcome, nm, safe),
            "{vpa}: {v}"
        );
    }
    // without a name only existence is checked, and that is never "safe to pay"
    let v = t
        .ok(
            a,
            "POST",
            "/v1/verify/vpa",
            None,
            json!({ "vpa": "someone@ybl" }),
        )
        .await;
    assert_eq!(
        (v["name_match"].clone(), v["safe_to_pay"].clone()),
        (json!("not_checked"), json!(false))
    );
    for bad in ["nohandle", "a@b", "two@@ats", "has space@ybl"] {
        assert_eq!(
            t.call(a, "POST", "/v1/verify/vpa", None, json!({ "vpa": bad }))
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }

    // bank accounts (penny drop through the provider abstraction)
    let a = bank_t;
    let acct = "026291800001191";
    let v = t
        .ok(
            a,
            "POST",
            "/v1/verify/bank_account",
            None,
            json!({ "account_number": acct, "ifsc": "YESB0000262", "name": "Priya Sharma" }),
        )
        .await;
    assert_eq!(
        (
            v["outcome"].clone(),
            v["name_match"].clone(),
            v["safe_to_pay"].clone()
        ),
        (json!("valid"), json!("match"), json!(true))
    );
    assert_eq!(v["subject"], "******1191");
    assert_eq!(v["simulated"], true);
    assert!(
        !v.to_string().contains(acct) && !v.to_string().contains("SHARMA"),
        "no account number, no registered name in the response"
    );
    for (acct, outcome, nm) in [
        ("026291800000000", "invalid", "not_checked"),
        ("026291800001111", "valid", "mismatch"),
        ("026291800002222", "valid", "review"),
        ("026291800003333", "inconclusive", "not_checked"),
    ] {
        let v = t
            .ok(
                a,
                "POST",
                "/v1/verify/bank_account",
                None,
                json!({ "account_number": acct, "ifsc": "YESB0000262", "name": "Priya Sharma" }),
            )
            .await;
        assert_eq!(
            (
                v["outcome"].as_str().unwrap(),
                v["name_match"].as_str().unwrap()
            ),
            (outcome, nm),
            "{acct}"
        );
        assert_eq!(v["safe_to_pay"], false);
    }
    for (acct, ifsc) in [
        ("12ab56", "YESB0000262"),
        ("123", "YESB0000262"),
        ("026291800001191", "yesb0000262x"),
        ("026291800001191", "YESB1000262"),
    ] {
        let (s, _) = t
            .call(
                a,
                "POST",
                "/v1/verify/bank_account",
                None,
                json!({ "account_number": acct, "ifsc": ifsc }),
            )
            .await;
        assert_eq!(s.as_u16(), 422, "{acct} {ifsc}");
    }

    // stored masked: no full account number, no UPI id name part, no registered name anywhere
    let rows: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT subject_masked, ifsc, detail FROM verifications")
            .fetch_all(&t.root)
            .await
            .unwrap();
    assert!(rows.len() >= 10);
    for (m, _, d) in &rows {
        assert!(
            !m.contains("0262918")
                && !m.contains("ravi.kumar")
                && !d.contains("SHARMA")
                && !d.contains("ELSE"),
            "{m} / {d}"
        );
    }
    let audit: String = sqlx::query_scalar(
        "SELECT string_agg(detail::text, ' ') FROM audit_events WHERE action = 'verification.run'",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert!(!audit.contains("0262918") && !audit.contains("ravi.kumar"));
    let listed = t.ok(a, "GET", "/v1/verifications", None, Value::Null).await;
    assert!(!listed.to_string().contains("0262918"));

    // a repeat inside ten minutes reuses the answer and does not call the provider again
    let before = t.count("SELECT count(*) FROM verifications").await;
    let again = t
        .ok(
            a,
            "POST",
            "/v1/verify/bank_account",
            None,
            json!({ "account_number": acct, "ifsc": "YESB0000262", "name": "Priya Sharma" }),
        )
        .await;
    assert_eq!(again["reused"], true);
    assert_eq!(t.count("SELECT count(*) FROM verifications").await, before);

    // another business cannot read a result
    let id = v["id"].as_str().unwrap();
    assert_eq!(
        t.call(
            a,
            "GET",
            &format!("/v1/verifications/{id}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/verifications/{id}"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert!(
        t.ok(b, "GET", "/v1/verifications", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // scope: reading is not enough to call a provider
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            limited(json!(["payments:read"])),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: ro["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(
            &ro,
            "POST",
            "/v1/verify/vpa",
            None,
            json!({ "vpa": "x.y@ybl" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // rate limit: a burst is slowed, and the hourly cap holds
    let mut codes = Vec::new();
    for i in 0..14 {
        let (s, _) = t
            .call(
                b,
                "POST",
                "/v1/verify/vpa",
                None,
                json!({ "vpa": format!("burst{i}@ybl") }),
            )
            .await;
        codes.push(s.as_u16());
    }
    assert!(codes.contains(&429) && codes[0] == 200, "{codes:?}");
    sqlx::query("INSERT INTO verifications (id, tenant_id, kind, subject_masked, request_hash, outcome, name_match, provider, simulated, detail, created_by) SELECT 'ver_cap' || g, $1, 'vpa', 'xx***@ybl', 'h' || g, 'valid', 'not_checked', 'simulator', true, '', 'test' FROM generate_series(1, 100) g")
        .bind(&capped.id).execute(&t.root).await.unwrap();
    let (s, e) = t
        .call(
            capped,
            "POST",
            "/v1/verify/vpa",
            None,
            json!({ "vpa": "one.more@ybl" }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (429, json!("verification_cap")),
        "the hourly cap should refuse"
    );
}

// ───────────────────────────── risk engine ─────────────────────────────

async fn customer(t: &T, tn: &Tenant, name: &str, phone: &str, email: &str) -> String {
    t.ok(
        tn,
        "POST",
        "/v1/customers",
        None,
        json!({ "name": name, "phone": phone, "email": email }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn attempt_for(
    t: &T,
    tn: &Tenant,
    cust: &str,
    amount: i64,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let intent = t.intent(tn, amount, json!({ "customer_id": cust })).await;
    let iid = intent["id"].as_str().unwrap();
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("/v1/payment_intents/{iid}/attempts"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", tn.key))
        .header("idempotency-key", format!("a-{iid}"));
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = t
        .app
        .clone()
        .oneshot(
            req.body(Body::from(json!({ "method": "upi" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[sqlx::test(migrations = "./migrations")]
async fn risk_rules_count_hashed_signals_explain_themselves_and_keep_no_personal_data(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // rules are validated
    for bad in [
        json!({ "name": "x", "kind": "velocity", "action": "allow", "params": { "dimension": "phone", "metric": "attempts", "window_secs": 3600, "max": 1 } }),
        json!({ "name": "x", "kind": "velocity", "action": "decline", "params": { "dimension": "phone", "metric": "attempts", "window_secs": 5, "max": 1 } }),
        json!({ "name": "x", "kind": "velocity", "action": "decline", "params": { "dimension": "shoe", "metric": "attempts", "window_secs": 3600, "max": 1 } }),
        json!({ "name": "x", "kind": "velocity", "action": "decline", "params": { "dimension": "phone", "metric": "attempts", "window_secs": 3600, "max": 1, "other": 1 } }),
        json!({ "name": "x", "kind": "amount", "action": "review", "params": {} }),
        json!({ "name": "x", "kind": "magic", "action": "review", "params": {} }),
        json!({ "name": "", "kind": "amount", "action": "review", "params": { "above_minor": 5 } }),
    ] {
        let (s, e) = t.call(a, "POST", "/v1/risk/rules", None, bad.clone()).await;
        assert_eq!(
            (s.as_u16(), e["error"]["code"].clone()),
            (422, json!("invalid_risk_rule")),
            "{bad}"
        );
    }

    let phone = "+91 98765 43210";
    let email = "ravi.secret@example.com";
    let c1 = customer(&t, a, "Ravi", phone, email).await;
    let c2 = customer(&t, a, "Sita", phone, "sita@example.com").await; // same phone, different person
    let c3 = customer(&t, a, "Amit", "9000000003", "amit@example.com").await;

    // two attempts an hour per phone; a third is declined
    t.ok(a, "POST", "/v1/risk/rules", None, json!({ "name": "phone velocity", "kind": "velocity", "action": "decline", "params": { "dimension": "phone", "metric": "attempts", "window_secs": 3600, "max": 2 } })).await;
    assert_eq!(
        attempt_for(&t, a, &c1, 1_000, &[]).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        attempt_for(&t, a, &c2, 1_000, &[]).await.0,
        StatusCode::CREATED,
        "the phone is what counts, not the customer"
    );
    let (s, e) = attempt_for(&t, a, &c1, 1_000, &[]).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(e["error"]["code"], "risk_blocked");
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("phone velocity")
    );
    // a different phone is unaffected
    assert_eq!(
        attempt_for(&t, a, &c3, 1_000, &[]).await.0,
        StatusCode::CREATED
    );
    // the refusal is explained: rule, window, what it saw, the limit
    let dec = t
        .ok(a, "GET", "/v1/risk/decisions", None, Value::Null)
        .await;
    let d = &dec["data"].as_array().unwrap()[0];
    assert_eq!(d["action"], "decline");
    let f = &d["fired"][0];
    assert_eq!(
        (
            f["rule"].clone(),
            f["observed"].clone(),
            f["limit"].clone(),
            f["window_secs"].clone()
        ),
        (json!("phone velocity"), json!(3), json!(2), json!(3600))
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment.risk_blocked'")
            .await,
        1
    );

    // an allow-list entry stops the velocity rule for that phone; a block-list entry declines
    let entry = t
        .ok(
            a,
            "POST",
            "/v1/risk/lists",
            None,
            json!({ "list": "allow", "dimension": "phone", "value": phone, "note": "staff" }),
        )
        .await;
    assert_eq!(entry["label"], "******3210 (staff)");
    assert_eq!(
        attempt_for(&t, a, &c1, 1_000, &[]).await.0,
        StatusCode::CREATED
    );
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/risk/lists",
            None,
            json!({ "list": "allow", "dimension": "phone", "value": "09876543210" }),
        )
        .await;
    assert_eq!(
        s.as_u16(),
        409,
        "the same number written another way is the same entry"
    );
    t.ok(
        a,
        "POST",
        "/v1/risk/lists",
        None,
        json!({ "list": "block", "dimension": "email", "value": "SITA@example.com" }),
    )
    .await;
    let (s, e) = attempt_for(&t, a, &c2, 1_000, &[]).await;
    assert_eq!(
        (
            s,
            e["error"]["message"]
                .as_str()
                .unwrap()
                .contains("blocklist:email")
        ),
        (StatusCode::FORBIDDEN, true),
        "block beats allow"
    );
    t.ok(
        a,
        "DELETE",
        &format!("/v1/risk/lists/{}", entry["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;

    // a device token the checkout sends: three customers on one device in a day
    t.ok(a, "POST", "/v1/risk/rules", None, json!({ "name": "shared device", "kind": "velocity", "action": "decline", "params": { "dimension": "device", "metric": "distinct_customers", "window_secs": 86400, "max": 2 } })).await;
    let c4 = customer(&t, a, "D1", "9000000004", "d1@example.com").await;
    let c5 = customer(&t, a, "D2", "9000000005", "d2@example.com").await;
    let c6 = customer(&t, a, "D3", "9000000006", "d3@example.com").await;
    let dev = [("x-zyvor-device", "device-token-abc")];
    assert_eq!(
        attempt_for(&t, a, &c4, 500, &dev).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        attempt_for(&t, a, &c5, 500, &dev).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        attempt_for(&t, a, &c4, 500, &dev).await.0,
        StatusCode::CREATED,
        "the same customer again is not a new customer"
    );
    assert_eq!(
        attempt_for(&t, a, &c6, 500, &dev).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        attempt_for(&t, a, &c6, 500, &[]).await.0,
        StatusCode::CREATED,
        "no device token: that rule is skipped, not failed"
    );

    // amount rules: review lets the payment go ahead and opens a case, once per payment
    t.ok(a, "POST", "/v1/risk/rules", None, json!({ "name": "large upi", "kind": "amount", "action": "review", "params": { "above_minor": 5_000_000, "method": "upi" } })).await;
    let c7 = customer(&t, a, "Big", "9000000007", "big@example.com").await;
    let (s, att) = attempt_for(&t, a, &c7, 7_000_000, &[]).await;
    assert_eq!(s, StatusCode::CREATED, "{att}");
    let q = t
        .ok(a, "GET", "/v1/risk/review_queue", None, Value::Null)
        .await;
    let items = q["data"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0]["kind"].clone(), items[0]["subject_type"].clone()),
        (json!("risk_review"), json!("payment_intent"))
    );
    assert!(
        items[0]["explanation"]
            .as_str()
            .unwrap()
            .contains("large upi")
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'risk.review_required'")
            .await,
        1
    );
    // resolving it through the existing case endpoint empties the queue
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/recon/cases/{}/resolve",
            items[0]["case_id"].as_str().unwrap()
        ),
        None,
        json!({ "resolution": "looked fine" }),
    )
    .await;
    assert!(
        t.ok(a, "GET", "/v1/risk/review_queue", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // the dry run says what would happen and records nothing
    let signals_before = t.count("SELECT count(*) FROM risk_signals").await;
    let dry = t
        .ok(
            a,
            "POST",
            "/v1/risk/evaluate",
            None,
            json!({ "amount_minor": 9_000_000, "method": "upi", "phone": "9000000099" }),
        )
        .await;
    assert_eq!(
        (dry["dry_run"].clone(), dry["decision"]["action"].clone()),
        (json!(true), json!("review"))
    );
    assert!(
        dry["decision"]["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("no device"))
    );
    assert_eq!(
        t.count("SELECT count(*) FROM risk_signals").await,
        signals_before
    );

    // no personal data in what is stored or logged: hashes only
    let blob: String = sqlx::query_scalar(
        "SELECT concat_ws(' ', (SELECT string_agg(concat_ws(' ', customer_h, device_h, phone_h, email_h, ip_h, card_h, vpa_h), ' ') FROM risk_signals),
                (SELECT string_agg(concat_ws(' ', label, value_hash), ' ') FROM risk_list_entries),
                (SELECT string_agg(fired::text, ' ') FROM risk_decisions),
                (SELECT string_agg(detail::text, ' ') FROM audit_events WHERE action LIKE 'risk%'),
                (SELECT string_agg(payload::text, ' ') FROM outbox_events WHERE type LIKE 'risk%' OR type = 'payment.risk_blocked'),
                (SELECT string_agg(detail::text, ' ') FROM recon_cases))",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    for secret in [
        "9876543210",
        "98765",
        "ravi.secret",
        "sita@example",
        "device-token-abc",
    ] {
        assert!(
            !blob.contains(secret),
            "{secret} must not be stored or logged in clear"
        );
    }
    let hashed: String =
        sqlx::query_scalar("SELECT phone_h FROM risk_signals WHERE phone_h IS NOT NULL LIMIT 1")
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(hashed.len(), 32);

    // isolation: the other business has no rules, lists, decisions or queue of ours, and its traffic is not counted against ours
    for path in [
        "/v1/risk/rules",
        "/v1/risk/lists",
        "/v1/risk/decisions",
        "/v1/risk/review_queue",
    ] {
        assert!(
            t.ok(b, "GET", path, None, Value::Null).await["data"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{path}"
        );
    }
    let bc = customer(&t, b, "Ravi", phone, email).await;
    for _ in 0..4 {
        assert_eq!(
            attempt_for(&t, b, &bc, 1_000, &[]).await.0,
            StatusCode::CREATED
        );
    }
    let rule_id = t.ok(a, "GET", "/v1/risk/rules", None, Value::Null).await["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        t.call(
            b,
            "DELETE",
            &format!("/v1/risk/rules/{rule_id}"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    // the same phone hashes differently for each business
    let (ha, hb): (String, String) = (
        sqlx::query_scalar(
            "SELECT phone_h FROM risk_signals WHERE tenant_id = $1 AND phone_h IS NOT NULL LIMIT 1",
        )
        .bind(&a.id)
        .fetch_one(&t.root)
        .await
        .unwrap(),
        sqlx::query_scalar(
            "SELECT phone_h FROM risk_signals WHERE tenant_id = $1 AND phone_h IS NOT NULL LIMIT 1",
        )
        .bind(&b.id)
        .fetch_one(&t.root)
        .await
        .unwrap(),
    );
    assert_ne!(ha, hb);

    // scope and retention
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            limited(json!(["payments:read"])),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: ro["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(t.call(&ro, "POST", "/v1/risk/rules", None, json!({ "name": "n", "kind": "amount", "action": "review", "params": { "above_minor": 5 } })).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        t.call(&ro, "GET", "/v1/risk/rules", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE risk_signals SET at = now() - interval '8 days' WHERE tenant_id = $1")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    assert!(risk_engine::sweep_tenant(&t.state, &a.id).await.unwrap() > 0);
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM risk_signals WHERE tenant_id = '{}'",
            a.id
        ))
        .await,
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_order_risk_score_is_explained_by_its_parts_and_opens_one_case_per_order(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let score = |body: Value| {
        let t = &t;
        async move { t.ok(a, "POST", "/v1/risk/order_score", None, body).await }
    };
    // small order, new buyer, no area data: 0 + 0 + 0 + 5
    let r = score(json!({ "pin_code": "560001", "order_value_minor": 30_000 })).await;
    assert_eq!(
        (
            r["score"].clone(),
            r["action"].clone(),
            r["band"].clone(),
            r["formula"].clone()
        ),
        (
            json!(5),
            json!("allow"),
            json!("low"),
            json!("order-risk-v1")
        )
    );
    let parts: i64 = r["contributions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["points"].as_i64().unwrap())
        .sum();
    assert_eq!(parts, 5);
    assert!(
        r["contributions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| !c["why"].as_str().unwrap().is_empty())
    );
    // the merchant's own pin tiers: the longest matching prefix wins
    t.ok(
        a,
        "PUT",
        "/v1/risk/pin_tiers",
        None,
        json!({ "pin_prefix": "560", "tier": "high" }),
    )
    .await;
    t.ok(
        a,
        "PUT",
        "/v1/risk/pin_tiers",
        None,
        json!({ "pin_prefix": "560001", "tier": "low" }),
    )
    .await;
    let r = score(json!({ "pin_code": "560001", "order_value_minor": 30_000 })).await;
    assert_eq!(
        (r["score"].clone(), r["pin_tier"].clone()),
        (json!(0), json!("low"))
    );
    let r = score(json!({ "pin_code": "560002", "order_value_minor": 30_000 })).await;
    assert_eq!(
        (r["score"].clone(), r["pin_tier"].clone()),
        (json!(20), json!("high"))
    );
    // big order, a refusal before, high-risk area: 30 + 25 + 0 + 20 = 75, declined by default
    let r =
        score(json!({ "pin_code": "560002", "order_value_minor": 600_000, "prior_refusals": 1 }))
            .await;
    assert_eq!(
        (r["score"].clone(), r["action"].clone()),
        (json!(75), json!("decline"))
    );
    // repeat buyer with a clean record is credited
    let r = score(
        json!({ "pin_code": "560001", "order_value_minor": 300_000, "repeat_customer": true }),
    )
    .await;
    assert_eq!(r["score"], 5);
    // thresholds are the merchant's
    let r = score(json!({ "pin_code": "560002", "order_value_minor": 600_000, "prior_refusals": 1, "review_at": 90, "decline_at": 95 })).await;
    assert_eq!(r["action"], "allow");
    // review opens one case per order reference and an event
    let body = json!({ "pin_code": "560002", "order_value_minor": 100_000, "prior_refusals": 1, "order_ref": "ORD-77" });
    let r = score(body.clone()).await;
    assert_eq!(
        (r["score"].clone(), r["action"].clone()),
        (json!(55), json!("review"))
    );
    assert!(r["case_id"].is_string());
    let again = score(body).await;
    assert_eq!(again["case_id"], r["case_id"]);
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'order_risk_review'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'risk.review_required'")
            .await,
        1
    );
    let q = t
        .ok(a, "GET", "/v1/risk/review_queue", None, Value::Null)
        .await;
    assert_eq!(q["data"][0]["kind"], "order_risk_review");
    // nothing about the buyer or the area is kept
    let kept: String = sqlx::query_scalar("SELECT concat_ws(' ', (SELECT string_agg(fired::text, ' ') FROM risk_decisions), (SELECT string_agg(detail::text, ' ') FROM recon_cases))").fetch_one(&t.root).await.unwrap();
    assert!(!kept.contains("560002") && !kept.contains("560001"));
    // validation
    for bad in [
        json!({ "pin_code": "060001", "order_value_minor": 100 }),
        json!({ "pin_code": "56000", "order_value_minor": 100 }),
        json!({ "pin_code": "560001", "order_value_minor": 0 }),
        json!({ "pin_code": "560001", "order_value_minor": 100, "review_at": 80, "decline_at": 70 }),
        json!({ "pin_code": "560001", "order_value_minor": 100, "order_ref": "" }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/risk/order_score", None, bad.clone())
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    // isolation
    assert!(
        t.ok(b, "GET", "/v1/risk/pin_tiers", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let r = t
        .ok(
            b,
            "POST",
            "/v1/risk/order_score",
            None,
            json!({ "pin_code": "560002", "order_value_minor": 30_000 }),
        )
        .await;
    assert_eq!(r["pin_tier"], Value::Null);
    assert_eq!(
        t.call(b, "DELETE", "/v1/risk/pin_tiers/560", None, Value::Null)
            .await
            .0
            .as_u16(),
        404
    );
}

// ───────────────────────────── payout links ─────────────────────────────

fn claim_path(tid: &str, tok: &str) -> String {
    format!("/v1/checkout/payout_links/{tid}/{tok}/claim")
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payout_link_verifies_the_recipients_details_then_follows_the_payout_flow_once(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // creating
    for bad in [
        json!({ "amount_minor": 0, "purpose": "x" }),
        json!({ "amount_minor": 100, "purpose": "" }),
        json!({ "amount_minor": 100, "purpose": "x", "expires_in_hours": 0 }),
        json!({ "amount_minor": 100, "purpose": "x", "expires_in_hours": 721 }),
        json!({ "amount_minor": 100, "purpose": "x", "expected_name": "Mr." }),
    ] {
        assert_eq!(
            t.call_as(a, "priya", "POST", "/v1/payout_links", None, bad.clone())
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    let l = t.call_as(a, "priya", "POST", "/v1/payout_links", None, json!({ "amount_minor": 250_000, "purpose": "Refund for order 55", "expected_name": "Priya Sharma", "external_ref": "REF-1" })).await;
    assert_eq!(l.0, StatusCode::CREATED);
    let (id, tok) = (
        l.1["id"].as_str().unwrap().to_string(),
        l.1["token"].as_str().unwrap().to_string(),
    );
    assert!(tok.starts_with("pol_"));
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM payout_links WHERE id = $1")
        .bind(&id)
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_ne!(stored, tok, "only a hash of the token is stored");
    let (s, same) = t.call_as(a, "priya", "POST", "/v1/payout_links", None, json!({ "amount_minor": 250_000, "purpose": "Refund for order 55", "external_ref": "REF-1" })).await;
    assert_eq!(
        (s.as_u16(), same["id"].clone(), same["token"].clone()),
        (200, json!(id), Value::Null)
    );

    // what the recipient sees
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/{tok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            info["amount_minor"].clone(),
            info["status"].clone(),
            info["name_required"].clone()
        ),
        (json!(250_000), json!("active"), json!(false))
    );
    assert!(
        !info.to_string().contains("Sharma") && !info.to_string().contains("priya"),
        "{info}"
    );
    // not another business's, not a wrong token
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/{tok}", b.id),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/pol_nope", a.id),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert!(
        t.ok(b, "GET", "/v1/payout_links", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/payout_links/{id}"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert_eq!(
        t.call_as(
            b,
            "x",
            "POST",
            &format!("/v1/payout_links/{id}/cancel"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        409
    );

    // shape errors and abuse before anything is verified
    let bank = json!({ "account_number": "026291800001191", "ifsc": "YESB0000262" });
    for bad in [
        json!({}),
        json!({ "bank_account": bank, "upi": { "vpa": "p@ybl" } }),
        json!({ "bank_account": { "account_number": "12", "ifsc": "X" } }),
        json!({ "upi": { "vpa": "nope" } }),
    ] {
        assert_eq!(
            t.public("POST", &claim_path(&a.id, &tok), None, bad.clone())
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    assert_eq!(
        t.count("SELECT count(*) FROM verifications").await,
        0,
        "bad shapes never reach the provider"
    );
    assert_eq!(t.count("SELECT count(*) FROM payouts").await, 0);

    // an account held by someone else, and one that does not exist: refused alike, counted against the link
    for acct in ["026291800001111", "026291800000000"] {
        let (s, e) = t.public("POST", &claim_path(&a.id, &tok), None, json!({ "bank_account": { "account_number": acct, "ifsc": "YESB0000262" }, "amount_minor": 1 })).await;
        assert_eq!(
            (s.as_u16(), e["error"]["code"].clone()),
            (422, json!("details_not_confirmed")),
            "{acct}"
        );
        assert!(
            !e.to_string().contains("ELSE"),
            "never the name the bank holds"
        );
    }
    // a partial match (the bank holds a longer name) is not enough either
    let (s, _) = t.public("POST", &claim_path(&a.id, &tok), None, json!({ "bank_account": { "account_number": "026291800002222", "ifsc": "YESB0000262" } })).await;
    assert_eq!(s.as_u16(), 422);
    // no answer from the bank is not the recipient's failure
    let (s, e) = t.public("POST", &claim_path(&a.id, &tok), None, json!({ "bank_account": { "account_number": "026291800003333", "ifsc": "YESB0000262" } })).await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("try_later"))
    );
    let fails: i32 = sqlx::query_scalar("SELECT failed_attempts FROM payout_links WHERE id = $1")
        .bind(&id)
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(fails, 3);
    assert_eq!(
        t.count("SELECT count(*) FROM payouts").await,
        0,
        "nothing is paid on a refused claim"
    );

    // the right account: verified, vendor made, payout requested at the fixed amount by the link's creator
    let (s, ok) = t
        .public(
            "POST",
            &claim_path(&a.id, &tok),
            None,
            json!({ "bank_account": bank, "amount_minor": 999_999_999 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");
    assert_eq!(
        (
            ok["status"].clone(),
            ok["payout_status"].clone(),
            ok["amount_minor"].clone(),
            ok["simulated"].clone()
        ),
        (
            json!("claimed"),
            json!("pending_approval"),
            json!(250_000),
            json!(true)
        )
    );
    assert!(ok["message"].as_str().unwrap().starts_with("SIMULATED"));
    let (pid, amount, by, ext): (String, i64, String, Option<String>) =
        sqlx::query_as("SELECT id, amount_minor, requested_by, external_ref FROM payouts")
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(
        (amount, by.as_str(), ext.as_deref()),
        (250_000, "user:priya", Some(format!("plink:{id}").as_str()))
    );
    // the vendor holds encrypted details and shows only masks
    let vendors = t.ok(a, "GET", "/v1/vendors", None, Value::Null).await;
    assert_eq!(
        (
            vendors["data"][0]["name"].clone(),
            vendors["data"][0]["account_last4"].clone()
        ),
        (json!("Priya Sharma"), json!("1191"))
    );
    assert!(!vendors.to_string().contains("026291800001191"));
    let enc: Vec<u8> = sqlx::query_scalar("SELECT details_enc FROM vendors")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&enc).contains("026291800001191"));

    // single use: the same link cannot be claimed again, nor by another account
    let (s, e) = t
        .public(
            "POST",
            &claim_path(&a.id, &tok),
            None,
            json!({ "bank_account": bank }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("link_unavailable"))
    );
    let (s, _) = t
        .public(
            "POST",
            &claim_path(&a.id, &tok),
            None,
            json!({ "upi": { "vpa": "priya.sharma@ybl" } }),
        )
        .await;
    assert_eq!(s.as_u16(), 409);
    assert_eq!(t.count("SELECT count(*) FROM payouts").await, 1);
    let st = t
        .ok(
            a,
            "GET",
            &format!("/v1/payout_links/{id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (st["status"].clone(), st["payout_id"].clone()),
        (json!("claimed"), json!(pid))
    );
    let (s, _) = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payout_links/{id}/cancel"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        s.as_u16(),
        409,
        "a claimed link is cancelled by cancelling its payout"
    );

    // the normal maker-checker flow: the creator cannot approve it; another person can; then it is paid
    let (s, e) = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{pid}/approve"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (403, json!("separation_of_duties"))
    );
    let ap = t
        .ok_as(
            a,
            "meera",
            "POST",
            &format!("/v1/payouts/{pid}/approve"),
            Value::Null,
        )
        .await;
    assert_eq!(ap["status"], "approved");
    t.ok_as(
        a,
        "meera",
        "POST",
        &format!("/v1/payouts/{pid}/mark_paid"),
        json!({ "bank_ref": "UTR0000123456" }),
    )
    .await;
    let st = t
        .ok(
            a,
            "GET",
            &format!("/v1/payout_links/{id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (st["status"].clone(), st["payout_status"].clone()),
        (json!("paid"), json!("paid"))
    );
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/{tok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(info["status"], "paid");
}

impl T {
    async fn ok_as(
        &self,
        tn: &Tenant,
        actor: &str,
        method: &str,
        path: &str,
        body: Value,
    ) -> Value {
        let (s, b) = self.call_as(tn, actor, method, path, None, body).await;
        assert!(s.is_success(), "{method} {path} -> {s}: {b}");
        b
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn payout_links_expire_cancel_lock_and_pay_by_upi(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let make = |name: Option<&str>, ext: &str| {
        let mut body =
            json!({ "amount_minor": 10_000, "purpose": "Cashback", "external_ref": ext });
        if let Some(n) = name {
            body["expected_name"] = json!(n);
        }
        body
    };

    // UPI, with the name typed by the recipient when the merchant gave none
    let l = t
        .ok(a, "POST", "/v1/payout_links", None, make(None, "U-1"))
        .await;
    let tok = l["token"].as_str().unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/{tok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(info["name_required"], true);
    let (s, e) = t
        .public(
            "POST",
            &claim_path(&a.id, tok),
            None,
            json!({ "upi": { "vpa": "asha.rao@okhdfcbank" } }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (422, json!("name_required"))
    );
    let (s, ok) = t
        .public(
            "POST",
            &claim_path(&a.id, tok),
            None,
            json!({ "name": "Asha Rao", "upi": { "vpa": "asha.rao@okhdfcbank" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");
    let vendors = t.ok(a, "GET", "/v1/vendors", None, Value::Null).await;
    assert_eq!(vendors["data"][0]["vpa"], "***@okhdfcbank");
    assert!(!vendors.to_string().contains("asha.rao"));

    // cancelled and expired links cannot be claimed
    let c = t
        .ok(
            a,
            "POST",
            "/v1/payout_links",
            None,
            make(Some("Asha Rao"), "U-2"),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/payout_links/{}/cancel", c["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    let (s, e) = t
        .public(
            "POST",
            &claim_path(&a.id, c["token"].as_str().unwrap()),
            None,
            json!({ "upi": { "vpa": "asha.rao@ybl" } }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("link_unavailable"))
    );
    let x = t
        .ok(
            a,
            "POST",
            "/v1/payout_links",
            None,
            make(Some("Asha Rao"), "U-3"),
        )
        .await;
    sqlx::query("UPDATE payout_links SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(x["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/payout_links/{}/{}",
                a.id,
                x["token"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(info["status"], "expired");
    let (s, _) = t
        .public(
            "POST",
            &claim_path(&a.id, x["token"].as_str().unwrap()),
            None,
            json!({ "upi": { "vpa": "asha.rao@ybl" } }),
        )
        .await;
    assert_eq!(s.as_u16(), 409);

    // five wrong answers lock a link, and a locked link refuses even the right details
    let k = t
        .ok(
            a,
            "POST",
            "/v1/payout_links",
            None,
            make(Some("Asha Rao"), "U-4"),
        )
        .await;
    let ktok = k["token"].as_str().unwrap();
    for _ in 0..5 {
        let (s, _) = t
            .public(
                "POST",
                &claim_path(&a.id, ktok),
                None,
                json!({ "upi": { "vpa": "other.person@ybl" } }),
            )
            .await;
        assert_eq!(s.as_u16(), 422);
    }
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/payout_links/{}/{ktok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(info["status"], "locked");
    let (s, _) = t
        .public(
            "POST",
            &claim_path(&a.id, ktok),
            None,
            json!({ "upi": { "vpa": "asha.rao@ybl" } }),
        )
        .await;
    assert_eq!(s.as_u16(), 409);
    assert_eq!(
        t.count("SELECT count(*) FROM payouts").await,
        1,
        "only the first link paid out"
    );

    // the scope to create is the payout scope
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            limited(json!(["payments:read"])),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: ro["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&ro, "POST", "/v1/payout_links", None, make(None, "U-5"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/payout_links", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn simultaneous_claims_of_one_payout_link_make_exactly_one_payout(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let l = t
        .ok(
            a,
            "POST",
            "/v1/payout_links",
            None,
            json!({ "amount_minor": 70_000, "purpose": "Refund", "expected_name": "Asha Rao" }),
        )
        .await;
    let path = claim_path(&a.id, l["token"].as_str().unwrap());
    let body = json!({ "upi": { "vpa": "asha.rao@ybl" } });
    let (r1, r2, r3, r4) = tokio::join!(
        t.public("POST", &path, None, body.clone()),
        t.public("POST", &path, None, body.clone()),
        t.public("POST", &path, None, body.clone()),
        t.public("POST", &path, None, body.clone()),
    );
    let codes: Vec<u16> = [r1.0, r2.0, r3.0, r4.0]
        .iter()
        .map(|s| s.as_u16())
        .collect();
    assert_eq!(codes.iter().filter(|c| **c == 201).count(), 1, "{codes:?}");
    assert!(codes.iter().all(|c| matches!(*c, 201 | 409)), "{codes:?}");
    assert_eq!(t.count("SELECT count(*) FROM payouts").await, 1);
    assert_eq!(
        t.count("SELECT count(*) FROM vendors").await,
        1,
        "a refused claim leaves no vendor behind"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn part_payment_links_follow_the_lifecycle_and_combine_only_with_an_exact_amount(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    // part payments need an exact amount: every open mode with allow_partial is refused, and nothing is made
    for body in [
        json!({ "amount_mode": "minimum", "min_amount_minor": 1_000, "allow_partial": true, "min_part_minor": 100 }),
        json!({ "amount_mode": "up_to", "max_amount_minor": 1_000, "allow_partial": true, "min_part_minor": 100 }),
        json!({ "amount_mode": "pay_what_you_want", "allow_partial": true, "min_part_minor": 100 }),
        json!({ "open_amount": true, "allow_partial": true, "min_part_minor": 100 }),
    ] {
        let (s, e) = t
            .call(a, "POST", "/v1/payment_links", None, body.clone())
            .await;
        assert_eq!(
            (s.as_u16(), e["error"]["code"].clone()),
            (422, json!("invalid_amount_mode")),
            "{body}"
        );
    }
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 0);

    // exact + allow_partial: the wrapper hands the main branch's fields through untouched
    let l = t
        .ok(a, "POST", "/v1/payment_links", None, json!({ "amount_mode": "exact", "amount_minor": 100_000, "allow_partial": true, "min_part_minor": 20_000, "message": "Course fee", "allowed_methods": ["upi"] }))
        .await;
    assert_eq!(l["amount_mode"], "exact");
    let (id, tok) = (
        l["id"].as_str().unwrap().to_string(),
        l["token"].as_str().unwrap().to_string(),
    );
    let (partial, msg, methods): (bool, Option<String>, Option<Vec<String>>) = sqlx::query_as(
        "SELECT allow_partial, message, allowed_methods FROM payment_links WHERE id = $1",
    )
    .bind(&id)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(
        (partial, msg.as_deref(), methods),
        (true, Some("Course fee"), Some(vec!["upi".to_string()]))
    );
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{}/{tok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            info["amount_mode"].clone(),
            info["partial"]["balance_minor"].clone()
        ),
        (json!("exact"), json!(100_000))
    );
    assert_eq!(
        info["message"], "Course fee",
        "the main handler's fields are still there"
    );

    assert_eq!(lifecycle(&t, a, &id).await["status"], "created");
    // first part
    let s1 = start(&t, &a.id, &tok, json!({ "amount_minor": 30_000 }))
        .await
        .1;
    assert_eq!(lifecycle(&t, a, &id).await["status"], "active");
    pay(
        &t,
        a,
        s1["intent_id"].as_str().unwrap(),
        "captured",
        json!({}),
    )
    .await;
    sweep_links(&t, a).await;
    let lc = lifecycle(&t, a, &id).await;
    assert_eq!(
        (
            lc["status"].clone(),
            lc["paid_minor"].clone(),
            lc["payable"].clone()
        ),
        (json!("partially_paid"), json!(30_000), json!(true))
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment_link.partially_paid'")
            .await,
        1
    );
    let listed = t.ok(a, "GET", "/v1/payment_links", None, Value::Null).await;
    assert_eq!(listed["data"][0]["lifecycle_status"], "partially_paid");
    // the rest: paid
    let s2 = start(&t, &a.id, &tok, json!({ "amount_minor": 70_000 }))
        .await
        .1;
    pay(
        &t,
        a,
        s2["intent_id"].as_str().unwrap(),
        "captured",
        json!({}),
    )
    .await;
    sweep_links(&t, a).await;
    let lc = lifecycle(&t, a, &id).await;
    assert_eq!(
        (
            lc["status"].clone(),
            lc["paid_minor"].clone(),
            lc["payable"].clone()
        ),
        (json!("paid"), json!(100_000), json!(false))
    );
    let hist: Vec<String> = lc["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["to"].as_str().unwrap().to_string())
        .collect();
    // the worker first looked after the first part was paid, so `active` was never a recorded stop
    assert_eq!(hist, ["created", "partially_paid", "paid"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payout_link_claim_respects_the_payout_policy_and_leaves_nothing_behind_when_refused(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "PUT",
        "/v1/payout_policy",
        None,
        json!({ "max_payout_minor": 100_000 }),
    )
    .await;
    let l = t
        .ok(
            a,
            "POST",
            "/v1/payout_links",
            None,
            json!({ "amount_minor": 250_000, "purpose": "Refund", "expected_name": "Asha Rao" }),
        )
        .await;
    let path = claim_path(&a.id, l["token"].as_str().unwrap());
    let (s, e) = t
        .public(
            "POST",
            &path,
            None,
            json!({ "upi": { "vpa": "asha.rao@ybl" } }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("try_later"))
    );
    assert!(
        !e.to_string().contains("limit"),
        "the recipient is not shown the business's limits: {e}"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM vendors").await,
        0,
        "a refused claim rolls the vendor back"
    );
    assert_eq!(t.count("SELECT count(*) FROM payouts").await, 0);
    // the link was not used up: raising the limit lets the same link be claimed
    t.ok(
        a,
        "PUT",
        "/v1/payout_policy",
        None,
        json!({ "max_payout_minor": 300_000 }),
    )
    .await;
    let (s, _) = t
        .public(
            "POST",
            &path,
            None,
            json!({ "upi": { "vpa": "asha.rao@ybl" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    // the vendor made for the link is an ordinary active vendor, and the payout is pending the usual second person
    let v: (String,) = sqlx::query_as("SELECT status FROM vendors")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(v.0, "active");
    let p: (String, String) = sqlx::query_as("SELECT status, requested_by FROM payouts")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(p.0, "pending_approval");
}
