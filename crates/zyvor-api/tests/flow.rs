//! End-to-end flows: sandbox journey, reconciliation, refund operations, routing, security.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn today() -> String {
    chrono::Utc::now().date_naive().to_string()
}

async fn set_routing(t: &T, tn: &Tenant, routing: Value) {
    t.ok(
        tn,
        "POST",
        "/v1/settings",
        None,
        json!({ "routing": routing }),
    )
    .await;
}

// 8 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn complete_sandbox_flow_checkout_to_invoice_to_settlement_to_partial_refund(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    set_routing(
        &t,
        a,
        json!({ "rules": [], "fees": { "razorpay": { "upi": { "bps": 200, "fixed_minor": 0 } } } }),
    )
    .await;

    // business context: a customer and a GST invoice raised by the billing product
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha Rao", "phone": "9876543210", "external_ref": "STU-2231" }),
        )
        .await;
    let inv = t.ok(a, "POST", "/v1/invoices", None, json!({ "customer_id": cust["id"], "number": "INV-2026-0042", "amount_minor": 100_000, "tax_minor": 15_254, "external_ref": "gst:IRN-ab12" })).await;
    let inv_id = inv["id"].as_str().unwrap();

    // checkout: intent for the invoice, provider-hosted attempt
    let (iid, aid, pref) = t.paid_razorpay(a, 100_000, json!({ "customer_id": cust["id"], "invoice_id": inv_id, "external_ref": "FEE-STU-2231-T2" })).await;
    let invoice = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{inv_id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            invoice["status"].as_str(),
            invoice["paid_minor"].as_i64(),
            invoice["outstanding_minor"].as_i64()
        ),
        (Some("paid"), Some(100_000), Some(0))
    );
    let captured_event = sqlx::query_scalar::<_, Value>(
        "SELECT payload FROM outbox_events WHERE type = 'payment.captured'",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(
        captured_event["data"]["external_ref"], "FEE-STU-2231-T2",
        "the business object travels with the payment event"
    );
    assert_eq!(captured_event["data"]["invoice_id"], inv_id);

    // settlement: provider pays out 100,000 less 2% fee and 18% GST on the fee
    t.mock_post(
        "razorpay/settle",
        json!({ "fee_bps": 200, "tax_bps": 1800 }),
    )
    .await;
    let synced = t
        .ok(
            a,
            "POST",
            "/v1/settlements/sync",
            None,
            json!({ "provider": "razorpay", "date": today() }),
        )
        .await;
    let sid = synced["imported"][0].as_str().unwrap();
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["payments"][0]["attempt_id"], aid.as_str());
    assert_eq!(ex["payments"][0]["matched"], true);
    assert_eq!(ex["payments"][0]["external_ref"], "FEE-STU-2231-T2");
    assert_eq!(ex["fees"]["observed_minor"], 2_360);
    assert_eq!(ex["fees"]["configured_estimate_minor"], 2_000);
    assert_eq!(ex["received_minor"], 97_640);
    assert_eq!(
        ex["difference_minor"], -360,
        "short by exactly the GST on the fee"
    );
    assert_eq!(ex["difference_breakdown"][0]["amount_minor"], -360);
    assert_eq!(ex["arithmetic"]["balances"], true);
    assert!(
        ex["narrative"].as_str().unwrap().contains("short"),
        "{}",
        ex["narrative"]
    );
    assert!(ex["bank_match"].is_null(), "no bank statement yet");

    // bank statement arrives and matches by UTR
    let utr = ex["settlement"]["utr"].as_str().unwrap();
    let bank = t.ok(a, "POST", "/v1/bank_statements/import", None, json!({ "lines": [{ "posted_on": today(), "amount_minor": 97_640, "narration": "NEFT RAZORPAY", "utr": utr }] })).await;
    assert_eq!(bank["added"], 1);
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["settlement"]["status"], "matched");
    assert_eq!(ex["bank_match"]["amount_minor"], 97_640);
    let open = t
        .ok(a, "GET", "/v1/recon/cases?status=open", None, Value::Null)
        .await;
    assert_eq!(
        open["data"].as_array().unwrap().len(),
        0,
        "everything is explained: {open}"
    );

    // partial refund linked to a return, reflected on the invoice, the ledger and the next settlement
    let rf = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("ret-9"),
            json!({ "amount_minor": 25_000, "reason": "returned item", "external_ref": "RET-9" }),
        )
        .await;
    assert_eq!(rf["status"], "processed");
    assert_eq!(rf["external_ref"], "RET-9");
    let invoice = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{inv_id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            invoice["status"].as_str(),
            invoice["outstanding_minor"].as_i64()
        ),
        (Some("partially_paid"), Some(25_000))
    );
    t.mock_post(
        "razorpay/settle",
        json!({ "fee_bps": 200, "tax_bps": 1800 }),
    )
    .await;
    let synced = t
        .ok(
            a,
            "POST",
            "/v1/settlements/sync",
            None,
            json!({ "provider": "razorpay", "date": today() }),
        )
        .await;
    let sid2 = synced["imported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .find(|s| *s != sid)
        .unwrap()
        .to_string();
    let ex2 = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid2}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex2["refunds"][0]["matched"], true);
    assert_eq!(ex2["refunds"][0]["external_ref"], "RET-9");
    assert_eq!(ex2["received_minor"], -25_000);

    // the books: debits and credits agree; provider receivable = captured − refunded
    let (d, c): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries").fetch_one(&t.root).await.unwrap();
    assert_eq!(d, c);
    let recv: i64 = t.count("SELECT COALESCE(SUM(CASE side WHEN 'debit' THEN amount_minor ELSE -amount_minor END),0)::BIGINT FROM ledger_entries WHERE account LIKE 'provider_receivable:%'").await;
    assert_eq!(recv, 75_000);
    let _ = pref;
}

// 9 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn reconciliation_turns_every_unexplained_difference_into_an_assigned_task(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    set_routing(
        &t,
        a,
        json!({ "rules": [], "fees": { "razorpay": { "upi": { "bps": 200, "fixed_minor": 0 } } } }),
    )
    .await;
    let (_, _, p1) = t.paid_razorpay(a, 50_000, json!({})).await;
    let (_, p2_att, p2) = t.paid_razorpay(a, 30_000, json!({})).await;

    // the provider settles only p1, with a fee far above the configured 2%, and takes a chargeback
    t.mock_post("razorpay/settle", json!({ "fee_bps": 500, "tax_bps": 1800, "skip": [p2], "adjustment_minor": -5_000, "adjustment_note": "chargeback" })).await;
    let synced = t
        .ok(
            a,
            "POST",
            "/v1/settlements/sync",
            None,
            json!({ "provider": "razorpay", "date": today() }),
        )
        .await;
    let sid = synced["imported"][0].as_str().unwrap();
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["settlement"]["status"], "exception");
    let reasons: Vec<String> = ex["difference_breakdown"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["reason"].as_str().unwrap().to_string())
        .collect();
    assert!(
        reasons.iter().any(|r| r.contains("fees"))
            && reasons.iter().any(|r| r.contains("adjustments")),
        "{reasons:?}"
    );
    assert!(ex["narrative"].as_str().unwrap().contains("short"));
    let kinds = |cases: &Value| {
        cases["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["kind"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let cases = t
        .ok(a, "GET", "/v1/recon/cases?status=open", None, Value::Null)
        .await;
    assert!(
        kinds(&cases).contains(&"fee_variance".to_string()),
        "{cases}"
    );

    // the second payment never appeared in a settlement
    let scan = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=0",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(scan["new_cases"], 1);
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=missing_settlement",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"][0]["subject_id"], p2_att.as_str());
    assert_eq!(cases["data"][0]["expected_minor"], 30_000);

    // a hand-imported statement exposes duplicates, unknown lines and partial settlement
    t.ok(a, "POST", "/v1/settlements/import", None, json!({ "provider": "razorpay", "provider_ref": "manual_1", "settled_on": today(), "items": [
        { "kind": "payment", "provider_ref": p1, "amount_minor": 30_000 },
        { "kind": "payment", "provider_ref": p1, "amount_minor": 30_000 },
        { "kind": "payment", "provider_ref": "pay_NEVERHEARD", "amount_minor": 10_000 },
        { "kind": "fee", "provider_ref": p1, "amount_minor": -500 } ] })).await;
    let all = t
        .ok(a, "GET", "/v1/recon/cases?limit=200", None, Value::Null)
        .await;
    for k in [
        "settled_amount_mismatch",
        "duplicate_settlement_item",
        "unknown_settlement_item",
    ] {
        assert!(kinds(&all).contains(&k.to_string()), "{k} in {all}");
    }

    // an unexplained bank credit is a task too
    t.ok(a, "POST", "/v1/bank_statements/import", None, json!({ "lines": [{ "posted_on": today(), "amount_minor": 12_345, "narration": "UNKNOWN CREDIT", "utr": "UTRX1" }] })).await;
    let all = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=unmatched_bank_credit",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(all["data"].as_array().unwrap().len(), 1);

    // work them: assign, then resolve; the settlement leaves `exception` when its last case closes
    let open = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?status=open&limit=200",
            None,
            Value::Null,
        )
        .await;
    for c in open["data"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        let asg = t
            .ok(
                a,
                "POST",
                &format!("/v1/recon/cases/{id}/assign"),
                None,
                json!({ "assignee": "finance@acme.test" }),
            )
            .await;
        assert_eq!(asg["status"], "assigned");
    }
    assert_eq!(
        t.ok(a, "GET", "/v1/recon/cases?status=open", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "all assigned"
    );
    for c in t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?status=assigned&limit=200",
            None,
            Value::Null,
        )
        .await["data"]
        .as_array()
        .unwrap()
    {
        t.ok(
            a,
            "POST",
            &format!("/v1/recon/cases/{}/resolve", c["id"].as_str().unwrap()),
            None,
            json!({ "resolution": "reviewed with provider" }),
        )
        .await;
    }
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_ne!(ex["settlement"]["status"], "exception");
    assert!(
        t.count("SELECT COUNT(*) FROM audit_events WHERE action = 'recon_case.resolved'")
            .await
            > 0
    );
}

