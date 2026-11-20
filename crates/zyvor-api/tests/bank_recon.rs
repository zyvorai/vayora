// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Bank statement import and matching: synthetic statements only (made-up references, no real account numbers).

mod common;

use axum::http::StatusCode;
use chrono::{Duration, NaiveDate};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn today() -> NaiveDate {
    zyvor_api::ist_today()
}

fn dmy(d: NaiveDate) -> String {
    d.format("%d/%m/%Y").to_string()
}

const HEADER: &str = "Date,Narration,Reference,Debit,Credit\n";

async fn import(t: &T, tn: &Tenant, csv: &str) -> (StatusCode, Value) {
    t.call(
        tn,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv, "filename": "oct.csv" }),
    )
    .await
}

async fn lines(t: &T, tn: &Tenant, q: &str) -> Vec<Value> {
    t.ok(
        tn,
        "GET",
        &format!("/v1/bank_statements/lines{q}"),
        None,
        Value::Null,
    )
    .await["data"]
        .as_array()
        .unwrap()
        .clone()
}

/// A settled Razorpay payment (through the simulator) and its settlement: net 97,640 paise, UTR from the report.
async fn settled(t: &T, tn: &Tenant) -> (String, String) {
    t.paid_razorpay(tn, 100_000, json!({})).await;
    t.mock_post(
        "razorpay/settle",
        json!({ "fee_bps": 200, "tax_bps": 1800 }),
    )
    .await;
    let synced = t
        .ok(
            tn,
            "POST",
            "/v1/settlements/sync",
            None,
            json!({ "provider": "razorpay", "date": today().to_string() }),
        )
        .await;
    let sid = synced["imported"][0].as_str().unwrap().to_string();
    let ex = t
        .ok(
            tn,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    (sid, ex["settlement"]["utr"].as_str().unwrap().to_string())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_statement_is_all_or_nothing_with_line_errors_a_dry_run_and_no_duplicates(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let d = today();
    let (d1, d2, d3) = (d - Duration::days(2), d - Duration::days(1), d);
    let good = format!(
        "{HEADER}{},NEFT-SYNTH0001-ACME,SYNTH0001,,\"1,23,456.50\"\n{},Bank charges,,10.00,\n{},UPI/SYNTH0003,SYNTH0003,,500.00\n",
        dmy(d1),
        dmy(d2),
        dmy(d3)
    );

    // a file with problems is refused whole, every problem named with its line, and nothing is stored
    let bad = format!(
        "{HEADER}{},ok,SYNTH0001,,10.00\n32/13/2026,bad date,,,5.00\n{},both,,3.00,4.00\n{},comma,,,\"1,50\"\n",
        dmy(d1),
        dmy(d2),
        dmy(d3)
    );
    let (s, e) = import(&t, a, &bad).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["code"], "invalid_statement");
    assert_eq!(e["error"]["stored"], false);
    let errs = e["error"]["errors"].as_array().unwrap();
    assert_eq!(
        errs.iter()
            .map(|x| x["line"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![3, 4, 5],
        "{errs:?}"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_lines").await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_imports").await,
        0
    );

    // the dry run says the same thing, as a 200, and stores nothing either
    let (s, p) = t
        .call(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": bad }),
        )
        .await;
    assert_eq!((s, p["ok"].clone()), (StatusCode::OK, json!(false)), "{p}");
    assert_eq!(p["errors"].as_array().unwrap().len(), 3);

    // a good file: the preview counts, the import stores
    let p = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": good }),
        )
        .await;
    assert_eq!(p["ok"], true);
    assert_eq!(p["rows"], 3);
    assert_eq!(
        p["credits"],
        json!({ "count": 2, "total_minor": 12_395_650 })
    );
    assert_eq!(p["debits"], json!({ "count": 1, "total_minor": 1_000 }));
    assert_eq!(p["new_rows"], 3);
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_lines").await,
        0,
        "a preview stores nothing"
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": good, "filename": "oct.csv" }),
        )
        .await;
    assert_eq!(
        (r["added"].as_i64(), r["duplicates"].as_i64()),
        (Some(3), Some(0)),
        "{r}"
    );
    let ls = lines(&t, a, "").await;
    assert_eq!(ls.len(), 3);
    let big = ls.iter().find(|l| l["amount_minor"] == 12_345_650).unwrap();
    assert_eq!(big["reference"], "SYNTH0001");
    assert_eq!(big["direction"], "credit");
    assert!(
        ls.iter()
            .any(|l| l["amount_minor"] == -1_000 && l["direction"] == "debit")
    );

    // the same file again adds nothing; the preview already says so
    let p = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": good }),
        )
        .await;
    assert_eq!(
        (p["new_rows"].clone(), p["duplicate_rows"].clone()),
        (json!(0), json!(3))
    );
    assert!(p["already_imported_as"].is_i64());
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": good }),
        )
        .await;
    assert_eq!(
        (r["added"].as_i64(), r["duplicates"].as_i64()),
        (Some(0), Some(3))
    );

    // a file that overlaps it adds only what is new, and two identical rows in one file are two transactions
    let overlap = format!(
        "{HEADER}{},Bank charges,,10.00,\n{},UPI/SYNTH0003,SYNTH0003,,500.00\n{},Cash,,,7.00\n{},Cash,,,7.00\n",
        dmy(d2),
        dmy(d3),
        dmy(d3),
        dmy(d3)
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": overlap }),
        )
        .await;
    assert_eq!(
        (r["added"].as_i64(), r["duplicates"].as_i64()),
        (Some(2), Some(2)),
        "{r}"
    );
    assert_eq!(lines(&t, a, "?limit=100").await.len(), 5);
    let again = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": overlap }),
        )
        .await;
    assert_eq!(again["added"], 0);

    // every upload is on record
    let imports = t
        .ok(a, "GET", "/v1/bank_statements/imports", None, Value::Null)
        .await;
    let rows = imports["data"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().any(|i| i["filename"] == "oct.csv"
        && i["added"] == 3
        && i["credits_minor"] == 12_395_650));

    // a different organisation importing the same file gets its own lines
    let r = t
        .ok(
            b,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": good }),
        )
        .await;
    assert_eq!(r["added"], 3);
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_lines").await,
        8
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn layouts_and_custom_column_mappings_read_the_files_banks_produce(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let lay = t
        .ok(a, "GET", "/v1/bank_statements/layouts", None, Value::Null)
        .await;
    let names: Vec<_> = lay["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_string())
        .collect();
    for want in [
        "auto",
        "hdfc",
        "icici",
        "sbi",
        "axis",
        "kotak",
        "generic_split",
    ] {
        assert!(names.contains(&want.to_string()), "{names:?}");
    }
    // the same transaction in three layouts
    let hdfc = "Date,Narration,Chq./Ref.No.,Value Dt,Withdrawal Amt.,Deposit Amt.,Closing Balance\n01/10/26,NEFT-SYNTHA0001,SYNTHA0001,01/10/26,,\"2,500.00\",9\n";
    let sbi = "Txn Date,Value Date,Description,Ref No./Cheque No.,Debit,Credit,Balance\n01 Oct 2026,01 Oct 2026,NEFT-SYNTHA0001,SYNTHA0001,,\"2,500.00\",9\n";
    let semi = "Posted;Memo;In;Out;Ref\n2026-10-01;NEFT-SYNTHA0001;2500.00;;SYNTHA0001\n";
    let mut previews = Vec::new();
    previews.push(
        t.ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": hdfc, "layout": "hdfc" }),
        )
        .await,
    );
    previews.push(
        t.ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": sbi, "layout": "sbi" }),
        )
        .await,
    );
    previews.push(
        t.ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": semi, "mapping": { "date": "Posted", "narration": "Memo", "credit": "In", "debit": "Out", "reference": "Ref" } }),
        )
        .await,
    );
    for p in &previews {
        assert_eq!(p["ok"], true, "{p}");
        assert_eq!(p["sample"][0]["amount_minor"], 250_000);
        assert_eq!(p["sample"][0]["reference"], "SYNTHA0001");
        assert_eq!(p["sample"][0]["posted_on"], "2026-10-01");
    }
    // a layout that does not fit the file says which columns are missing instead of guessing
    let p = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": sbi, "layout": "axis" }),
        )
        .await;
    assert_eq!(p["ok"], false);
    assert!(
        p["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("not found"),
        "{p}"
    );
    // and a file too big is refused up front
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": hdfc, "layout": "nope" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_reference_in_the_narration_confirms_the_match_and_the_settlement_explains_the_bank_line(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let (sid, utr) = settled(&t, a).await;
    // the provider's settlement report carries the payment's bank reference too
    let rrn: String = sqlx::query_scalar(
        "SELECT bank_reference FROM payment_attempts WHERE bank_reference IS NOT NULL",
    )
    .fetch_one(&t.root)
    .await
    .expect("the simulator's Razorpay payment carries acquirer_data.rrn");
    assert_eq!(rrn.len(), 12);

    let csv = format!(
        "{HEADER}{},NEFT CR-{utr}-RAZORPAY SOFTWARE,,,\"976.40\"\n",
        dmy(today())
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": csv }),
        )
        .await;
    assert_eq!(r["matched"]["confirmed"], 1, "{r}");
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["settlement"]["status"], "matched", "{ex}");
    let m = &ex["bank_match"];
    assert_eq!(m["rule"], "reference_in_narration");
    assert_eq!(m["key_matched"], zyvor_api::bankmatch::norm_key(&utr));
    assert_eq!(m["amount_minor"], 97_640);
    assert_eq!(
        m["reference"],
        Value::Null,
        "the reference column was empty; the narration carried it"
    );
    assert_eq!(m["evidence"]["amount_equal"], true);
    assert_eq!(m["confirmed_by"], "system");
    // the difference from what we expected is the provider's fees, with the reason
    let diff = &ex["bank_difference"];
    assert_eq!(diff["bank_minor"], 97_640);
    assert_eq!(diff["unexplained_minor"], 0);
    assert_eq!(diff["difference_from_expected_minor"], -2_360);
    let reasons: Vec<_> = diff["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["reason"].as_str().unwrap().to_string())
        .collect();
    assert!(reasons.iter().any(|r| r.contains("fees")), "{reasons:?}");
    assert!(
        ex["narrative"]
            .as_str()
            .unwrap()
            .contains("matched on reference"),
        "{}",
        ex["narrative"]
    );
    assert_eq!(ex["payments"][0]["bank_reference"], rrn);

    // the line shows what it matched and why
    let l = lines(&t, a, "?status=matched").await;
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["match"]["subject_id"], sid.as_str());
    assert_eq!(l[0]["match"]["rule"], "reference_in_narration");
    assert!(lines(&t, a, "?status=unmatched").await.is_empty());
    let detail = t
        .ok(
            a,
            "GET",
            &format!("/v1/bank_statements/lines/{}", l[0]["id"]),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(detail["matches"][0]["status"], "confirmed");
    // no case for a credit that has been explained
    assert!(
        t.ok(a, "GET", "/v1/recon/cases?status=open", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_credit_that_arrives_before_its_settlement_report_is_matched_when_the_report_comes(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    // the statement first: a credit with a reference, nothing to match it yet, so a case is opened
    let csv = format!(
        "{HEADER}{},NEFT-UTRLATE000001-RZP,UTRLATE000001,,976.40\n",
        dmy(today())
    );
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=unmatched_bank_credit",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"].as_array().unwrap().len(), 1);
    assert_eq!(cases["data"][0]["status"], "open");
    assert_eq!(cases["data"][0]["age_days"], 0);

    // then the provider's settlement with that UTR
    let st = t
        .ok(
            a,
            "POST",
            "/v1/settlements/import",
            None,
            json!({ "provider": "razorpay", "provider_ref": "late_1", "utr": "utrlate000001", "settled_on": today().to_string(),
                    "items": [{ "kind": "payment", "provider_ref": "pay_LATE1", "amount_minor": 97_640 }] }),
        )
        .await;
    assert_eq!(st["bank_match"]["rule"], "reference_column", "{st}");
    assert_eq!(st["bank_match"]["reference"], "UTRLATE000001");
    // the credit's case closed itself
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=unmatched_bank_credit",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"][0]["status"], "resolved", "{cases}");
    assert!(
        cases["data"][0]["explanation"]
            .as_str()
            .unwrap()
            .contains("credited")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_credit_that_differs_from_the_settlement_is_matched_by_reference_and_opens_a_case(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let (sid, utr) = settled(&t, a).await;
    let csv = format!("{HEADER}{},NEFT-{utr},,,970.00\n", dmy(today()));
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{sid}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["bank_match"]["amount_minor"], 97_000);
    assert_eq!(ex["bank_difference"]["unexplained_minor"], -640);
    assert_eq!(ex["settlement"]["status"], "exception");
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=bank_amount_mismatch",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"][0]["diff_minor"], -640);
    assert!(
        ex["narrative"]
            .as_str()
            .unwrap()
            .contains("less than what the provider says"),
        "{}",
        ex["narrative"]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn amount_and_date_alone_only_suggest_and_a_person_confirms_or_rejects(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let d = today();
    let import_settlement = |r: &'static str, net: i64| {
        json!({ "provider": "razorpay", "provider_ref": r, "settled_on": d.to_string(),
                "items": [{ "kind": "payment", "provider_ref": format!("pay_{r}"), "amount_minor": net }] })
    };
    let s1 = t
        .ok(
            a,
            "POST",
            "/v1/settlements/import",
            None,
            import_settlement("w_1", 45_000),
        )
        .await["settlement"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let s2 = t
        .ok(
            a,
            "POST",
            "/v1/settlements/import",
            None,
            import_settlement("w_2", 30_000),
        )
        .await["settlement"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // credits with the same amounts, a day later, and no reference anywhere
    let next = d + Duration::days(1);
    let csv = format!(
        "{HEADER}{},NEFT RZP SETTLEMENT,,,450.00\n{},NEFT RZP SETTLEMENT,,,300.00\n",
        dmy(next),
        dmy(next)
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": csv }),
        )
        .await;
    assert_eq!(r["matched"]["confirmed"], 0, "never auto-confirmed: {r}");
    assert_eq!(r["matched"]["suggested"], 2);
    for sid in [&s1, &s2] {
        let ex = t
            .ok(
                a,
                "GET",
                &format!("/v1/settlements/{sid}/explain"),
                None,
                Value::Null,
            )
            .await;
        assert!(ex["bank_match"].is_null(), "{ex}");
        let sug = ex["suggested_bank_matches"].as_array().unwrap();
        assert_eq!(sug.len(), 1);
        assert_eq!(sug[0]["rule"], "amount_date_window");
        assert_eq!(sug[0]["evidence"]["days_apart"], 1);
        assert_eq!(sug[0]["evidence"]["amount_equal"], true);
        assert!(
            ex["narrative"]
                .as_str()
                .unwrap()
                .contains("a person has to confirm")
        );
        assert_ne!(ex["settlement"]["status"], "matched");
    }
    let sugg = lines(&t, a, "?status=suggested").await;
    assert_eq!(sugg.len(), 2);
    // the credit still has its case, which says a match is waiting
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=unmatched_bank_credit&status=open",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"].as_array().unwrap().len(), 2);
    assert!(
        cases["data"][0]["explanation"]
            .as_str()
            .unwrap()
            .contains("waiting for a person")
    );

    // confirm one, as a named person
    let line_450 = sugg.iter().find(|l| l["amount_minor"] == 45_000).unwrap();
    let mid = line_450["suggestions"][0]["id"].as_i64().unwrap();
    let (st, c) = t
        .call_as(
            a,
            "meera",
            "POST",
            &format!("/v1/bank_statements/matches/{mid}/confirm"),
            None,
            json!({ "note": "checked the narration with the bank" }),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "{c}");
    assert_eq!(c["decided_by"], "user:meera");
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{s1}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(ex["bank_match"]["rule"], "amount_date_window");
    assert_eq!(ex["bank_match"]["confirmed_by"], "user:meera");
    assert_eq!(ex["bank_match"]["confirmed_on"], "person");
    assert!(ex["suggested_bank_matches"].as_array().unwrap().is_empty());
    assert!(
        ex["narrative"]
            .as_str()
            .unwrap()
            .contains("confirmed by a person")
    );
    // its case closed; the other credit's stays
    let open = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=unmatched_bank_credit&status=open",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(open["data"].as_array().unwrap().len(), 1);
    assert_eq!(open["data"][0]["actual_minor"], 30_000);
    // a decision is final
    let (st, e) = t
        .call(
            a,
            "POST",
            &format!("/v1/bank_statements/matches/{mid}/confirm"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(
        (st, e["error"]["code"].clone()),
        (StatusCode::CONFLICT, json!("not_suggested"))
    );

    // reject the other: it is not suggested again, and the credit stays unmatched with its case
    let line_300 = lines(&t, a, "?status=suggested").await;
    assert_eq!(line_300.len(), 1);
    let mid2 = line_300[0]["suggestions"][0]["id"].as_i64().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/bank_statements/matches/{mid2}/reject"),
        None,
        json!({ "note": "different customer" }),
    )
    .await;
    t.ok(a, "POST", "/v1/bank_statements/rematch", None, json!({}))
        .await;
    let ex = t
        .ok(
            a,
            "GET",
            &format!("/v1/settlements/{s2}/explain"),
            None,
            Value::Null,
        )
        .await;
    assert!(
        ex["suggested_bank_matches"].as_array().unwrap().is_empty(),
        "{ex}"
    );
    assert!(ex["bank_match"].is_null());
    assert_eq!(lines(&t, a, "?status=unmatched").await.len(), 1);
    let detail = t
        .ok(
            a,
            "GET",
            &format!("/v1/bank_statements/lines/{}", line_300[0]["id"]),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(detail["matches"][0]["status"], "rejected");
    assert!(detail["matches"][0]["decided_by"].as_str().is_some());
    assert_eq!(t.count("SELECT count(*) FROM audit_events WHERE action IN ('bank_match.confirmed', 'bank_match.rejected')").await, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_weak_match_is_not_made_outside_the_window_or_for_another_amount(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let d = today();
    t.ok(
        a,
        "POST",
        "/v1/settlements/import",
        None,
        json!({ "provider": "razorpay", "provider_ref": "far_1", "settled_on": d.to_string(),
        "items": [{ "kind": "payment", "provider_ref": "pay_FAR1", "amount_minor": 45_000 }] }),
    )
    .await;
    let csv = format!(
        "{HEADER}{},too late,,,450.00\n{},wrong amount,,,450.01\n",
        dmy(d + Duration::days(4)),
        dmy(d)
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": csv }),
        )
        .await;
    assert_eq!(
        r["matched"],
        json!({ "confirmed": 0, "suggested": 0 }),
        "{r}"
    );
    assert_eq!(t.count("SELECT count(*) FROM bank_line_matches").await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn debits_match_payouts_by_their_bank_reference_and_only_suggest_by_amount(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let v = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Synthetic Stationers", "account_number": "0000111122223", "ifsc": "SYNT0000001", "external_ref": "V1" })).await;
    let vid = v["id"].as_str().unwrap().to_string();
    let mut ids = Vec::new();
    for (i, (amount, bank_ref)) in [
        (500_000, "SYNTHPO000001"),
        (250_000, "SYNTHPO000002"),
        (125_000, "SYNTHPO000003"),
    ]
    .into_iter()
    .enumerate()
    {
        let p = t
            .call_as(a, "priya", "POST", "/v1/payouts", None, json!({ "vendor_id": vid, "amount_minor": amount, "purpose": "Supplies", "external_ref": format!("PO-{i}") }))
            .await
            .1;
        let pid = p["id"].as_str().unwrap().to_string();
        t.call_as(
            a,
            "ravi",
            "POST",
            &format!("/v1/payouts/{pid}/approve"),
            None,
            json!({}),
        )
        .await;
        let (s, e) = t
            .call_as(
                a,
                "priya",
                "POST",
                &format!("/v1/payouts/{pid}/mark_paid"),
                None,
                json!({ "bank_ref": bank_ref }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{e}");
        ids.push(pid);
    }
    let d = today();
    // 1: reference in the narration; 2: right amount, no reference; 3: the reference with the wrong amount
    let csv = format!(
        "{HEADER}{d0},IMPS/SYNTHPO000001/SYNTHETIC STATIONERS,,5000.00,\n{d0},TRANSFER TO SOMEONE,,2500.00,\n{d0},NEFT SYNTHPO000003,,1200.00,\n",
        d0 = dmy(d)
    );
    let r = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": csv }),
        )
        .await;
    assert_eq!(r["matched"]["confirmed"], 1, "{r}");
    assert_eq!(r["matched"]["suggested"], 2, "{r}");
    let ls = lines(&t, a, "?direction=debit").await;
    let by_amt = |m: i64| ls.iter().find(|l| l["amount_minor"] == -m).unwrap().clone();
    let l1 = by_amt(500_000);
    assert_eq!(
        (
            l1["status"].clone(),
            l1["match"]["subject_type"].clone(),
            l1["match"]["subject_id"].clone()
        ),
        (json!("matched"), json!("payout"), json!(ids[0]))
    );
    assert_eq!(l1["match"]["rule"], "reference_in_narration");
    assert_eq!(l1["match"]["key_matched"], "SYNTHPO000001");
    let l2 = by_amt(250_000);
    assert_eq!(l2["status"], "suggested");
    assert_eq!(l2["suggestions"][0]["rule"], "amount_date_window");
    assert_eq!(l2["suggestions"][0]["subject_id"], ids[1].as_str());
    let l3 = by_amt(120_000);
    assert_eq!(l3["status"], "suggested");
    assert_eq!(l3["suggestions"][0]["rule"], "reference_amount_differs");
    assert_eq!(l3["suggestions"][0]["subject_id"], ids[2].as_str());
    // debits are not opened as cases (a statement is full of charges and transfers that are not ours)
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'unmatched_bank_debit'")
            .await,
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn settlements_and_payouts_the_statement_never_shows_become_aged_cases_once_it_covers_them(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let d = today();
    let old = d - Duration::days(6);
    t.ok(a, "POST", "/v1/settlements/import", None, json!({ "provider": "razorpay", "provider_ref": "nb_1", "utr": "UTRNOBANK0001", "settled_on": old.to_string(),
        "items": [{ "kind": "payment", "provider_ref": "pay_NB1", "amount_minor": 99_000 }] })).await;
    // no statement at all yet: nothing can be called missing
    let n = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=3",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(n["settlements_without_bank_credit"], 0);
    // a statement that stops before the settlement is not evidence either
    let csv = format!(
        "{HEADER}{},unrelated interest,,,1.00\n",
        dmy(old + Duration::days(1))
    );
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let n = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=3",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(n["settlements_without_bank_credit"], 0);
    // one that runs past it is
    let csv = format!("{HEADER}{},unrelated interest,,,2.00\n", dmy(d));
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let n = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=3",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(n["settlements_without_bank_credit"], 1);
    let n = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=3",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        n["settlements_without_bank_credit"], 0,
        "once per settlement"
    );
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=settlement_no_bank_credit",
            None,
            Value::Null,
        )
        .await;
    let case = cases["data"][0].clone();
    assert_eq!(case["expected_minor"], 99_000);
    let cid = case["id"].as_str().unwrap();

    // cases are assignable like any other, and age
    t.ok(
        a,
        "POST",
        &format!("/v1/recon/cases/{cid}/assign"),
        None,
        json!({ "assignee": "meera" }),
    )
    .await;
    sqlx::query("UPDATE recon_cases SET created_at = now() - interval '10 days' WHERE id = $1")
        .bind(cid)
        .execute(&t.root)
        .await
        .unwrap();
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=settlement_no_bank_credit",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"][0]["age_days"], 10);
    let ag = t.ok(a, "GET", "/v1/recon/ageing", None, Value::Null).await;
    assert_eq!(
        ag["by_kind"]["settlement_no_bank_credit"]["buckets"]["8-14"], 1,
        "{ag}"
    );
    assert_eq!(ag["by_kind"]["settlement_no_bank_credit"]["unassigned"], 0);
    assert_eq!(ag["by_kind"]["unmatched_bank_credit"]["unassigned"], 2);
    assert!(ag["oldest_days"].as_i64().unwrap() >= 10);

    // the credit shows up late with the reference: the case closes by itself
    let csv = format!("{HEADER}{},NEFT-UTRNOBANK0001,,,990.00\n", dmy(d));
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let cases = t
        .ok(
            a,
            "GET",
            "/v1/recon/cases?kind=settlement_no_bank_credit",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cases["data"][0]["status"], "resolved", "{cases}");
    let ag = t.ok(a, "GET", "/v1/recon/ageing", None, Value::Null).await;
    assert!(ag["by_kind"].get("settlement_no_bank_credit").is_none());

    // a payout recorded as paid with no debit on a later statement
    let v = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Synthetic Vendor", "account_number": "0000999988887", "ifsc": "SYNT0000001", "external_ref": "V2" })).await;
    let p = t.call_as(a, "priya", "POST", "/v1/payouts", None, json!({ "vendor_id": v["id"], "amount_minor": 70_000, "purpose": "x", "external_ref": "PO-X" })).await.1;
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
    t.call_as(
        a,
        "priya",
        "POST",
        &format!("/v1/payouts/{pid}/mark_paid"),
        None,
        json!({ "bank_ref": "SYNTHPO999999" }),
    )
    .await;
    let n = t
        .ok(
            a,
            "POST",
            "/v1/recon/scan?older_than_days=0",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(n["payouts_without_bank_debit"], 1, "{n}");
}

#[sqlx::test(migrations = "./migrations")]
async fn one_organisation_cannot_see_or_decide_anothers_bank_lines(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let d = today();
    t.ok(
        a,
        "POST",
        "/v1/settlements/import",
        None,
        json!({ "provider": "razorpay", "provider_ref": "iso_1", "settled_on": d.to_string(),
        "items": [{ "kind": "payment", "provider_ref": "pay_ISO1", "amount_minor": 12_300 }] }),
    )
    .await;
    let csv = format!(
        "{HEADER}{},NEFT SYNTHISO0001,SYNTHISO0001,,123.00\n",
        dmy(d)
    );
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
    let la = lines(&t, a, "").await;
    assert_eq!(la.len(), 1);
    let line_id = la[0]["id"].as_i64().unwrap();
    let match_id = la[0]["suggestions"][0]["id"].as_i64().unwrap();

    // B sees nothing of it, by any door
    assert!(lines(&t, b, "").await.is_empty());
    assert!(
        t.ok(b, "GET", "/v1/bank_statements/imports", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/bank_statements/lines/{line_id}"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for verb in ["confirm", "reject"] {
        let (s, _) = t
            .call(
                b,
                "POST",
                &format!("/v1/bank_statements/matches/{match_id}/{verb}"),
                None,
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{verb}");
    }
    assert!(
        t.ok(b, "GET", "/v1/recon/ageing", None, Value::Null).await["by_kind"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    // ... and A's suggestion is untouched
    assert_eq!(lines(&t, a, "?status=suggested").await.len(), 1);

    // the database itself hides the rows when no organisation is named (row-level security on the new tables)
    for table in [
        "bank_statement_imports",
        "bank_line_matches",
        "bank_statement_lines",
    ] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.state.pool)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table} is visible without a tenant");
    }
    // a tenant cannot write a row for another tenant either
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
        .await
        .unwrap();
    let r = sqlx::query("INSERT INTO bank_statement_imports (tenant_id, source, layout, file_sha256, rows_total, rows_added, rows_duplicate, created_by) VALUES ($1, 'csv', 'auto', 'x', 0, 0, 0, 'x')")
        .bind(&a.id)
        .execute(&mut *tx)
        .await;
    assert!(r.is_err(), "WITH CHECK refuses a row for another tenant");
}

#[sqlx::test(migrations = "./migrations")]
async fn importing_needs_the_write_scope_and_a_preview_only_the_read_scope(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let mk = |scopes: Value| {
        let t = &t;
        async move {
            let k = t
                .ok(
                    &t.a,
                    "POST",
                    "/v1/api_keys",
                    None,
                    json!({ "name": "k", "scopes": scopes }),
                )
                .await;
            Tenant {
                id: t.a.id.clone(),
                key: k["key"].as_str().unwrap().into(),
            }
        }
    };
    let reader = mk(json!(["recon:read"])).await;
    let nothing = mk(json!(["payments:read"])).await;
    let csv = format!("{HEADER}{},x,SYNTHSC0001,,5.00\n", dmy(today()));
    assert_eq!(
        t.call(
            &reader,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": csv })
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(
            &reader,
            "GET",
            "/v1/bank_statements/lines",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        t.call(&reader, "GET", "/v1/recon/ageing", None, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    for (m, p) in [
        ("POST", "/v1/bank_statements/import_csv"),
        ("POST", "/v1/bank_statements/rematch"),
        ("POST", "/v1/bank_statements/matches/1/confirm"),
        ("POST", "/v1/bank_statements/matches/1/reject"),
    ] {
        assert_eq!(
            t.call(&reader, m, p, None, json!({ "csv": csv })).await.0,
            StatusCode::FORBIDDEN,
            "{p}"
        );
    }
    for (m, p) in [
        ("POST", "/v1/bank_statements/preview"),
        ("GET", "/v1/bank_statements/lines"),
        ("GET", "/v1/bank_statements/imports"),
        ("GET", "/v1/bank_statements/layouts"),
        ("GET", "/v1/recon/ageing"),
    ] {
        assert_eq!(
            t.call(&nothing, m, p, None, json!({ "csv": csv })).await.0,
            StatusCode::FORBIDDEN,
            "{p}"
        );
    }
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_lines").await,
        0
    );
    // the owner key can
    t.ok(
        a,
        "POST",
        "/v1/bank_statements/import_csv",
        None,
        json!({ "csv": csv }),
    )
    .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn simulated_payments_carry_a_bank_reference_and_a_test_reset_clears_the_new_tables(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let r = t.ok_public("POST", "/v1/signup", json!({ "organisation_name": "Sim Bank", "email": "s@bank.in", "password": "correct horse battery" })).await;
    let tn = Tenant {
        id: r["test_tenant"].as_str().unwrap().into(),
        key: r["test_key"].as_str().unwrap().into(),
    };
    let i = t
        .ok(
            &tn,
            "POST",
            "/v1/payment_intents",
            Some("sr-1"),
            json!({ "amount_minor": 100_000, "currency": "INR" }),
        )
        .await;
    t.ok(
        &tn,
        "POST",
        &format!("/v1/payment_intents/{}/attempts", i["id"].as_str().unwrap()),
        Some("sra-1"),
        json!({ "method": "upi", "vpa": "ok@bank" }),
    )
    .await;
    let refs: Vec<String> = sqlx::query_scalar(
        "SELECT bank_reference FROM payment_attempts WHERE tenant_id = $1 AND status = 'captured'",
    )
    .bind(&tn.id)
    .fetch_all(&t.root)
    .await
    .unwrap();
    assert_eq!(refs.len(), 1);
    assert!(
        refs[0].len() == 12 && refs[0].bytes().all(|b| b.is_ascii_digit()),
        "{refs:?}"
    );

    // a credit carrying that reference is matched to the payment (a direct credit, not a settlement)
    let csv = format!(
        "{HEADER}{},UPI/{}/SYNTH PAYER,,,1000.00\n",
        dmy(today()),
        refs[0]
    );
    let r = t
        .ok(
            &tn,
            "POST",
            "/v1/bank_statements/import_csv",
            None,
            json!({ "csv": csv }),
        )
        .await;
    assert_eq!(r["matched"]["confirmed"], 1, "{r}");
    let l = lines(&t, &tn, "?status=matched").await;
    assert_eq!(l[0]["match"]["subject_type"], "payment_attempt");
    assert_eq!(l[0]["match"]["rule"], "reference_in_narration");
    assert_eq!(
        t.ok(&tn, "GET", "/v1/recon/cases?status=open", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    t.ok(
        &tn,
        "POST",
        "/v1/test_data/reset",
        None,
        json!({ "confirm": "reset test data" }),
    )
    .await;
    for table in [
        "bank_statement_lines",
        "bank_line_matches",
        "bank_statement_imports",
    ] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE tenant_id = '{}'",
            tn.id
        )))
        .fetch_one(&t.root)
        .await
        .unwrap();
        assert_eq!(n, 0, "{table}");
    }
}

/// The same three transactions as each layout's file: every named layout (and auto) reads them identically.
#[sqlx::test(migrations = "./migrations")]
async fn every_fixture_layout_reads_the_same_three_transactions(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let fixtures: [(&str, &str, &str); 9] = [
        ("hdfc", include_str!("fixtures/bank/hdfc.csv"), "hdfc"),
        ("icici", include_str!("fixtures/bank/icici.csv"), "icici"),
        ("sbi", include_str!("fixtures/bank/sbi.csv"), "sbi"),
        ("axis", include_str!("fixtures/bank/axis.csv"), "axis"),
        ("kotak", include_str!("fixtures/bank/kotak.csv"), "kotak"),
        (
            "generic_split",
            include_str!("fixtures/bank/generic_split.csv"),
            "generic_split",
        ),
        (
            "generic_signed",
            include_str!("fixtures/bank/generic_signed.csv"),
            "generic_signed",
        ),
        (
            "generic_drcr",
            include_str!("fixtures/bank/generic_drcr.csv"),
            "generic_drcr",
        ),
        (
            "auto on hdfc",
            include_str!("fixtures/bank/hdfc.csv"),
            "auto",
        ),
    ];
    for (name, csv, layout) in fixtures {
        let p = t
            .ok(
                a,
                "POST",
                "/v1/bank_statements/preview",
                None,
                json!({ "csv": csv, "layout": layout }),
            )
            .await;
        assert_eq!(p["ok"], true, "{name}: {p}");
        assert_eq!(p["rows"], 3, "{name}");
        let got: Vec<(String, i64, Option<String>)> = p["sample"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["posted_on"].as_str().unwrap().to_string(),
                    r["amount_minor"].as_i64().unwrap(),
                    r["reference"].as_str().map(str::to_string),
                )
            })
            .collect();
        // the Dr/Cr layout has no reference column
        let r = |x: &str| (layout != "generic_drcr").then(|| x.to_string());
        assert_eq!(
            got,
            vec![
                ("2026-10-01".to_string(), 250_000, r("SYNTHA0001")),
                ("2026-10-02".to_string(), -1_000, None),
                ("2026-10-03".to_string(), 12_345_650, r("SYNTHB0002")),
            ],
            "{name}"
        );
        assert_eq!(p["credits"]["total_minor"], 12_595_650, "{name}");
        assert_eq!(p["debits"]["total_minor"], 1_000, "{name}");
        // every named layout's file is also found without naming the layout
        let auto = t
            .ok(
                a,
                "POST",
                "/v1/bank_statements/preview",
                None,
                json!({ "csv": csv }),
            )
            .await;
        assert_eq!(auto["rows"], 3, "{name} on auto: {auto}");
    }
    // the balances in the file follow from the amounts, so no running-balance warning
    let p = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": include_str!("fixtures/bank/hdfc.csv") }),
        )
        .await;
    assert!(
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["column"] != "balance"),
        "{p}"
    );
    // the footer row is noted, not silently dropped
    assert!(
        p["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["message"].as_str().unwrap().contains("no date"))
    );

    // a file with several kinds of problem names each of them on its own line
    let bad = include_str!("fixtures/bank/bad_mixed_problems.csv");
    let p = t
        .ok(
            a,
            "POST",
            "/v1/bank_statements/preview",
            None,
            json!({ "csv": bad }),
        )
        .await;
    assert_eq!(p["ok"], false);
    let got: Vec<(i64, String)> = p["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["line"].as_i64().unwrap(),
                e["column"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (3, "date".into()),
            (4, "credit".into()),
            (5, "credit".into()),
            (6, "reference".into())
        ]
    );
    let (s, _) = import(&t, a, bad).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        t.count("SELECT count(*) FROM bank_statement_lines").await,
        0,
        "all or nothing"
    );
}
