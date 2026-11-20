// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Fee checks: what a provider deducted from a settlement against the merchant's own fee rules.
//! All rates used here are arbitrary round numbers invented for the tests. They are not any provider's, network's
//! or tax authority's rate, and nothing here ran against a real provider.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn today() -> String {
    zyvor_api::ist_today().to_string()
}

async fn rule(t: &T, tn: &Tenant, extra: Value) -> Value {
    let mut b = json!({ "provider": "razorpay", "method": "upi", "bps": 150, "effective_from": "2020-01-01" });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.ok(tn, "POST", "/v1/fee_rules", None, b).await
}

/// Import a settlement for one payment with the given payment / refund / fee amounts (paise). Returns the explain JSON.
async fn settle(
    t: &T,
    tn: &Tenant,
    n: &str,
    pay_ref: &str,
    payment: Option<i64>,
    refund: Option<i64>,
    fees: &[i64],
) -> Value {
    let mut items = vec![];
    if let Some(p) = payment {
        items.push(json!({ "kind": "payment", "provider_ref": pay_ref, "amount_minor": p }));
    }
    if let Some(r) = refund {
        items.push(json!({ "kind": "refund", "provider_ref": pay_ref, "amount_minor": -r }));
    }
    for f in fees {
        // deducted fees are negative lines; a positive value in `fees` here means "deducted"
        items.push(json!({ "kind": "fee", "provider_ref": pay_ref, "amount_minor": -f }));
    }
    t.ok(
        tn,
        "POST",
        "/v1/settlements/import",
        None,
        json!({ "provider": "razorpay", "provider_ref": format!("stl_{n}"), "utr": format!("UTR{n}"), "settled_on": today(), "items": items }),
    )
    .await
}

