//! Payouts: money the merchant sends to vendors. Zyvor does the controlling and the bookkeeping:
//! a request, a second person's approval (separation of duties; small amounts can be waved through
//! by a tenant threshold), the bank reference once the transfer is made from the merchant's own
//! bank, and the ledger entry. It does not move money: there is no payout rail adapter yet.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::{Currency, new_id};
use zyvor_ledger::{Entry, Transaction, accounts};

use crate::{ApiError, AppState, auth::Auth, tenant};

fn mask_tail(s: &str, keep: usize) -> String {
    let t: Vec<char> = s.chars().collect();
    t[t.len().saturating_sub(keep)..].iter().collect()
}

#[derive(Deserialize)]
pub struct NewVendor {
    name: String,
    external_ref: Option<String>,
    account_number: Option<String>,
    ifsc: Option<String>,
    vpa: Option<String>,
}

pub async fn create_vendor(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewVendor>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    if b.name.trim().is_empty() {
        return Err(ApiError::invalid("invalid_vendor", "name is required"));
    }
    if b.account_number.is_none() && b.vpa.is_none() {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "give a bank account (account_number and ifsc) or a UPI id (vpa)",
        ));
    }
    if let Some(a) = &b.account_number
        && (!(6..=20).contains(&a.len())
            || !a.chars().all(|c| c.is_ascii_digit())
            || b.ifsc
                .as_deref()
                .is_none_or(|i| i.len() != 11 || !i.chars().all(|c| c.is_ascii_alphanumeric())))
    {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "account_number must be 6-20 digits with an 11-character ifsc",
        ));
    }
    if b.vpa
        .as_deref()
        .is_some_and(|v| v.matches('@').count() != 1 || v.len() > 80)
    {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "vpa must look like name@bank",
        ));
    }
    let id = new_id("ven");
    let details = json!({ "account_number": b.account_number, "ifsc": b.ifsc, "vpa": b.vpa });
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO vendors (id, tenant_id, name, external_ref, account_last4, ifsc, vpa_masked, details_enc, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(&id).bind(&auth.tenant_id).bind(b.name.trim()).bind(&b.external_ref)
        .bind(b.account_number.as_deref().map(|a| mask_tail(a, 4)))
        .bind(&b.ifsc)
        .bind(b.vpa.as_deref().map(|v| format!("***{}", v.split_once('@').map(|(_, h)| format!("@{h}")).unwrap_or_default())))
        .bind(s.crypto.encrypt(details.to_string().as_bytes()))
        .bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.created",
        Some(&id),
        json!({ "name": b.name.trim() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "name": b.name.trim() })),
    ))
}

pub async fn list_vendors(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, name, external_ref, account_last4, ifsc, vpa_masked, archived_at FROM vendors ORDER BY name LIMIT 500",
    )
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, name, ext, last4, ifsc, vpa, arch)| json!({ "id": id, "name": name, "external_ref": ext, "account_last4": last4, "ifsc": ifsc, "vpa": vpa, "archived": arch.is_some() })).collect();
    Ok(Json(json!({ "data": data })))
}

/// The full bank details, for the person about to make the transfer. Each reveal is audited.
pub async fn vendor_details(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let enc: Option<Option<Vec<u8>>> =
        sqlx::query_scalar("SELECT details_enc FROM vendors WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let blob = enc.flatten().ok_or_else(|| ApiError::not_found("vendor"))?;
    let details: Value = s
        .crypto
        .decrypt(&blob)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(ApiError::internal)?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.details_revealed",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "details": details })))
}

#[derive(Deserialize)]
pub struct NewPayout {
    vendor_id: String,
    amount_minor: i64,
    currency: Option<String>,
    purpose: String,
    /// Your own reference; repeating a request with the same one returns the original payout.
    external_ref: Option<String>,
    /// Pay out of what the vendor is owed from split payments, instead of your own funds.
    from_balance: Option<bool>,
}

const COLS: &str = "id, vendor_id, amount_minor, currency, status, purpose, external_ref, requested_by, approved_by, paid_by, bank_ref, created_at";
type Row = (
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
);

fn row_json(r: Row) -> Value {
    json!({ "id": r.0, "vendor_id": r.1, "amount_minor": r.2, "currency": r.3, "status": r.4, "purpose": r.5, "external_ref": r.6, "requested_by": r.7, "approved_by": r.8, "paid_by": r.9, "bank_ref": r.10, "created_at": r.11 })
}

async fn fetch(tx: &mut sqlx::PgConnection, id: &str, lock: bool) -> Result<Row, ApiError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM payouts WHERE id = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    )))
    .bind(id)
    .fetch_optional(tx)
    .await?
    .ok_or_else(|| ApiError::not_found("payout"))
}

