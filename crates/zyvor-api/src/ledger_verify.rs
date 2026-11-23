// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `GET /v1/ledger/verify`: re-check the whole ledger of the calling organisation against the ledger rules
//! (debits = credits per transaction and per currency, positive amounts, valid reversals, equal trial balance)
//! and report every violation. Strictly read-only: it runs in a tenant-scoped transaction, writes nothing (not
//! even an audit row) and never "fixes" anything; the rules themselves live in `zyvor_ledger::audit`.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use serde_json::{Value, json};
use zyvor_ledger::audit::{self, EntryRow, TxnRow};

use crate::{ApiError, AppState, auth::Auth};

/// Above this many entries the check is refused instead of loading an unbounded amount of rows into memory.
pub const MAX_ENTRIES: i64 = 1_000_000;
/// Violations listed in the response; the counts by kind always cover all of them.
pub const MAX_LISTED: usize = 500;

/// The JSON for a report: counts, trial balance per currency, violations (capped) and totals by kind.
pub fn report_json(r: &audit::Report) -> Value {
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for v in &r.violations {
        *by_kind.entry(v.kind.as_str()).or_default() += 1;
    }
    let errors = r
        .violations
        .iter()
        .filter(|v| v.kind.severity() == audit::Severity::Error)
        .count();
    let listed: Vec<Value> = r
        .violations
        .iter()
        .take(MAX_LISTED)
        .map(|v| {
            json!({
                "kind": v.kind.as_str(),
                "severity": v.kind.severity().as_str(),
                "subject": v.subject,
                "detail": v.detail,
            })
        })
        .collect();
    json!({
        "ok": r.ok(),
        "transactions_checked": r.transactions_checked,
        "entries_checked": r.entries_checked,
        "errors": errors,
        "warnings": r.violations.len() - errors,
        "violations_by_kind": by_kind,
        "violations": listed,
        "violations_truncated": r.violations.len() > MAX_LISTED,
        // amounts are strings: sums can exceed what JSON numbers carry exactly
        "trial_balance": r.trial_balance.iter().map(|t| json!({
            "currency": t.currency,
            "debits_minor": t.debits.to_string(),
            "credits_minor": t.credits.to_string(),
            "balanced": t.balanced(),
        })).collect::<Vec<_>>(),
    })
}

pub async fn verify(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    // One snapshot for all three reads, so a transaction posted meanwhile cannot make a half-seen one look
    // unbalanced. The isolation level must be chosen by BEGIN itself, so this does not use `tenant::begin`;
    // the tenant scope (row-level security) is set the same way, as a transaction-local setting.
    let mut conn = s.pool.acquire().await?;
    let tx = &mut *conn;
    sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let read = read_rows(&mut *tx, &auth.tenant_id).await;
    // always end the snapshot, whatever the reads returned
    let _ = sqlx::query("ROLLBACK").execute(&mut *tx).await;
    let (txns, entries) = read?;
    Ok(Json(report_json(&audit::check(&txns, &entries))))
}

async fn read_rows(
    tx: &mut sqlx::PgConnection,
    tenant_id: &str,
) -> Result<(Vec<TxnRow>, Vec<EntryRow>), ApiError> {
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM ledger_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
    if n > MAX_ENTRIES {
        return Err(ApiError::invalid(
            "ledger_too_large",
            format!("{n} entries is more than the {MAX_ENTRIES} one check will load"),
        ));
    }
    let txns: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, reference, reverses FROM ledger_transactions WHERE tenant_id = $1 ORDER BY id",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let entries: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT transaction_id, account, side, amount_minor, currency FROM ledger_entries
         WHERE tenant_id = $1 ORDER BY id",
    )
    .bind(tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let txns: Vec<TxnRow> = txns
        .into_iter()
        .map(|(id, reference, reverses)| TxnRow {
            id,
            reference,
            reverses,
        })
        .collect();
    let entries: Vec<EntryRow> = entries
        .into_iter()
        .map(
            |(transaction_id, account, side, amount_minor, currency)| EntryRow {
                transaction_id,
                account,
                side,
                amount_minor,
                currency,
            },
        )
        .collect();
    Ok((txns, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_and_cap() {
        let txns = vec![TxnRow {
            id: "t".into(),
            reference: "r".into(),
            reverses: None,
        }];
        let entries = vec![
            EntryRow {
                transaction_id: "t".into(),
                account: "a".into(),
                side: "debit".into(),
                amount_minor: i64::MAX,
                currency: "INR".into(),
            },
            EntryRow {
                transaction_id: "t".into(),
                account: "b".into(),
                side: "credit".into(),
                amount_minor: i64::MAX - 1,
                currency: "INR".into(),
            },
        ];
        let j = report_json(&audit::check(&txns, &entries));
        assert_eq!(j["ok"], false);
        assert_eq!(j["errors"], 2); // unbalanced + trial balance
        assert_eq!(j["violations_by_kind"]["unbalanced"], 1);
        assert_eq!(j["trial_balance"][0]["debits_minor"], i64::MAX.to_string());
        assert_eq!(j["trial_balance"][0]["balanced"], false);
        assert_eq!(j["violations_truncated"], false);
    }

    #[test]
    fn listing_is_capped_but_counts_are_not() {
        let txns: Vec<TxnRow> = (0..600)
            .map(|i| TxnRow {
                id: format!("t{i}"),
                reference: format!("r{i}"),
                reverses: None,
            })
            .collect();
        let j = report_json(&audit::check(&txns, &[]));
        assert_eq!(j["violations"].as_array().unwrap().len(), MAX_LISTED);
        assert_eq!(j["violations_by_kind"]["no_entries"], 600);
        assert_eq!(j["violations_truncated"], true);
    }
}