// 10 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn large_refunds_need_a_second_persons_approval(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "refund_approval_threshold_minor": 10_000 }),
    )
    .await;
    let (iid, aid, _) = t.paid_razorpay(a, 50_000, json!({})).await;
    let path = format!("/v1/payment_intents/{iid}/refunds");

    let (s, r) = t
        .call_as(
            a,
            "priya",
            "POST",
            &path,
            Some("big-1"),
            json!({ "amount_minor": 20_000 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r["status"], "pending_approval");
    let rid = r["id"].as_str().unwrap();
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved_refund_minor FROM payment_attempts WHERE id = $1")
            .bind(&aid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(
        reserved, 20_000,
        "the amount is held while awaiting approval"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions WHERE reference LIKE 'refund:%'")
            .await,
        0
    );

    // the requester cannot approve their own refund; a key without the scope cannot either
    let (s, e) = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/refunds/{rid}/approve"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("separation_of_duties"))
    );
    let limited = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "support", "scopes": ["refunds:write", "payments:read"] }),
        )
        .await;
    let support = Tenant {
        id: a.id.clone(),
        key: limited["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call_as(
            &support,
            "arjun",
            "POST",
            &format!("/v1/refunds/{rid}/approve"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let (s, done) = t
        .call_as(
            a,
            "arjun",
            "POST",
            &format!("/v1/refunds/{rid}/approve"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, done["status"].as_str(), done["approved_by"].as_str()),
        (StatusCode::OK, Some("processed"), Some("user:arjun"))
    );
    assert_eq!(
        t.call_as(
            a,
            "arjun",
            "POST",
            &format!("/v1/refunds/{rid}/approve"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "cannot approve twice"
    );

    // rejection releases the reservation
    let (_, r2) = t
        .call_as(
            a,
            "priya",
            "POST",
            &path,
            Some("big-2"),
            json!({ "amount_minor": 25_000 }),
        )
        .await;
    let rid2 = r2["id"].as_str().unwrap();
    let rej = t
        .call_as(
            a,
            "arjun",
            "POST",
            &format!("/v1/refunds/{rid2}/reject"),
            None,
            Value::Null,
        )
        .await
        .1;
    assert_eq!(rej["status"], "rejected");
    let (refunded, reserved): (i64, i64) = sqlx::query_as(
        "SELECT refunded_minor, reserved_refund_minor FROM payment_attempts WHERE id = $1",
    )
    .bind(&aid)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((refunded, reserved), (20_000, 0));
    assert!(t.count("SELECT COUNT(*) FROM audit_events WHERE action IN ('refund.requested','refund.approved','refund.rejected')").await >= 4);
}

// 11 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn refund_outcomes_that_are_unknown_stay_reserved_and_failures_go_to_a_queue(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (iid, aid, _) = t.paid_razorpay(a, 50_000, json!({})).await;
    let path = format!("/v1/payment_intents/{iid}/refunds");

    // timeout after the provider accepted the refund
    t.mock_post(
        "behaviour",
        json!({ "op": "razorpay.refund", "behaviour": "hang_after_accept" }),
    )
    .await;
    let r = t
        .ok(
            a,
            "POST",
            &path,
            Some("u1"),
            json!({ "amount_minor": 10_000 }),
        )
        .await;
    assert_eq!(r["status"], "unknown");
    let rid = r["id"].as_str().unwrap();
    let (s, e) = t
        .call(
            a,
            "POST",
            &path,
            Some("u2"),
            json!({ "amount_minor": 45_000 }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("refund_exceeds_refundable")
        ),
        "the unknown refund still counts against the balance"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions WHERE reference LIKE 'refund:%'")
            .await,
        0
    );
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'unresolved_refund' AND status = 'open'"
        )
        .await,
        1
    );

    let done = t
        .ok(
            a,
            "POST",
            &format!("/v1/refunds/{rid}/refresh"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(done["status"], "processed");
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions WHERE reference LIKE 'refund:%'")
            .await,
        1
    );
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'unresolved_refund' AND status = 'open'"
        )
        .await,
        0
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/refunds/{rid}/refresh"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions WHERE reference LIKE 'refund:%'")
            .await,
        1,
        "refreshing again posts nothing"
    );

    // definitive failure: reservation released, visible in the failed-refund queue, retry succeeds
    t.mock_post(
        "behaviour",
        json!({ "op": "razorpay.refund", "behaviour": "reject" }),
    )
    .await;
    let f = t
        .ok(
            a,
            "POST",
            &path,
            Some("f1"),
            json!({ "amount_minor": 5_000 }),
        )
        .await;
    assert_eq!(f["status"], "failed");
    let reserved: i64 =
        sqlx::query_scalar("SELECT reserved_refund_minor FROM payment_attempts WHERE id = $1")
            .bind(&aid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(reserved, 0);
    let queue = t
        .ok(a, "GET", "/v1/refunds?status=failed", None, Value::Null)
        .await;
    assert_eq!(queue["data"].as_array().unwrap().len(), 1);
    let retried = t
        .ok(
            a,
            "POST",
            &format!("/v1/refunds/{}/retry", f["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(retried["status"], "processed");
    assert_eq!(
        t.ok(a, "GET", "/v1/refunds?status=failed", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

// 12 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn routing_respects_capabilities_picks_by_cost_and_records_why(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    set_routing(&t, a, json!({ "rules": [], "fees": { "razorpay": { "card": { "bps": 250, "fixed_minor": 0 } }, "cashfree": { "card": { "bps": 180, "fixed_minor": 0 } } } })).await;

    let i = t
        .ok(
            a,
            "POST",
            "/v1/payment_intents",
            Some("i1"),
            json!({ "amount_minor": 100_000, "currency": "INR" }),
        )
        .await;
    let att = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{}/attempts", i["id"].as_str().unwrap()),
            Some("a1"),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(att["provider"], "cashfree");
    let considered = att["route"]["considered"].as_array().unwrap();
    assert!(
        considered
            .iter()
            .any(|c| c["provider"] == "razorpay" && c["outcome"] == "not_chosen"),
        "{considered:?}"
    );
    assert!(
        att["route"]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("lowest estimated fee")
    );

    // the capability matrix is enforced: only Razorpay supports USD
    let u = t
        .ok(
            a,
            "POST",
            "/v1/payment_intents",
            Some("i2"),
            json!({ "amount_minor": 5_000, "currency": "USD" }),
        )
        .await;
    let att = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{}/attempts", u["id"].as_str().unwrap()),
            Some("a2"),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(att["provider"], "razorpay");
    assert!(
        att["route"]["considered"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["provider"] == "cashfree" && c["reason"].as_str().unwrap().contains("USD"))
    );

    // a rule can prefer a provider, but never one that cannot do the job
    set_routing(
        &t,
        a,
        json!({ "rules": [{ "method": "card", "prefer": ["razorpay"] }], "fees": {} }),
    )
    .await;
    let i3 = t
        .ok(
            a,
            "POST",
            "/v1/payment_intents",
            Some("i3"),
            json!({ "amount_minor": 100_000, "currency": "INR" }),
        )
        .await;
    let att = t
        .ok(
            a,
            "POST",
            &format!(
                "/v1/payment_intents/{}/attempts",
                i3["id"].as_str().unwrap()
            ),
            Some("a3"),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(att["provider"], "razorpay");

    // an attempt on an intent that is already in flight is refused; failed attempts can be retried elsewhere
    let (s, e) = t
        .call(
            a,
            "POST",
            &format!(
                "/v1/payment_intents/{}/attempts",
                i3["id"].as_str().unwrap()
            ),
            Some("a3b"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("attempt_in_progress"))
    );
    let cancelled = t
        .ok(
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
    assert_eq!(cancelled["status"], "failed");
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/payment_intents/{}/attempts",
            i3["id"].as_str().unwrap()
        ),
        Some("a3c"),
        json!({ "method": "upi" }),
    )
    .await;

    // nothing eligible: clear error with reasons
    t.ok(
        a,
        "POST",
        "/v1/provider_accounts/razorpay/enabled",
        None,
        json!({ "enabled": false }),
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
    let i4 = t
        .ok(
            a,
            "POST",
            "/v1/payment_intents",
            Some("i4"),
            json!({ "amount_minor": 1_000, "currency": "INR" }),
        )
        .await;
    let (s, e) = t
        .call(
            a,
            "POST",
            &format!(
                "/v1/payment_intents/{}/attempts",
                i4["id"].as_str().unwrap()
            ),
            Some("a4"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("no_route"))
    );
    assert!(e["error"]["message"].as_str().unwrap().contains("disabled"));
}

// 13 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn api_keys_are_scoped_revocable_and_secrets_are_encrypted_at_rest(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "dashboard (read only)", "scopes": ["payments:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert!(
        ro.key.starts_with("zk_test_"),
        "test keys are visibly test keys"
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/payment_intents", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(
            &ro,
            "POST",
            "/v1/payment_intents",
            Some("x"),
            json!({ "amount_minor": 100, "currency": "INR" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/api_keys", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/export", None, Value::Null).await.0,
        StatusCode::FORBIDDEN
    );
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "bad", "scopes": ["root"] }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_scopes"))
    );

    t.ok(
        a,
        "POST",
        &format!("/v1/api_keys/{}/revoke", k["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        t.call(&ro, "GET", "/v1/payment_intents", None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        raw(
            &t.app,
            "zk_test_garbage",
            None,
            "GET",
            "/v1/payment_intents",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let listed = t
        .ok(a, "GET", "/v1/api_keys", None, Value::Null)
        .await
        .to_string();
    assert!(
        !listed.contains(k["key"].as_str().unwrap()),
        "keys are shown once, then only their prefix"
    );
    let stored: String = sqlx::query_scalar("SELECT key_hash FROM api_keys WHERE id = $1")
        .bind(k["id"].as_str().unwrap())
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_ne!(stored, k["key"].as_str().unwrap());

    // provider secrets: encrypted in the database, never returned by the API
    let blob: Vec<u8> = sqlx::query_scalar("SELECT credentials_enc FROM provider_accounts WHERE tenant_id = $1 AND provider = 'razorpay'").bind(&a.id).fetch_one(&t.root).await.unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("rzp_secret_x"));
    let accounts = t
        .ok(a, "GET", "/v1/provider_accounts", None, Value::Null)
        .await
        .to_string();
    assert!(
        !accounts.contains("rzp_secret_x") && !accounts.contains("secret"),
        "{accounts}"
    );
    assert!(
        accounts.contains("server_initiated_upi"),
        "the capability matrix is exposed"
    );
    // the simulator can never be used in a live environment
    let live = t.root.clone();
    let live_tenant: String =
        sqlx::query_scalar("SELECT id FROM tenants WHERE env = 'live' LIMIT 1")
            .fetch_one(&live)
            .await
            .unwrap();
    let key: (String,) = (zyvor_api::auth::mint_key("live").0,);
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes) VALUES ('k_live', $1, 'live', 'zk_live_xxxxxx', $2, ARRAY['admin'])").bind(&live_tenant).bind(zyvor_api::auth::hash_key(&key.0)).execute(&live).await.unwrap();
    let live_t = Tenant {
        id: live_tenant,
        key: key.0,
    };
    let (s, e) = t
        .call(
            &live_t,
            "POST",
            "/v1/provider_accounts",
            None,
            json!({ "provider": "sim", "credentials": {}, "webhook_secret": "x" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("sim_in_live"))
    );
    // test and live data never mix: the live tenant sees none of the test tenant's payments
    t.paid_razorpay(a, 10_000, json!({})).await;
    assert_eq!(
        t.ok(&live_t, "GET", "/v1/payment_intents", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

// 15 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn effective_cost_is_measured_and_routing_suggestions_cite_evidence(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    set_routing(&t, a, json!({ "rules": [], "fees": { "razorpay": { "upi": { "bps": 200, "fixed_minor": 0 } }, "cashfree": { "upi": { "bps": 150, "fixed_minor": 0 } } } })).await;
    t.paid_razorpay(a, 100_000, json!({})).await;
    t.mock_post(
        "razorpay/settle",
        json!({ "fee_bps": 200, "tax_bps": 1800 }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/settlements/sync",
        None,
        json!({ "provider": "razorpay", "date": today() }),
    )
    .await;
    let cost = t
        .ok(a, "GET", "/v1/analytics/cost", None, Value::Null)
        .await;
    let row = &cost["by_provider_method"][0];
    assert_eq!(
        (row["provider"].as_str(), row["method"].as_str()),
        (Some("razorpay"), Some("upi"))
    );
    assert_eq!(
        row["observed_rate_bps"], 236,
        "2% fee plus 18% GST on the fee"
    );
    assert_eq!(row["configured_rate_bps"], 200);
    let sug = &cost["suggestions"][0];
    assert_eq!(
        (sug["move_from"].as_str(), sug["move_to"].as_str()),
        (Some("razorpay"), Some("cashfree"))
    );
    assert_eq!(sug["estimated_saving_minor"], 860);
    assert_eq!(sug["evidence"]["from_basis"], "observed");
    assert_eq!(sug["evidence"]["to_basis"], "configured");
    assert!(
        sug["action"].as_str().unwrap().contains("suggestion"),
        "advice, never an automatic change"
    );
    // nothing was changed on our behalf
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["routing"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

// 16 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn public_checkout_is_secret_scoped_resumable_and_shows_clear_states(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let intent = t.ok(a, "POST", "/v1/payment_intents", Some("co-1"), json!({ "amount_minor": 49_900, "currency": "INR", "allowed_providers": ["cashfree"], "description": "Term 2 fee" })).await;
    let (iid, secret) = (
        intent["id"].as_str().unwrap(),
        intent["client_secret"].as_str().unwrap(),
    );
    assert!(secret.starts_with("pis_"));
    let stored: Option<String> =
        sqlx::query_scalar("SELECT client_secret_hash FROM payment_intents WHERE id = $1")
            .bind(iid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_ne!(stored.as_deref(), Some(secret), "only a hash is stored");
    assert!(
        t.ok(
            a,
            "GET",
            &format!("/v1/payment_intents/{iid}"),
            None,
            Value::Null
        )
        .await["intent"]
            .get("client_secret")
            .is_none(),
        "the secret is never shown again"
    );

    let url = format!("/v1/checkout/{}/{iid}", a.id);
    // wrong secret, other tenant and unknown intent all look the same
    assert_eq!(
        t.public("GET", &format!("{url}?secret=pis_wrong"), None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        raw(
            &t.app,
            "",
            None,
            "GET",
            &format!("/v1/checkout/{}/{iid}?secret={secret}", t.b.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, view) = t
        .public("GET", &format!("{url}?secret={secret}"), None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (view["phase"].as_str(), view["amount_minor"].as_i64()),
        (Some("choose_method"), Some(49_900))
    );
    assert_eq!(
        view["methods"],
        json!(["card", "netbanking", "upi", "wallet"])
    );
    assert_eq!(view["description"], "Term 2 fee");

    // pay with UPI from the browser, no API key; an Idempotency-Key is still required
    assert_eq!(
        pay_attempt(
            &t,
            &url,
            None,
            json!({ "secret": secret, "method": "upi", "vpa": "asha@okvayora" })
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        pay_attempt(
            &t,
            &url,
            Some("c1"),
            json!({ "secret": "pis_wrong", "method": "upi", "vpa": "asha@okvayora" })
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, bad) = pay_attempt(
        &t,
        &url,
        Some("c1"),
        json!({ "secret": secret, "method": "upi", "vpa": "not a vpa" }),
    )
    .await;
    assert_eq!(
        (s, bad["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_vpa")),
        "an accessible, specific error"
    );
    let (s, att) = pay_attempt(
        &t,
        &url,
        Some("c2"),
        json!({ "secret": secret, "method": "upi", "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(
        (s, att["status"].as_str()),
        (StatusCode::CREATED, Some("pending"))
    );

    // the page can be reloaded (or reopened on another device) and resumes at the right step
    let (_, view) = t
        .public("GET", &format!("{url}?secret={secret}"), None, Value::Null)
        .await;
    assert_eq!(view["phase"], "awaiting_payment");
    assert_eq!(view["attempt"]["action"]["type"], "upi_collect");
    assert!(
        !view.to_string().contains("provider_order_ref") && !view.to_string().contains("route"),
        "no provider internals reach the browser"
    );

    // payer approves; the provider's webhook completes it
    let paid = t
        .mock_post(
            "cashfree/pay",
            json!({ "order_id": att["id"], "outcome": "SUCCESS" }),
        )
        .await;
    assert_eq!(
        t.deliver("cashfree", &a.id, &paid["webhook"]).await.0,
        StatusCode::OK
    );
    let (_, view) = t
        .public("GET", &format!("{url}?secret={secret}"), None, Value::Null)
        .await;
    assert_eq!(
        (view["phase"].as_str(), view["status"].as_str()),
        (Some("paid"), Some("succeeded"))
    );
    // paying again is refused cleanly
    let (s, e) = pay_attempt(
        &t,
        &url,
        Some("c3"),
        json!({ "secret": secret, "method": "upi", "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("attempt_in_progress"))
    );

    // a lost response shows as "confirming", not as an error
    let i2 = t
        .ok(
            a,
            "POST",
            "/v1/payment_intents",
            Some("co-2"),
            json!({ "amount_minor": 20_000, "currency": "INR", "allowed_providers": ["cashfree"] }),
        )
        .await;
    let (iid2, secret2) = (
        i2["id"].as_str().unwrap(),
        i2["client_secret"].as_str().unwrap(),
    );
    t.mock_post(
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let (_, att2) = raw(
        &t.app,
        "",
        None,
        "POST",
        &format!("/v1/checkout/{}/{iid2}/attempts", a.id),
        Some("c4"),
        json!({ "secret": secret2, "method": "upi", "vpa": "ravi@ybl" }),
    )
    .await;
    assert_eq!(att2["status"], "unknown");
    let (_, v2) = raw(
        &t.app,
        "",
        None,
        "GET",
        &format!("/v1/checkout/{}/{iid2}?secret={secret2}", a.id),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(v2["phase"], "confirming");
}

// 17 ──────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn webhook_inspector_and_replay_tools(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.paid_razorpay(a, 30_000, json!({})).await;
    let inbox = t.ok(a, "GET", "/v1/inbox", None, Value::Null).await;
    assert_eq!(inbox["data"][0]["provider"], "razorpay");
    assert!(
        inbox["data"][0]["outcome"]
            .as_str()
            .unwrap()
            .contains("Captured")
    );

    sqlx::query("UPDATE outbox_events SET status = 'delivered', attempts = 1 WHERE type = 'payment.captured'").execute(&t.root).await.unwrap();
    let outbox = t
        .ok(a, "GET", "/v1/outbox?status=delivered", None, Value::Null)
        .await;
    let id = outbox["data"][0]["id"].as_str().unwrap();
    assert_eq!(outbox["data"][0]["type"], "payment.captured");
    let r = t
        .ok(
            a,
            "POST",
            &format!("/v1/outbox/{id}/replay"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(r["status"], "pending");
    assert_eq!(t.count("SELECT COUNT(*) FROM outbox_events WHERE status = 'pending' AND type = 'payment.captured'").await, 1);
    assert_eq!(
        t.call(
            &t.b,
            "POST",
            &format!("/v1/outbox/{id}/replay"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "another tenant cannot replay it"
    );
}

async fn pay_attempt(t: &T, url: &str, key: Option<&str>, body: Value) -> (StatusCode, Value) {
    t.public("POST", &format!("{url}/attempts"), key, body)
        .await
}

// docs ────────────────────────────────────────────────────────────────────────
#[test]
fn openapi_documents_every_route() {
    let lib = include_str!("../src/lib.rs");
    let spec = include_str!("../../../openapi/zyvor.yaml");
    let mut missing = Vec::new();
    for line in lib.lines().filter(|l| l.contains(".route(\"/")) {
        let path = line.split('"').nth(1).unwrap();
        if matches!(path, "/healthz" | "/readyz" | "/metrics") {
            continue;
        }
        if !spec.contains(&format!("/v1{path}:")) {
            missing.push(path.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "routes missing from openapi/zyvor.yaml: {missing:?}"
    );
}

// Collections ────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn collections_ageing_promises_and_reminders(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let today = chrono::Utc::now().date_naive();
    let mk = |n: &str, amt: i64, due: chrono::NaiveDate| json!({ "number": n, "amount_minor": amt, "due_date": due.to_string() });
    let cur = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("C-1", 1000, today + chrono::Duration::days(5)),
        )
        .await;
    let late = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("L-1", 5000, today - chrono::Duration::days(45)),
        )
        .await;
    let old = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("O-1", 7000, today - chrono::Duration::days(120)),
        )
        .await;
    t.ok(
        b,
        "POST",
        "/v1/invoices",
        None,
        mk("B-1", 9999, today - chrono::Duration::days(200)),
    )
    .await;

    let report = t
        .ok(a, "GET", "/v1/collections/ageing", None, Value::Null)
        .await;
    let by: std::collections::HashMap<_, _> = report["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["bucket"].as_str().unwrap().to_string(),
                r["outstanding_minor"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(by.get("current"), Some(&1000));
    assert_eq!(by.get("31_60"), Some(&5000));
    assert_eq!(by.get("over_90"), Some(&7000));
    assert_eq!(
        by.len(),
        3,
        "another tenant's invoice must not appear: {by:?}"
    );

    // a promise cannot exceed what is owed, and is hidden from other tenants
    let late_id = late["id"].as_str().unwrap();
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/invoices/{late_id}/promises"),
            None,
            json!({ "promised_minor": 5001, "promised_date": today.to_string() }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    let p = t.ok(a, "POST", &format!("/v1/invoices/{late_id}/promises"), None, json!({ "promised_minor": 5000, "promised_date": (today + chrono::Duration::days(3)).to_string(), "note": "dad will pay Friday" })).await;
    let past = t
        .ok(
            a,
            "POST",
            &format!("/v1/invoices/{}/promises", old["id"].as_str().unwrap()),
            None,
            json!({ "promised_minor": 7000, "promised_date": today.to_string() }),
        )
        .await;
    let _ = past;
    sqlx::query(
        "UPDATE payment_promises SET promised_date = promised_date - 5 WHERE invoice_id = $1",
    )
    .bind(old["id"].as_str().unwrap())
    .execute(&t.root)
    .await
    .unwrap();
    let list = t
        .ok(a, "GET", "/v1/collections/promises", None, Value::Null)
        .await;
    let st: std::collections::HashMap<_, _> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["invoice_number"].as_str().unwrap().to_string(),
                r["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(st["L-1"], "open");
    assert_eq!(st["O-1"], "broken");
    assert!(
        t.ok(b, "GET", "/v1/collections/promises", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/collections/promises/{}/cancel",
            p["id"].as_str().unwrap()
        ),
        None,
        json!({}),
    )
    .await;

    // reminders are logged and emitted for the merchant's own channel to send
    let cur_id = cur["id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{cur_id}/reminders"),
        None,
        json!({ "channel": "whatsapp", "message": "Fee due Friday" }),
    )
    .await;
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/invoices/{cur_id}/reminders"),
            None,
            json!({ "channel": "pigeon" }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        1
    );
}

// Billing ────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn subscriptions_bill_once_per_period_with_usage_credits_void_and_statements(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let today = chrono::Utc::now().date_naive();
    let cust = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "Ravi" }))
        .await;
    let cid = cust["id"].as_str().unwrap();
    let plan = t.ok(a, "POST", "/v1/plans", None, json!({ "name": "Pro", "amount_minor": 99_900, "interval_unit": "month", "usage_unit_minor": 100 })).await;
    let (st, _) = t
        .call(
            a,
            "POST",
            "/v1/plans",
            None,
            json!({ "name": "x", "amount_minor": 1, "interval_unit": "fortnight" }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    // started 2 months + a bit ago: 3 periods are due, and re-running must not duplicate any
    let start = today.checked_sub_months(chrono::Months::new(2)).unwrap();
    let sub = t.ok(a, "POST", "/v1/subscriptions", None, json!({ "customer_id": cid, "plan_id": plan["id"], "quantity": 2, "start_date": start.to_string() })).await;
    let sid = sub["id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{sid}/usage"),
        None,
        json!({ "quantity": 30, "reference": "u1" }),
    )
    .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{sid}/usage"),
        None,
        json!({ "quantity": 30, "reference": "u1" }),
    )
    .await; // retry
    let run = t.ok(a, "POST", "/v1/billing/run", None, json!({})).await;
    assert_eq!(run["invoices_created"], 3);
    assert_eq!(
        t.ok(a, "POST", "/v1/billing/run", None, json!({})).await["invoices_created"],
        0
    );
    let invoices = t.ok(a, "GET", "/v1/invoices", None, Value::Null).await;
    let mut amounts: Vec<i64> = invoices["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["amount_minor"].as_i64().unwrap())
        .collect();
    amounts.sort();
    // the retried usage counted once (30 * 100), landing on the first period's invoice
    assert_eq!(amounts, vec![199_800, 199_800, 202_800]);
    assert_eq!(
        t.ok(b, "GET", "/v1/invoices", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // credit note reduces what is owed and cannot over-credit
    let inv = invoices["data"][0]["id"].as_str().unwrap().to_string();
    let amt = invoices["data"][0]["amount_minor"].as_i64().unwrap();
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/invoices/{inv}/credit_notes"),
            None,
            json!({ "amount_minor": amt + 1, "reason": "x" }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{inv}/credit_notes"),
        None,
        json!({ "amount_minor": 50_000, "reason": "goodwill" }),
    )
    .await;
    let got = t
        .ok(a, "GET", &format!("/v1/invoices/{inv}"), None, Value::Null)
        .await;
    assert_eq!(got["credited_minor"], 50_000);
    assert_eq!(got["outstanding_minor"].as_i64().unwrap(), amt - 50_000);
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{inv}/credit_notes"),
        None,
        json!({ "amount_minor": amt - 50_000, "reason": "waived" }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", &format!("/v1/invoices/{inv}"), None, Value::Null)
            .await["status"],
        "paid"
    );
    // credited invoices cannot be voided; untouched ones can
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/invoices/{inv}/void"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
    let other = invoices["data"][1]["id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{other}/void"),
        None,
        json!({}),
    )
    .await;

    // statement: invoices minus credits, with a running balance; other tenants can't read it
    let stmt = t
        .ok(
            a,
            "GET",
            &format!("/v1/customers/{cid}/statement"),
            None,
            Value::Null,
        )
        .await;
    let want =
        amounts.iter().sum::<i64>() - invoices["data"][1]["amount_minor"].as_i64().unwrap() - amt;
    assert_eq!(stmt["balance_minor"]["INR"].as_i64().unwrap(), want);
    let (st, _) = t
        .call(
            b,
            "GET",
            &format!("/v1/customers/{cid}/statement"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(st.as_u16(), 404);

    // lifecycle: paused subscriptions are not billed, canceled ones cannot be resumed
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{sid}/pause"),
        None,
        json!({}),
    )
    .await;
    sqlx::query("UPDATE subscriptions SET next_bill_date = CURRENT_DATE")
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(
        t.ok(a, "POST", "/v1/billing/run", None, json!({})).await["invoices_created"],
        0
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{sid}/cancel"),
        None,
        json!({}),
    )
    .await;
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/subscriptions/{sid}/resume"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
}

#[sqlx::test(migrations = "./migrations")]
async fn upgrades_are_prorated_and_milestones_invoice_exactly_once(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let today = chrono::Utc::now().date_naive();
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Factory Co" }),
        )
        .await;
    let cid = cust["id"].as_str().unwrap();
    let basic = t.ok(a, "POST", "/v1/plans", None, json!({ "name": "Basic", "amount_minor": 30_000, "interval_unit": "day", "interval_count": 30 })).await;
    let pro = t.ok(a, "POST", "/v1/plans", None, json!({ "name": "Pro", "amount_minor": 60_000, "interval_unit": "day", "interval_count": 30 })).await;
    // 30-day period with 10 days left: upgrade owes (60000-30000) * 10/30
    let sub = t.ok(a, "POST", "/v1/subscriptions", None, json!({ "customer_id": cid, "plan_id": basic["id"], "start_date": (today + chrono::Duration::days(10)).to_string() })).await;
    let sid = sub["id"].as_str().unwrap();
    let up = t
        .ok(
            a,
            "POST",
            &format!("/v1/subscriptions/{sid}/change_plan"),
            None,
            json!({ "plan_id": pro["id"] }),
        )
        .await;
    assert_eq!(up["proration_minor"], 10_000);
    let inv = up["proration_invoice_id"].as_str().unwrap();
    assert_eq!(
        t.ok(a, "GET", &format!("/v1/invoices/{inv}"), None, Value::Null)
            .await["amount_minor"],
        10_000
    );
    // downgrade is free now; the lower price applies from the next invoice
    let down = t
        .ok(
            a,
            "POST",
            &format!("/v1/subscriptions/{sid}/change_plan"),
            None,
            json!({ "plan_id": basic["id"] }),
        )
        .await;
    assert_eq!(down["proration_minor"], 0);
    assert!(down["proration_invoice_id"].is_null());
    let (st, _) = t
        .call(
            b,
            "POST",
            &format!("/v1/subscriptions/{sid}/change_plan"),
            None,
            json!({ "plan_id": pro["id"] }),
        )
        .await;
    assert_eq!(st.as_u16(), 404);

    // milestones: completing twice, even concurrently, yields one invoice
    let ms = t
        .ok(
            a,
            "POST",
            "/v1/milestones",
            None,
            json!({ "customer_id": cid, "name": "Machine delivered", "amount_minor": 500_000 }),
        )
        .await;
    let mid = ms["id"].as_str().unwrap().to_string();
    let path = format!("/v1/milestones/{mid}/complete");
    let (r1, r2) = tokio::join!(
        t.call(a, "POST", &path, None, json!({})),
        t.call(a, "POST", &path, None, json!({}))
    );
    assert!(r1.0.is_success() && r2.0.is_success());
    assert_eq!(r1.1["invoice_id"], r2.1["invoice_id"]);
    assert_eq!(
        t.count("SELECT count(*) FROM invoices WHERE number LIKE 'MS-%'")
            .await,
        1
    );
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/milestones/{mid}/cancel"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
    let (st, _) = t.call(b, "POST", &path, None, json!({})).await;
    assert_eq!(st.as_u16(), 404);
}

#[sqlx::test(migrations = "./migrations")]
async fn requests_get_an_id_that_is_echoed_and_hostile_ids_are_replaced(pool: PgPool) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let t = setup(pool).await;
    let send = |id: Option<&str>| {
        let mut r = Request::builder()
            .uri("/v1/plans")
            .header("authorization", format!("Bearer {}", t.a.key));
        if let Some(i) = id {
            r = r.header("x-request-id", i);
        }
        t.app.clone().oneshot(r.body(Body::empty()).unwrap())
    };
    let generated = send(None).await.unwrap();
    assert!(
        generated.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .starts_with("req_")
    );
    assert_eq!(
        send(Some("ticket-4821")).await.unwrap().headers()["x-request-id"],
        "ticket-4821"
    );
    let hostile = send(Some("a b\"<script>")).await.unwrap();
    assert!(
        hostile.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .starts_with("req_")
    );
}

// Disputes ───────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn disputes_hold_the_money_and_a_lost_one_hits_the_ledger_and_reopens_the_invoice(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "D-1", "amount_minor": 10_000 }),
        )
        .await;
    let inv_id = inv["id"].as_str().unwrap();
    let (iid, att, _) = t
        .paid_razorpay(a, 10_000, json!({ "invoice_id": inv_id }))
        .await;
    assert_eq!(
        t.ok(
            a,
            "GET",
            &format!("/v1/invoices/{inv_id}"),
            None,
            Value::Null
        )
        .await["status"],
        "paid"
    );

    // opening a dispute holds the money: a refund of the disputed part is refused, the rest is allowed
    let d = t.ok(a, "POST", "/v1/disputes", None, json!({ "attempt_id": att, "amount_minor": 6_000, "reason": "fraudulent", "respond_by": "2026-12-01" })).await;
    let did = d["id"].as_str().unwrap().to_string();
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("r-too-much"),
            json!({ "amount_minor": 5_000 }),
        )
        .await;
    assert_eq!(st.as_u16(), 422, "only 4000 is still free");
    // a second dispute on the same payment, or one by another tenant, is refused
    let (st, _) = t
        .call(
            a,
            "POST",
            "/v1/disputes",
            None,
            json!({ "attempt_id": att, "amount_minor": 1_000, "reason": "dup" }),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
    let (st, _) = t
        .call(
            b,
            "POST",
            "/v1/disputes",
            None,
            json!({ "attempt_id": att, "reason": "x" }),
        )
        .await;
    assert_eq!(st.as_u16(), 404);
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/disputes/{did}/evidence"),
            None,
            json!({ "note": "  " }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    t.ok(
        a,
        "POST",
        &format!("/v1/disputes/{did}/evidence"),
        None,
        json!({ "note": "Delivery receipt signed", "url": "https://files.example/receipt.pdf" }),
    )
    .await;
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/disputes?status=under_review",
            None,
            Value::Null
        )
        .await["data"][0]["evidence_count"],
        1
    );

    // lost: 6000 leaves the merchant, the ledger stays balanced, the invoice reopens for what was clawed back
    let before = t.count("SELECT count(*) FROM ledger_transactions").await;
    t.ok(
        a,
        "POST",
        &format!("/v1/disputes/{did}/resolve"),
        None,
        json!({ "outcome": "lost" }),
    )
    .await;
    assert_eq!(
        t.count("SELECT count(*) FROM ledger_transactions").await,
        before + 1
    );
    let (dr, cr): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries").fetch_one(&t.root).await.unwrap();
    assert_eq!(dr, cr);
    assert_eq!(t.count("SELECT COALESCE(SUM(amount_minor),0)::BIGINT FROM ledger_entries WHERE account = 'settlement_adjustments' AND side = 'debit'").await, 6_000);
    let got = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{inv_id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            got["status"].as_str().unwrap(),
            got["outstanding_minor"].as_i64().unwrap()
        ),
        ("partially_paid", 6_000)
    );
    // resolving again changes nothing
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/disputes/{did}/resolve"),
            None,
            json!({ "outcome": "won" }),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
    assert_eq!(
        t.count("SELECT count(*) FROM ledger_transactions").await,
        before + 1
    );

    // won: the hold is released and the remaining 4000 can be refunded
    let (_, att2, _) = t.paid_razorpay(a, 5_000, json!({})).await;
    let d2 = t
        .ok(
            a,
            "POST",
            "/v1/disputes",
            None,
            json!({ "attempt_id": att2, "reason": "not recognised" }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/disputes/{}/resolve", d2["id"].as_str().unwrap()),
        None,
        json!({ "outcome": "won" }),
    )
    .await;
    let after: i64 = t
        .count(&format!(
            "SELECT reserved_refund_minor FROM payment_attempts WHERE id = '{att2}'"
        ))
        .await;
    assert_eq!(after, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn customers_are_found_by_your_own_reference_and_deposits_appear_on_the_statement(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let pat = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "R. Menon", "external_ref": "PAT-7731" }),
        )
        .await;
    t.ok(
        a,
        "POST",
        "/v1/customers",
        None,
        json!({ "name": "Someone else", "external_ref": "PAT-1" }),
    )
    .await;
    t.ok(
        b,
        "POST",
        "/v1/customers",
        None,
        json!({ "name": "Other tenant", "external_ref": "PAT-7731" }),
    )
    .await;
    let found = t
        .ok(
            a,
            "GET",
            "/v1/customers?external_ref=PAT-7731",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        found["data"].as_array().unwrap().len(),
        1,
        "only this tenant's match"
    );
    assert_eq!(found["data"][0]["id"], pat["id"]);
    let cid = pat["id"].as_str().unwrap();

    // an admission deposit with no invoice: a credit on the statement, and refunding it restores the balance
    let (iid, _, _) = t
        .paid_razorpay(
            a,
            25_000,
            json!({ "customer_id": cid, "external_ref": "DEP-ADM-1" }),
        )
        .await;
    let stmt = t
        .ok(
            a,
            "GET",
            &format!("/v1/customers/{cid}/statement"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(stmt["balance_minor"]["INR"], -25_000);
    assert_eq!(stmt["lines"][0]["type"], "deposit");
    let r = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("dep-refund"),
            json!({ "amount_minor": 10_000, "external_ref": "DEP-RET-1" }),
        )
        .await;
    if r["status"] == "processed" {
        let stmt = t
            .ok(
                a,
                "GET",
                &format!("/v1/customers/{cid}/statement"),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(stmt["balance_minor"]["INR"], -15_000);
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn dunning_reminds_overdue_invoices_once_per_window_and_respects_promises(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let today = chrono::Utc::now().date_naive();
    let mk = |n: &str, due: chrono::NaiveDate| json!({ "number": n, "amount_minor": 5000, "due_date": due.to_string() });
    let late = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("D-LATE", today - chrono::Duration::days(10)),
        )
        .await;
    let promised = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("D-PROM", today - chrono::Duration::days(10)),
        )
        .await;
    let _current = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            mk("D-CUR", today + chrono::Duration::days(3)),
        )
        .await;
    t.ok(a, "POST", &format!("/v1/invoices/{}/promises", promised["id"].as_str().unwrap()), None, json!({ "promised_minor": 5000, "promised_date": (today + chrono::Duration::days(2)).to_string() })).await;

    let r = t
        .ok(
            a,
            "POST",
            "/v1/collections/dunning/run",
            None,
            json!({ "channel": "sms" }),
        )
        .await;
    assert_eq!(
        r["reminded"], 1,
        "only the overdue invoice with no live promise"
    );
    // a second run inside the repeat window adds nothing
    let r = t
        .ok(a, "POST", "/v1/collections/dunning/run", None, json!({}))
        .await;
    assert_eq!(r["reminded"], 0);
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        1
    );

    let hist = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{}/reminders", late["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    let rows = hist["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0]["channel"].as_str(), rows[0]["message"].as_str()),
        (Some("sms"), Some("10 days overdue"))
    );

    // once the window has passed the invoice is chased again
    sqlx::query("UPDATE invoice_reminders SET created_at = created_at - interval '8 days'")
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(
        t.ok(a, "POST", "/v1/collections/dunning/run", None, json!({}))
            .await["reminded"],
        1
    );
    let (st, _) = t
        .call(
            a,
            "POST",
            "/v1/collections/dunning/run",
            None,
            json!({ "channel": "pigeon" }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_worker_dunning_is_opt_in_per_tenant_and_repeat_safe(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let due = (chrono::Utc::now().date_naive() - chrono::Duration::days(12)).to_string();
    for who in [a, b] {
        t.ok(
            who,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "W-1", "amount_minor": 900, "due_date": due }),
        )
        .await;
    }
    let client = zyvor_api::outbox::http_client();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(
        t.count("SELECT count(*) FROM invoice_reminders").await,
        0,
        "off by default"
    );

    let (st, _) = t
        .call(
            a,
            "POST",
            "/v1/settings",
            None,
            json!({ "dunning": { "enabled": true, "channel": "pigeon" } }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "dunning": { "enabled": true, "channel": "whatsapp", "repeat_after_days": 3 } }),
    )
    .await;
    let s = t.ok(a, "GET", "/v1/settings", None, Value::Null).await;
    assert_eq!(
        (
            s["dunning"]["enabled"].clone(),
            s["dunning"]["channel"].clone()
        ),
        (json!(true), json!("whatsapp"))
    );

    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    assert_eq!(
        t.count("SELECT count(*) FROM invoice_reminders").await,
        1,
        "only the opted-in tenant, once"
    );
    assert_eq!(t.count("SELECT count(*) FROM invoice_reminders WHERE sent_by = 'dunning' AND channel = 'whatsapp'").await, 1);
}

/// A GET with an optional `X-Forwarded-For`, returning the status and `Retry-After`.
async fn probe(app: &axum::Router, key: &str, xff: Option<&str>) -> (u16, Option<String>) {
    use tower::ServiceExt;
    let mut r = axum::http::Request::builder()
        .uri("/v1/settings")
        .header("authorization", format!("Bearer {key}"));
    if let Some(x) = xff {
        r = r.header("x-forwarded-for", x);
    }
    let res = app
        .clone()
        .oneshot(r.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    (
        res.status().as_u16(),
        res.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(String::from),
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn api_keys_can_expire_be_pinned_to_addresses_and_rotate_with_an_overlap(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let mut st = t.state.clone();
    st.trusted_hops = 1;
    let app = zyvor_api::app(st);

    // validation
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "x", "scopes": ["payments:read"], "expires_at": past }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "x", "scopes": ["payments:read"], "allowed_cidrs": ["10.0.0.0/40"] }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);

    // address allow-list: only the trusted proxy's view of the caller counts
    let pinned = t.ok(a, "POST", "/v1/api_keys", None, json!({ "name": "office", "scopes": ["payments:read"], "allowed_cidrs": ["203.0.113.0/24"] })).await;
    let k = pinned["key"].as_str().unwrap();
    assert_eq!(probe(&app, k, Some("203.0.113.5")).await.0, 200);
    assert_eq!(probe(&app, k, Some("198.51.100.1")).await.0, 403);
    assert_eq!(
        probe(&app, k, Some("198.51.100.1, 203.0.113.5")).await.0,
        200,
        "the client-supplied first hop is ignored"
    );
    assert_eq!(
        probe(&app, k, Some("203.0.113.5, 198.51.100.1")).await.0,
        403,
        "spoofing an allowed address first does not work"
    );
    assert_eq!(
        probe(&app, k, None).await.0,
        403,
        "unknown address is not allowed"
    );

    // expiry is enforced
    let short = t.ok(a, "POST", "/v1/api_keys", None, json!({ "name": "short", "scopes": ["payments:read"], "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339() })).await;
    let sk = short["key"].as_str().unwrap();
    assert_eq!(probe(&app, sk, None).await.0, 200);
    sqlx::query("UPDATE api_keys SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(short["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(probe(&app, sk, None).await.0, 401);

    // rotation: both keys work during the overlap, the old one then lapses, the new one inherits scopes and allow-list
    let rot = t
        .ok(
            a,
            "POST",
            &format!("/v1/api_keys/{}/rotate", pinned["id"].as_str().unwrap()),
            None,
            json!({ "overlap_hours": 2 }),
        )
        .await;
    let nk = rot["key"].as_str().unwrap();
    assert_ne!(nk, k);
    assert_eq!(probe(&app, k, Some("203.0.113.5")).await.0, 200);
    assert_eq!(probe(&app, nk, Some("203.0.113.5")).await.0, 200);
    assert_eq!(
        probe(&app, nk, Some("198.51.100.1")).await.0,
        403,
        "allow-list carried over"
    );
    sqlx::query("UPDATE api_keys SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(pinned["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(probe(&app, k, Some("203.0.113.5")).await.0, 401);
    // an expired key cannot be rotated into a fresh one
    let (s, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/api_keys/{}/rotate", pinned["id"].as_str().unwrap()),
            None,
            json!({}),
        )
        .await;
    assert_eq!(s.as_u16(), 404);

    // last use is recorded; listing shows the controls
    let list = t.ok(a, "GET", "/v1/api_keys", None, Value::Null).await;
    let row = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == rot["id"])
        .unwrap();
    assert!(!row["last_used_at"].is_null());
    assert_eq!(row["allowed_cidrs"], json!(["203.0.113.0/24"]));
    assert!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'api_key.rotated'")
            .await
            == 1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn rate_limits_apply_per_key_and_per_address_and_say_when_to_retry(pool: PgPool) {
    use zyvor_api::ratelimit::{Bucket, Limits, RateLimiter};
    let t = setup(pool).await;
    let mut st = t.state.clone();
    st.trusted_hops = 1;
    st.limiter = std::sync::Arc::new(RateLimiter::new(Limits {
        key: Bucket {
            burst: 3.0,
            per_sec: 0.5,
        },
        ip_authed: Bucket {
            burst: 100.0,
            per_sec: 100.0,
        },
        ip_public: Bucket {
            burst: 2.0,
            per_sec: 0.5,
        },
    }));
    let app = zyvor_api::app(st);
    let (ka, kb) = (t.a.key.as_str(), t.b.key.as_str());
    for _ in 0..3 {
        assert_eq!(probe(&app, ka, Some("203.0.113.1")).await.0, 200);
    }
    let (s, retry) = probe(&app, ka, Some("203.0.113.1")).await;
    assert_eq!(s, 429);
    assert!(retry.unwrap().parse::<u64>().unwrap() >= 1);
    assert_eq!(
        probe(&app, kb, Some("203.0.113.1")).await.0,
        200,
        "another tenant's key is unaffected"
    );

    // callers with no key share a small per-address budget; other addresses are separate
    use tower::ServiceExt;
    let public = |ip: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                axum::http::Request::builder()
                    .uri("/v1/checkout/x/y")
                    .header("x-forwarded-for", ip)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
            .as_u16()
        }
    };
    assert_ne!(public("198.51.100.7").await, 429);
    assert_ne!(public("198.51.100.7").await, 429);
    assert_eq!(public("198.51.100.7").await, 429);
    assert_ne!(public("198.51.100.8").await, 429);

    // bogus keys cannot dodge the address bucket by changing the key each time
    let mut st2 = t.state.clone();
    st2.trusted_hops = 1;
    st2.limiter = std::sync::Arc::new(RateLimiter::new(Limits {
        key: Bucket {
            burst: 100.0,
            per_sec: 100.0,
        },
        ip_authed: Bucket {
            burst: 4.0,
            per_sec: 0.1,
        },
        ip_public: Bucket {
            burst: 2.0,
            per_sec: 0.5,
        },
    }));
    let app2 = zyvor_api::app(st2);
    let mut codes = Vec::new();
    for i in 0..6 {
        codes.push(
            probe(&app2, &format!("zk_test_bogus{i}"), Some("192.0.2.9"))
                .await
                .0,
        );
    }
    assert_eq!(codes[..4], [401, 401, 401, 401]);
    assert_eq!(codes[4], 429);
}

#[sqlx::test(migrations = "./migrations")]
async fn payment_links_mint_a_fresh_intent_per_payer_and_respect_caps_expiry_and_cancel(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let today = chrono::Utc::now().date_naive();

    // validation: exactly one of amount / invoice
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "description": "x" }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100, "invoice_id": "inv_x" }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100, "expires_at": "2020-01-01T00:00:00Z" }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);

    // a fixed-amount link capped at 2 uses
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "description": "Term 2 fee", "amount_minor": 125_000, "max_uses": 2 }),
        )
        .await;
    let (tid, token) = (&a.id, link["token"].as_str().unwrap());
    assert_eq!(link["path"], json!(format!("/pay/link/{tid}/{token}")));
    let info = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/links/{tid}/{token}"),
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            info["amount_minor"].clone(),
            info["status"].clone(),
            info["description"].clone()
        ),
        (json!(125_000), json!("active"), json!("Term 2 fee"))
    );

    let s1 = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{tid}/{token}/start"),
            json!({}),
        )
        .await;
    let s2 = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{tid}/{token}/start"),
            json!({}),
        )
        .await;
    assert_ne!(
        s1["intent_id"], s2["intent_id"],
        "each payer gets their own intent"
    );
    assert_ne!(s1["client_secret"], s2["client_secret"]);
    let (st, body) = t
        .public(
            "POST",
            &format!("/v1/checkout/links/{tid}/{token}/start"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(
        (st.as_u16(), body["error"]["code"].clone()),
        (409, json!("link_unavailable"))
    );
    // the minted intent is a normal checkout session, payable with its secret
    let sess = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/{tid}/{}?secret={}",
                s1["intent_id"].as_str().unwrap(),
                s1["client_secret"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(
        (sess["amount_minor"].clone(), sess["phase"].clone()),
        (json!(125_000), json!("choose_method"))
    );

    // another tenant cannot see or use this link
    let (st, _) = t
        .public(
            "GET",
            &format!("/v1/checkout/links/{}/{token}", b.id),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(st.as_u16(), 404);
    assert!(
        t.ok(b, "GET", "/v1/payment_links", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let listed = t.ok(a, "GET", "/v1/payment_links", None, Value::Null).await;
    assert_eq!(
        (
            listed["data"][0]["uses"].clone(),
            listed["data"][0]["status"].clone()
        ),
        (json!(2), json!("used_up"))
    );

    // an invoice link collects what is still owed, and stops once it is paid
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "PL-1", "amount_minor": 9000, "due_date": today.to_string() }),
        )
        .await;
    let il = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "invoice_id": inv["id"] }),
        )
        .await;
    let itoken = il["token"].as_str().unwrap();
    let start = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{tid}/{itoken}/start"),
            json!({}),
        )
        .await;
    assert_eq!(start["amount_minor"], 9000);
    let pi = t
        .ok(
            a,
            "GET",
            &format!(
                "/v1/payment_intents/{}",
                start["intent_id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(pi["intent"]["invoice_id"], inv["id"]);
    // fully credited (a waived fee): nothing left to pay, so the link stops
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/credit_notes", inv["id"].as_str().unwrap()),
        None,
        json!({ "amount_minor": 9000, "reason": "waived" }),
    )
    .await;
    let (st, _) = t
        .public(
            "POST",
            &format!("/v1/checkout/links/{tid}/{itoken}/start"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 409);

    // expiry and cancel
    let exp = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 500, "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339() })).await;
    sqlx::query("UPDATE payment_links SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(exp["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    let info = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/links/{tid}/{}",
                exp["token"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(info["status"], "expired");
    let c = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 500 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_links/{}/cancel", c["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    let (st, _) = t
        .public(
            "POST",
            &format!(
                "/v1/checkout/links/{tid}/{}/start",
                c["token"].as_str().unwrap()
            ),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 409);
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_links/{}/cancel", c["id"].as_str().unwrap()),
            None,
            json!({}),
        )
        .await;
    assert_eq!(st.as_u16(), 404, "already canceled");
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_urls_cannot_target_private_networks_when_saved_or_delivered(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let mut st = t.state.clone();
    st.allow_private_targets = false; // production posture
    let app = zyvor_api::app(st.clone());
    let set = |url: &'static str| {
        let (app, key) = (app.clone(), a.key.clone());
        async move {
            raw(
                &app,
                &key,
                None,
                "POST",
                "/v1/settings",
                None,
                json!({ "outbound_webhook_url": url, "outbound_webhook_secret": "whsec_x" }),
            )
            .await
        }
    };
    for bad in [
        "http://hooks.example.com/z",
        "https://169.254.169.254/latest",
        "https://127.0.0.1:9/z",
        "https://internal-db/z",
        "https://u:p@hooks.example.com/z",
    ] {
        let (s, body) = set(bad).await;
        assert_eq!(
            (s.as_u16(), body["error"]["code"].clone()),
            (422, json!("invalid_webhook_url")),
            "{bad}"
        );
    }
    assert_eq!(set("https://hooks.example.com/zyvor").await.0.as_u16(), 200);

    // a URL that was fine when saved but points inward at delivery time is not called
    t.ok(
        a,
        "POST",
        "/v1/invoices",
        None,
        json!({ "number": "S-1", "amount_minor": 100 }),
    )
    .await; // emits an event
    sqlx::query("UPDATE tenant_settings SET outbound_webhook_url = $2 WHERE tenant_id = $1")
        .bind(&a.id)
        .bind("https://127.0.0.1:9/hook")
        .execute(&t.root)
        .await
        .unwrap();
    let pending = t
        .count("SELECT count(*) FROM outbox_events WHERE status = 'pending'")
        .await;
    let n = zyvor_api::outbox::deliver_tenant(
        &st.pool,
        &st.crypto,
        &zyvor_api::outbox::http_client(),
        &a.id,
        false,
    )
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE status = 'pending'")
            .await,
        pending,
        "refused, not failed: events wait for a valid URL"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE attempts > 0")
            .await,
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_audit_trail_is_sealed_in_a_chain_that_shows_any_edit_or_deletion(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let ev = |n: &str| json!({ "name": n, "scopes": ["payments:read"] });
    for n in ["one", "two", "three"] {
        t.ok(a, "POST", "/v1/api_keys", None, ev(n)).await;
    }
    t.ok(b, "POST", "/v1/api_keys", None, ev("other-tenant"))
        .await;

    // nothing sealed yet; very fresh events are left alone by the default delay
    let v = t
        .ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
        .await;
    assert_eq!(
        (v["ok"].clone(), v["seals_checked"].clone()),
        (json!(true), json!(0))
    );
    assert_eq!(
        t.ok(a, "POST", "/v1/audit_events/seal", None, json!({}))
            .await["sealed_events"],
        0
    );

    let first = t
        .ok(
            a,
            "POST",
            "/v1/audit_events/seal?min_age_seconds=0",
            None,
            json!({}),
        )
        .await["sealed_events"]
        .as_i64()
        .unwrap();
    assert!(first >= 3);
    t.ok(a, "POST", "/v1/api_keys", None, ev("four")).await;
    let v = t
        .ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
        .await;
    assert_eq!(
        (
            v["ok"].clone(),
            v["sealed_events"].as_i64().unwrap(),
            v["unsealed_events"].as_i64().unwrap() >= 1
        ),
        (json!(true), first, true)
    );
    assert!(
        t.ok(
            a,
            "POST",
            "/v1/audit_events/seal?min_age_seconds=0",
            None,
            json!({})
        )
        .await["sealed_events"]
            .as_i64()
            .unwrap()
            >= 1
    );
    let v = t
        .ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
        .await;
    assert_eq!(
        (
            v["ok"].clone(),
            v["seals_checked"].clone(),
            v["unsealed_events"].clone()
        ),
        (json!(true), json!(2), json!(0))
    );

    // the application role cannot rewrite history or its seals
    for sql in [
        "UPDATE audit_events SET actor = 'x'",
        "DELETE FROM audit_events",
        "UPDATE audit_seals SET hash = 'x'",
        "DELETE FROM audit_seals",
    ] {
        let mut tx = zyvor_api::tenant::begin(&t.state.pool, &a.id)
            .await
            .unwrap();
        assert!(
            sqlx::query(sql).execute(&mut *tx).await.is_err(),
            "{sql} must be refused"
        );
    }

    // someone with superuser power bypasses the triggers and edits one event: the chain shows it
    let victim: i64 = sqlx::query_scalar("SELECT min(id) FROM audit_events WHERE tenant_id = $1")
        .bind(&a.id)
        .fetch_one(&t.root)
        .await
        .unwrap();
    let tamper = |sql: String| {
        let root = t.root.clone();
        async move {
            let mut c = root.begin().await.unwrap();
            sqlx::query("ALTER TABLE audit_events DISABLE TRIGGER audit_events_immutable")
                .execute(&mut *c)
                .await
                .unwrap();
            sqlx::query(sqlx::AssertSqlSafe(sql))
                .execute(&mut *c)
                .await
                .unwrap();
            sqlx::query("ALTER TABLE audit_events ENABLE TRIGGER audit_events_immutable")
                .execute(&mut *c)
                .await
                .unwrap();
            c.commit().await.unwrap();
        }
    };
    tamper(format!(
        "UPDATE audit_events SET actor = 'someone-else' WHERE id = {victim}"
    ))
    .await;
    let v = t
        .ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
        .await;
    assert_eq!(v["ok"], false);
    assert_eq!(v["reason"], "an event was changed after it was sealed");
    assert!(
        v["events_range"][0].as_i64().unwrap() <= victim
            && victim <= v["events_range"][1].as_i64().unwrap()
    );
    // putting it back makes the chain valid again: the check is about content, not a flag
    let original: String = sqlx::query_scalar(
        "SELECT 'key:' || id FROM api_keys WHERE tenant_id = $1 AND name = 'bootstrap admin'",
    )
    .bind(&a.id)
    .fetch_one(&t.root)
    .await
    .unwrap();
    tamper(format!(
        "UPDATE audit_events SET actor = '{original}' WHERE id = {victim}"
    ))
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
            .await["ok"],
        true
    );

    // deleting an event inside a sealed range is caught too
    tamper(format!("DELETE FROM audit_events WHERE id = {victim}")).await;
    let v = t
        .ok(a, "GET", "/v1/audit_events/verify", None, Value::Null)
        .await;
    assert_eq!(
        (v["ok"].clone(), v["reason"].clone()),
        (
            json!(false),
            json!("events were added or deleted inside a sealed range")
        )
    );

    // tenants are independent chains
    t.ok(
        b,
        "POST",
        "/v1/audit_events/seal?min_age_seconds=0",
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        t.ok(b, "GET", "/v1/audit_events/verify", None, Value::Null)
            .await["ok"],
        true
    );
}