/// Create one payout inside the caller's transaction. A repeated `external_ref` returns the original.
async fn create_one(
    tx: &mut sqlx::PgConnection,
    auth: &Auth,
    b: &NewPayout,
) -> Result<(StatusCode, Value), ApiError> {
    crate::onboarding::require_approved(tx).await?;
    let ccy: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if b.amount_minor <= 0
        || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR
        || b.purpose.trim().is_empty()
    {
        return Err(ApiError::invalid(
            "invalid_payout",
            "amount_minor must be positive and a purpose is required",
        ));
    }
    if let Some(r) = &b.external_ref {
        // a retried request returns the original, not a second payout
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM payouts WHERE external_ref = $1")
                .bind(r)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(id) = existing {
            return Ok((StatusCode::OK, row_json(fetch(tx, &id, false).await?)));
        }
    }
    // lock the vendor so two payouts cannot both spend the same balance
    let vendor_ok: Option<Option<DateTime<Utc>>> =
        sqlx::query_scalar("SELECT archived_at FROM vendors WHERE id = $1 FOR UPDATE")
            .bind(&b.vendor_id)
            .fetch_optional(&mut *tx)
            .await?;
    match vendor_ok {
        None => return Err(ApiError::not_found("vendor")),
        Some(Some(_)) => {
            return Err(ApiError::conflict(
                "vendor_archived",
                "this vendor is archived",
            ));
        }
        Some(None) => {}
    }
    let from_balance = b.from_balance.unwrap_or(false);
    if from_balance {
        let (_, free) = crate::splits::available(tx, &b.vendor_id, ccy.code()).await?;
        if b.amount_minor > free {
            return Err(ApiError::conflict(
                "insufficient_balance",
                format!(
                    "this vendor has {} free to pay out; the rest of what they are owed is already promised to other payouts",
                    crate::ai::money(free.max(0), ccy.code())
                ),
            ));
        }
    }
    let threshold: i64 = sqlx::query_scalar(
        "SELECT payout_approval_threshold_minor FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    let needs_approval = b.amount_minor > threshold;
    let id = new_id("pay");
    sqlx::query("INSERT INTO payouts (id, tenant_id, vendor_id, amount_minor, currency, status, purpose, external_ref, requested_by, source) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.vendor_id).bind(b.amount_minor).bind(ccy.code())
        .bind(if needs_approval { "pending_approval" } else { "approved" })
        .bind(b.purpose.trim()).bind(&b.external_ref).bind(&auth.actor).bind(if from_balance { "balance" } else { "own_funds" })
        .execute(&mut *tx).await?;
    tenant::audit(tx, &auth.tenant_id, &auth.actor, "payout.requested", Some(&id), json!({ "amount_minor": b.amount_minor, "needs_approval": needs_approval, "from_balance": from_balance })).await?;
    tenant::emit(
        tx,
        &auth.tenant_id,
        if needs_approval {
            "payout.approval_required"
        } else {
            "payout.approved"
        },
        json!({ "payout_id": id, "amount_minor": b.amount_minor, "currency": ccy.code() }),
    )
    .await?;
    Ok((StatusCode::CREATED, row_json(fetch(tx, &id, false).await?)))
}

pub async fn create_payout(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewPayout>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (code, out) = create_one(&mut tx, &auth, &b).await?;
    tx.commit().await?;
    Ok((code, Json(out)))
}

#[derive(Deserialize)]
pub struct BulkPayouts {
    payouts: Vec<NewPayout>,
}

/// `POST /v1/payouts/bulk`: up to 200 payouts, all created or none (salaries, vendor runs). Each follows the
/// same approval rules as a single payout; repeating the run with the same references changes nothing.
pub async fn create_bulk(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<BulkPayouts>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    if b.payouts.is_empty() || b.payouts.len() > 200 {
        return Err(ApiError::invalid(
            "invalid_bulk",
            "give between 1 and 200 payouts",
        ));
    }
    let mut refs: Vec<&str> = b
        .payouts
        .iter()
        .filter_map(|p| p.external_ref.as_deref())
        .collect();
    refs.sort_unstable();
    let n = refs.len();
    refs.dedup();
    if refs.len() != n {
        return Err(ApiError::invalid(
            "invalid_bulk",
            "external_ref must be unique within a bulk request",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (mut created, mut existing, mut total) = (Vec::new(), 0, 0i64);
    for (i, p) in b.payouts.iter().enumerate() {
        let (code, row) = create_one(&mut tx, &auth, p).await.map_err(|e| {
            ApiError::new(
                e.status,
                e.code,
                format!("payout {} of {}: {}", i + 1, b.payouts.len(), e.message),
            )
        })?;
        if code == StatusCode::CREATED {
            total += p.amount_minor;
        } else {
            existing += 1;
        }
        created.push(row);
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "count": created.len(), "already_existed": existing, "new_total_minor": total, "data": created }),
        ),
    ))
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

pub async fn list_payouts(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM payouts WHERE ($1::text IS NULL OR status = $1) ORDER BY created_at DESC, id DESC LIMIT 200")))
        .bind(&q.status)
        .fetch_all(&mut *tx)
        .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(row_json).collect::<Vec<_>>() }),
    ))
}

