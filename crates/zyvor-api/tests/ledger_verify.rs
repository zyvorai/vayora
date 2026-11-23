// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `GET /v1/ledger/verify`: a ledger written through the normal path verifies clean, a hand-damaged one is
//! reported without being changed, and one organisation never sees another's rows.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn clean_then_damaged_ledger(root: PgPool) {
    let t = setup(root.clone()).await;
    let a = t.a.clone();

    // post through the same code the API uses, in one database transaction (the balance trigger runs at commit)
    let mut c = root.begin().await.unwrap();
    for (i, amt) in [500i64, 1_25_000].into_iter().enumerate() {
        zyvor_ledger::capture_entries(
            zyvor_ledger::MoneyMode::Orchestrate,
            "razorpay",
            "org",
            amt,
            zyvor_core::Currency::Inr,
        )
        .unwrap()
        .post(&mut c, &a.id, &format!("capture:test{i}"), None)
        .await
        .unwrap();
    }
    c.commit().await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM ledger_entries")
        .fetch_one(&root)
        .await
        .unwrap();

    let j = t
        .ok(&a, "GET", "/v1/ledger/verify", None, json!(null))
        .await;
    assert_eq!(j["ok"], true, "{j}");
    assert_eq!(j["transactions_checked"], 2);
    assert_eq!(j["entries_checked"], 4);
    assert_eq!(j["trial_balance"][0]["currency"], "INR");
    assert_eq!(j["trial_balance"][0]["debits_minor"], "125500");
    assert_eq!(j["violations"].as_array().unwrap().len(), 0);

    // damage the ledger behind the triggers' back (the replica role skips them; local to this transaction)
    let mut c = root.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *c)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO ledger_transactions (id, tenant_id, reference) VALUES ('jt_bad', $1, 'bad')",
    )
    .bind(&a.id)
    .execute(&mut *c)
    .await
    .unwrap();
    sqlx::query("INSERT INTO ledger_entries (tenant_id, transaction_id, account, side, amount_minor, currency) VALUES ($1, 'jt_bad', 'x', 'debit', 7, 'INR'), ($1, 'jt_bad', 'y', 'credit', 6, 'INR')")
        .bind(&a.id)
        .execute(&mut *c)
        .await
        .unwrap();
    c.commit().await.unwrap();

    let j = t
        .ok(&a, "GET", "/v1/ledger/verify", None, json!(null))
        .await;
    assert_eq!(j["ok"], false, "{j}");
    assert_eq!(j["violations_by_kind"]["unbalanced"], 1);
    assert_eq!(j["violations_by_kind"]["trial_balance_mismatch"], 1);
    let subjects: Vec<&str> = j["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["kind"] == "unbalanced")
        .map(|v| v["subject"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["jt_bad"]);

    // read-only: nothing was added or changed by the checks
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM ledger_entries")
        .fetch_one(&root)
        .await
        .unwrap();
    assert_eq!(after, before + 2);

    // the other organisation sees an empty, clean ledger
    let jb = t
        .ok(&t.b, "GET", "/v1/ledger/verify", None, json!(null))
        .await;
    assert_eq!(jb["ok"], true);
    assert_eq!(jb["transactions_checked"], 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn rejects_an_unknown_key(root: PgPool) {
    let t = setup(root).await;
    let (s, _) = raw(
        &t.app,
        "zy_not_a_key",
        None,
        "GET",
        "/v1/ledger/verify",
        None,
        json!(null),
    )
    .await;
    assert!(s == StatusCode::UNAUTHORIZED || s == StatusCode::FORBIDDEN);
}