fn fee_case(cases: &Value) -> Option<&Value> {
    cases["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "fee_check")
}

#[sqlx::test(migrations = "./migrations")]
async fn rules_are_validated_versioned_and_need_the_right_scope(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // nothing is configured by default: no rate is shipped
    let list = t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    let sum = t
        .ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
        .await;
    assert_eq!(sum["message"], "no expected fee configured");
    assert_eq!(sum["rules_configured"], 0);

    let r = rule(&t, a, json!({ "flat_minor": 200, "cap_minor": 5000, "gst_bps": 1800, "note": "from contract p.4" })).await;
    assert_eq!(
        (
            r["bps"].as_i64(),
            r["flat_minor"].as_i64(),
            r["cap_minor"].as_i64(),
            r["gst_bps"].as_i64()
        ),
        (Some(150), Some(200), Some(5000), Some(1800))
    );
    // the same start date again is refused; a new rate needs a later date
    let (s, e) = t.call(a, "POST", "/v1/fee_rules", None, json!({ "provider": "razorpay", "method": "upi", "bps": 100, "effective_from": "2020-01-01" })).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("duplicate_rule"))
    );
    rule(&t, a, json!({ "bps": 100, "effective_from": "2026-04-01" })).await;
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for (bad, code) in [
        (json!({ "bps": 10_001 }), "invalid_bps"),
        (json!({ "bps": -1 }), "invalid_bps"),
        (json!({ "method": "cheque" }), "invalid_method"),
        (json!({ "flat_minor": -5 }), "invalid_amount"),
        (json!({ "gst_bps": 20_000 }), "invalid_gst_bps"),
        (json!({ "provider": " " }), "invalid_provider"),
    ] {
        let mut b = json!({ "provider": "razorpay", "method": "upi", "bps": 150, "effective_from": "2021-01-01" });
        for (k, v) in bad.as_object().unwrap() {
            b[k] = v.clone();
        }
        let (s, e) = t.call(a, "POST", "/v1/fee_rules", None, b).await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some(code)),
            "{bad}"
        );
    }
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/fee_checks/settings",
            None,
            json!({ "tolerance_minor": -1 }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_tolerance"))
    );
    let st = t
        .ok(
            a,
            "POST",
            "/v1/fee_checks/settings",
            None,
            json!({ "tolerance_minor": 2, "owner": "meena" }),
        )
        .await;
    assert_eq!(
        (st["tolerance_minor"].as_i64(), st["owner"].as_str()),
        (Some(2), Some("meena"))
    );
    // a partial update keeps the other value
    let st = t
        .ok(
            a,
            "POST",
            "/v1/fee_checks/settings",
            None,
            json!({ "tolerance_minor": 3 }),
        )
        .await;
    assert_eq!(st["owner"], "meena");

    // a read-only key may read but not configure
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "ro", "scopes": ["recon:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&ro, "GET", "/v1/fee_rules", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    for (m, p, b) in [
        (
            "POST",
            "/v1/fee_rules",
            json!({ "provider": "razorpay", "method": "upi", "bps": 1, "effective_from": "2022-01-01" }),
        ),
        (
            "POST",
            "/v1/fee_checks/settings",
            json!({ "tolerance_minor": 9 }),
        ),
        ("POST", "/v1/fee_checks/run", json!({})),
    ] {
        assert_eq!(
            t.call(&ro, m, p, None, b).await.0,
            StatusCode::FORBIDDEN,
            "{p}"
        );
    }
    assert_eq!(
        t.call(
            &ro,
            "DELETE",
            &format!("/v1/fee_rules/{}", r["id"].as_str().unwrap()),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // a key with no recon scope cannot even read
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "pay", "scopes": ["payments:read"] }),
        )
        .await;
    let pay = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&pay, "GET", "/v1/fee_checks/findings", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // deleting a rule is audited
    t.ok(
        a,
        "DELETE",
        &format!("/v1/fee_rules/{}", r["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(t.count(&format!("SELECT count(*) FROM audit_events WHERE tenant_id = '{}' AND action IN ('fee_rule.created', 'fee_rule.deleted')", a.id)).await, 3);
    let (s, _) = t
        .call(a, "DELETE", "/v1/fee_rules/fr_nope", None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn over_and_under_deduction_become_one_owned_case_and_change_no_money(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // 150 bps + 18% GST on the fee (test numbers): 100_000 -> 1_500 + 270 = 1_770
    rule(&t, a, json!({ "gst_bps": 1800 })).await;
    t.ok(
        a,
        "POST",
        "/v1/fee_checks/settings",
        None,
        json!({ "tolerance_minor": 1, "owner": "meena" }),
    )
    .await;

    let (_, att_ok, ref_ok) = t.paid_razorpay(a, 100_000, json!({})).await;
    let (_, att_over, ref_over) = t.paid_razorpay(a, 100_000, json!({})).await;
    let (_, att_under, ref_under) = t.paid_razorpay(a, 100_000, json!({})).await;
    let ledger_before = t
        .count(&format!(
            "SELECT count(*) FROM ledger_entries WHERE tenant_id = '{}'",
            a.id
        ))
        .await;
    let status_before = t
        .count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE tenant_id = '{}' AND status = 'captured'",
            a.id
        ))
        .await;

    // within the tolerance of 1 paisa: not flagged
    let ok = settle(&t, a, "1", &ref_ok, Some(100_000), None, &[1_771]).await;
    assert_eq!(ok["fee_check"]["status"], "ok");
    assert_eq!(ok["fee_check"]["expected_minor"], 1_770);
    assert!(!ok["narrative"].as_str().unwrap().contains("Fee differs"));
    assert!(fee_case(&t.ok(a, "GET", "/v1/recon/cases", None, Value::Null).await).is_none());

    // the provider took 2_500: over by 730
    let over = settle(&t, a, "2", &ref_over, Some(100_000), None, &[2_500]).await;
    assert_eq!(over["fee_check"]["status"], "flagged");
    assert_eq!(over["fee_check"]["variance_minor"], 730);
    let line = &over["fee_check"]["lines"][0];
    assert_eq!(
        (
            line["attempt_id"].as_str(),
            line["expected_minor"].as_i64(),
            line["actual_minor"].as_i64()
        ),
        (Some(att_over.as_str()), Some(1_770), Some(2_500))
    );
    assert_eq!(
        line["rule"]["bps"], 150,
        "the rule that was applied is recorded with the line"
    );
    assert_eq!(line["rule"]["breakdown"]["gst_minor"], 270);
    let narrative = over["narrative"].as_str().unwrap();
    assert!(
        narrative.contains("Fee differs from your configured rate by ₹7.30"),
        "{narrative}"
    );
    assert!(narrative.contains("more than expected"));

    // the provider took only 1_000: under by 770
    let under = settle(&t, a, "3", &ref_under, Some(100_000), None, &[1_000]).await;
    assert_eq!(under["fee_check"]["variance_minor"], -770);
    assert!(
        under["narrative"]
            .as_str()
            .unwrap()
            .contains("less than expected")
    );

    // one case per settlement, owned, with both settlements flagged
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=fee_check",
            None,
            Value::Null,
        )
        .await;
    let list = cases["data"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{cases}");
    for c in list {
        assert_eq!(
            (c["status"].as_str(), c["assignee"].as_str()),
            (Some("assigned"), Some("meena"))
        );
        assert!(
            c["explanation"]
                .as_str()
                .unwrap()
                .contains("Check it against your contract")
        );
    }
    // a flagged settlement shows as an exception, like any open case
    let sid = over["settlement"]["id"].as_str().unwrap();
    assert_eq!(over["settlement"]["status"], "exception");

    let sum = t
        .ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
        .await;
    assert_eq!(sum["open_findings"], 2);
    assert_eq!(sum["over_deducted_minor"], 730);
    assert_eq!(sum["under_deducted_minor"], 770);
    assert_eq!(sum["total_variance_minor"], -40);
    assert_eq!(sum["total_absolute_variance_minor"], 1_500);
    let row = &sum["by_provider_method"][0];
    assert_eq!(
        (
            row["provider"].as_str(),
            row["method"].as_str(),
            row["open_findings"].as_i64()
        ),
        (Some("razorpay"), Some("upi"), Some(2))
    );

    let findings = t
        .ok(a, "GET", "/v1/fee_checks/findings", None, Value::Null)
        .await;
    let f = findings["data"].as_array().unwrap();
    assert_eq!(f.len(), 2);
    assert!(f.iter().any(|x| x["direction"] == "over_deducted"
        && x["variance_minor"] == 730
        && x["attempt_id"] == att_over.as_str()));
    assert!(f.iter().any(|x| x["direction"] == "under_deducted"
        && x["variance_minor"] == -770
        && x["attempt_id"] == att_under.as_str()));
    assert!(
        f.iter()
            .all(|x| x["owner"] == "meena" && x["case_status"] == "assigned")
    );
    assert!(f.iter().all(|x| x["attempt_id"] != att_ok.as_str()));
    let filtered = t
        .ok(
            a,
            "GET",
            "/v1/fee_checks/findings?provider=cashfree",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(filtered["data"].as_array().unwrap().len(), 0);
    assert_eq!(
        t.call(
            a,
            "GET",
            "/v1/fee_checks/findings?status=weird",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // nothing about the money moved: the ledger and the payments are as they were
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM ledger_entries WHERE tenant_id = '{}'",
            a.id
        ))
        .await,
        ledger_before
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE tenant_id = '{}' AND status = 'captured'",
            a.id
        ))
        .await,
        status_before
    );

    // re-running is idempotent: no second case, no second finding
    t.ok(
        a,
        "POST",
        &format!("/v1/settlements/{sid}/fee_check"),
        None,
        Value::Null,
    )
    .await;
    t.ok(a, "POST", "/v1/fee_checks/run", None, json!({})).await;
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/recon/cases?kind=fee_check",
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["open_findings"],
        2
    );

    // a person resolves one case: it leaves the open list and appears as reviewed
    let case_id = fee_case(
        &t.ok(
            a,
            "GET",
            "/v1/recon/cases?kind=fee_check&limit=200",
            None,
            Value::Null,
        )
        .await,
    )
    .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(
        a,
        "POST",
        &format!("/v1/recon/cases/{case_id}/resolve"),
        None,
        json!({ "resolution": "asked the provider; credit note promised" }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["open_findings"],
        1
    );
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/fee_checks/findings?status=reviewed",
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/fee_checks/findings?status=all",
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn no_rule_is_said_plainly_and_never_flagged_and_a_zero_fee_rule_flags_any_deduction(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let (_, _, pref) = t.paid_razorpay(a, 50_000, json!({})).await;

    // no rules at all
    let s = settle(&t, a, "1", &pref, Some(50_000), None, &[4_000]).await;
    assert_eq!(s["fee_check"]["status"], "no_rule");
    assert!(
        s["fee_check"]["message"]
            .as_str()
            .unwrap()
            .contains("No expected fee configured")
    );
    assert!(!s["narrative"].as_str().unwrap().contains("Fee differs"));
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["message"],
        "no expected fee configured"
    );
    assert!(fee_case(&t.ok(a, "GET", "/v1/recon/cases", None, Value::Null).await).is_none());

    // a rule for a different provider does not apply to this one
    rule(&t, a, json!({ "provider": "cashfree" })).await;
    let sid = s["settlement"]["id"].as_str().unwrap();
    let again = t
        .ok(
            a,
            "POST",
            &format!("/v1/settlements/{sid}/fee_check"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(again["status"], "no_rule");
    assert_eq!(again["lines"][0]["status"], "no_rule");
    assert!(again["lines"][0]["variance_minor"].is_null());
    let sum = t
        .ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
        .await;
    assert_eq!(
        (
            sum["open_findings"].as_i64(),
            sum["lines_without_a_rule"].as_i64()
        ),
        (Some(0), Some(1))
    );

    // declaring the method zero-fee makes any deduction a finding; an honest zero is fine
    let (_, _, p2) = t.paid_razorpay(a, 50_000, json!({})).await;
    let (_, _, p3) = t.paid_razorpay(a, 50_000, json!({})).await;
    rule(&t, a, json!({ "bps": 0 })).await;
    let zero_ok = settle(&t, a, "2", &p2, Some(50_000), None, &[]).await;
    assert_eq!(zero_ok["fee_check"]["status"], "ok");
    let zero_bad = settle(&t, a, "3", &p3, Some(50_000), None, &[300]).await;
    assert_eq!(
        (
            zero_bad["fee_check"]["status"].as_str(),
            zero_bad["fee_check"]["variance_minor"].as_i64()
        ),
        (Some("flagged"), Some(300))
    );
    // and with no owner configured the case is open and unassigned
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=fee_check",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            fee_case(&cases).unwrap()["status"].as_str(),
            fee_case(&cases).unwrap()["assignee"].is_null()
        ),
        (Some("open"), true)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn cap_rounding_and_effective_dates_use_the_rule_in_force_on_the_payment_date(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // cap: 200 bps would be 20_000 on 1_000_000, capped at 1_500 before GST; GST 1_000 bps -> 150
    let old = rule(
        &t,
        a,
        json!({ "bps": 200, "cap_minor": 1_500, "gst_bps": 1000, "effective_from": "2020-01-01" }),
    )
    .await;
    // a later, different rate that has not started yet must not touch today's payment
    let future = rule(&t, a, json!({ "bps": 900, "effective_from": "2999-01-01" })).await;
    let (_, _, pref) = t.paid_razorpay(a, 1_000_000, json!({})).await;
    let s = settle(&t, a, "1", &pref, Some(1_000_000), None, &[1_650]).await;
    assert_eq!(s["fee_check"]["status"], "ok", "{s}");
    let line = &s["fee_check"]["lines"][0];
    assert_eq!(line["rule"]["id"], old["id"]);
    assert_ne!(line["rule"]["id"], future["id"]);
    assert_eq!(line["rule"]["breakdown"]["capped"], true);
    assert_eq!(line["expected_minor"], 1_650);

    // a rule that started yesterday replaces the older one for today's payments, and history keeps the old
    let yesterday = (zyvor_api::ist_today() - chrono::Days::new(1)).to_string();
    let newer = rule(&t, a, json!({ "bps": 100, "effective_from": yesterday })).await;
    let (_, _, p2) = t.paid_razorpay(a, 100_050, json!({})).await;
    // 100 bps of 100_050 = 1_000.5 -> 1_001 (half up)
    let s2 = settle(&t, a, "2", &p2, Some(100_050), None, &[1_001]).await;
    assert_eq!(s2["fee_check"]["status"], "ok", "{s2}");
    assert_eq!(s2["fee_check"]["lines"][0]["rule"]["id"], newer["id"]);
    // the earlier record still shows the rule it was checked against, even after that rule is removed
    t.ok(
        a,
        "DELETE",
        &format!("/v1/fee_rules/{}", old["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    let kept: Value =
        sqlx::query_scalar("SELECT rule FROM fee_check_lines WHERE settlement_id = $1")
            .bind(s["settlement"]["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(kept["id"], old["id"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn refunds_that_return_the_fee_are_checked_and_a_corrected_rule_clears_a_stale_finding(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    rule(
        &t,
        a,
        json!({ "bps": 100, "flat_minor": 50, "refund_returns_fee": true }),
    )
    .await;
    // payment 100_000: fee 1_050. A 40_000 refund leaves 60_000: fee 650, so 400 comes back. Net 650.
    let (_, _, p1) = t.paid_razorpay(a, 100_000, json!({})).await;
    let good = settle(&t, a, "1", &p1, Some(100_000), Some(40_000), &[1_050, -400]).await;
    assert_eq!(good["fee_check"]["status"], "ok", "{good}");
    assert_eq!(good["fee_check"]["lines"][0]["expected_minor"], 650);
    assert_eq!(good["fee_check"]["lines"][0]["refunded_minor"], 40_000);

    // the provider kept the whole fee on the refunded value: 400 over
    let (_, _, p2) = t.paid_razorpay(a, 100_000, json!({})).await;
    let kept = settle(&t, a, "2", &p2, Some(100_000), Some(40_000), &[1_050]).await;
    assert_eq!(
        (
            kept["fee_check"]["status"].as_str(),
            kept["fee_check"]["variance_minor"].as_i64()
        ),
        (Some("flagged"), Some(400))
    );

    // a full refund returns the flat fee too
    let (_, _, p3) = t.paid_razorpay(a, 100_000, json!({})).await;
    let full = settle(
        &t,
        a,
        "3",
        &p3,
        Some(100_000),
        Some(100_000),
        &[1_050, -1_050],
    )
    .await;
    assert_eq!(full["fee_check"]["status"], "ok", "{full}");

    // a refund that settles on a later day, on its own: the fee comes back as a positive line
    let (_, _, p4) = t.paid_razorpay(a, 100_000, json!({})).await;
    let later = settle(&t, a, "4", &p4, None, Some(40_000), &[-400]).await;
    assert_eq!(later["fee_check"]["status"], "ok", "{later}");
    assert_eq!(later["fee_check"]["lines"][0]["expected_minor"], -400);
    // ...and if the refund settles with no fee returned, that is a finding too
    let (_, _, p5) = t.paid_razorpay(a, 100_000, json!({})).await;
    let none_back = settle(&t, a, "5", &p5, None, Some(40_000), &[]).await;
    assert_eq!(
        (
            none_back["fee_check"]["status"].as_str(),
            none_back["fee_check"]["variance_minor"].as_i64()
        ),
        (Some("flagged"), Some(400))
    );

    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["open_findings"],
        2
    );

    // the contract turns out not to return fees: correcting the rule and re-running clears both stale findings
    let rules = t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await;
    t.ok(
        a,
        "DELETE",
        &format!("/v1/fee_rules/{}", rules["data"][0]["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    rule(
        &t,
        a,
        json!({ "bps": 100, "flat_minor": 50, "refund_returns_fee": false }),
    )
    .await;
    // with fees kept: "kept" (1_050 deducted, nothing returned) is now right and its case closes; "none_back" has
    // nothing to compare and closes; "good" and "full" are now under (they were given fee back), and so is the
    // refund-only line that returned 400
    let run = t.ok(a, "POST", "/v1/fee_checks/run", None, json!({})).await;
    assert_eq!(run["settlements_checked"], 5);
    let sum = t
        .ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
        .await;
    assert_eq!(sum["open_findings"], 3, "{sum}");
    assert_eq!(sum["over_deducted_minor"], 0);
    assert_eq!(sum["under_deducted_minor"], 400 + 1_050 + 400, "{sum}");
    // the settlement that is right under the corrected rule has its case closed with the reason on record
    let kept_id = kept["settlement"]["id"].as_str().unwrap();
    let case: (String, Option<String>) = sqlx::query_as(
        "SELECT status, resolution FROM recon_cases WHERE kind = 'fee_check' AND subject_id = $1",
    )
    .bind(kept_id)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(case.0, "resolved");
    assert!(case.1.unwrap().starts_with("Re-checked"));
    // the finding row that no longer applies is gone, not left flagged
    assert_eq!(
        t.count(
            "SELECT count(*) FROM fee_check_lines WHERE status = 'flagged' AND variance_minor > 0"
        )
        .await,
        0
    );

    // with every rule removed there is nothing to compare against: the rest clear too
    for r in t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await["data"]
        .as_array()
        .unwrap()
    {
        t.ok(
            a,
            "DELETE",
            &format!("/v1/fee_rules/{}", r["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    }
    t.ok(a, "POST", "/v1/fee_checks/run", None, json!({})).await;
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["open_findings"],
        0
    );
    assert_eq!(t.count("SELECT count(*) FROM fee_check_lines").await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn fee_checks_are_isolated_between_businesses(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    rule(&t, a, json!({})).await;
    t.ok(
        a,
        "POST",
        "/v1/fee_checks/settings",
        None,
        json!({ "tolerance_minor": 7, "owner": "meena" }),
    )
    .await;
    let (_, _, pref) = t.paid_razorpay(a, 100_000, json!({})).await;
    let s = settle(&t, a, "1", &pref, Some(100_000), None, &[9_000]).await;
    let sid = s["settlement"]["id"].as_str().unwrap();
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_checks/summary", None, Value::Null)
            .await["open_findings"],
        1
    );

    // B sees none of it
    assert_eq!(
        t.ok(b, "GET", "/v1/fee_rules", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.ok(
            b,
            "GET",
            "/v1/fee_checks/findings?status=all",
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let sum = t
        .ok(b, "GET", "/v1/fee_checks/summary", None, Value::Null)
        .await;
    assert_eq!(
        (
            sum["open_findings"].as_i64(),
            sum["rules_configured"].as_i64()
        ),
        (Some(0), Some(0))
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/fee_checks/settings", None, Value::Null)
            .await["tolerance_minor"],
        0
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/settlements/{sid}/fee_check"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.call(
            b,
            "POST",
            &format!("/v1/settlements/{sid}/fee_check"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let rule_id = t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        t.call(
            b,
            "DELETE",
            &format!("/v1/fee_rules/{rule_id}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // B's run checks nothing of A's
    assert_eq!(
        t.ok(b, "POST", "/v1/fee_checks/run", None, json!({})).await["settlements_checked"],
        0
    );
    // and A's rule is still there
    assert_eq!(
        t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // the database itself refuses: a connection scoped to B reads no rows of A's, and cannot write one for A
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
        .await
        .unwrap();
    for table in ["fee_rules", "fee_check_settings", "fee_check_lines"] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let forged = sqlx::query("INSERT INTO fee_rules (id, tenant_id, provider, method, bps, effective_from, created_by) VALUES ('fr_x', $1, 'razorpay', 'upi', 1, '2020-01-01', 'x')")
        .bind(&a.id)
        .execute(&mut *tx)
        .await;
    assert!(
        forged.is_err(),
        "row-level security must refuse a row for another tenant"
    );
}