pub async fn get_payout(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(row_json(fetch(&mut tx, &id, false).await?)))
}

async fn decide(
    s: &AppState,
    auth: &Auth,
    id: &str,
    approve: bool,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = fetch(&mut tx, id, true).await?;
    if r.4 != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("payout is {}", r.4),
        ));
    }
    if r.7 == auth.actor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "separation_of_duties",
            "the person who requested a payout cannot approve or reject it",
        ));
    }
    let status = if approve { "approved" } else { "rejected" };
    sqlx::query(
        "UPDATE payouts SET status = $2, approved_by = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        if approve {
            "payout.approved"
        } else {
            "payout.rejected"
        },
        Some(id),
        json!({ "amount_minor": r.2 }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        if approve {
            "payout.approved"
        } else {
            "payout.rejected"
        },
        json!({ "payout_id": id, "amount_minor": r.2 }),
    )
    .await?;
    let out = row_json(fetch(&mut tx, id, false).await?);
    tx.commit().await?;
    Ok(Json(out))
}

pub async fn approve_payout(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    decide(&s, &auth, &id, true).await
}

pub async fn reject_payout(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    decide(&s, &auth, &id, false).await
}

#[derive(Deserialize)]
pub struct Paid {
    /// The bank's own reference (UTR) for the transfer you made.
    bank_ref: String,
}

/// Record that the transfer was made: posts Dr payouts / Cr bank, once.
pub async fn mark_paid(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<Paid>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let bank_ref = b.bank_ref.trim().to_string();
    if bank_ref.len() < 6 || bank_ref.len() > 40 {
        return Err(ApiError::invalid(
            "invalid_bank_ref",
            "bank_ref (the UTR) must be 6-40 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = fetch(&mut tx, &id, true).await?;
    if r.4 != "approved" {
        return Err(ApiError::conflict(
            "not_approved",
            format!("payout is {}", r.4),
        ));
    }
    let ccy: Currency = r.3.parse().map_err(|_| ApiError::internal())?;
    let source: String = sqlx::query_scalar("SELECT source FROM payouts WHERE id = $1")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("UPDATE payouts SET status = 'paid', paid_by = $2, bank_ref = $3, updated_at = now() WHERE id = $1")
        .bind(&id).bind(&auth.actor).bind(&bank_ref)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("bank_ref_used", "that bank reference is already recorded against another payout"),
            e => e.into(),
        })?;
    // paid from a vendor's balance, the liability is cleared; paid from your own funds, it is an expense
    let debit_account = if source == "balance" {
        accounts::vendor_payable(&r.1)
    } else {
        accounts::PAYOUTS.to_string()
    };
    Transaction::new(vec![
        Entry::debit(debit_account, r.2, ccy),
        Entry::credit(accounts::BANK, r.2, ccy),
    ])
    .map_err(zyvor_ledger::StoreError::from)?
    .post(&mut tx, &auth.tenant_id, &format!("payout:{id}"), None)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout.paid",
        Some(&id),
        json!({ "amount_minor": r.2, "bank_ref": bank_ref }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "payout.paid",
        json!({ "payout_id": id, "amount_minor": r.2, "currency": r.3, "bank_ref": bank_ref }),
    )
    .await?;
    let out = row_json(fetch(&mut tx, &id, false).await?);
    tx.commit().await?;
    Ok(Json(out))
}

pub async fn cancel_payout(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = fetch(&mut tx, &id, true).await?;
    if !matches!(r.4.as_str(), "pending_approval" | "approved") {
        return Err(ApiError::conflict(
            "cannot_cancel",
            format!("payout is {}", r.4),
        ));
    }
    sqlx::query("UPDATE payouts SET status = 'canceled', updated_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout.canceled",
        Some(&id),
        json!({}),
    )
    .await?;
    let out = row_json(fetch(&mut tx, &id, false).await?);
    tx.commit().await?;
    Ok(Json(out))
}
