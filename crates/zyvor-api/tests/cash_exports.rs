// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Cash visibility, refund analytics and accounting exports. Simulators and hand-built synthetic histories only:
//! every amount, reference, date and rate here is made up, nothing ran against a real provider or bank.

mod common;

use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use chrono::{Duration, NaiveDate};
use common::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

fn today() -> NaiveDate {
    zyvor_api::ist_today()
}

fn dmy(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

/// A GET whose body is text (CSV or XML): the status, headers and body.
async fn get_text(t: &T, tn: &Tenant, path: &str) -> (StatusCode, HeaderMap, String) {
    let res = t
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {}", tn.key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn key_with(t: &T, scopes: &[&str]) -> Tenant {
    let k = t
        .ok(
            &t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "scoped", "scopes": scopes }),
        )
        .await;
    Tenant {
        id: t.a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    }
}

/// A made-up captured payment (and, when `settle_id` is given, its place in a settlement) written straight to the
/// tables, so a test can choose the dates.
async fn synth_payment(
    t: &T,
    tenant: &str,
    n: &str,
    provider: &str,
    method: &str,
    amount: i64,
    captured_on: NaiveDate,
) -> String {
    let iid = format!("pi_syn{n}");
    let aid = format!("att_syn{n}");
    let at = captured_on.and_hms_opt(12, 0, 0).unwrap() - Duration::minutes(330);
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status) VALUES ($1, $2, $3, 'INR', 'succeeded')")
        .bind(&iid).bind(tenant).bind(amount).execute(&t.root).await.unwrap();
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, 'captured', $6, 'INR', '{}', $7, $7)")
        .bind(&aid).bind(tenant).bind(&iid).bind(provider).bind(method).bind(amount).bind(at).execute(&t.root).await.unwrap();
    aid
}

async fn synth_settlement(
    t: &T,
    tenant: &str,
    id: &str,
    provider: &str,
    settled_on: NaiveDate,
    items: &[(String, i64)],
) {
    let gross: i64 = items.iter().map(|i| i.1).sum();
    sqlx::query("INSERT INTO settlements (id, tenant_id, provider, provider_ref, currency, gross_minor, refunds_minor, fee_minor, adjustment_minor, net_minor, settled_on) VALUES ($1, $2, $3, $1, 'INR', $4, 0, 0, 0, $4, $5)")
        .bind(id).bind(tenant).bind(provider).bind(gross).bind(settled_on).execute(&t.root).await.unwrap();
    for (aid, amt) in items {
        sqlx::query("INSERT INTO settlement_items (tenant_id, settlement_id, kind, provider_ref, attempt_id, amount_minor) VALUES ($1, $2, 'payment', $3, $4, $5)")
            .bind(tenant).bind(id).bind(aid).bind(aid).bind(amt).execute(&t.root).await.unwrap();
    }
}

/// Settled history: one payment per entry of `lags`, each paid `lag` days before it settled, settled `age` days ago.
async fn history(
    t: &T,
    tenant: &str,
    tag: &str,
    provider: &str,
    lags: &[i64],
    settled_days_ago: i64,
) {
    for (i, lag) in lags.iter().enumerate() {
        let settled_on = today() - Duration::days(settled_days_ago + i as i64);
        let aid = synth_payment(
            t,
            tenant,
            &format!("{tag}{i}"),
            provider,
            "upi",
            10_000,
            settled_on - Duration::days(*lag),
        )
        .await;
        synth_settlement(
            t,
            tenant,
            &format!("stl_{tag}{i}"),
            provider,
            settled_on,
            &[(aid, 10_000)],
        )
        .await;
    }
}

// ───────────────────────────── exports ─────────────────────────────

