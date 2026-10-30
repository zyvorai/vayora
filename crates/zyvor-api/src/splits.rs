//! Split payments for marketplaces: a payment carries fixed shares for vendors. The merchant still
//! collects the whole amount; each share is recorded as owed to the vendor (a liability) when the
//! payment is captured, reversed in proportion when it is refunded, and cleared when the vendor is
//! paid out of that balance. Zyvor tracks who is owed what; it does not move money to vendors.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};
use zyvor_ledger::{Entry, Transaction, accounts};

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Deserialize, Clone, Debug)]
pub struct SplitIn {
    pub vendor_id: String,
    pub amount_minor: i64,
}

/// Shares must be positive, to distinct vendors, and together no more than the payment.
pub fn validate(splits: &[SplitIn], total: i64) -> Result<(), ApiError> {
    if splits.is_empty() || splits.len() > 20 {
        return Err(ApiError::invalid(
            "invalid_splits",
            "give between 1 and 20 splits",
        ));
    }
    if splits.iter().any(|s| s.amount_minor <= 0) {
        return Err(ApiError::invalid(
            "invalid_splits",
            "each split must be a positive amount",
        ));
    }
    let mut ids: Vec<&str> = splits.iter().map(|s| s.vendor_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != splits.len() {
        return Err(ApiError::invalid(
            "invalid_splits",
            "each vendor may appear once",
        ));
    }
    if splits.iter().map(|s| s.amount_minor).sum::<i64>() > total {
        return Err(ApiError::invalid(
            "splits_exceed_payment",
            "the splits add up to more than the payment",
        ));
    }
    Ok(())
}

/// Record the shares against an intent (inside the caller's transaction).
pub async fn attach(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    splits: &[SplitIn],
) -> Result<(), ApiError> {
    for s in splits {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM vendors WHERE id = $1 AND archived_at IS NULL)",
        )
        .bind(&s.vendor_id)
        .fetch_one(&mut *conn)
        .await?;
        if !ok {
            return Err(ApiError::not_found("vendor"));
        }
        sqlx::query("INSERT INTO payment_splits (id, tenant_id, intent_id, vendor_id, amount_minor) VALUES ($1,$2,$3,$4,$5)")
            .bind(new_id("spl")).bind(tenant_id).bind(intent_id).bind(&s.vendor_id).bind(s.amount_minor)
            .execute(&mut *conn).await?;
    }
    Ok(())
}

/// On capture: each share becomes owed to its vendor. Exactly once per split.
pub async fn accrue(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    currency: Currency,
) -> Result<(), ApiError> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as("SELECT id, vendor_id, amount_minor FROM payment_splits WHERE intent_id = $1 AND NOT accrued ORDER BY id FOR UPDATE")
        .bind(intent_id)
        .fetch_all(&mut *conn)
        .await?;
    for (id, vendor, amount) in rows {
        Transaction::new(vec![
            Entry::debit(accounts::COLLECTIONS, amount, currency),
            Entry::credit(accounts::vendor_payable(&vendor), amount, currency),
        ])
        .map_err(zyvor_ledger::StoreError::from)?
        .post(conn, tenant_id, &format!("split:{id}"), None)
        .await?;
        sqlx::query("UPDATE payment_splits SET accrued = true WHERE id = $1")
            .bind(&id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// After a refund is processed: bring each share's reversal up to its proportion of the refunded
/// total (a full refund reverses every share in full). `offset_account` receives the credit: collections
/// for refunds, adjustments for lost disputes, where the vendor bears their share of the loss. Safe to repeat.
pub async fn reverse_for_refund(
    conn: &mut PgConnection,
    tenant_id: &str,
    refund_id: &str,
    intent_id: &str,
    attempt_id: &str,
    currency: Currency,
    offset_account: &str,
) -> Result<(), ApiError> {
    let (refunded_total, payment_amount): (i64, i64) =
        sqlx::query_as("SELECT refunded_minor, amount_minor FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&mut *conn)
            .await?;
    if payment_amount <= 0 {
        return Ok(());
    }
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as("SELECT id, vendor_id, amount_minor, reversed_minor FROM payment_splits WHERE intent_id = $1 AND accrued ORDER BY id FOR UPDATE")
        .bind(intent_id)
        .fetch_all(&mut *conn)
        .await?;
    for (id, vendor, amount, reversed) in rows {
        let target = ((amount as i128 * refunded_total as i128) / payment_amount as i128) as i64;
        let delta = target.min(amount) - reversed;
        if delta <= 0 {
            continue;
        }
        Transaction::new(vec![
            Entry::debit(accounts::vendor_payable(&vendor), delta, currency),
            Entry::credit(offset_account, delta, currency),
        ])
        .map_err(zyvor_ledger::StoreError::from)?
        .post(conn, tenant_id, &format!("splitrev:{refund_id}:{id}"), None)
        .await?;
        sqlx::query("UPDATE payment_splits SET reversed_minor = reversed_minor + $2 WHERE id = $1")
            .bind(&id)
            .bind(delta)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Owed to a vendor right now, less what is already promised to unpaid balance-funded payouts.
pub async fn available(
    conn: &mut PgConnection,
    vendor_id: &str,
    currency: &str,
) -> Result<(i64, i64), sqlx::Error> {
    let (credit, debit): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side = 'credit'), 0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side = 'debit'), 0)::BIGINT
         FROM ledger_entries WHERE account = $1 AND currency = $2",
    )
    .bind(accounts::vendor_payable(vendor_id))
    .bind(currency)
    .fetch_one(&mut *conn)
    .await?;
    let reserved: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM payouts WHERE vendor_id = $1 AND source = 'balance' AND status IN ('pending_approval', 'approved') AND currency = $2")
        .bind(vendor_id)
        .bind(currency)
        .fetch_one(&mut *conn)
        .await?;
    Ok((credit - debit, credit - debit - reserved))
}

/// `GET /v1/vendors/{id}/balance`
pub async fn balance(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM vendors WHERE id = $1)")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    if !known {
        return Err(ApiError::not_found("vendor"));
    }
    let mut out = Vec::new();
    for ccy in ["INR", "USD"] {
        let (owed, free) = available(&mut tx, &id, ccy).await?;
        if owed != 0 || free != 0 {
            out.push(json!({ "currency": ccy, "owed_minor": owed, "available_minor": free }));
        }
    }
    let pending: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(s.amount_minor - s.reversed_minor), 0)::BIGINT FROM payment_splits s WHERE s.vendor_id = $1 AND NOT s.accrued").bind(&id).fetch_one(&mut *tx).await?;
    Ok(Json(
        json!({ "vendor_id": id, "balances": out, "awaiting_capture_minor": pending }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str, a: i64) -> SplitIn {
        SplitIn {
            vendor_id: v.into(),
            amount_minor: a,
        }
    }

    #[test]
    fn splits_must_fit_inside_the_payment() {
        assert!(validate(&[s("a", 600), s("b", 400)], 1000).is_ok());
        assert!(validate(&[s("a", 600), s("b", 401)], 1000).is_err());
        assert!(validate(&[], 1000).is_err() && validate(&[s("a", 0)], 1000).is_err());
        assert!(
            validate(&[s("a", 1), s("a", 1)], 1000).is_err(),
            "one share per vendor"
        );
    }
}
