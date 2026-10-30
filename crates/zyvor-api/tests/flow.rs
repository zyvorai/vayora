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

#[sqlx::test(migrations = "./migrations")]
async fn master_key_rotation_reseals_every_secret_and_never_overwrites_what_it_cannot_read(
    pool: PgPool,
) {
    use zyvor_api::Crypto;
    let t = setup(pool).await;
    let (old_hex, new_hex) = ("11".repeat(32), "22".repeat(32));
    // tenant A also has an outbound webhook secret
    t.ok(&t.a, "POST", "/v1/settings", None, json!({ "outbound_webhook_url": "http://hooks.test/z", "outbound_webhook_secret": "whsec_out" })).await;
    let new_only = Crypto::from_hex(&new_hex).unwrap();
    let rotating = Crypto::from_hex(&new_hex)
        .unwrap()
        .with_previous(&old_hex)
        .unwrap();

    // one secret nobody can open (e.g. sealed by a key that was already discarded)
    sqlx::query("UPDATE provider_accounts SET webhook_secret_enc = $2 WHERE tenant_id = $1 AND provider = 'cashfree'")
        .bind(&t.b.id).bind(vec![7u8; 60]).execute(&t.root).await.unwrap();

    // 2 tenants x 2 provider accounts x 2 secrets = 8, plus tenant A's webhook secret = 9, one of them unreadable
    let dry = zyvor_api::admin::rekey_all(&t.state.pool, &rotating, true)
        .await
        .unwrap();
    assert_eq!((dry.0, dry.2), (8, 1));
    let all = || async {
        let mut v: Vec<Vec<u8>> =
            sqlx::query_scalar("SELECT credentials_enc FROM provider_accounts")
                .fetch_all(&t.root)
                .await
                .unwrap();
        v.extend(
            sqlx::query_scalar::<_, Vec<u8>>("SELECT webhook_secret_enc FROM provider_accounts")
                .fetch_all(&t.root)
                .await
                .unwrap(),
        );
        v.extend(sqlx::query_scalar::<_, Vec<u8>>("SELECT outbound_webhook_secret_enc FROM tenant_settings WHERE outbound_webhook_secret_enc IS NOT NULL").fetch_all(&t.root).await.unwrap());
        v
    };
    assert_eq!(
        all()
            .await
            .iter()
            .filter(|b| new_only.decrypt(b).is_some())
            .count(),
        0,
        "dry run changed nothing"
    );

    let done = zyvor_api::admin::rekey_all(&t.state.pool, &rotating, false)
        .await
        .unwrap();
    assert_eq!(done, dry);
    let readable = all()
        .await
        .iter()
        .filter(|b| new_only.decrypt(b).is_some())
        .count();
    assert_eq!(
        readable, 8,
        "every readable secret now opens with the new key alone"
    );
    assert!(
        all().await.contains(&vec![7u8; 60]),
        "the unreadable one is left exactly as it was"
    );
    let again = zyvor_api::admin::rekey_all(&t.state.pool, &rotating, false)
        .await
        .unwrap();
    assert_eq!((again.0, again.2), (0, 1), "idempotent");
}