/// A made-up balanced ledger transaction written straight to the tables, in one database transaction (the
/// balance check runs at commit).
async fn post_ledger(t: &T, tenant: &str, id: &str, reference: &str, lines: &[(&str, &str, i64)]) {
    let mut tx = t.root.begin().await.unwrap();
    sqlx::query("INSERT INTO ledger_transactions (id, tenant_id, reference) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(tenant)
        .bind(reference)
        .execute(&mut *tx)
        .await
        .unwrap();
    for (side, acct, amount) in lines {
        sqlx::query("INSERT INTO ledger_entries (tenant_id, transaction_id, account, side, amount_minor, currency) VALUES ($1, $2, $3, $4, $5, 'INR')")
            .bind(tenant).bind(id).bind(acct).bind(side).bind(amount).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
}

/// A made-up capture for a tenant whose provider webhook the simulator is not configured to sign for (only the
/// first test tenant is).
async fn synth_capture(t: &T, tenant: &str, n: &str, amount: i64) {
    post_ledger(
        t,
        tenant,
        &format!("jt_synth{n}"),
        &format!("capture:att_synth{n}"),
        &[
            ("debit", "provider_receivable:razorpay", amount),
            ("credit", "collections", amount),
        ],
    )
    .await;
}

async fn vendor_payout_paid(t: &T, tn: &Tenant, amount: i64, r: &str) {
    let v = t
        .ok(
            tn,
            "POST",
            "/v1/vendors",
            None,
            json!({ "name": "Synthetic Vendor", "account_number": "000111222333", "ifsc": "TEST0000001", "external_ref": format!("V-{r}") }),
        )
        .await;
    let p = t
        .call_as(
            tn,
            "priya",
            "POST",
            "/v1/payouts",
            None,
            json!({ "vendor_id": v["id"], "amount_minor": amount, "purpose": "Synthetic supplies", "external_ref": r }),
        )
        .await
        .1;
    let pid = p["id"].as_str().unwrap();
    t.call_as(
        tn,
        "ravi",
        "POST",
        &format!("/v1/payouts/{pid}/approve"),
        None,
        json!({}),
    )
    .await;
    let (s, b) = t
        .call_as(
            tn,
            "priya",
            "POST",
            &format!("/v1/payouts/{pid}/mark_paid"),
            None,
            json!({ "bank_ref": format!("SYNTHBANK{r}") }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_journal_balances_is_deterministic_paged_audited_and_tenant_scoped(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let (iid, _, _) = t.paid_razorpay(a, 100_000, json!({})).await;
    t.paid_razorpay(a, 50_000, json!({})).await;
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("r1"),
        json!({ "amount_minor": 25_000 }),
    )
    .await;
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
        json!({ "provider": "razorpay", "date": today().to_string() }),
    )
    .await;
    vendor_payout_paid(&t, a, 500_000, "1").await;
    synth_capture(&t, &b.id, "b1", 70_000).await;

    let d = today();
    let range = format!("from={d}&to={d}");
    let full = t
        .ok(
            a,
            "GET",
            &format!("/v1/exports/journal?{range}&format=json"),
            None,
            Value::Null,
        )
        .await;
    let vouchers = full["vouchers"].as_array().unwrap();
    let kinds = |k: &str| vouchers.iter().filter(|v| v["kind"] == k).count();
    assert_eq!(
        (
            kinds("payments"),
            kinds("refunds"),
            kinds("payouts"),
            kinds("settlements"),
            kinds("fees")
        ),
        (2, 1, 1, 1, 1),
        "{full}"
    );

    // every voucher balances, and so does the whole export
    for v in vouchers {
        assert_eq!(v["debit_minor"], v["credit_minor"], "{v}");
        let (mut dr, mut cr) = (0, 0);
        for l in v["lines"].as_array().unwrap() {
            dr += l["debit_minor"].as_i64().unwrap();
            cr += l["credit_minor"].as_i64().unwrap();
        }
        assert_eq!(dr, cr, "{v}");
    }
    assert_eq!(
        full["totals"]["debit_minor"],
        full["totals"]["credit_minor"]
    );

    // the settlement voucher is the bank credit, the fee voucher the fee the provider deducted
    let (net, fee): (i64, i64) = sqlx::query_as("SELECT net_minor, fee_minor FROM settlements")
        .fetch_one(&t.root)
        .await
        .unwrap();
    let bank_in: i64 = vouchers
        .iter()
        .filter(|v| v["kind"] == "settlements")
        .flat_map(|v| v["lines"].as_array().unwrap())
        .filter(|l| l["ledger_account"] == "bank")
        .map(|l| l["debit_minor"].as_i64().unwrap())
        .sum();
    assert_eq!(bank_in, net);
    let fee_dr: i64 = vouchers
        .iter()
        .filter(|v| v["kind"] == "fees")
        .flat_map(|v| v["lines"].as_array().unwrap())
        .filter(|l| l["ledger_account"] == "processing_fees")
        .map(|l| l["debit_minor"].as_i64().unwrap())
        .sum();
    assert_eq!(fee_dr, fee);
    assert!(fee > 0);
    // the provider receivable rolls to what the settlement did not explain (nothing, for a settlement that adds up)
    let recv: i64 = vouchers
        .iter()
        .flat_map(|v| v["lines"].as_array().unwrap())
        .filter(|l| l["ledger_account"] == "provider_receivable:razorpay")
        .map(|l| l["debit_minor"].as_i64().unwrap() - l["credit_minor"].as_i64().unwrap())
        .sum();
    let adj: i64 = vouchers
        .iter()
        .flat_map(|v| v["lines"].as_array().unwrap())
        .filter(|l| l["ledger_account"] == "settlement_adjustments")
        .map(|l| l["debit_minor"].as_i64().unwrap() - l["credit_minor"].as_i64().unwrap())
        .sum();
    assert_eq!(
        recv, adj,
        "receivable left over is exactly the adjustments line"
    );

    // deterministic: the same request returns the same bytes, in every format
    for f in ["csv", "tally_csv", "tally_xml"] {
        let p = format!("/v1/exports/journal?{range}&format={f}");
        let (s1, h1, b1) = get_text(&t, a, &p).await;
        let (_, h2, b2) = get_text(&t, a, &p).await;
        assert_eq!(s1, StatusCode::OK);
        assert_eq!(b1, b2, "{f}");
        assert_eq!(h1["x-export-sha256"], h2["x-export-sha256"]);
        assert_eq!(
            h1["x-export-debit-total-minor"], h1["x-export-credit-total-minor"],
            "{f}"
        );
    }

    // CSV: header once, one row per line, debit column total = credit column total
    let (_, h, csv) = get_text(&t, a, &format!("/v1/exports/journal?{range}")).await;
    assert_eq!(h["content-type"], "text/csv; charset=utf-8");
    let rows: Vec<Vec<&str>> = csv.lines().map(|l| l.split(',').collect()).collect();
    assert_eq!(rows[0][0], "voucher_no");
    let money = |s: &str| -> i64 {
        if s.is_empty() {
            0
        } else {
            s.replace('.', "").parse().unwrap()
        }
    };
    let (dr, cr) = rows[1..]
        .iter()
        .fold((0, 0), |a, r| (a.0 + money(r[7]), a.1 + money(r[8])));
    assert_eq!(dr, cr);
    assert_eq!(dr, full["totals"]["debit_minor"].as_i64().unwrap());

    // Tally XML: one VOUCHER per voucher, amounts sum to zero (debits are negative, credits positive)
    let (_, _, xml) = get_text(
        &t,
        a,
        &format!("/v1/exports/journal?{range}&format=tally_xml"),
    )
    .await;
    assert_eq!(xml.matches("<VOUCHER ").count(), vouchers.len());
    let sum: i64 = xml
        .lines()
        .filter_map(|l| l.trim().strip_prefix("<AMOUNT>")?.strip_suffix("</AMOUNT>"))
        .map(money)
        .sum();
    assert_eq!(sum, 0);

    // paging: small pages follow the cursor and add up to the same vouchers in the same order, no repeats
    let mut seen: Vec<String> = vec![];
    let mut cursor = String::new();
    let mut pages = 0;
    loop {
        let after = if cursor.is_empty() {
            String::new()
        } else {
            format!("&after={cursor}")
        };
        let page = t
            .ok(
                a,
                "GET",
                &format!("/v1/exports/journal?{range}&format=json&limit=2{after}"),
                None,
                Value::Null,
            )
            .await;
        seen.extend(
            page["vouchers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["number"].as_str().unwrap().to_string()),
        );
        pages += 1;
        match page["next_cursor"].as_str() {
            Some(c) => cursor = c.to_string(),
            None => break,
        }
        assert!(pages < 10);
    }
    let all: Vec<String> = vouchers
        .iter()
        .map(|v| v["number"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(seen, all);
    assert_eq!(pages, all.len().div_ceil(2));
    // the CSV header is only on the first page
    let (_, h, _) = get_text(&t, a, &format!("/v1/exports/journal?{range}&limit=1")).await;
    let next = h["x-next-cursor"].to_str().unwrap().to_string();
    let (_, _, p2) = get_text(
        &t,
        a,
        &format!("/v1/exports/journal?{range}&limit=1&after={next}"),
    )
    .await;
    assert!(!p2.starts_with("voucher_no"));

    // kinds filter, other currency, outside the range
    let only = t
        .ok(
            a,
            "GET",
            &format!("/v1/exports/journal?{range}&format=json&kinds=refunds,payouts"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(only["vouchers"].as_array().unwrap().len(), 2);
    let usd = t
        .ok(
            a,
            "GET",
            &format!("/v1/exports/journal?{range}&format=json&currency=USD"),
            None,
            Value::Null,
        )
        .await;
    assert!(usd["vouchers"].as_array().unwrap().is_empty());
    let y = d - Duration::days(10);
    let none = t
        .ok(
            a,
            "GET",
            &format!(
                "/v1/exports/journal?from={y}&to={}&format=json",
                y + Duration::days(1)
            ),
            None,
            Value::Null,
        )
        .await;
    assert!(none["vouchers"].as_array().unwrap().is_empty());

    // tenant isolation: B sees only its own capture
    let jb = t
        .ok(
            b,
            "GET",
            &format!("/v1/exports/journal?{range}&format=json"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(jb["vouchers"].as_array().unwrap().len(), 1, "{jb}");
    let mine: Vec<&str> = all.iter().map(String::as_str).collect();
    assert!(!mine.contains(&jb["vouchers"][0]["number"].as_str().unwrap()));

    // bad requests
    for q in [
        "from=2026-02-01&to=2026-01-01",
        "from=2025-01-01&to=2026-11-01",
        "from=bad&to=2026-01-01",
        "to=2026-01-01",
        &format!("{range}&format=xlsx"),
        &format!("{range}&kinds=bogus"),
        &format!("{range}&currency=EUR"),
        &format!("{range}&after=zz"),
    ] {
        let (s, e) = t
            .call(
                a,
                "GET",
                &format!("/v1/exports/journal?{q}"),
                None,
                Value::Null,
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{q}: {e}");
    }

    // audited: who exported what, with the hash of the bytes returned
    let (_, h, _) = get_text(
        &t,
        a,
        &format!("/v1/exports/journal?{range}&format=tally_xml"),
    )
    .await;
    let sha: String = sqlx::query_scalar(
        "SELECT detail->>'sha256' FROM audit_events WHERE tenant_id = $1 AND action = 'export.journal' AND subject = 'tally_xml' ORDER BY id DESC LIMIT 1",
    )
    .bind(&a.id)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(sha, h["x-export-sha256"].to_str().unwrap());
    assert!(t.count(&format!("SELECT count(*) FROM audit_events WHERE tenant_id = '{}' AND action = 'export.journal'", a.id)).await >= 10);
    assert_eq!(t.count(&format!("SELECT count(*) FROM audit_events WHERE tenant_id = '{}' AND action = 'export.journal'", b.id)).await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn exports_need_their_scopes_and_account_codes_are_per_tenant(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    t.paid_razorpay(a, 100_000, json!({})).await;
    let d = today();
    let q = format!("from={d}&to={d}");

    let pay_only = key_with(&t, &["payments:read"]).await;
    assert_eq!(
        t.call(
            &pay_only,
            "GET",
            &format!("/v1/exports/journal?{q}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&pay_only, "GET", "/v1/cash/dashboard", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let reader = key_with(&t, &["exports:read"]).await;
    assert_eq!(
        get_text(&t, &reader, &format!("/v1/exports/journal?{q}"))
            .await
            .0,
        StatusCode::OK
    );
    let (s, _) = t
        .call(
            &reader,
            "POST",
            "/v1/exports/account_mappings",
            None,
            json!({ "ledger_account": "bank", "code": "1", "name": "x" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // built-in codes by default
    let (_, _, csv) = get_text(&t, a, &format!("/v1/exports/journal?{q}")).await;
    assert!(
        csv.contains(",1200,Payment provider receivable,provider_receivable:razorpay,"),
        "{csv}"
    );
    assert!(csv.contains(",4000,Collections,collections,"));

    // a tenant's own codes: an exact account and a family, validated
    for bad in [
        json!({ "ledger_account": "a b", "code": "1", "name": "x" }),
        json!({ "ledger_account": "*", "code": "1", "name": "x" }),
        json!({ "ledger_account": "bank", "code": "", "name": "x" }),
        json!({ "ledger_account": "bank", "code": "1", "name": "x\ny" }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/exports/account_mappings", None, bad)
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    t.ok(a, "POST", "/v1/exports/account_mappings", None, json!({ "ledger_account": "provider_receivable:*", "code": "1250", "name": "PG receivable" })).await;
    let m = t
        .ok(
            a,
            "POST",
            "/v1/exports/account_mappings",
            None,
            json!({ "ledger_account": "collections", "code": "4010", "name": "Fee collections" }),
        )
        .await;
    let (_, h, csv) = get_text(&t, a, &format!("/v1/exports/journal?{q}")).await;
    assert!(csv.contains(",1250,PG receivable,provider_receivable:razorpay,"));
    assert!(csv.contains(",4010,Fee collections,collections,"));
    assert_eq!(h["x-export-unmapped-accounts"], "");
    let list = t
        .ok(a, "GET", "/v1/exports/account_mappings", None, Value::Null)
        .await;
    assert_eq!(list["overrides"].as_array().unwrap().len(), 2);
    assert!(
        list["effective"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["ledger_account"] == "collections" && e["code"] == "4010")
    );

    // B is untouched by A's mapping
    synth_capture(&t, &b.id, "b2", 10_000).await;
    let (_, _, csvb) = get_text(&t, b, &format!("/v1/exports/journal?{q}")).await;
    assert!(csvb.contains(",4000,Collections,collections,"), "{csvb}");
    assert!(!csvb.contains("4010"));
    assert_eq!(
        t.ok(b, "GET", "/v1/exports/account_mappings", None, Value::Null)
            .await["overrides"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // and cannot delete it
    let id = m["id"].as_i64().unwrap();
    assert_eq!(
        t.call(
            b,
            "DELETE",
            &format!("/v1/exports/account_mappings/{id}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    t.ok(
        a,
        "DELETE",
        &format!("/v1/exports/account_mappings/{id}"),
        None,
        Value::Null,
    )
    .await;
    let (_, _, csv) = get_text(&t, a, &format!("/v1/exports/journal?{q}")).await;
    assert!(csv.contains(",4000,Collections,collections,"));
    assert!(t.count(&format!("SELECT count(*) FROM audit_events WHERE tenant_id = '{}' AND action LIKE 'export.mapping_%'", a.id)).await >= 3);

    // an account nobody mapped is visible, not dropped
    post_ledger(
        &t,
        &a.id,
        "jt_syn1",
        "odd:1",
        &[("debit", "mystery:acct", 777), ("credit", "bank", 777)],
    )
    .await;
    let (_, h, csv) = get_text(&t, a, &format!("/v1/exports/journal?{q}")).await;
    assert_eq!(h["x-export-unmapped-accounts"], "mystery:acct");
    assert!(csv.contains(",9999,mystery:acct,mystery:acct,"));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_gst_summary_adds_up_sales_and_credit_notes_and_is_audited(pool: PgPool) {
    use zyvor_api::gst::gstin_check_char;
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let head = "07AABCT1332L1Z";
    let buyer = format!("{head}{}", gstin_check_char(head));
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "gst": { "gstin": "27AAPFU0939F1ZV", "legal_name": "Synthetic Trust" } }),
    )
    .await;
    let line = |qty: i64, price: i64, rate: i32| json!({ "description": "Tuition", "hsn_sac": "999293", "quantity": qty, "unit_price_minor": price, "rate_bp": rate });
    let g1 = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "number": "G-1", "lines": [line(2, 50_000, 1800)], "place_of_supply": "27" }),
        )
        .await;
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Delhi Co", "gstin": buyer }),
        )
        .await;
    t.ok(
        a,
        "POST",
        "/v1/invoices",
        None,
        json!({ "number": "G-2", "customer_id": cust["id"], "lines": [line(1, 200_000, 500)] }),
    )
    .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/invoices/{}/credit_notes", g1["id"].as_str().unwrap()),
        None,
        json!({ "amount_minor": 59_000, "reason": "concession" }),
    )
    .await;

    let d = today();
    let (s, h, csv) = get_text(&t, a, &format!("/v1/exports/gst_summary?from={d}&to={d}")).await;
    assert_eq!(s, StatusCode::OK, "{csv}");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines,
        vec![
            "section,supply,rate_percent,documents,taxable,cgst,sgst,igst,tax_total",
            "sales,b2b,5.00,1,2000.00,0.00,0.00,100.00,100.00",
            "sales,b2c,18.00,1,1000.00,90.00,90.00,0.00,180.00",
            "credit_notes,b2c,,1,500.00,45.00,45.00,0.00,90.00",
            "net,all,,3,2500.00,45.00,45.00,100.00,190.00",
        ]
    );
    let j = t
        .ok(
            a,
            "GET",
            &format!("/v1/exports/gst_summary?from={d}&to={d}&format=json"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(j["net"]["tax_minor"], 19_000);
    let sha: String = sqlx::query_scalar("SELECT detail->>'sha256' FROM audit_events WHERE action = 'export.gst_summary' AND subject = 'csv'")
        .fetch_one(&t.root).await.unwrap();
    assert_eq!(sha, h["x-export-sha256"].to_str().unwrap());
    // another tenant has none of it, and a different range is empty
    let (_, _, other) = get_text(&t, b, &format!("/v1/exports/gst_summary?from={d}&to={d}")).await;
    assert_eq!(other.lines().count(), 2, "{other}");
    let y = d - Duration::days(30);
    let (_, _, none) = get_text(&t, a, &format!("/v1/exports/gst_summary?from={y}&to={y}")).await;
    assert_eq!(
        none.lines().last().unwrap(),
        "net,all,,0,0.00,0.00,0.00,0.00,0.00"
    );
    assert_eq!(
        t.call(
            a,
            "GET",
            "/v1/exports/gst_summary?from=2026-02-01&to=2026-01-01",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

// ───────────────────────────── lag, forecast, alerts ─────────────────────────────

/// Twelve made-up settled payments for Razorpay UPI: lags 1,1,1,2,2,2,2,3,3,3,4,5 days.
/// n = 12: median (2+2)/2 = 2, p10 = rank 2 = 1, p90 = rank 11 = 4.
const LAGS: [i64; 12] = [1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 4, 5];

#[sqlx::test(migrations = "./migrations")]
async fn observed_lag_needs_enough_samples_and_the_forecast_is_honest(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    // 12 settled payments for Razorpay UPI; only 3 for Cashfree UPI
    history(&t, &a.id, "rz", "razorpay", &LAGS, 20).await;
    history(&t, &a.id, "cf", "cashfree", &[1, 2, 3], 20).await;

    let lag = t.ok(a, "GET", "/v1/cash/lag", None, Value::Null).await;
    let find = |p: &str| {
        lag["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["provider"] == p)
            .unwrap()
            .clone()
    };
    let rz = find("razorpay");
    assert_eq!(
        (
            rz["status"].as_str(),
            rz["samples"].as_i64(),
            rz["median_days"].as_f64(),
            rz["p10_days"].as_i64(),
            rz["p90_days"].as_i64(),
            rz["max_days"].as_i64()
        ),
        (
            Some("observed"),
            Some(12),
            Some(2.0),
            Some(1),
            Some(4),
            Some(5)
        ),
        "{rz}"
    );
    let cf = find("cashfree");
    assert_eq!(
        (cf["status"].as_str(), cf["samples"].as_i64()),
        (Some("not_enough_data"), Some(3))
    );
    assert!(cf["message"].as_str().unwrap().contains("at least 10"));

    // pending: 1000 paid today, 2000 yesterday, 4000 five days ago (past its p90 of 4), and Cashfree 3000 yesterday
    let rz1 = synth_payment(&t, &a.id, "p1", "razorpay", "upi", 1_000, today()).await;
    let _ = rz1;
    synth_payment(
        &t,
        &a.id,
        "p2",
        "razorpay",
        "upi",
        2_000,
        today() - Duration::days(1),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "p3",
        "razorpay",
        "upi",
        4_000,
        today() - Duration::days(5),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "p4",
        "cashfree",
        "upi",
        3_000,
        today() - Duration::days(1),
    )
    .await;

    let f = t
        .ok(a, "GET", "/v1/cash/forecast?days=7", None, Value::Null)
        .await;
    let day = |i: usize, k: &str| f["days"][i][k].as_i64().unwrap();
    // 2000 (paid yesterday): expected tomorrow (+2 days from yesterday), could be here today (p10), nearly always by day 3
    // 1000 (paid today): expected day 2, could be day 1, nearly always by day 4
    assert_eq!(f["days"][0]["date"], today().to_string());
    assert_eq!(
        (
            day(0, "expected_minor"),
            day(1, "expected_minor"),
            day(2, "expected_minor")
        ),
        (0, 2_000, 1_000),
        "{f}"
    );
    assert_eq!(
        (
            day(0, "cumulative_high_minor"),
            day(1, "cumulative_high_minor")
        ),
        (2_000, 3_000)
    );
    assert_eq!(
        (
            day(2, "cumulative_low_minor"),
            day(3, "cumulative_low_minor"),
            day(4, "cumulative_low_minor")
        ),
        (0, 2_000, 3_000)
    );
    assert_eq!(
        (
            day(1, "cumulative_expected_minor"),
            day(2, "cumulative_expected_minor")
        ),
        (2_000, 3_000)
    );
    assert_eq!(
        f["overdue"]["amount_minor"], 4_000,
        "past its provider's own p90"
    );
    assert_eq!(f["expected_without_range_minor"], 0);
    // Cashfree has too little history and no stated lag: left out, said so
    let nf = f["not_forecast"].as_array().unwrap();
    assert_eq!(nf.len(), 1);
    assert_eq!(
        (
            nf[0]["provider"].as_str(),
            nf[0]["amount_minor"].as_i64(),
            nf[0]["status"].as_str()
        ),
        (Some("cashfree"), Some(3_000), Some("not_enough_data"))
    );

    // the merchant states its own lag for Cashfree: forecast with no range, flagged
    t.ok(
        a,
        "POST",
        "/v1/cash/lag_rules",
        None,
        json!({ "provider": "cashfree", "expected_lag_days": 3, "note": "from our contract" }),
    )
    .await;
    let f = t
        .ok(a, "GET", "/v1/cash/forecast?days=7", None, Value::Null)
        .await;
    assert_eq!(f["expected_without_range_minor"], 3_000);
    assert_eq!(f["not_forecast"].as_array().unwrap().len(), 0);
    assert_eq!(
        f["days"][2]["expected_minor"], 4_000,
        "1000 observed + 3000 stated (paid yesterday, 3 days)"
    );
    assert_eq!(
        f["days"][6]["cumulative_low_minor"], 3_000,
        "the stated-lag payment is not in the range"
    );

    // lowering the sample minimum makes Cashfree's three samples count
    t.ok(
        a,
        "POST",
        "/v1/cash/settings",
        None,
        json!({ "min_samples": 3 }),
    )
    .await;
    let lag = t.ok(a, "GET", "/v1/cash/lag", None, Value::Null).await;
    let cf = lag["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["provider"] == "cashfree")
        .unwrap()
        .clone();
    assert_eq!(
        (cf["status"].as_str(), cf["median_days"].as_f64()),
        (Some("observed"), Some(2.0))
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/cash/settings",
            None,
            json!({ "min_samples": 2 })
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // horizon validation is a clamp, not an error; B sees nothing of A's data
    assert_eq!(
        t.ok(a, "GET", "/v1/cash/forecast?days=99", None, Value::Null)
            .await["days"]
            .as_array()
            .unwrap()
            .len(),
        30
    );
    let fb = t
        .ok(b, "GET", "/v1/cash/forecast?days=7", None, Value::Null)
        .await;
    assert!(
        fb["days"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["cumulative_expected_minor"] == 0)
    );
    assert_eq!(fb["overdue"]["amount_minor"], 0);
    assert!(
        t.ok(b, "GET", "/v1/cash/lag", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        t.ok(b, "GET", "/v1/cash/lag_rules", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // lag rule validation
    for bad in [
        json!({ "provider": "x", "expected_lag_days": 61 }),
        json!({ "provider": "x", "expected_lag_days": -1 }),
        json!({ "provider": "x y", "expected_lag_days": 1 }),
        json!({ "provider": "x", "method": "u*", "expected_lag_days": 1 }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/cash/lag_rules", None, bad).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn delay_alerts_open_one_case_and_one_event_per_subject_and_never_invent_an_sla(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    history(&t, &a.id, "rz", "razorpay", &LAGS, 20).await; // p90 = 4 on its own
    history(&t, &a.id, "cf", "cashfree", &[1, 2, 3], 20).await; // too few to use
    t.ok(
        a,
        "POST",
        "/v1/cash/lag_rules",
        None,
        json!({ "provider": "cashfree", "expected_lag_days": 2 }),
    )
    .await;
    // one Razorpay settlement that arrived on day 9, inside the last 14 days
    let late = synth_payment(
        &t,
        &a.id,
        "late",
        "razorpay",
        "upi",
        6_000,
        today() - Duration::days(10),
    )
    .await;
    synth_settlement(
        &t,
        &a.id,
        "stl_late",
        "razorpay",
        today() - Duration::days(1),
        &[(late, 6_000)],
    )
    .await;
    // pending: Razorpay 7 days ago (later than its own range) and 2 days ago (inside it);
    // Cashfree 3 days ago (later than the 2 days stated) and yesterday (inside);
    // Razorpay card 20 days ago: no history, nothing stated, so nothing to compare with
    synth_payment(
        &t,
        &a.id,
        "q1",
        "razorpay",
        "upi",
        5_000,
        today() - Duration::days(7),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "q2",
        "razorpay",
        "upi",
        1_000,
        today() - Duration::days(2),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "q3",
        "cashfree",
        "upi",
        3_000,
        today() - Duration::days(3),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "q4",
        "cashfree",
        "upi",
        2_000,
        today() - Duration::days(1),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "q5",
        "razorpay",
        "card",
        9_000,
        today() - Duration::days(20),
    )
    .await;

    let r = t
        .ok(a, "POST", "/v1/cash/alerts/scan", None, json!({}))
        .await;
    assert_eq!(
        (
            r["pending_cases_opened"].as_i64(),
            r["late_cases_opened"].as_i64()
        ),
        (Some(2), Some(1)),
        "{r}"
    );
    let cases = |kind: &str| {
        format!(
            "SELECT count(*) FROM recon_cases WHERE tenant_id = '{}' AND kind = '{kind}'",
            a.id
        )
    };
    assert_eq!(
        (
            t.count(&cases("settlement_delayed")).await,
            t.count(&cases("settlement_late")).await
        ),
        (2, 1)
    );
    let subjects: Vec<String> = sqlx::query_scalar(
        "SELECT subject_id FROM recon_cases WHERE kind = 'settlement_delayed' ORDER BY subject_id",
    )
    .fetch_all(&t.root)
    .await
    .unwrap();
    assert_eq!(
        subjects,
        vec![
            format!("cashfree:upi:{}", today() - Duration::days(3)),
            format!("razorpay:upi:{}", today() - Duration::days(7)),
        ]
    );
    assert!(
        !subjects.iter().any(|s| s.contains("card")),
        "no history and nothing stated: no alert"
    );
    let ev = format!(
        "SELECT count(*) FROM outbox_events WHERE tenant_id = '{}' AND type = 'settlement.delayed'",
        a.id
    );
    assert_eq!(t.count(&ev).await, 3);

    // the cases say why, in numbers a person can check
    let (expl, detail): (String, Value) = sqlx::query_as("SELECT explanation, detail FROM recon_cases WHERE kind = 'settlement_delayed' AND subject_id LIKE 'cashfree:%'")
        .fetch_one(&t.root).await.unwrap();
    assert_eq!(
        (
            detail["threshold_days"].as_i64(),
            detail["basis"].as_str(),
            detail["age_days"].as_i64()
        ),
        (Some(2), Some("tenant_expected_lag"), Some(3)),
        "{expl}"
    );
    // the Razorpay threshold comes from history that includes the day-9 settlement: 13 samples, p90 = rank 12 = 5
    let detail: Value = sqlx::query_scalar("SELECT detail FROM recon_cases WHERE kind = 'settlement_delayed' AND subject_id LIKE 'razorpay:%'")
        .fetch_one(&t.root).await.unwrap();
    assert_eq!(
        (detail["threshold_days"].as_i64(), detail["basis"].as_str()),
        (Some(5), Some("observed_range")),
        "{detail}"
    );
    let detail: Value =
        sqlx::query_scalar("SELECT detail FROM recon_cases WHERE kind = 'settlement_late'")
            .fetch_one(&t.root)
            .await
            .unwrap();
    // judged against the other 12 settled payments (p90 = 4), not itself
    assert_eq!(
        (
            detail["lag_days"].as_i64(),
            detail["threshold_days"].as_i64(),
            detail["state"].as_str()
        ),
        (Some(9), Some(4), Some("arrived_late")),
        "{detail}"
    );

    // the event carries aggregates only: no customer, payment or staff identifiers
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE type = 'settlement.delayed' ORDER BY id LIMIT 1",
    )
    .fetch_one(&t.root)
    .await
    .unwrap();
    let text = payload.to_string();
    assert!(
        text.contains("amount_minor")
            && !text.contains("att_")
            && !text.contains("customer")
            && !text.contains("email"),
        "{text}"
    );

    // idempotent: running again, or by the worker, opens nothing and queues nothing, even after a case is resolved
    let again = t
        .ok(a, "POST", "/v1/cash/alerts/scan", None, json!({}))
        .await;
    assert_eq!(
        (
            again["pending_cases_opened"].as_i64(),
            again["late_cases_opened"].as_i64()
        ),
        (Some(0), Some(0))
    );
    sqlx::query("UPDATE recon_cases SET status = 'resolved' WHERE kind = 'settlement_delayed'")
        .execute(&t.root)
        .await
        .unwrap();
    t.ok(a, "POST", "/v1/cash/alerts/scan", None, json!({}))
        .await;
    zyvor_api::worker::run_cycle(&t.state, &reqwest::Client::new())
        .await
        .unwrap();
    assert_eq!(
        (
            t.count(&cases("settlement_delayed")).await,
            t.count(&cases("settlement_late")).await,
            t.count(&ev).await
        ),
        (2, 1, 3)
    );

    // a grace period widens the threshold: nothing new for a cohort just inside it
    t.ok(
        a,
        "POST",
        "/v1/cash/settings",
        None,
        json!({ "alert_grace_days": 30 }),
    )
    .await;
    synth_payment(
        &t,
        &a.id,
        "q6",
        "razorpay",
        "upi",
        700,
        today() - Duration::days(15),
    )
    .await;
    let r = t
        .ok(a, "POST", "/v1/cash/alerts/scan", None, json!({}))
        .await;
    assert_eq!(r["pending_cases_opened"], 0);

    // tenant B has no cases and no events from any of this, and the scan needs its scope
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM recon_cases WHERE tenant_id = '{}'",
            b.id
        ))
        .await,
        0
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM outbox_events WHERE tenant_id = '{}'",
            b.id
        ))
        .await,
        0
    );
    let ro = key_with(&t, &["cash:read"]).await;
    assert_eq!(
        t.call(&ro, "POST", "/v1/cash/alerts/scan", None, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&ro, "GET", "/v1/cash/forecast", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    // the webhook catalogue knows the event, so it can be subscribed to
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "webhook_events": ["settlement.delayed"] }),
    )
    .await;
}

// ───────────────────────────── dashboard ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn the_dashboard_shows_where_the_money_is_by_provider(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    // empty to begin with
    let d0 = t
        .ok(a, "GET", "/v1/cash/dashboard", None, Value::Null)
        .await;
    assert!(d0["by_provider"].as_array().unwrap().is_empty());

    // P1 settles (net 97,640) but its bank credit has not been seen
    t.paid_razorpay(a, 100_000, json!({})).await;
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
            json!({ "provider": "razorpay", "date": today().to_string() }),
        )
        .await;
    let sid = synced["imported"][0].as_str().unwrap().to_string();
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    let (utr, net) = (
        ex["settlement"]["utr"].as_str().unwrap().to_string(),
        ex["settlement"]["net_minor"].as_i64().unwrap(),
    );
    let dash = t
        .ok(a, "GET", "/v1/cash/dashboard", None, Value::Null)
        .await;
    let rz = &dash["by_provider"][0];
    assert_eq!(rz["provider"], "razorpay");
    assert_eq!(
        (
            rz["in_transit"]["payments"].as_i64(),
            rz["settled_not_matched_to_bank"]["net_minor"].as_i64(),
            rz["matched"]["net_minor"].as_i64()
        ),
        (Some(0), Some(net), Some(0)),
        "{dash}"
    );

    // the credit arrives and is matched on its reference: it moves to matched
    let csv = format!(
        "Date,Narration,Reference,Debit,Credit\n{},NEFT CR-{utr}-RAZORPAY,,,\"{}.{:02}\"\n",
        dmy(today()),
        net / 100,
        net % 100
    );
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let dash = t
        .ok(a, "GET", "/v1/cash/dashboard", None, Value::Null)
        .await;
    let rz = &dash["by_provider"][0];
    assert_eq!(
        (
            rz["settled_not_matched_to_bank"]["settlements"].as_i64(),
            rz["matched"]["net_minor"].as_i64(),
            rz["matched"]["settlements"].as_i64()
        ),
        (Some(0), Some(net), Some(1)),
        "{dash}"
    );

    // P2 is captured and in transit; 25,000 of it is refunded, a further 20,000 waits for approval
    let (iid, _, _) = t.paid_razorpay(a, 100_000, json!({})).await;
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("d1"),
        json!({ "amount_minor": 25_000 }),
    )
    .await;
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "refund_approval_threshold_minor": 10_000 }),
    )
    .await;
    t.call_as(
        a,
        "priya",
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("d2"),
        json!({ "amount_minor": 20_000 }),
    )
    .await;
    // and a payout is waiting to be approved
    let v = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Synthetic Vendor", "account_number": "000111222333", "ifsc": "TEST0000001", "external_ref": "V-D" })).await;
    t.call_as(a, "priya", "POST", "/v1/payouts", None, json!({ "vendor_id": v["id"], "amount_minor": 500_000, "purpose": "Synthetic", "external_ref": "PO-D" })).await;

    let dash = t
        .ok(a, "GET", "/v1/cash/dashboard", None, Value::Null)
        .await;
    let rz = &dash["by_provider"][0];
    assert_eq!(
        (
            rz["in_transit"]["payments"].as_i64(),
            rz["in_transit"]["gross_minor"].as_i64(),
            rz["in_transit"]["refunded_minor"].as_i64(),
            rz["in_transit"]["net_minor"].as_i64()
        ),
        (Some(1), Some(100_000), Some(25_000), Some(75_000)),
        "{dash}"
    );
    assert_eq!(
        (
            rz["refunds_pending"]["count"].as_i64(),
            rz["refunds_pending"]["amount_minor"].as_i64()
        ),
        (Some(1), Some(20_000))
    );
    assert_eq!(
        rz["refunds_pending"]["by_status"]["pending_approval"]["amount_minor"],
        20_000
    );
    assert_eq!(dash["payouts_pending"]["amount_minor"], 500_000);
    assert_eq!(dash["totals"]["in_transit_net_minor"], 75_000);
    assert_eq!(dash["totals"]["matched_minor"], net);

    // another business sees none of it
    let db = t
        .ok(b, "GET", "/v1/cash/dashboard", None, Value::Null)
        .await;
    assert!(db["by_provider"].as_array().unwrap().is_empty(), "{db}");
    assert_eq!(db["payouts_pending"]["amount_minor"], 0);
}

// ───────────────────────────── refund analytics ─────────────────────────────

#[sqlx::test(migrations = "./migrations")]
async fn refund_analytics_rate_time_sla_and_duplicates_with_no_personal_data(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let mut p = vec![];
    for _ in 0..5 {
        p.push(t.paid_razorpay(a, 100_000, json!({})).await);
    }
    let refund = |i: usize, key: &'static str, amt: Option<i64>| {
        let path = format!("/v1/payment_intents/{}/refunds", p[i].0);
        let body = amt.map_or(json!({}), |a| json!({ "amount_minor": a }));
        let t = &t;
        async move {
            t.call_as(&t.a, "priya", "POST", &path, Some(key), body)
                .await
        }
    };
    // processed: 25,000 + 100,000 + 10,000
    assert_eq!(refund(0, "k0", Some(25_000)).await.1["status"], "processed");
    assert_eq!(refund(1, "k1", None).await.1["status"], "processed");
    assert_eq!(refund(3, "k3", Some(10_000)).await.1["status"], "processed");
    // P5's refund goes processed, then is made unknown and old (as a provider that never answered would leave it)
    let r5 = refund(4, "k4", Some(30_000)).await.1;
    // P3's large refund waits for approval, and has waited 3 days
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "refund_approval_threshold_minor": 15_000 }),
    )
    .await;
    let r3 = refund(2, "k2", Some(20_000)).await.1;
    assert_eq!(r3["status"], "pending_approval");
    sqlx::query("UPDATE refunds SET created_at = now() - interval '3 days' WHERE id = $1")
        .bind(r3["id"].as_str().unwrap())
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE refunds SET status = 'unknown', created_at = now() - interval '60 hours' WHERE id = $1").bind(r5["id"].as_str().unwrap()).execute(&t.root).await.unwrap();

    // duplicates blocked: the same request again (replay), more than is left, a second full refund of a refunded payment
    let again = refund(0, "k0", Some(25_000)).await;
    assert_eq!(again.1["id"], refund(0, "k0", Some(25_000)).await.1["id"]);
    assert_eq!(
        refund(0, "k0b", Some(80_000)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        refund(1, "k1b", None).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let an = t
        .ok(a, "GET", "/v1/analytics/refunds", None, Value::Null)
        .await;
    // 135,000 refunded and processed against 500,000 captured: 27.00%
    assert_eq!(
        (
            an["totals"]["captured_minor"].as_i64(),
            an["totals"]["refunds_processed_minor"].as_i64(),
            an["totals"]["refund_rate_bps"].as_i64()
        ),
        (Some(500_000), Some(135_000), Some(2_700)),
        "{an}"
    );
    assert_eq!(
        an["totals"]["refunds_requested_minor"],
        135_000 + 20_000 + 30_000,
        "requested counts everything not rejected"
    );
    let series = an["series"].as_array().unwrap();
    assert_eq!(series.len(), 30);
    assert_eq!(
        series
            .iter()
            .map(|s| s["captured_minor"].as_i64().unwrap())
            .sum::<i64>(),
        500_000
    );

    // SLA (48h): the 3 day approval wait and the 60 hour unknown refund
    assert_eq!(an["sla"]["threshold_hours"], 48);
    assert_eq!(
        (
            an["sla"]["breach_count"].as_i64(),
            an["sla"]["breach_amount_minor"].as_i64()
        ),
        (Some(2), Some(50_000)),
        "{an}"
    );
    let list = an["sla"]["list"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["status"], "pending_approval");
    assert!(list[0]["age_hours"].as_f64().unwrap() >= 72.0);
    // a tighter SLA catches nothing new here, a looser one releases them
    t.ok(
        a,
        "POST",
        "/v1/cash/settings",
        None,
        json!({ "refund_sla_hours": 100 }),
    )
    .await;
    assert_eq!(
        t.ok(a, "GET", "/v1/analytics/refunds", None, Value::Null)
            .await["sla"]["breach_count"],
        0
    );
    t.ok(
        a,
        "POST",
        "/v1/cash/settings",
        None,
        json!({ "refund_sla_hours": 48 }),
    )
    .await;

    // duplicates: two replays of the same request, two over-refund attempts
    let dup = &an["leakage"]["duplicate_refunds_blocked"];
    assert_eq!(dup["count"], 4, "{an}");
    let by = |r: &str| {
        dup["by_reason"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["reason"] == r)
            .map(|x| x["count"].as_i64().unwrap())
    };
    assert_eq!(
        (by("idempotent_replay"), by("exceeds_refundable")),
        (Some(2), Some(2))
    );

    // time to refund: three processed refunds, below the default minimum of ten
    assert_eq!(an["time_to_refund"]["all"]["status"], "not_enough_data");
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM refunds WHERE status = 'processed' ORDER BY amount_minor",
    )
    .fetch_all(&t.root)
    .await
    .unwrap();
    for (id, hrs) in ids.iter().zip([1, 2, 5]) {
        sqlx::query("UPDATE refunds SET created_at = now() - interval '10 hours', updated_at = now() - interval '10 hours' + make_interval(hours => $2::int) WHERE id = $1")
            .bind(id).bind(hrs).execute(&t.root).await.unwrap();
    }
    t.ok(
        a,
        "POST",
        "/v1/cash/settings",
        None,
        json!({ "min_samples": 3 }),
    )
    .await;
    let an = t
        .ok(a, "GET", "/v1/analytics/refunds", None, Value::Null)
        .await;
    let tt = &an["time_to_refund"]["all"];
    assert_eq!(
        (
            tt["status"].as_str(),
            tt["samples"].as_i64(),
            tt["median_hours"].as_f64(),
            tt["p90_hours"].as_f64(),
            tt["max_hours"].as_f64()
        ),
        (Some("observed"), Some(3), Some(2.0), Some(5.0), Some(5.0)),
        "{tt}"
    );
    assert_eq!(
        an["time_to_refund"]["by_provider"][0]["provider"],
        "razorpay"
    );

    // weekly buckets and validation
    let w = t
        .ok(
            a,
            "GET",
            "/v1/analytics/refunds?bucket=week",
            None,
            Value::Null,
        )
        .await;
    assert!(w["series"].as_array().unwrap().len() <= 6);
    assert_eq!(w["totals"]["captured_minor"], 500_000);
    for q in [
        "bucket=month",
        "from=2020-01-01&to=2026-01-01",
        "from=2026-02-01&to=2026-01-01",
        "from=x",
    ] {
        assert_eq!(
            t.call(
                a,
                "GET",
                &format!("/v1/analytics/refunds?{q}"),
                None,
                Value::Null
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{q}"
        );
    }

    // no personal data anywhere in the aggregates: not the requester, not ids of people
    let text = an.to_string();
    for needle in ["priya", "requested_by", "email", "phone", "customer"] {
        assert!(!text.contains(needle), "{needle} in {text}");
    }

    // another business sees none of it
    let bn = t
        .ok(b, "GET", "/v1/analytics/refunds", None, Value::Null)
        .await;
    assert_eq!(
        (
            bn["totals"]["captured_minor"].as_i64(),
            bn["sla"]["breach_count"].as_i64(),
            bn["leakage"]["duplicate_refunds_blocked"]["count"].as_i64()
        ),
        (Some(0), Some(0), Some(0))
    );
    assert_eq!(bn["totals"]["refund_rate_bps"], Value::Null);
}

#[sqlx::test(migrations = "./migrations")]
async fn leakage_counts_refunds_where_the_fee_was_not_given_back(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(a, "POST", "/v1/fee_rules", None, json!({ "provider": "razorpay", "method": "upi", "bps": 100, "flat_minor": 50, "refund_returns_fee": true, "effective_from": "2020-01-01" })).await;
    let settle = |n: &str, pay_ref: String, fees: Vec<i64>| {
        let t = &t;
        let n = n.to_string();
        async move {
            let mut items = vec![
                json!({ "kind": "payment", "provider_ref": pay_ref, "amount_minor": 100_000 }),
                json!({ "kind": "refund", "provider_ref": pay_ref, "amount_minor": -40_000 }),
            ];
            for f in fees {
                items.push(json!({ "kind": "fee", "provider_ref": pay_ref, "amount_minor": -f }));
            }
            t.ok(&t.a, "POST", "/v1/settlements/import", None, json!({ "provider": "razorpay", "provider_ref": format!("stl_{n}"), "utr": format!("UTR{n}"), "settled_on": today().to_string(), "items": items })).await
        }
    };
    // a 40,000 refund of 100,000 should return 400 of the 1,050 fee. Returned: fine. Kept whole: 400 leaked.
    let (_, _, p1) = t.paid_razorpay(a, 100_000, json!({})).await;
    assert_eq!(
        settle("1", p1, vec![1_050, -400]).await["fee_check"]["status"],
        "ok"
    );
    let (_, _, p2) = t.paid_razorpay(a, 100_000, json!({})).await;
    assert_eq!(
        settle("2", p2, vec![1_050]).await["fee_check"]["status"],
        "flagged"
    );

    let an = t
        .ok(a, "GET", "/v1/analytics/refunds", None, Value::Null)
        .await;
    let l = &an["leakage"]["fee_not_returned"];
    assert_eq!(
        (
            l["lines"].as_i64(),
            l["settlements"].as_i64(),
            l["amount_minor"].as_i64()
        ),
        (Some(1), Some(1), Some(400)),
        "{an}"
    );
    assert_eq!(l["by_provider_method"][0]["provider"], "razorpay");
    assert_eq!(l["refund_lines_without_a_fee_rule"], 0);

    // with the rule saying fees are NOT returned, there is nothing to call a leak
    let rules = t.ok(a, "GET", "/v1/fee_rules", None, Value::Null).await;
    t.ok(
        a,
        "DELETE",
        &format!("/v1/fee_rules/{}", rules["data"][0]["id"].as_str().unwrap()),
        None,
        Value::Null,
    )
    .await;
    t.ok(a, "POST", "/v1/fee_rules", None, json!({ "provider": "razorpay", "method": "upi", "bps": 100, "flat_minor": 50, "refund_returns_fee": false, "effective_from": "2020-01-01" })).await;
    t.ok(a, "POST", "/v1/fee_checks/run", None, json!({})).await;
    let an = t
        .ok(a, "GET", "/v1/analytics/refunds", None, Value::Null)
        .await;
    assert_eq!(an["leakage"]["fee_not_returned"]["amount_minor"], 0, "{an}");
}
