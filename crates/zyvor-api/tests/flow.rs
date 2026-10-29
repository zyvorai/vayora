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