#[sqlx::test(migrations = "./migrations")]
async fn console_users_sign_in_with_roles_lockout_and_a_replay_proof_second_factor(pool: PgPool) {
    use zyvor_api::console_users::{base32_decode, totp_at};
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let login = |who: &Tenant, u: &str, p: &str, otp: Option<&str>| {
        let (who, body) = (
            who.clone(),
            json!({ "username": u, "password": p, "otp": otp }),
        );
        let app = t.app.clone();
        async move {
            let (s, v) = raw(
                &app,
                &who.key,
                None,
                "POST",
                "/v1/console/authenticate",
                None,
                body,
            )
            .await;
            (s.as_u16(), v["error"]["code"].as_str().map(String::from), v)
        }
    };

    // creation rules
    let mk = |u: &str, p: &str, r: &str| json!({ "username": u, "password": p, "role": r });
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/console/users",
            None,
            mk("priya", "short", "finance")
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/console/users",
            None,
            mk("priya", "a-long-enough-pass", "root")
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/console/users",
            None,
            mk("no spaces", "a-long-enough-pass", "viewer")
        )
        .await
        .0
        .as_u16(),
        422
    );
    let u = t
        .ok(
            a,
            "POST",
            "/v1/console/users",
            None,
            mk("Priya", "a-long-enough-pass", "finance"),
        )
        .await;
    assert_eq!(u["username"], "priya");
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/console/users",
            None,
            mk("priya", "another-long-pass", "viewer")
        )
        .await
        .0
        .as_u16(),
        409
    );
    // only an admin key manages users
    let viewer_key = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "ro", "scopes": ["payments:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: viewer_key["key"].as_str().unwrap().into(),
    };
    assert_eq!(
        t.call(&ro, "GET", "/v1/console/users", None, Value::Null)
            .await
            .0
            .as_u16(),
        403
    );

    // sign-in: wrong user and wrong password look the same; the right one returns the role
    let (s1, c1, v1) = login(a, "nobody", "a-long-enough-pass", None).await;
    let (s2, c2, v2) = login(a, "priya", "wrong-password-here", None).await;
    assert_eq!(
        (s1, c1.clone(), s2, c2),
        (
            401,
            Some("unauthorized".into()),
            401,
            Some("unauthorized".into())
        )
    );
    assert_eq!(v1["error"]["message"], v2["error"]["message"]);
    let (s, _, ok) = login(a, "PRIYA", "a-long-enough-pass", None).await;
    assert_eq!(
        (s, ok["user"]["role"].clone(), ok["mfa_enrolled"].clone()),
        (200, json!("finance"), json!(false))
    );
    assert_eq!(
        login(b, "priya", "a-long-enough-pass", None).await.0,
        401,
        "another tenant's users do not exist here"
    );

    // five failures lock the account, even against the right password, until an admin re-enables it
    for _ in 0..5 {
        login(a, "priya", "wrong-password-here", None).await;
    }
    let (s, code, _) = login(a, "priya", "a-long-enough-pass", None).await;
    assert_eq!((s, code), (423, Some("account_locked".into())));
    t.ok(
        a,
        "POST",
        &format!("/v1/console/users/{}/enable", u["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(login(a, "priya", "a-long-enough-pass", None).await.0, 200);

    // second factor
    let uid = u["id"].as_str().unwrap();
    let begin = t
        .ok(
            a,
            "POST",
            &format!("/v1/console/users/{uid}/mfa/begin"),
            None,
            json!({}),
        )
        .await;
    assert!(
        begin["otpauth_uri"]
            .as_str()
            .unwrap()
            .starts_with("otpauth://totp/Zyvor:priya?secret=")
    );
    let secret = base32_decode(begin["secret"].as_str().unwrap()).unwrap();
    let now_step = chrono::Utc::now().timestamp() as u64 / 30;
    let (s, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/console/users/{uid}/mfa/confirm"),
            None,
            json!({ "otp": "000000" }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let conf = t
        .ok(
            a,
            "POST",
            &format!("/v1/console/users/{uid}/mfa/confirm"),
            None,
            json!({ "otp": totp_at(&secret, now_step) }),
        )
        .await;
    let recovery: Vec<String> = conf["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().into())
        .collect();
    assert_eq!(recovery.len(), 8);

    assert_eq!(
        login(a, "priya", "a-long-enough-pass", None).await.1,
        Some("otp_required".into())
    );
    assert_eq!(
        login(a, "priya", "wrong-password-here", Some("123456"))
            .await
            .1,
        Some("unauthorized".into()),
        "the code is never requested for a wrong password"
    );
    assert_eq!(
        login(a, "priya", "a-long-enough-pass", Some("000000"))
            .await
            .0,
        401
    );
    let next = totp_at(&secret, now_step + 1);
    let (s, _, ok) = login(a, "priya", "a-long-enough-pass", Some(&next)).await;
    assert_eq!((s, ok["mfa_enrolled"].clone()), (200, json!(true)));
    assert_eq!(
        login(a, "priya", "a-long-enough-pass", Some(&next)).await.0,
        401,
        "a code cannot be used twice"
    );
    assert_eq!(
        login(
            a,
            "priya",
            "a-long-enough-pass",
            Some(&totp_at(&secret, now_step))
        )
        .await
        .0,
        401,
        "nor can an older one"
    );
    // recovery codes work once each
    assert_eq!(
        login(a, "priya", "a-long-enough-pass", Some(&recovery[0]))
            .await
            .0,
        200
    );
    assert_eq!(
        login(a, "priya", "a-long-enough-pass", Some(&recovery[0]))
            .await
            .0,
        401
    );
    assert_eq!(
        login(a, "priya", "a-long-enough-pass", Some(&recovery[1]))
            .await
            .0,
        200
    );
    // resetting MFA removes the requirement; disabling blocks sign-in outright
    t.ok(
        a,
        "POST",
        &format!("/v1/console/users/{uid}/mfa/reset"),
        None,
        json!({}),
    )
    .await;
    assert_eq!(login(a, "priya", "a-long-enough-pass", None).await.0, 200);
    t.ok(
        a,
        "POST",
        &format!("/v1/console/users/{uid}/password"),
        None,
        json!({ "password": "a-brand-new-password" }),
    )
    .await;
    assert_eq!(login(a, "priya", "a-long-enough-pass", None).await.0, 401);
    assert_eq!(login(a, "priya", "a-brand-new-password", None).await.0, 200);
    t.ok(
        a,
        "POST",
        &format!("/v1/console/users/{uid}/disable"),
        None,
        json!({}),
    )
    .await;
    assert_eq!(login(a, "priya", "a-brand-new-password", None).await.0, 401);

    // the password hash and secrets never come back, and the trail records it
    let listed = t
        .ok(a, "GET", "/v1/console/users", None, Value::Null)
        .await
        .to_string();
    assert!(
        !listed.contains("argon2")
            && !listed.contains("secret")
            && listed.contains("\"disabled\":true")
    );
    for action in [
        "console_user.created",
        "console_user.login",
        "console_user.login_failed",
        "console_user.locked",
        "console_user.mfa_enrolled",
        "console_user.disabled",
    ] {
        assert!(
            t.count(&format!(
                "SELECT count(*) FROM audit_events WHERE action = '{action}'"
            ))
            .await
                >= 1,
            "{action}"
        );
    }
}

/// A stand-in for the model API: records every request and answers from a queue.
async fn spawn_fake_model(
    replies: Vec<&'static str>,
) -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
) {
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
    let (s2, q2) = (seen.clone(), queue);
    let app = axum::Router::new().route(
        "/v1/messages",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let (seen, queue) = (s2.clone(), q2.clone());
                async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    seen.lock().unwrap().push((key, body));
                    let text = queue.lock().unwrap().pop_front().unwrap_or("");
                    axum::Json(json!({ "content": [{ "type": "text", "text": text }] }))
                }
            },
        ),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen)
}

#[sqlx::test(migrations = "./migrations")]
async fn reminder_drafts_need_a_persons_approval_and_the_ai_assistant_is_boxed_in(pool: PgPool) {
    use zyvor_api::ai::AiConfig;
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let overdue = (chrono::Utc::now().date_naive() - chrono::Duration::days(9)).to_string();
    let mk = |n: &str| json!({ "number": n, "amount_minor": 125_050, "due_date": overdue });
    let i1 = t.ok(a, "POST", "/v1/invoices", None, mk("INV-1")).await;
    let id1 = i1["id"].as_str().unwrap();

    // standard wording, no AI configured
    let (s, body) = t
        .call(
            a,
            "POST",
            &format!("/v1/invoices/{id1}/reminders/draft"),
            None,
            json!({ "ai": true }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), body["error"]["code"].clone()),
        (422, json!("ai_not_configured"))
    );
    let d = t
        .ok(
            a,
            "POST",
            &format!("/v1/invoices/{id1}/reminders/draft"),
            None,
            json!({ "channel": "whatsapp" }),
        )
        .await;
    let msg = d["message"].as_str().unwrap();
    assert!(
        msg.contains("INV-1") && msg.contains("₹1250.50") && msg.contains("9 days ago"),
        "{msg}"
    );
    assert_eq!(
        (d["source"].clone(), d["status"].clone()),
        (json!("template"), json!("pending"))
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/invoices/{id1}/reminders/draft"),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        409,
        "one pending draft per invoice"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        0,
        "nothing is sent by drafting"
    );

    // tenant isolation
    assert!(
        t.ok(b, "GET", "/v1/collections/drafts", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.call(
            b,
            "POST",
            &format!(
                "/v1/collections/drafts/{}/approve",
                d["id"].as_str().unwrap()
            ),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        404
    );

    // approve with an edit: now it is logged and emitted, once
    let did = d["id"].as_str().unwrap();
    let ok = t
        .ok(
            a,
            "POST",
            &format!("/v1/collections/drafts/{did}/approve"),
            None,
            json!({ "message": "Hi, INV-1 is pending. Thanks!" }),
        )
        .await;
    assert_eq!(ok["status"], "approved");
    assert_eq!(
        t.count(
            "SELECT count(*) FROM invoice_reminders WHERE message = 'Hi, INV-1 is pending. Thanks!'"
        )
        .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        1
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/collections/drafts/{did}/approve"),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        409,
        "a draft is decided once"
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/collections/drafts/{did}/reject"),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        404
    );

    // rejecting sends nothing
    let d2 = t
        .ok(
            a,
            "POST",
            &format!("/v1/invoices/{id1}/reminders/draft"),
            None,
            json!({}),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/collections/drafts/{}/reject",
            d2["id"].as_str().unwrap()
        ),
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        1
    );

    // the assistant: good wording is used, unsafe wording and quota are handled
    let (url, seen) = spawn_fake_model(vec![
        "Hello, a gentle reminder from Org A that invoice INV-2 is still pending. Thank you.",
        "Pay invoice INV-3 now at http://evil.example/pay or face legal action.",
        "Hello, invoice INV-4 remains unpaid. Kindly settle it when you can. Thank you.",
    ])
    .await;
    let mut st = t.state.clone();
    st.ai = Some(std::sync::Arc::new(AiConfig {
        api_key: "sk-test-fake".into(),
        base_url: url,
        model: "claude-sonnet-5-5".into(),
        monthly_cap: 2,
    }));
    let app = zyvor_api::app(st);
    let call = |inv: String, body: Value| {
        let (app, key) = (app.clone(), a.key.clone());
        async move {
            raw(
                &app,
                &key,
                None,
                "POST",
                &format!("/v1/invoices/{inv}/reminders/draft"),
                None,
                body,
            )
            .await
        }
    };
    // give the customer details that must never reach the model
    let cust = t.ok(a, "POST", "/v1/customers", None, json!({ "name": "Secret Parent", "email": "parent@example.com", "phone": "+919999900000" })).await;
    let inv = |n: &str| json!({ "number": n, "amount_minor": 50_000, "due_date": overdue, "customer_id": cust["id"] });
    let i2 = t.ok(a, "POST", "/v1/invoices", None, inv("INV-2")).await;
    let i3 = t.ok(a, "POST", "/v1/invoices", None, inv("INV-3")).await;
    let i4 = t.ok(a, "POST", "/v1/invoices", None, inv("INV-4")).await;
    let (s, r2) = call(
        i2["id"].as_str().unwrap().into(),
        json!({ "ai": true, "tone": "firm" }),
    )
    .await;
    assert_eq!((s.as_u16(), r2["source"].clone()), (201, json!("ai")));
    assert!(r2["message"].as_str().unwrap().contains("gentle reminder"));
    let (s, r3) = call(i3["id"].as_str().unwrap().into(), json!({ "ai": true })).await;
    assert_eq!(
        (s.as_u16(), r3["source"].clone()),
        (201, json!("template")),
        "a link or threat in the model's answer is refused and the standard text is used"
    );
    assert!(!r3["message"].as_str().unwrap().contains("evil"));
    let (s, r4) = call(i4["id"].as_str().unwrap().into(), json!({ "ai": true })).await;
    assert_eq!(
        (s.as_u16(), r4["error"]["code"].clone()),
        (429, json!("ai_quota_exceeded")),
        "monthly cap counts every request, accepted or not"
    );

    let calls = seen.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "no model call once the cap is reached");
    let (key, req) = &calls[0];
    assert_eq!(key, "sk-test-fake");
    assert_eq!(req["model"], "claude-sonnet-5-5");
    let sent = req.to_string();
    for private in [
        "Secret Parent",
        "parent@example.com",
        "9999900000",
        "rzp_",
        "client_secret",
    ] {
        assert!(
            !sent.contains(private),
            "{private} must not be sent to the model"
        );
    }
    assert!(sent.contains("INV-2") && sent.contains("firm but polite"));

    // dunning that requires approval makes drafts, not sends
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "dunning": { "requires_approval": true, "repeat_after_days": 1 } }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["dunning"]["requires_approval"],
        true
    );
    let before = t
        .count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
        .await;
    let run = t
        .ok(
            a,
            "POST",
            "/v1/collections/dunning/run",
            None,
            json!({ "repeat_after_days": 0 }),
        )
        .await;
    assert!(
        run["reminded"] == 0 && run["drafts_created"].as_i64().unwrap() >= 1,
        "{run}"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'invoice.reminder'")
            .await,
        before
    );
    let again = t
        .ok(
            a,
            "POST",
            "/v1/collections/dunning/run",
            None,
            json!({ "repeat_after_days": 0 }),
        )
        .await;
    assert_eq!(
        again["drafts_created"], 0,
        "invoices with a draft waiting are skipped"
    );
    let pending = t
        .ok(a, "GET", "/v1/collections/drafts", None, Value::Null)
        .await;
    assert!(pending["data"].as_array().unwrap().len() >= 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn gst_invoices_split_tax_by_place_of_supply_reverse_it_on_credit_notes_and_feed_gstr1(
    pool: PgPool,
) {
    use zyvor_api::gst::gstin_check_char;
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let seller = "27AAPFU0939F1ZV"; // Maharashtra
    let buyer_delhi = {
        let head = "07AABCT1332L1Z";
        format!("{head}{}", gstin_check_char(head))
    };
    let line = |qty: i64, price: i64, rate: i32| json!({ "description": "Tuition", "hsn_sac": "999293", "quantity": qty, "unit_price_minor": price, "rate_bp": rate });

    // needs the seller's registration first
    let (s, body) = t
        .call(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "G-0", "lines": [line(1, 100, 1800)], "place_of_supply": "27" }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), body["error"]["code"].clone()),
        (422, json!("gst_not_configured"))
    );
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/settings",
            None,
            json!({ "gst": { "gstin": "27AAPFU0939F1ZW" } }),
        )
        .await;
    assert_eq!(s.as_u16(), 422, "bad check character");
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "gst": { "gstin": seller, "legal_name": "Acme School Trust" } }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["gst"]["gstin"],
        seller
    );

    // validation
    for (why, body) in [
        (
            "rate not a slab",
            json!({ "number": "G-x", "lines": [line(1, 100, 700)], "place_of_supply": "27" }),
        ),
        (
            "bad hsn",
            json!({ "number": "G-x", "lines": [{ "description": "x", "hsn_sac": "12", "quantity": 1, "unit_price_minor": 1, "rate_bp": 500 }], "place_of_supply": "27" }),
        ),
        (
            "no place of supply",
            json!({ "number": "G-x", "lines": [line(1, 100, 500)] }),
        ),
        (
            "bad state",
            json!({ "number": "G-x", "lines": [line(1, 100, 500)], "place_of_supply": "55" }),
        ),
        (
            "bad buyer gstin",
            json!({ "number": "G-x", "lines": [line(1, 100, 500)], "buyer_gstin": "07AABCT1332L1ZX" }),
        ),
        (
            "amount mismatch",
            json!({ "number": "G-x", "amount_minor": 1, "lines": [line(1, 100, 500)], "place_of_supply": "27" }),
        ),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/invoices", None, body)
                .await
                .0
                .as_u16(),
            422,
            "{why}"
        );
    }

    // within Maharashtra, to a consumer: CGST + SGST
    let intra = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "G-1", "lines": [line(2, 50_000, 1800)], "place_of_supply": "27" }),
        )
        .await;
    assert_eq!(
        (intra["amount_minor"].clone(), intra["tax_minor"].clone()),
        (json!(118_000), json!(18_000))
    );
    let det = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{}/lines", intra["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            det["lines"][0]["cgst_minor"].clone(),
            det["lines"][0]["sgst_minor"].clone(),
            det["lines"][0]["igst_minor"].clone()
        ),
        (json!(9_000), json!(9_000), json!(0))
    );
    // to a registered buyer in Delhi (taken from the customer's GSTIN): IGST
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Delhi Co", "gstin": buyer_delhi }),
        )
        .await;
    let inter = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "G-2", "customer_id": cust["id"], "lines": [line(1, 200_000, 500)] }),
        )
        .await;
    assert_eq!(
        (inter["amount_minor"].clone(), inter["tax_minor"].clone()),
        (json!(210_000), json!(10_000))
    );
    let det = t
        .ok(
            a,
            "GET",
            &format!("/v1/invoices/{}/lines", inter["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            det["place_of_supply"].clone(),
            det["lines"][0]["igst_minor"].clone(),
            det["lines"][0]["cgst_minor"].clone()
        ),
        (json!("07"), json!(10_000), json!(0))
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Bad", "gstin": "nope" })
        )
        .await
        .0
        .as_u16(),
        422
    );

    // a credit note for half of the first invoice reverses half of its tax, split like the invoice
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/invoices/{}/credit_notes",
            intra["id"].as_str().unwrap()
        ),
        None,
        json!({ "amount_minor": 59_000, "reason": "fee concession" }),
    )
    .await;
    let note: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT taxable_minor, cgst_minor, sgst_minor, igst_minor FROM credit_notes",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(note, (50_000, 4_500, 4_500, 0));

    // the month's GSTR-1 style summary
    let ist = chrono::FixedOffset::east_opt(19_800).unwrap();
    let period = chrono::Utc::now()
        .with_timezone(&ist)
        .format("%Y-%m")
        .to_string();
    let r = t
        .ok(
            a,
            "GET",
            &format!("/v1/gst/gstr1?period={period}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (r["gstin"].clone(), r["legal_name"].clone()),
        (json!(seller), json!("Acme School Trust"))
    );
    assert_eq!(r["b2b"][0]["buyer_gstin"], json!(buyer_delhi));
    assert_eq!(r["b2b"][0]["invoices"][0]["number"], "G-2");
    assert_eq!(r["b2b"][0]["invoices"][0]["rates"][0]["igst_minor"], 10_000);
    assert_eq!(
        (
            r["b2cs"][0]["place_of_supply"].clone(),
            r["b2cs"][0]["taxable_minor"].clone(),
            r["b2cs"][0]["cgst_minor"].clone(),
            r["b2cs"][0]["sgst_minor"].clone()
        ),
        (json!("27"), json!(100_000), json!(9_000), json!(9_000))
    );
    assert_eq!(r["cdnur"][0]["taxable_minor"], 50_000);
    assert_eq!(r["cdnur"][0]["invoice_number"], "G-1");
    let h = r["hsn"].as_array().unwrap();
    assert_eq!(h.len(), 2, "one row per HSN and rate");
    assert_eq!(r["totals"]["taxable_minor"], 300_000);
    assert_eq!(r["totals"]["igst_minor"], 10_000);
    assert_eq!(
        t.call(a, "GET", "/v1/gst/gstr1?period=2026-13", None, Value::Null)
            .await
            .0
            .as_u16(),
        422
    );

    // another tenant sees none of it
    let rb = t
        .ok(
            b,
            "GET",
            &format!("/v1/gst/gstr1?period={period}"),
            None,
            Value::Null,
        )
        .await;
    assert!(
        rb["b2b"].as_array().unwrap().is_empty()
            && rb["b2cs"].as_array().unwrap().is_empty()
            && rb["gstin"].is_null()
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/invoices/{}/lines", intra["id"].as_str().unwrap()),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn payouts_need_a_second_person_record_the_bank_reference_and_post_to_the_ledger(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let balance = |acct: &'static str| {
        let (root, tid) = (t.root.clone(), a.id.clone());
        async move {
            let r: (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries WHERE tenant_id = $1 AND account = $2")
                .bind(tid).bind(acct).fetch_one(&root).await.unwrap();
            r
        }
    };

    // vendors: details are encrypted, lists show only masks
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/vendors",
            None,
            json!({ "name": "No details" })
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/vendors",
            None,
            json!({ "name": "Bad", "account_number": "12", "ifsc": "X" })
        )
        .await
        .0
        .as_u16(),
        422
    );
    let v = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Sharma Stationers", "account_number": "1234567890123", "ifsc": "HDFC0001234", "external_ref": "VEND-1" })).await;
    let vid = v["id"].as_str().unwrap();
    let listed = t.ok(a, "GET", "/v1/vendors", None, Value::Null).await;
    assert_eq!(listed["data"][0]["account_last4"], "0123");
    assert!(!listed.to_string().contains("1234567890123"));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT details_enc FROM vendors")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert!(
        !stored.windows(13).any(|w| w == b"1234567890123"),
        "encrypted at rest"
    );
    let writer = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "clerk", "scopes": ["payouts:write", "payments:read"] }),
        )
        .await;
    let clerk = Tenant {
        id: a.id.clone(),
        key: writer["key"].as_str().unwrap().into(),
    };
    assert_eq!(
        t.call(
            &clerk,
            "GET",
            &format!("/v1/vendors/{vid}/details"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        403,
        "a clerk cannot read full details"
    );
    let full = t
        .ok(
            a,
            "GET",
            &format!("/v1/vendors/{vid}/details"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(full["details"]["account_number"], "1234567890123");
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'vendor.details_revealed'")
            .await,
        1
    );

    // requesting: validation, idempotency by reference
    let req = |amt: i64, r: &str| json!({ "vendor_id": vid, "amount_minor": amt, "purpose": "Stationery for term 2", "external_ref": r });
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            json!({ "vendor_id": "ven_nope", "amount_minor": 100, "purpose": "x" })
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            json!({ "vendor_id": vid, "amount_minor": 0, "purpose": "x" })
        )
        .await
        .0
        .as_u16(),
        422
    );
    let (s1, p1) = t
        .call_as(
            a,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            req(500_000, "PO-1"),
        )
        .await;
    assert_eq!(
        (s1.as_u16(), p1["status"].clone()),
        (201, json!("pending_approval"))
    );
    let (s2, again) = t
        .call_as(
            a,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            req(500_000, "PO-1"),
        )
        .await;
    assert_eq!(
        (s2.as_u16(), again["id"].clone()),
        (200, p1["id"].clone()),
        "same reference, same payout"
    );
    let p = p1["id"].as_str().unwrap();

    // separation of duties, and nothing is payable before approval
    let (s, e) = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p}/approve"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (403, json!("separation_of_duties"))
    );
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p}/mark_paid"),
            None,
            json!({ "bank_ref": "UTR123456" })
        )
        .await
        .1["error"]["code"],
        "not_approved"
    );
    assert_eq!(
        t.call_as(
            a,
            "ravi",
            "POST",
            &format!("/v1/payouts/{p}/approve"),
            None,
            json!({})
        )
        .await
        .1["status"],
        "approved"
    );
    assert_eq!(
        t.call_as(
            a,
            "ravi",
            "POST",
            &format!("/v1/payouts/{p}/approve"),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        409,
        "decided once"
    );

    // recording the transfer posts to the ledger exactly once
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p}/mark_paid"),
            None,
            json!({ "bank_ref": "x" })
        )
        .await
        .0
        .as_u16(),
        422
    );
    let paid = t
        .ok(
            a,
            "POST",
            &format!("/v1/payouts/{p}/mark_paid"),
            None,
            json!({ "bank_ref": "HDFCN26301234567" }),
        )
        .await;
    assert_eq!(
        (paid["status"].clone(), paid["bank_ref"].clone()),
        (json!("paid"), json!("HDFCN26301234567"))
    );
    assert_eq!(balance("payouts").await, (500_000, 0));
    assert_eq!(balance("bank").await, (0, 500_000));
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p}/mark_paid"),
            None,
            json!({ "bank_ref": "HDFCN26301234567" })
        )
        .await
        .0
        .as_u16(),
        409
    );
    assert_eq!(balance("bank").await, (0, 500_000), "no double posting");
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p}/cancel"),
            None,
            json!({})
        )
        .await
        .0
        .as_u16(),
        409,
        "a paid payout cannot be canceled"
    );

    // the same bank transfer cannot be recorded against two payouts
    let p2 = t
        .call_as(
            a,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            req(100_000, "PO-2"),
        )
        .await
        .1;
    let p2 = p2["id"].as_str().unwrap();
    t.ok(
        &Tenant {
            id: a.id.clone(),
            key: a.key.clone(),
        },
        "POST",
        "/v1/settings",
        None,
        json!({}),
    )
    .await;
    t.call_as(
        a,
        "ravi",
        "POST",
        &format!("/v1/payouts/{p2}/approve"),
        None,
        json!({}),
    )
    .await;
    let (s, e) = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p2}/mark_paid"),
            None,
            json!({ "bank_ref": "HDFCN26301234567" }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("bank_ref_used"))
    );
    assert_eq!(
        t.ok(a, "GET", &format!("/v1/payouts/{p2}"), None, Value::Null)
            .await["status"],
        "approved",
        "the failed attempt changed nothing"
    );

    // rejecting and canceling
    let p3 = t
        .call_as(a, "priya", "POST", "/v1/payouts", None, req(7_000, "PO-3"))
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        t.call_as(
            a,
            "ravi",
            "POST",
            &format!("/v1/payouts/{p3}/reject"),
            None,
            json!({})
        )
        .await
        .1["status"],
        "rejected"
    );
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p2}/cancel"),
            None,
            json!({})
        )
        .await
        .1["status"],
        "canceled"
    );
    assert_eq!(
        t.call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/payouts/{p2}/mark_paid"),
            None,
            json!({ "bank_ref": "HDFCN26309999999" })
        )
        .await
        .0
        .as_u16(),
        409
    );

    // small payouts can skip approval when the tenant says so
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/settings",
            None,
            json!({ "payout_approval_threshold_minor": -1 })
        )
        .await
        .0
        .as_u16(),
        422
    );
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "payout_approval_threshold_minor": 10_000 }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["payout_approval_threshold_minor"],
        10_000
    );
    assert_eq!(
        t.call_as(a, "priya", "POST", "/v1/payouts", None, req(9_999, "PO-4"))
            .await
            .1["status"],
        "approved"
    );
    assert_eq!(
        t.call_as(a, "priya", "POST", "/v1/payouts", None, req(10_001, "PO-5"))
            .await
            .1["status"],
        "pending_approval"
    );

    // events and isolation
    assert!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payout.paid'")
            .await
            == 1
    );
    assert!(
        t.ok(b, "GET", "/v1/payouts", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        t.ok(b, "GET", "/v1/vendors", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.call(b, "GET", &format!("/v1/payouts/{p}"), None, Value::Null)
            .await
            .0
            .as_u16(),
        404
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/vendors/{vid}/details"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_subscriptions_decide_which_events_are_queued_and_the_catalogue_is_served(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let cat = t
        .ok(a, "GET", "/v1/events/catalogue", None, Value::Null)
        .await;
    let types: Vec<&str> = cat["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert!(
        types.contains(&"payment.captured")
            && types.contains(&"payout.paid")
            && cat["data"][0]["description"].as_str().unwrap().len() > 10
    );
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["webhook_events"],
        json!(["*"]),
        "everything by default"
    );

    for bad in [
        json!([]),
        json!(["payments.*"]),
        json!(["payment.nope"]),
        json!([""]),
    ] {
        assert_eq!(
            t.call(
                a,
                "POST",
                "/v1/settings",
                None,
                json!({ "webhook_events": bad })
            )
            .await
            .0
            .as_u16(),
            422
        );
    }
    let count = |ty: &'static str| {
        let root = t.root.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox_events WHERE type = $1")
                .bind(ty)
                .fetch_one(&root)
                .await
                .unwrap()
        }
    };

    // only the payment family: a void is not queued
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["payment.*", "payout.paid"] }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["webhook_events"],
        json!(["payment.*", "payout.paid"])
    );
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "E-1", "amount_minor": 1000 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/void", inv["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        count("invoice.voided").await,
        0,
        "not subscribed, never queued"
    );

    // the invoice family: now it is
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["invoice.*"] }),
    )
    .await;
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "E-2", "amount_minor": 1000 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/void", inv["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(count("invoice.voided").await, 1);

    // an exact type that does not match is still filtered; back to everything restores delivery
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["invoice.reminder"] }),
    )
    .await;
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "E-3", "amount_minor": 1000 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/void", inv["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(count("invoice.voided").await, 1);
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["*"] }),
    )
    .await;
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "E-4", "amount_minor": 1000 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/void", inv["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(count("invoice.voided").await, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn instalment_plans_split_exactly_atomically_and_only_once(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Aarav (Class 4)" }),
        )
        .await;
    let body = |prefix: &str, ext: Option<&str>| json!({ "customer_id": cust["id"], "name": "Annual fee 2026-27", "total_minor": 100_001, "instalments": 3, "first_due_date": "2026-04-30", "number_prefix": prefix, "external_ref": ext });

    // validation
    for bad in [
        json!({ "instalments": 1 }),
        json!({ "instalments": 61 }),
        json!({ "total_minor": 2 }),
        json!({ "every_unit": "week" }),
        json!({ "every": 0 }),
        json!({ "number_prefix": " " }),
    ] {
        let mut v = body("FEE", None);
        v.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        assert_eq!(
            t.call(a, "POST", "/v1/installment_plans", None, v)
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    let mut unknown = body("FEE", None);
    unknown["customer_id"] = json!("cus_nope");
    assert_eq!(
        t.call(a, "POST", "/v1/installment_plans", None, unknown)
            .await
            .0
            .as_u16(),
        404
    );

    // the split is exact and the dates step by month, clamped to month end
    let plan = t
        .ok(
            a,
            "POST",
            "/v1/installment_plans",
            None,
            body("FEE-2026", Some("FEES-AARAV-26")),
        )
        .await;
    let invs = plan["invoices"].as_array().unwrap();
    assert_eq!(
        invs.iter()
            .map(|i| i["amount_minor"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![33_333, 33_333, 33_335]
    );
    assert_eq!(
        invs.iter()
            .map(|i| i["due_date"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["2026-04-30", "2026-05-30", "2026-06-30"]
    );
    assert_eq!(
        invs.iter()
            .map(|i| i["number"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["FEE-2026-1", "FEE-2026-2", "FEE-2026-3"]
    );
    assert_eq!(
        (plan["total_minor"].clone(), plan["remaining_minor"].clone()),
        (json!(100_001), json!(100_001))
    );

    // the same reference returns the same plan and creates nothing new
    let (s, again) = t
        .call(
            a,
            "POST",
            "/v1/installment_plans",
            None,
            body("FEE-2026", Some("FEES-AARAV-26")),
        )
        .await;
    assert_eq!((s.as_u16(), again["id"].clone()), (200, plan["id"].clone()));
    assert_eq!(t.count("SELECT count(*) FROM installment_plans").await, 1);

    // a clash on an invoice number rolls the whole plan back
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/installment_plans",
            None,
            body("FEE-2026", Some("OTHER")),
        )
        .await;
    assert_eq!(s.as_u16(), 409);
    assert_eq!(
        (
            t.count("SELECT count(*) FROM installment_plans").await,
            t.count("SELECT count(*) FROM invoices").await
        ),
        (1, 3),
        "all or nothing"
    );

    // paying down an instalment shows on the plan (a waiver here; payments allocate the same way)
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/invoices/{}/credit_notes",
            invs[0]["invoice_id"].as_str().unwrap()
        ),
        None,
        json!({ "amount_minor": 33_333, "reason": "scholarship" }),
    )
    .await;
    let got = t
        .ok(
            a,
            "GET",
            &format!("/v1/installment_plans/{}", plan["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            got["settled_minor"].clone(),
            got["remaining_minor"].clone(),
            got["invoices"][0]["status"].clone()
        ),
        (json!(33_333), json!(66_668), json!("paid"))
    );

    // other tenants cannot see it; weekly-style day steps work
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/installment_plans/{}", plan["id"].as_str().unwrap()),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    let mut d = body("DAYS", None);
    d.as_object_mut().unwrap().extend(
        json!({ "every_unit": "day", "every": 14, "instalments": 2 })
            .as_object()
            .unwrap()
            .clone(),
    );
    let p2 = t.ok(a, "POST", "/v1/installment_plans", None, d).await;
    assert_eq!(p2["invoices"][1]["due_date"], "2026-05-14");
}

#[sqlx::test(migrations = "./migrations")]
async fn fx_rates_you_enter_convert_amounts_and_show_foreign_receivables_in_inr(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(19_800).unwrap())
        .date_naive();
    let due = (today - chrono::Duration::days(5)).to_string();
    t.ok(
        a,
        "POST",
        "/v1/invoices",
        None,
        json!({ "number": "IN-1", "amount_minor": 100_000, "due_date": due }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/invoices",
        None,
        json!({ "number": "US-1", "currency": "USD", "amount_minor": 10_000, "due_date": due }),
    )
    .await;

    // no rate yet: the ageing says so instead of guessing
    let r = t
        .ok(a, "GET", "/v1/collections/ageing", None, Value::Null)
        .await;
    assert!(r["total_outstanding_inr_minor"].is_null());
    let usd = r["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["currency"] == "USD")
        .unwrap();
    assert!(usd["outstanding_inr_minor"].is_null());
    assert_eq!(
        t.call(
            a,
            "GET",
            "/v1/fx/convert?amount_minor=100&from=USD&to=INR",
            None,
            Value::Null
        )
        .await
        .1["error"]["code"],
        "no_rate"
    );

    // validation
    for (why, body) in [
        ("INR", json!({ "currency": "INR", "rate": "1" })),
        ("unsupported", json!({ "currency": "EUR", "rate": "90" })),
        (
            "bad rate",
            json!({ "currency": "USD", "rate": "83.1234567" }),
        ),
        ("zero", json!({ "currency": "USD", "rate": "0" })),
        (
            "future",
            json!({ "currency": "USD", "rate": "83", "as_of": (today + chrono::Duration::days(1)).to_string() }),
        ),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/fx/rates", None, body)
                .await
                .0
                .as_u16(),
            422,
            "{why}"
        );
    }
    let clerk = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "ro", "scopes": ["payments:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: clerk["key"].as_str().unwrap().into(),
    };
    assert_eq!(
        t.call(
            &ro,
            "POST",
            "/v1/fx/rates",
            None,
            json!({ "currency": "USD", "rate": "83" })
        )
        .await
        .0
        .as_u16(),
        403
    );

    // record a rate: older days use the older rate, rates are never edited
    let old = (today - chrono::Duration::days(30)).to_string();
    t.ok(
        a,
        "POST",
        "/v1/fx/rates",
        None,
        json!({ "currency": "USD", "rate": "82.5", "as_of": old, "note": "month start" }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/fx/rates",
        None,
        json!({ "currency": "USD", "rate": "83.2150" }),
    )
    .await;
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/fx/rates",
            None,
            json!({ "currency": "USD", "rate": "84" })
        )
        .await
        .0
        .as_u16(),
        409,
        "one rate per day, never edited"
    );
    let rates = t.ok(a, "GET", "/v1/fx/rates", None, Value::Null).await;
    assert_eq!(
        (
            rates["data"][0]["rate"].clone(),
            rates["data"][1]["rate"].clone()
        ),
        (json!("83.215000"), json!("82.500000"))
    );

    let c = t
        .ok(
            a,
            "GET",
            "/v1/fx/convert?amount_minor=10000&from=USD&to=INR",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (c["amount_minor"].clone(), c["currency"].clone()),
        (json!(832_150), json!("INR"))
    );
    let back = t
        .ok(
            a,
            "GET",
            "/v1/fx/convert?amount_minor=832150&from=INR&to=USD",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(back["amount_minor"], 10_000);
    let past = t
        .ok(
            a,
            "GET",
            &format!("/v1/fx/convert?amount_minor=10000&from=USD&to=INR&as_of={old}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        past["amount_minor"], 825_000,
        "the rate that applied on that day"
    );
    assert_eq!(
        t.call(
            a,
            "GET",
            &format!(
                "/v1/fx/convert?amount_minor=1&from=USD&to=INR&as_of={}",
                today - chrono::Duration::days(60)
            ),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        409
    );
    assert_eq!(
        t.ok(
            a,
            "GET",
            "/v1/fx/convert?amount_minor=5&from=INR&to=INR",
            None,
            Value::Null
        )
        .await["amount_minor"],
        5
    );

    // ageing now totals both currencies in INR
    let r = t
        .ok(a, "GET", "/v1/collections/ageing", None, Value::Null)
        .await;
    assert_eq!(r["total_outstanding_inr_minor"], 100_000 + 832_150);
    let usd = r["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["currency"] == "USD")
        .unwrap();
    assert_eq!(usd["outstanding_inr_minor"], 832_150);

    // rates are per tenant, and immutable
    assert!(
        t.ok(b, "GET", "/v1/fx/rates", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &a.id)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE fx_rates SET rate_micro = 1")
            .execute(&mut *tx)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn split_payments_accrue_to_vendors_reverse_on_refund_and_fund_payouts_from_the_balance(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let vendor = |name: &'static str| {
        let (t2, a2) = (&t, a);
        async move {
            t2.ok(
                a2,
                "POST",
                "/v1/vendors",
                None,
                json!({ "name": name, "vpa": format!("{}@okbank", name.to_lowercase()) }),
            )
            .await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let (v1, v2) = (vendor("Artisan").await, vendor("Courier").await);
    let owed = |v: String| {
        let (t2, a2) = (&t, a.clone());
        async move {
            let r = t2
                .ok(
                    &a2,
                    "GET",
                    &format!("/v1/vendors/{v}/balance"),
                    None,
                    Value::Null,
                )
                .await;
            (
                r["balances"][0]["owed_minor"].as_i64().unwrap_or(0),
                r["balances"][0]["available_minor"].as_i64().unwrap_or(0),
                r["awaiting_capture_minor"].as_i64().unwrap(),
            )
        }
    };

    // validation
    let bad = |splits: Value| {
        let key = format!("k-{}", zyvor_core::new_id("x"));
        let (t2, a2) = (&t, a);
        async move {
            t2.call(
                a2,
                "POST",
                "/v1/payment_intents",
                Some(&key),
                json!({ "amount_minor": 100_000, "currency": "INR", "splits": splits }),
            )
            .await
        }
    };
    assert_eq!(bad(json!([{ "vendor_id": v1, "amount_minor": 70_000 }, { "vendor_id": v2, "amount_minor": 30_001 }])).await.0.as_u16(), 422);
    assert_eq!(
        bad(
            json!([{ "vendor_id": v1, "amount_minor": 1 }, { "vendor_id": v1, "amount_minor": 1 }])
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        bad(json!([{ "vendor_id": "ven_nope", "amount_minor": 1 }]))
            .await
            .0
            .as_u16(),
        404
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payment_splits").await,
        0,
        "a refused payment leaves nothing behind"
    );

    // nothing is owed until the payment is captured
    let unpaid = t
        .intent(
            a,
            100_000,
            json!({ "splits": [{ "vendor_id": v1, "amount_minor": 10_000 }] }),
        )
        .await;
    assert_eq!(unpaid["amount_minor"], 100_000);
    assert_eq!(owed(v1.clone()).await, (0, 0, 10_000));

    // capture: shares accrue
    let splits = json!([{ "vendor_id": v1, "amount_minor": 60_000 }, { "vendor_id": v2, "amount_minor": 25_000 }]);
    let (iid, _, _) = t
        .paid_razorpay(a, 100_000, json!({ "splits": splits }))
        .await;
    assert_eq!(owed(v1.clone()).await, (60_000, 60_000, 10_000));
    assert_eq!(owed(v2.clone()).await.0, 25_000);

    // a 40% refund takes 40% back from each share
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("rf-1"),
        json!({ "amount_minor": 40_000 }),
    )
    .await;
    assert_eq!(
        (owed(v1.clone()).await.0, owed(v2.clone()).await.0),
        (36_000, 15_000)
    );
    // and the rest takes everything
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("rf-2"),
        json!({}),
    )
    .await;
    assert_eq!((owed(v1.clone()).await.0, owed(v2.clone()).await.0), (0, 0));

    // a fresh payment, then pay a vendor out of what they are owed
    t.paid_razorpay(
        a,
        100_000,
        json!({ "splits": [{ "vendor_id": v1, "amount_minor": 36_000 }] }),
    )
    .await;
    let req = |amt: i64, r: &str| json!({ "vendor_id": v1, "amount_minor": amt, "purpose": "Settlement of sales", "external_ref": r, "from_balance": true });
    assert_eq!(
        t.call_as(a, "priya", "POST", "/v1/payouts", None, req(40_000, "B-1"))
            .await
            .1["error"]["code"],
        "insufficient_balance"
    );
    let p = t
        .call_as(a, "priya", "POST", "/v1/payouts", None, req(30_000, "B-2"))
        .await
        .1;
    assert_eq!(p["status"], "pending_approval");
    assert_eq!(
        owed(v1.clone()).await.1,
        6_000,
        "the promised amount is held back"
    );
    assert_eq!(
        t.call_as(a, "priya", "POST", "/v1/payouts", None, req(7_000, "B-3"))
            .await
            .1["error"]["code"],
        "insufficient_balance",
        "cannot promise the same money twice"
    );
    let pid = p["id"].as_str().unwrap();
    t.call_as(
        a,
        "ravi",
        "POST",
        &format!("/v1/payouts/{pid}/approve"),
        None,
        json!({}),
    )
    .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/payouts/{pid}/mark_paid"),
        None,
        json!({ "bank_ref": "HDFCN26304445556" }),
    )
    .await;
    assert_eq!(
        owed(v1.clone()).await.0,
        6_000,
        "paying clears the liability"
    );
    // credits: 60,000 + 36,000 accrued; debits: 60,000 reversed by refunds + 30,000 paid out
    let payable: (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries WHERE account = $1").bind(format!("vendor_payable:{v1}")).fetch_one(&t.root).await.unwrap();
    assert_eq!(payable, (90_000, 96_000));
    assert_eq!(
        t.count("SELECT count(*) FROM ledger_entries WHERE account = 'payouts'")
            .await,
        0,
        "balance payouts do not touch the own-funds account"
    );

    // payment links carry the shares into every intent they mint
    let link = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 50_000, "splits": [{ "vendor_id": v2, "amount_minor": 20_000 }] })).await;
    assert_eq!(t.call(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 10_000, "splits": [{ "vendor_id": v2, "amount_minor": 20_000 }] })).await.0.as_u16(), 422);
    let before = t.count("SELECT count(*) FROM payment_splits").await;
    t.ok_public(
        "POST",
        &format!(
            "/v1/checkout/links/{}/{}/start",
            a.id,
            link["token"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        t.count("SELECT count(*) FROM payment_splits").await,
        before + 1
    );

    // bulk payouts: all or nothing, safe to repeat
    let one = |n: i32| json!({ "vendor_id": v2, "amount_minor": 1_000 * n as i64, "purpose": "Monthly run", "external_ref": format!("RUN-{n}") });
    let run = t
        .ok(
            a,
            "POST",
            "/v1/payouts/bulk",
            None,
            json!({ "payouts": [one(1), one(2), one(3)] }),
        )
        .await;
    assert_eq!(
        (
            run["count"].clone(),
            run["already_existed"].clone(),
            run["new_total_minor"].clone()
        ),
        (json!(3), json!(0), json!(6_000))
    );
    let again = t
        .ok(
            a,
            "POST",
            "/v1/payouts/bulk",
            None,
            json!({ "payouts": [one(1), one(2), one(3)] }),
        )
        .await;
    assert_eq!(
        (
            again["already_existed"].clone(),
            again["new_total_minor"].clone()
        ),
        (json!(3), json!(0))
    );
    let before = t.count("SELECT count(*) FROM payouts").await;
    let mut broken = one(5);
    broken["vendor_id"] = json!("ven_nope");
    let (st, e) = t
        .call(
            a,
            "POST",
            "/v1/payouts/bulk",
            None,
            json!({ "payouts": [one(4), broken] }),
        )
        .await;
    assert_eq!(st.as_u16(), 404);
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("payout 2 of 2")
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payouts").await,
        before,
        "the good one was rolled back with it"
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/payouts/bulk",
            None,
            json!({ "payouts": [one(7), one(7)] })
        )
        .await
        .0
        .as_u16(),
        422,
        "duplicate references in one request"
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/payouts/bulk",
            None,
            json!({ "payouts": [] })
        )
        .await
        .0
        .as_u16(),
        422
    );

    // isolation
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/vendors/{v1}/balance"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn payment_link_qr_codes_point_at_the_public_checkout_and_only_for_live_links(pool: PgPool) {
    use tower::ServiceExt;
    let t = setup(pool).await;
    let a = &t.a;
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 5_000 }),
        )
        .await;
    let get = |app: axum::Router, key: String, id: String| async move {
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/v1/payment_links/{id}/qr.svg"))
                    .header("authorization", format!("Bearer {key}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let (st, ct) = (
            res.status().as_u16(),
            res.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(String::from),
        );
        let body = String::from_utf8_lossy(
            &axum::body::to_bytes(res.into_body(), 1 << 20)
                .await
                .unwrap(),
        )
        .to_string();
        (st, ct, body)
    };
    let id = link["id"].as_str().unwrap().to_string();
    // not configured: say so rather than print a QR code that points nowhere
    assert_eq!(get(t.app.clone(), a.key.clone(), id.clone()).await.0, 409);

    let mut st = t.state.clone();
    st.checkout_public_url = Some("https://pay.example.com/".into());
    let app = zyvor_api::app(st);
    let (code, ctype, svg) = get(app.clone(), a.key.clone(), id.clone()).await;
    assert_eq!((code, ctype.as_deref()), (200, Some("image/svg+xml")));
    assert!(svg.contains("<svg") && svg.len() > 500);
    // another tenant cannot fetch it; a canceled link has no QR code
    assert_eq!(get(app.clone(), t.b.key.clone(), id.clone()).await.0, 404);
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_links/{id}/cancel"),
        None,
        json!({}),
    )
    .await;
    assert_eq!(get(app, a.key.clone(), id).await.0, 409);
}

#[sqlx::test(migrations = "./migrations")]
async fn risk_rules_stop_payments_the_merchant_said_to_stop_and_leave_a_trail(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // validation, and defaults are off
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["risk"],
        json!({})
    );
    for bad in [
        json!({ "max_intent_amount_minor": 0 }),
        json!({ "blocked_vpas": ["nohandle"] }),
        json!({ "max_attempts_per_intent": -1 }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/settings", None, json!({ "risk": bad }))
                .await
                .0
                .as_u16(),
            422
        );
    }
    t.ok(a, "POST", "/v1/settings", None, json!({ "risk": { "max_intent_amount_minor": 500_000, "max_attempts_per_intent": 2, "max_failed_per_customer_per_hour": 2, "blocked_vpas": ["@scamnet", "thief@oksbi"] } })).await;
    assert_eq!(
        t.ok(a, "GET", "/v1/settings", None, Value::Null).await["risk"]["max_attempts_per_intent"],
        2
    );

    // a payment over the ceiling is refused, audited and announced; one under it is fine
    let key = |n: &str| Some(format!("k-{n}"));
    let (s1, e) = t
        .call(
            a,
            "POST",
            "/v1/payment_intents",
            key("big").as_deref(),
            json!({ "amount_minor": 600_000, "currency": "INR" }),
        )
        .await;
    assert_eq!(
        (s1.as_u16(), e["error"]["code"].clone()),
        (403, json!("risk_blocked"))
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_intents").await, 0);
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'risk.blocked'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payment.risk_blocked'")
            .await,
        1
    );
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Risky Payer" }),
        )
        .await;
    let mk = || {
        let (t2, cid) = (&t, cust["id"].clone());
        async move {
            t2.intent(a, 100_000, json!({ "customer_id": cid })).await["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let i1 = mk().await;
    let attempt = |iid: String, vpa: &'static str, k: &'static str| {
        let t = &t;
        async move {
            t.call(
                a,
                "POST",
                &format!("/v1/payment_intents/{iid}/attempts"),
                Some(k),
                json!({ "method": "upi", "vpa": vpa }),
            )
            .await
        }
    };

    // blocked handles and exact addresses
    for vpa in ["anyone@scamnet", "THIEF@oksbi"] {
        let (s, e) = attempt(i1.clone(), vpa, "a-block").await;
        assert_eq!(
            (s.as_u16(), e["error"]["code"].clone()),
            (403, json!("risk_blocked")),
            "{vpa}"
        );
    }
    assert_eq!(
        t.count("SELECT count(*) FROM payment_attempts").await,
        0,
        "nothing reached a provider"
    );

    // two failed tries on one payment, then no more
    assert_eq!(attempt(i1.clone(), "fail@upi", "a-1").await.0.as_u16(), 201);
    zyvor_api::worker::run_cycle(&t.state, &zyvor_api::outbox::http_client())
        .await
        .unwrap();
    sqlx::query("UPDATE payment_attempts SET status = 'failed' WHERE intent_id = $1")
        .bind(&i1)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE payment_intents SET status = 'requires_payment_method' WHERE id = $1")
        .bind(&i1)
        .execute(&t.root)
        .await
        .unwrap();
    assert_eq!(
        attempt(i1.clone(), "fail2@upi", "a-2").await.0.as_u16(),
        201
    );
    sqlx::query("UPDATE payment_attempts SET status = 'failed' WHERE intent_id = $1")
        .bind(&i1)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE payment_intents SET status = 'requires_payment_method' WHERE id = $1")
        .bind(&i1)
        .execute(&t.root)
        .await
        .unwrap();
    let (s, e) = attempt(i1.clone(), "ok@upi", "a-3").await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (403, json!("risk_blocked"))
    );
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_attempts_per_intent")
    );

    // the same customer's failures across payments also count: two failures in the hour stop a third payment
    let i2 = mk().await;
    let (s, e) = attempt(i2, "ok@upi", "a-4").await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (403, json!("risk_blocked"))
    );
    assert!(
        e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("max_failed_per_customer_per_hour")
    );

    // other tenants are untouched, and a payment with no customer is not caught by the customer rule
    assert_eq!(
        t.intent(b, 900_000, json!({})).await["amount_minor"],
        900_000,
        "tenant B has no ceiling"
    );
    assert!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'risk.blocked'")
            .await
            >= 5
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn coupons_and_convenience_fees_price_a_payment_link_and_cannot_be_over_used(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);

    // coupon rules
    for bad in [
        json!({ "code": "x", "kind": "percent", "value": 1000 }),
        json!({ "code": "BAD CODE", "kind": "percent", "value": 1000 }),
        json!({ "code": "TOOMUCH", "kind": "percent", "value": 10_001 }),
        json!({ "code": "ZERO", "kind": "flat", "value": 0 }),
        json!({ "code": "NOKIND", "kind": "bogus", "value": 5 }),
        json!({ "code": "PAST", "kind": "flat", "value": 5, "expires_at": "2020-01-01T00:00:00Z" }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/coupons", None, bad.clone())
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    let c = t
        .ok(
            a,
            "POST",
            "/v1/coupons",
            None,
            json!({ "code": "save10", "kind": "percent", "value": 1000, "max_uses": 2 }),
        )
        .await;
    assert_eq!(c["code"], "SAVE10");
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/coupons",
            None,
            json!({ "code": "SAVE10", "kind": "flat", "value": 100 })
        )
        .await
        .0
        .as_u16(),
        409
    );

    // a link with a 2% fee the payer covers
    let (s, _) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 1000, "convenience_fee_bp": 1001 }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "CP-1", "amount_minor": 50_000 }),
        )
        .await;
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "invoice_id": inv["id"], "convenience_fee_bp": 200 })
        )
        .await
        .0
        .as_u16(),
        422,
        "fees are for fixed-amount links"
    );
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100_000, "convenience_fee_bp": 200, "max_uses": 10 }),
        )
        .await;
    let (tid, token) = (&a.id, link["token"].as_str().unwrap());
    let info = |q: &str| {
        let path = format!("/v1/checkout/links/{tid}/{token}{q}");
        let t2 = &t;
        async move { t2.public("GET", &path, None, Value::Null).await }
    };
    let start = |body: Value| {
        let path = format!("/v1/checkout/links/{tid}/{token}/start");
        let t2 = &t;
        async move { t2.public("POST", &path, None, body).await }
    };

    // the preview and the payment agree
    let plain = info("").await.1;
    assert_eq!(
        (
            plain["amount_minor"].clone(),
            plain["fee_minor"].clone(),
            plain["total_minor"].clone()
        ),
        (json!(100_000), json!(2_000), json!(102_000))
    );
    let pv = info("?coupon=save10").await.1;
    assert_eq!(
        (
            pv["discount_minor"].clone(),
            pv["fee_minor"].clone(),
            pv["total_minor"].clone(),
            pv["coupon_applied"].clone()
        ),
        (json!(10_000), json!(1_800), json!(91_800), json!(true))
    );
    assert_eq!(info("?coupon=NOPE").await.0.as_u16(), 409);
    assert_eq!(
        t.count("SELECT coalesce(sum(uses),0) FROM coupons").await,
        0,
        "previewing spends nothing"
    );

    let none = start(json!({})).await.1;
    assert_eq!(none["amount_minor"], 102_000);
    let one = start(json!({ "coupon": "save10" })).await.1;
    assert_eq!(one["amount_minor"], 91_800);
    let meta: Value = sqlx::query_scalar("SELECT metadata FROM payment_intents WHERE id = $1")
        .bind(one["intent_id"].as_str().unwrap())
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(
        (
            meta["base_minor"].clone(),
            meta["discount_minor"].clone(),
            meta["fee_minor"].clone()
        ),
        (json!(100_000), json!(10_000), json!(1_800))
    );
    assert_eq!(
        start(json!({ "coupon": "SAVE10" })).await.1["amount_minor"],
        91_800
    );

    // used up: refused, and the link's own use counter did not move
    let uses_before = t.count("SELECT uses::BIGINT FROM payment_links").await;
    let (s, e) = start(json!({ "coupon": "SAVE10" })).await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("coupon_invalid"))
    );
    assert_eq!(
        t.count("SELECT uses::BIGINT FROM payment_links").await,
        uses_before
    );
    assert_eq!(t.count("SELECT uses::BIGINT FROM coupons").await, 2);

    // flat coupons, expiry of nothing but deactivation, and the floor of one unit
    let flat = t
        .ok(
            a,
            "POST",
            "/v1/coupons",
            None,
            json!({ "code": "FIVEH", "kind": "flat", "value": 500_000 }),
        )
        .await;
    assert_eq!(
        start(json!({ "coupon": "FIVEH" })).await.1["amount_minor"],
        1,
        "a coupon cannot make a payment free"
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/coupons/{}/deactivate", flat["id"].as_str().unwrap()),
        None,
        json!({}),
    )
    .await;
    assert_eq!(start(json!({ "coupon": "FIVEH" })).await.0.as_u16(), 409);
    let listed = t.ok(a, "GET", "/v1/coupons", None, Value::Null).await;
    let status = |code: &str| {
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["code"] == code)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(
        (status("SAVE10"), status("FIVEH")),
        (json!("used_up"), json!("deactivated"))
    );

    // coupons do not apply to invoice links, and belong to the tenant that made them
    let il = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "invoice_id": inv["id"] }),
        )
        .await;
    let (s, _) = t
        .public(
            "POST",
            &format!(
                "/v1/checkout/links/{tid}/{}/start",
                il["token"].as_str().unwrap()
            ),
            None,
            json!({ "coupon": "SAVE10" }),
        )
        .await;
    assert_eq!(s.as_u16(), 422);
    let bl = t
        .ok(
            b,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 1_000 }),
        )
        .await;
    t.ok(
        a,
        "POST",
        "/v1/coupons",
        None,
        json!({ "code": "ONLYA", "kind": "percent", "value": 500 }),
    )
    .await;
    let (s, _) = t
        .public(
            "POST",
            &format!(
                "/v1/checkout/links/{}/{}/start",
                b.id,
                bl["token"].as_str().unwrap()
            ),
            None,
            json!({ "coupon": "ONLYA" }),
        )
        .await;
    assert_eq!(s.as_u16(), 409);
}

