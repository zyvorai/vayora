// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! School fees, society dues and donations, built on invoices, instalments and payment intents. All against the
//! simulators; no real provider, money, student or donor. Names below are invented.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Days, NaiveDate};
use common::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

fn days_from_now(n: i64) -> NaiveDate {
    let t = zyvor_api::ist_today();
    if n >= 0 {
        t.checked_add_days(Days::new(n as u64)).unwrap()
    } else {
        t.checked_sub_days(Days::new((-n) as u64)).unwrap()
    }
}

async fn get_bytes(t: &T, tn: &Tenant, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {}", tn.key))
        .body(Body::empty())
        .unwrap();
    let res = t.app.clone().oneshot(req).await.unwrap();
    let (s, h) = (res.status(), res.headers().clone());
    (
        s,
        h,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

fn has(bytes: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(bytes).contains(needle)
}

const ROSTER: &str = "Class,Roll No,Student Name,Guardian,Guardian Email,Guardian Phone\nClass 5,101,Aarav Shah,Neha Shah,neha@example.com,9876500001\nClass 5,102,Diya Rao,Kiran Rao,,9876500002\n";

/// A school with one class, two fee heads (60000 + 15000 paise) and two students with guardians.
async fn school(t: &T, tn: &Tenant) -> String {
    let g = t
        .ok(
            tn,
            "POST",
            "/v1/dues/school/groups",
            None,
            json!({ "name": "Class 5" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    for (n, a) in [("Tuition", 60_000), ("Transport", 15_000)] {
        t.ok(
            tn,
            "POST",
            "/v1/dues/school/heads",
            None,
            json!({ "group_id": g, "name": n, "amount_minor": a }),
        )
        .await;
    }
    let r = t
        .ok(
            tn,
            "POST",
            "/v1/dues/school/members/import",
            None,
            json!({ "csv": ROSTER }),
        )
        .await;
    assert_eq!(
        (r["saved"].as_bool(), r["created"].as_i64()),
        (Some(true), Some(2)),
        "{r}"
    );
    g
}

async fn pay(t: &T, tn: &Tenant, intent_id: &str) {
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(intent_id)
        .execute(&t.root)
        .await
        .unwrap();
    let att = t
        .ok(
            tn,
            "POST",
            &format!("/v1/payment_intents/{intent_id}/attempts"),
            Some(&format!("att-{intent_id}")),
            json!({ "method": "upi" }),
        )
        .await;
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "captured" }),
        )
        .await;
    assert_eq!(
        t.deliver("razorpay", &tn.id, &paid["webhook"]).await.0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn school_fees_flow_from_roster_to_receipt(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let g = school(&t, a).await;

    // the roster is re-importable: same references update, nothing duplicates
    let again = t
        .ok(
            a,
            "POST",
            "/v1/dues/school/members/import",
            None,
            json!({ "csv": ROSTER.replace("Diya Rao", "Diya R. Rao") }),
        )
        .await;
    assert_eq!(
        (again["created"].as_i64(), again["updated"].as_i64()),
        (Some(0), Some(2))
    );
    // a student with no payer cannot be billed
    t.ok(
        a,
        "POST",
        "/v1/dues/school/members",
        None,
        json!({ "group": "Class 5", "name": "Ishaan Mehta", "member_ref": "103" }),
    )
    .await;

    let plan = json!({ "label": "2026-27 Term 1", "group_id": g, "instalments": 3, "first_due_date": days_from_now(-10).to_string(), "number_prefix": "FEE26T1" });
    let out = t
        .ok(a, "POST", "/v1/dues/school/assign", None, plan.clone())
        .await;
    assert_eq!(
        (
            out["created"].as_i64(),
            out["skipped_no_payer"].as_i64(),
            out["already_assigned"].as_i64()
        ),
        (Some(2), Some(1), Some(0))
    );
    let out = t
        .ok(a, "POST", "/v1/dues/school/assign", None, plan.clone())
        .await;
    assert_eq!(
        (out["created"].as_i64(), out["already_assigned"].as_i64()),
        (Some(0), Some(2)),
        "safe to repeat"
    );
    // the same numbers under another label are refused, and nothing is half-made
    let (s, e) = t.call(a, "POST", "/v1/dues/school/assign", None, json!({ "label": "Term 2", "group_id": g, "instalments": 2, "first_due_date": days_from_now(60).to_string(), "number_prefix": "FEE26T1" })).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("number_taken"))
    );
    assert_eq!(t.count("SELECT count(*) FROM dues_assignments").await, 2);

    let rep = t
        .ok(a, "GET", "/v1/dues/school/report", None, Value::Null)
        .await;
    assert_eq!(
        (
            rep["totals"]["billed_minor"].as_i64(),
            rep["totals"]["paid_minor"].as_i64(),
            rep["totals"]["overdue_minor"].as_i64()
        ),
        (Some(150_000), Some(0), Some(50_000)),
        "{rep}"
    );
    let aarav = rep["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["member_ref"] == "101")
        .unwrap()
        .clone();
    let invs = aarav["assignments"][0]["invoices"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        invs.iter()
            .map(|i| i["amount_minor"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![25_000; 3]
    );
    assert_eq!(invs[0]["number"], "FEE26T1-101-1");

    // the guardian pays the first instalment through the ordinary flow
    let inv0 = invs[0]["invoice_id"].as_str().unwrap();
    let cust: String = sqlx::query_scalar("SELECT customer_id FROM invoices WHERE id = $1")
        .bind(inv0)
        .fetch_one(&t.root)
        .await
        .unwrap();
    t.paid_razorpay(
        a,
        25_000,
        json!({ "invoice_id": inv0, "customer_id": cust }),
    )
    .await;
    let rep = t
        .ok(a, "GET", "/v1/dues/school/report", None, Value::Null)
        .await;
    assert_eq!(
        (
            rep["totals"]["paid_minor"].as_i64(),
            rep["totals"]["due_minor"].as_i64(),
            rep["totals"]["overdue_minor"].as_i64()
        ),
        (Some(25_000), Some(125_000), Some(25_000))
    );
    let due_only = t
        .ok(
            a,
            "GET",
            &format!("/v1/dues/school/report?status=due&group_id={g}&label=2026-27%20Term%201"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(due_only["data"].as_array().unwrap().len(), 2);
    let csv = get_bytes(&t, a, "/v1/dues/school/report.csv").await;
    assert!(
        has(&csv.2, "group,member_ref,name,billed,paid,due,overdue\n")
            && has(&csv.2, "Class 5,101,Aarav Shah,750.00,250.00,500.00,0.00"),
        "{}",
        String::from_utf8_lossy(&csv.2)
    );
    assert_eq!(csv.1["content-type"], "text/csv; charset=utf-8");

    // receipt: only for money received, with the school's own wording
    let (s, e) = t
        .call(
            a,
            "GET",
            &format!(
                "/v1/dues/school/invoices/{}/receipt.pdf",
                invs[1]["invoice_id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("nothing_paid"))
    );
    t.ok(
        a,
        "PUT",
        "/v1/dues/school/settings",
        None,
        json!({ "receipt_note": "Sunrise Public School, affiliation 000000 (example)" }),
    )
    .await;
    let (s, h, pdf) = get_bytes(
        &t,
        a,
        &format!("/v1/dues/school/invoices/{inv0}/receipt.pdf"),
    )
    .await;
    assert_eq!(
        (s, h["content-type"].to_str().unwrap()),
        (StatusCode::OK, "application/pdf")
    );
    assert_eq!(h["x-content-type-options"], "nosniff");
    for needle in [
        "%PDF-1.4",
        "Fee receipt",
        "RC-FEE26T1-101-1",
        "Student: Aarav Shah",
        "Class: Class 5",
        "instalment 1 of 3",
        "Rs.250.00",
        "Balance: Rs.0.00",
        "Sunrise Public School",
    ] {
        assert!(has(&pdf, needle), "{needle} missing");
    }

    // reminders: due or overdue, once a day, counts only; no email means no mail
    let r = t
        .ok(
            a,
            "POST",
            "/v1/dues/school/reminders",
            None,
            json!({ "days_before": 0 }),
        )
        .await;
    assert_eq!(
        (r["queued"].as_i64(), r["skipped_no_email"].as_i64()),
        (Some(0), Some(1)),
        "Diya's guardian has no email"
    );
    t.ok(a, "POST", "/v1/dues/school/members", None, json!({ "group": "Class 5", "name": "Diya R. Rao", "member_ref": "102", "payer_name": "Kiran Rao", "payer_email": "kiran@example.com" })).await;
    let r = t
        .ok(
            a,
            "POST",
            "/v1/dues/school/reminders",
            None,
            json!({ "days_before": 0 }),
        )
        .await;
    assert_eq!(r["queued"], 1);
    assert_eq!(
        t.ok(
            a,
            "POST",
            "/v1/dues/school/reminders",
            None,
            json!({ "days_before": 0 })
        )
        .await["skipped_already_sent_today"],
        1
    );
    let (to, body): (String, String) = sqlx::query_as(
        "SELECT to_email, body FROM mail_queue WHERE to_email = 'kiran@example.com'",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert!(
        to.contains('@') && body.contains("Diya R. Rao") && body.contains("FEE26T1-102-1"),
        "{body}"
    );

    // names and contacts are not written to the audit trail
    assert_eq!(t.count("SELECT count(*) FROM audit_events WHERE detail::text ILIKE '%Aarav%' OR detail::text ILIKE '%neha@%' OR detail::text ILIKE '%98765%'").await, 0);

    // a single-invoice assignment for the next term
    let one = t.ok(a, "POST", "/v1/dues/school/assign", None, json!({ "label": "Annual day", "member_ids": [aarav["id"]], "head_ids": [], "instalments": 1, "first_due_date": days_from_now(30).to_string(), "number_prefix": "ANN26" })).await;
    assert_eq!(
        one["skipped_no_fee_heads"], 1,
        "an empty head list means nothing to bill"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn roster_files_are_checked_whole_and_exports_cannot_run_as_formulas(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let bad = "Class,Roll No,Student Name\nClass 5,101,Aarav Shah\nClass 5,101,Duplicate Dev\nClass 5,123456789012,Looks Like An Id\n,104,No Class\nClass 5,105,\n";
    let r = t
        .ok(
            a,
            "POST",
            "/v1/dues/school/members/import",
            None,
            json!({ "csv": bad }),
        )
        .await;
    assert_eq!(r["saved"], false);
    let lines: Vec<i64> = r["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["line"].as_i64().unwrap())
        .collect();
    assert_eq!(lines, vec![3, 4, 5, 6], "{r}");
    assert_eq!(
        t.count("SELECT count(*) FROM dues_members").await,
        0,
        "a file with problems saves nothing"
    );
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/dues/school/members/import",
            None,
            json!({ "csv": "Name,Roll\nA,1\n" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("missing_column"))
    );
    let dry = t
        .ok(
            a,
            "POST",
            "/v1/dues/school/members/import",
            None,
            json!({ "csv": "Class;Roll;Name\nClass 1;7;\"=Cmd, Student\"\n", "dry_run": true }),
        )
        .await;
    assert_eq!(
        (dry["dry_run"].as_bool(), dry["rows"].as_i64()),
        (Some(true), Some(1)),
        "semicolons are detected"
    );
    assert_eq!(t.count("SELECT count(*) FROM dues_members").await, 0);
    t.ok(
        a,
        "POST",
        "/v1/dues/school/members/import",
        None,
        json!({ "csv": "Class;Roll;Name\nClass 1;7;\"=Cmd, Student\"\n" }),
    )
    .await;
    let exp = get_bytes(&t, a, "/v1/dues/school/members.csv").await;
    assert!(
        has(&exp.2, "Class 1,7,\"'=Cmd, Student\""),
        "{}",
        String::from_utf8_lossy(&exp.2)
    );
    // unknown template
    let (s, _) = t
        .call(a, "GET", "/v1/dues/hospital/groups", None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // a group needs a unique name
    t.ok(
        a,
        "POST",
        "/v1/dues/school/groups",
        None,
        json!({ "name": "Class 9" }),
    )
    .await;
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/dues/school/groups",
            None,
            json!({ "name": "Class 9" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("duplicate"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn society_dues_use_the_same_engine_and_businesses_stay_apart(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let g = t
        .ok(
            a,
            "POST",
            "/v1/dues/society/groups",
            None,
            json!({ "name": "Block A" }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(
        a,
        "POST",
        "/v1/dues/society/heads",
        None,
        json!({ "group_id": g, "name": "Maintenance", "amount_minor": 250_000 }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/dues/society/members/import",
        None,
        json!({ "csv": "Block,Flat,Resident,Email\nBlock A,A-101,R. Nair,nair@example.com\n" }),
    )
    .await;
    // the same reference in the other kind is a different member; in another business it is that business's own
    t.ok(
        a,
        "POST",
        "/v1/dues/school/members",
        None,
        json!({ "group": "Class 1", "name": "Same Ref", "member_ref": "A-101" }),
    )
    .await;
    let out = t.ok(a, "POST", "/v1/dues/society/assign", None, json!({ "label": "Q1 2026", "group_id": g, "instalments": 1, "first_due_date": days_from_now(5).to_string(), "number_prefix": "MNT26Q1" })).await;
    assert_eq!(
        out["skipped_no_payer"], 1,
        "a resident with an email but no payer name has nobody to bill"
    );
    t.ok(a, "POST", "/v1/dues/society/members", None, json!({ "group": "Block A", "name": "R. Nair", "member_ref": "A-101", "payer_name": "R. Nair", "payer_email": "nair@example.com" })).await;
    let out = t.ok(a, "POST", "/v1/dues/society/assign", None, json!({ "label": "Q1 2026", "group_id": g, "instalments": 1, "first_due_date": days_from_now(5).to_string(), "number_prefix": "MNT26Q1" })).await;
    assert_eq!(out["created"], 1);
    let rep = t
        .ok(a, "GET", "/v1/dues/society/report", None, Value::Null)
        .await;
    assert_eq!(
        (
            rep["totals"]["billed_minor"].as_i64(),
            rep["data"][0]["assignments"][0]["invoices"][0]["number"].as_str()
        ),
        (Some(250_000), Some("MNT26Q1-A-101-1"))
    );
    let inv = rep["data"][0]["assignments"][0]["invoices"][0]["invoice_id"]
        .as_str()
        .unwrap()
        .to_string();
    let cust: String = sqlx::query_scalar("SELECT customer_id FROM invoices WHERE id = $1")
        .bind(&inv)
        .fetch_one(&t.root)
        .await
        .unwrap();
    t.paid_razorpay(
        a,
        250_000,
        json!({ "invoice_id": inv, "customer_id": cust }),
    )
    .await;
    let (s, h, pdf) = get_bytes(
        &t,
        a,
        &format!("/v1/dues/society/invoices/{inv}/receipt.pdf"),
    )
    .await;
    assert_eq!(
        (s, h["content-type"].to_str().unwrap()),
        (StatusCode::OK, "application/pdf")
    );
    assert!(
        has(&pdf, "(Receipt) Tj") && has(&pdf, "Member: R. Nair") && has(&pdf, "Block: Block A")
    );
    // the receipt of a society invoice cannot be fetched as a school receipt
    let (s, _) = t
        .call(
            a,
            "GET",
            &format!("/v1/dues/school/invoices/{inv}/receipt.pdf"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // tenant isolation
    assert_eq!(
        t.ok(b, "GET", "/v1/dues/society/groups", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/dues/society/members", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/dues/society/report", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (s, _) = t
        .call(
            b,
            "GET",
            &format!("/v1/dues/society/invoices/{inv}/receipt.pdf"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "POST",
            "/v1/dues/society/heads",
            None,
            json!({ "group_id": g, "name": "Theft", "amount_minor": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t.call(b, "POST", "/v1/dues/society/assign", None, json!({ "label": "x", "group_id": g, "instalments": 1, "first_due_date": days_from_now(5).to_string(), "number_prefix": "X" })).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let own = t
        .ok(
            b,
            "POST",
            "/v1/dues/society/members",
            None,
            json!({ "group": "Block A", "name": "Other Resident", "member_ref": "A-101" }),
        )
        .await;
    assert_eq!(own["created"], true, "references are per business");
    // scopes
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "reader", "scopes": ["verticals:read"] }),
        )
        .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "GET",
            "/v1/dues/society/report",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "POST",
            "/v1/dues/society/groups",
            None,
            json!({ "name": "Block Z" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "POST",
            "/v1/dues/society/reminders",
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

async fn campaign(t: &T, tn: &Tenant) -> String {
    t.ok(tn, "POST", "/v1/donations/campaigns", None, json!({
        "name": "Winter blankets", "description": "Blankets for the night shelter.", "suggested_minor": [10_000, 50_000, 100_000],
        "min_minor": 5_000, "max_minor": 5_000_000, "goal_minor": 1_000_000,
        "receipt_fields": { "registration_text": "Trust reg. no. EX-0001 (example)", "exemption_text": "Wording chosen by the trust", "receipt_note": "Thank you" }
    })).await["id"].as_str().unwrap().to_string()
}

#[sqlx::test(migrations = "./migrations")]
async fn donations_from_start_to_receipt_and_export(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    for (bad, code) in [
        (
            json!({ "name": "x", "suggested_minor": [100], "min_minor": 5_000 }),
            "invalid_amounts",
        ),
        (
            json!({ "name": "x", "min_minor": 9_000, "max_minor": 5_000 }),
            "invalid_amounts",
        ),
        (
            json!({ "name": "x", "receipt_fields": { "receipt_note": "n".repeat(301) } }),
            "text_too_long",
        ),
    ] {
        let (s, e) = t
            .call(a, "POST", "/v1/donations/campaigns", None, bad.clone())
            .await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some(code)),
            "{bad}"
        );
    }
    let c = campaign(&t, a).await;
    let info = t
        .ok_public("GET", &format!("/v1/donate/{}/{c}", a.id), Value::Null)
        .await;
    assert_eq!(
        (
            info["name"].as_str(),
            info["suggested_minor"][1].as_i64(),
            info["min_minor"].as_i64()
        ),
        (Some("Winter blankets"), Some(50_000), Some(5_000))
    );
    assert!(
        info.get("receipt_fields").is_none(),
        "receipt wording is not part of the public page"
    );

    let start_path = format!("/v1/donate/{}/{c}/start", a.id);
    let start = |body: Value| t.public("POST", &start_path, None, body);
    let (s, e) = start(json!({ "amount_minor": 100 })).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("amount_out_of_range")
        )
    );
    let (s, e) = start(
        json!({ "amount_minor": 50_000, "donor_name": "Meera Kulkarni", "donor_email": "nope" }),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_email"))
    );
    let (s, named) = start(json!({ "amount_minor": 50_000, "donor_name": "=Meera K", "donor_email": "meera@example.com" })).await;
    assert_eq!(s, StatusCode::CREATED, "{named}");
    let (_, anon) =
        start(json!({ "amount_minor": 20_000, "anonymous": true, "donor_name": "Ignored Name" }))
            .await;
    let (_, unpaid) = start(json!({ "amount_minor": 10_000, "donor_name": "Never Paid" })).await;
    // someone else's tenant id with this campaign is simply not found
    let (s, _) = t
        .public(
            "POST",
            &format!("/v1/donate/{}/{c}/start", b.id),
            None,
            json!({ "amount_minor": 50_000 }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    pay(&t, a, named["intent_id"].as_str().unwrap()).await;
    pay(&t, a, anon["intent_id"].as_str().unwrap()).await;

    let list = t
        .ok(
            a,
            "GET",
            &format!("/v1/donations?campaign_id={c}"),
            None,
            Value::Null,
        )
        .await;
    let rows = list["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let by = |id: &Value| rows.iter().find(|r| r["id"] == *id).unwrap().clone();
    let (n, an, up) = (
        by(&named["donation_id"]),
        by(&anon["donation_id"]),
        by(&unpaid["donation_id"]),
    );
    assert_eq!(
        (n["status"].as_str(), up["status"].as_str()),
        (Some("paid"), Some("pending"))
    );
    assert!(n["receipt_no"].as_str().unwrap().starts_with("DN"));
    assert_eq!(
        (an["anonymous"].as_bool(), an["donor"].is_null()),
        (Some(true), true),
        "an anonymous donor is stored with no name"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM donations WHERE donor_name = 'Ignored Name'")
            .await,
        0
    );

    let (s, h, pdf) = get_bytes(
        &t,
        a,
        &format!(
            "/v1/donations/{}/receipt.pdf",
            named["donation_id"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(
        (s, h["content-type"].to_str().unwrap()),
        (StatusCode::OK, "application/pdf")
    );
    for needle in [
        "Donation receipt",
        "Received with thanks from: =Meera K",
        "Towards: Winter blankets",
        "Amount: Rs.500.00",
        "Trust reg. no. EX-0001",
        "Wording chosen by the trust",
        "Thank you",
        "gives no tax advice",
    ] {
        assert!(has(&pdf, needle), "{needle} missing");
    }
    for (id, why) in [
        (&anon["donation_id"], "anonymous"),
        (&unpaid["donation_id"], "unpaid"),
    ] {
        let (s, e) = t
            .call(
                a,
                "GET",
                &format!("/v1/donations/{}/receipt.pdf", id.as_str().unwrap()),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::CONFLICT, Some("no_receipt")),
            "{why}"
        );
    }
    // donor-supplied text never reaches the receipt wording, and the merchant's own wording can be changed
    t.ok(
        a,
        "PATCH",
        &format!("/v1/donations/campaigns/{c}"),
        None,
        json!({ "receipt_fields": { "receipt_note": "Updated note" } }),
    )
    .await;
    let (_, _, pdf) = get_bytes(
        &t,
        a,
        &format!(
            "/v1/donations/{}/receipt.pdf",
            named["donation_id"].as_str().unwrap()
        ),
    )
    .await;
    assert!(has(&pdf, "Updated note") && !has(&pdf, "Trust reg. no."));

    let sum = t
        .ok(
            a,
            "GET",
            &format!("/v1/donations/campaigns/{c}/summary"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            sum["paid_count"].as_i64(),
            sum["paid_minor"].as_i64(),
            sum["pending_count"].as_i64(),
            sum["goal_minor"].as_i64()
        ),
        (Some(2), Some(70_000), Some(1), Some(1_000_000))
    );
    // a refund through the ordinary flow shows in the summary and the export
    t.ok(
        a,
        "POST",
        &format!(
            "/v1/payment_intents/{}/refunds",
            anon["intent_id"].as_str().unwrap()
        ),
        Some("rf-1"),
        json!({}),
    )
    .await;
    let sum = t
        .ok(
            a,
            "GET",
            &format!("/v1/donations/campaigns/{c}/summary"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (sum["refunded_minor"].as_i64(), sum["net_minor"].as_i64()),
        (Some(20_000), Some(50_000))
    );
    let csv = get_bytes(&t, a, "/v1/donations.csv").await;
    let text = String::from_utf8(csv.2).unwrap();
    assert!(text.starts_with(
        "receipt_no,paid_on,campaign_id,amount,refunded,donor_name,donor_email,donor_phone\n"
    ));
    assert!(
        text.contains("'=Meera K,meera@example.com") && text.contains(",500.00,0.00,"),
        "{text}"
    );
    assert!(
        !text.contains("Never Paid"),
        "only paid donations are exported"
    );

    // a closed campaign takes nothing more
    t.ok(
        a,
        "PATCH",
        &format!("/v1/donations/campaigns/{c}"),
        None,
        json!({ "status": "closed" }),
    )
    .await;
    assert_eq!(
        start(json!({ "amount_minor": 50_000 })).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/donate/{}/{c}", a.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    // isolation and scopes
    assert_eq!(
        t.ok(b, "GET", "/v1/donations/campaigns", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/donations", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    for p in [
        format!(
            "/v1/donations/{}/receipt.pdf",
            named["donation_id"].as_str().unwrap()
        ),
        format!("/v1/donations/campaigns/{c}/summary"),
    ] {
        assert_eq!(
            t.call(b, "GET", &p, None, Value::Null).await.0,
            StatusCode::NOT_FOUND,
            "{p}"
        );
    }
    assert_eq!(
        t.call(
            b,
            "PATCH",
            &format!("/v1/donations/campaigns/{c}"),
            None,
            json!({ "name": "stolen" })
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let ro = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "reader", "scopes": ["verticals:read"] }),
        )
        .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        raw(&t.app, &ro, None, "GET", "/v1/donations", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "POST",
            "/v1/donations/campaigns",
            None,
            json!({ "name": "x" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // donor details are not in the audit trail
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE detail::text ILIKE '%meera%'")
            .await,
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_recurring_donor_is_a_mandate_the_donor_approves(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let c = campaign(&t, a).await;
    t.state.sim.set_manual_payer(true);
    t.ok(
        a,
        "POST",
        "/v1/provider_accounts",
        None,
        json!({ "provider": "sim", "credentials": {}, "webhook_secret": "test-only-placeholder" }),
    )
    .await;
    let body = json!({ "donor_name": "Meera Kulkarni", "donor_phone": "9876500003", "amount_minor": 50_000, "frequency": "monthly", "end_date": days_from_now(365).to_string(), "provider": "sim" });
    let (s, e) = t
        .call(
            a,
            "POST",
            &format!("/v1/donations/campaigns/{c}/pledges"),
            Some("p-0"),
            body.clone(),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("recurring_not_enabled"))
    );
    t.ok(
        a,
        "PATCH",
        &format!("/v1/donations/campaigns/{c}"),
        None,
        json!({ "recurring_enabled": true }),
    )
    .await;
    let (s, e) = t
        .call(
            a,
            "POST",
            &format!("/v1/donations/campaigns/{c}/pledges"),
            None,
            body.clone(),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("idempotency_key_required"))
    );
    let mut low = body.clone();
    low["amount_minor"] = json!(10);
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/donations/campaigns/{c}/pledges"),
            Some("p-low"),
            low
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let p = t
        .ok(
            a,
            "POST",
            &format!("/v1/donations/campaigns/{c}/pledges"),
            Some("p-1"),
            body,
        )
        .await;
    assert_eq!(p["mandate"]["status"], "pending_authorization");
    assert!(
        p["mandate"]["authorization"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("upi://mandate?")
    );
    let m = t
        .ok(
            a,
            "GET",
            &format!("/v1/mandates/{}", p["mandate"]["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(m["mandate"]["max_amount_minor"], 50_000);
    let sum = t
        .ok(
            a,
            "GET",
            &format!("/v1/donations/campaigns/{c}/summary"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(sum["recurring_pledges"], 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_test_data_reset_clears_orders_donations_and_rosters_but_keeps_the_setup(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // a school with a billed student, a donation and a storefront order, all made in test mode
    let g = school(&t, a).await;
    t.ok(a, "POST", "/v1/dues/school/assign", None, json!({ "label": "Term 1", "group_id": g, "instalments": 2, "first_due_date": days_from_now(5).to_string(), "number_prefix": "R1" })).await;
    let c = campaign(&t, a).await;
    t.ok_public(
        "POST",
        &format!("/v1/donate/{}/{c}/start", a.id),
        json!({ "amount_minor": 50_000, "donor_name": "Test Donor" }),
    )
    .await;
    t.ok(
        a,
        "PUT",
        "/v1/storefront",
        None,
        json!({ "slug": "reset-shop", "title": "Reset shop", "published": true }),
    )
    .await;
    let item = t
        .ok(
            a,
            "POST",
            "/v1/storefront/items",
            None,
            json!({ "name": "Thing", "price_minor": 1_000, "stock": 5 }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.ok_public("POST", "/v1/store/reset-shop/orders", json!({ "buyer_name": "Test Buyer", "buyer_phone": "9876543210", "items": [{ "item_id": item, "quantity": 1 }] })).await;

    let r = t
        .ok(
            a,
            "POST",
            "/v1/test_data/reset",
            None,
            json!({ "confirm": "reset test data" }),
        )
        .await;
    assert_eq!(r["ok"], true, "{r}");
    for (table, left) in [
        ("storefront_orders", 0),
        ("storefront_order_items", 0),
        ("donations", 0),
        ("dues_members", 0),
        ("dues_assignments", 0),
        ("dues_assignment_invoices", 0),
        ("invoices", 0),
        ("storefronts", 1),
        ("storefront_items", 1),
        ("dues_groups", 1),
        ("dues_heads", 2),
        ("donation_campaigns", 1),
    ] {
        assert_eq!(
            t.count(&format!("SELECT count(*) FROM {table}")).await,
            left,
            "{table}"
        );
    }
}