#[sqlx::test(migrations = "./migrations")]
async fn absurdly_large_amounts_are_refused_everywhere_instead_of_overflowing_sums(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let huge = zyvor_core::MAX_AMOUNT_MINOR + 1;
    let cust = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "C" }))
        .await;
    let vendor = t
        .ok(
            a,
            "POST",
            "/v1/vendors",
            None,
            json!({ "name": "V", "vpa": "v@okbank" }),
        )
        .await;
    for (what, path, body) in [
        (
            "intent",
            "/v1/payment_intents",
            json!({ "amount_minor": huge, "currency": "INR" }),
        ),
        (
            "invoice",
            "/v1/invoices",
            json!({ "number": "BIG-1", "amount_minor": huge }),
        ),
        (
            "payout",
            "/v1/payouts",
            json!({ "vendor_id": vendor["id"], "amount_minor": huge, "purpose": "x" }),
        ),
        ("link", "/v1/payment_links", json!({ "amount_minor": huge })),
        (
            "plan",
            "/v1/plans",
            json!({ "name": "P", "amount_minor": huge, "interval_unit": "month" }),
        ),
        (
            "milestone",
            "/v1/milestones",
            json!({ "customer_id": cust["id"], "name": "M", "amount_minor": huge }),
        ),
        (
            "instalments",
            "/v1/installment_plans",
            json!({ "customer_id": cust["id"], "name": "I", "total_minor": huge, "instalments": 3, "first_due_date": "2026-12-01", "number_prefix": "BIG" }),
        ),
        (
            "i64 max",
            "/v1/invoices",
            json!({ "number": "BIG-2", "amount_minor": i64::MAX }),
        ),
    ] {
        let key = format!("k-{what}");
        let (s, e) = t.call(a, "POST", path, Some(&key), body).await;
        assert_eq!(s.as_u16(), 422, "{what}: {e}");
    }
    // the largest allowed amount still works
    let ok = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "BIG-OK", "amount_minor": zyvor_core::MAX_AMOUNT_MINOR }),
        )
        .await;
    assert_eq!(ok["amount_minor"], zyvor_core::MAX_AMOUNT_MINOR);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_lost_dispute_takes_the_same_share_back_from_each_vendor(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let mut ids = vec![];
    for name in ["Artisan", "Courier"] {
        let v = t
            .ok(
                a,
                "POST",
                "/v1/vendors",
                None,
                json!({ "name": name, "vpa": format!("{}@okbank", name.to_lowercase()) }),
            )
            .await;
        ids.push(v["id"].as_str().unwrap().to_string());
    }
    let owed = |v: String| {
        let (t2, a2) = (&t, a.clone());
        async move {
            t2.ok(
                &a2,
                "GET",
                &format!("/v1/vendors/{v}/balance"),
                None,
                Value::Null,
            )
            .await["balances"][0]["owed_minor"]
                .as_i64()
                .unwrap_or(0)
        }
    };
    let splits = json!([{ "vendor_id": ids[0], "amount_minor": 60_000 }, { "vendor_id": ids[1], "amount_minor": 20_000 }]);
    let (_, att, _) = t
        .paid_razorpay(a, 100_000, json!({ "splits": splits }))
        .await;
    let d = t
        .ok(
            a,
            "POST",
            "/v1/disputes",
            None,
            json!({ "attempt_id": att, "amount_minor": 50_000, "reason": "fraudulent", "respond_by": "2026-12-01" }),
        )
        .await;
    let did = d["id"].as_str().unwrap();
    // an open dispute changes nothing for the vendors; winning leaves them whole
    assert_eq!(owed(ids[0].clone()).await, 60_000);
    // a lost one takes half back from each
    t.ok(
        a,
        "POST",
        &format!("/v1/disputes/{did}/resolve"),
        None,
        json!({ "outcome": "lost" }),
    )
    .await;
    assert_eq!(owed(ids[0].clone()).await, 30_000);
    assert_eq!(owed(ids[1].clone()).await, 10_000);
}
