//! Refunds: reservation-based over-refund protection, approval thresholds with separation of
//! duties, duplicate protection via idempotency, and resolution of unknown outcomes.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use zyvor_core::{Currency, new_id};
use zyvor_ledger::{Entry, Transaction, accounts};
use zyvor_providers::{ProviderError, RefundRequest, RefundStatus};

use crate::{
    AppState,
    auth::Auth,
    cases::open_case,
    error::ApiError,
    idempotency::{self, Begin, fingerprint, replay_response},
    payments::{ListQuery, refresh_invoice_status},
    registry, tenant,
};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct RefundRow {
    pub id: String,
    pub attempt_id: String,
    pub intent_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub reason: Option<String>,
    pub external_ref: Option<String>,
    pub requested_by: String,
    pub approved_by: Option<String>,
    pub provider_refund_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const COLS: &str = "id, attempt_id, intent_id, amount_minor, currency, status, reason, external_ref, requested_by, approved_by, provider_refund_ref, failure_reason, created_at, updated_at";

pub async fn fetch_refund(conn: &mut PgConnection, id: &str) -> Result<RefundRow, ApiError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM refunds WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("refund"))
}

#[derive(Deserialize, Default)]
pub struct CreateRefund {
    /// Omit to refund whatever is still refundable.
    amount_minor: Option<i64>,
    reason: Option<String>,
    /// The business record behind the refund, e.g. a pharmacy return id.
    external_ref: Option<String>,
}

pub async fn create_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(intent_id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CreateRefund>>,
) -> Result<Response, ApiError> {
    auth.require("refunds:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    if matches!(b.amount_minor, Some(a) if a <= 0) {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive integer",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let hash = fingerprint(&[
        &intent_id,
        &b.amount_minor
            .map(|a| a.to_string())
            .unwrap_or_else(|| "full".into()),
        b.reason.as_deref().unwrap_or(""),
        b.external_ref.as_deref().unwrap_or(""),
    ]);
    let key = match idempotency::begin(&mut tx, &auth.tenant_id, "refunds", &headers, &hash).await?
    {
        Begin::Replay {
            status,
            body,
            resource_id,
        } => {
            let body = match resource_id {
                Some(id) => serde_json::to_value(fetch_refund(&mut tx, &id).await?).unwrap(),
                None => body.unwrap_or(Value::Null),
            };
            return Ok(replay_response(status, body));
        }
        Begin::Proceed(k) => k,
    };

    // The attempt row lock serialises concurrent refunds of the same payment.
    let (attempt_id, total, refunded, reserved, ccy): (String, i64, i64, i64, String) = sqlx::query_as(
        "SELECT id, amount_minor, refunded_minor, reserved_refund_minor, currency FROM payment_attempts WHERE intent_id = $1 AND status = 'captured' FOR UPDATE",
    )
    .bind(&intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::conflict("payment_not_captured", "only captured payments can be refunded"))?;
    let remaining = total - refunded - reserved;
    let amount = b.amount_minor.unwrap_or(remaining);
    if amount <= 0 || amount > remaining {
        return Err(ApiError::invalid(
            "refund_exceeds_refundable",
            format!(
                "refundable amount remaining is {remaining} minor units (including refunds already in progress)"
            ),
        ));
    }
    let threshold: i64 = sqlx::query_scalar(
        "SELECT refund_approval_threshold_minor FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(5_000_000);
    let needs_approval = amount > threshold;

    sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor + $2, updated_at = now() WHERE id = $1").bind(&attempt_id).bind(amount).execute(&mut *tx).await?;
    let refund_id = new_id("rf");
    sqlx::query(
        "INSERT INTO refunds (id, tenant_id, attempt_id, intent_id, amount_minor, currency, status, reason, external_ref, requested_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&refund_id)
    .bind(&auth.tenant_id)
    .bind(&attempt_id)
    .bind(&intent_id)
    .bind(amount)
    .bind(&ccy)
    .bind(if needs_approval { "pending_approval" } else { "processing" })
    .bind(&b.reason)
    .bind(&b.external_ref)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    key.set_resource(&mut tx, &refund_id).await?;
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "refund.requested", Some(&refund_id), json!({ "amount_minor": amount, "needs_approval": needs_approval, "external_ref": b.external_ref })).await?;
    if needs_approval {
        tenant::emit(
            &mut tx,
            &auth.tenant_id,
            "refund.approval_required",
            json!({ "refund_id": refund_id, "amount_minor": amount }),
        )
        .await?;
        let row = fetch_refund(&mut tx, &refund_id).await?;
        let res = key.finish(&mut tx, StatusCode::CREATED, &row).await?;
        tx.commit().await?;
        return Ok(res);
    }
    tx.commit().await?;

    run_provider_refund(&s, &auth.tenant_id, &auth.env, &refund_id).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row = fetch_refund(&mut tx, &refund_id).await?;
    let res = key.finish(&mut tx, StatusCode::CREATED, &row).await?;
    tx.commit().await?;
    Ok(res)
}

/// Send a `processing` refund to the provider and record the result, including "unknown".
pub async fn run_provider_refund(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    refund_id: &str,
) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let r = fetch_refund(&mut tx, refund_id).await?;
    if r.status != "processing" {
        return Ok(());
    }
    let (provider_name, pref): (String, Option<String>) =
        sqlx::query_as("SELECT provider, provider_payment_ref FROM payment_attempts WHERE id = $1")
            .bind(&r.attempt_id)
            .fetch_one(&mut *tx)
            .await?;
    let provider = registry::load(s, &mut tx, tenant_id, env, &provider_name).await?;
    let currency: Currency = r.currency.parse().map_err(|_| ApiError::internal())?;
    tx.commit().await?;

    let result = provider
        .refund(&RefundRequest {
            refund_id: r.id.clone(),
            attempt_id: r.attempt_id.clone(),
            provider_payment_ref: pref.unwrap_or_default(),
            amount_minor: r.amount_minor,
            currency,
        })
        .await;

    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    match result {
        Ok(st) => match st.status {
            RefundStatus::Processed => {
                apply_refund_result(
                    &mut tx,
                    tenant_id,
                    refund_id,
                    true,
                    st.provider_refund_ref,
                    None,
                )
                .await?
            }
            RefundStatus::Failed => {
                apply_refund_result(
                    &mut tx,
                    tenant_id,
                    refund_id,
                    false,
                    st.provider_refund_ref,
                    st.failure_reason,
                )
                .await?
            }
            RefundStatus::Pending => {
                sqlx::query("UPDATE refunds SET provider_refund_ref = COALESCE($2, provider_refund_ref), updated_at = now() WHERE id = $1").bind(refund_id).bind(st.provider_refund_ref).execute(&mut *tx).await?;
                false
            }
        },
        Err(ProviderError::Uncertain(why)) => {
            sqlx::query("UPDATE refunds SET status = 'unknown', failure_reason = $2, updated_at = now() WHERE id = $1 AND status = 'processing'").bind(refund_id).bind(&why).execute(&mut *tx).await?;
            open_case(&mut tx, tenant_id, "unresolved_refund", "refund", refund_id, None, None, &format!("The provider did not confirm this refund ({why}). It stays reserved and will not be re-sent until its outcome is known."), json!({})).await?;
            false
        }
        Err(ProviderError::Rejected(why) | ProviderError::Unsupported(why)) => {
            apply_refund_result(&mut tx, tenant_id, refund_id, false, None, Some(why)).await?
        }
    };
    tx.commit().await?;
    Ok(())
}

/// Finish a refund exactly once. Duplicate webhooks and resolver runs find nothing to update.
pub async fn apply_refund_result(
    conn: &mut PgConnection,
    tenant_id: &str,
    refund_id: &str,
    processed: bool,
    provider_ref: Option<String>,
    reason: Option<String>,
) -> Result<bool, ApiError> {
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "UPDATE refunds SET status = $2, provider_refund_ref = COALESCE($3, provider_refund_ref), failure_reason = $4, updated_at = now()
         WHERE id = $1 AND status IN ('processing', 'unknown') RETURNING attempt_id, intent_id, amount_minor, currency",
    )
    .bind(refund_id)
    .bind(if processed { "processed" } else { "failed" })
    .bind(provider_ref)
    .bind(reason.as_deref().filter(|_| !processed))
    .fetch_optional(&mut *conn)
    .await?;
    let Some((attempt_id, intent_id, amount, ccy)) = row else {
        return Ok(false);
    };
    let currency: Currency = ccy.parse().map_err(|_| ApiError::internal())?;
    let provider: String =
        sqlx::query_scalar("SELECT provider FROM payment_attempts WHERE id = $1 FOR UPDATE")
            .bind(&attempt_id)
            .fetch_one(&mut *conn)
            .await?;

    if processed {
        sqlx::query("UPDATE payment_attempts SET refunded_minor = refunded_minor + $2, reserved_refund_minor = reserved_refund_minor - $2, updated_at = now() WHERE id = $1").bind(&attempt_id).bind(amount).execute(&mut *conn).await?;
        Transaction::new(vec![
            Entry::debit(accounts::COLLECTIONS, amount, currency),
            Entry::credit(accounts::provider_receivable(&provider), amount, currency),
        ])
        .map_err(zyvor_ledger::StoreError::from)?
        .post(conn, tenant_id, &format!("refund:{refund_id}"), None)
        .await?;
        // marketplace shares owed to vendors shrink in proportion to what has been refunded
        let (refunded_total, payment_amount): (i64, i64) = sqlx::query_as(
            "SELECT refunded_minor, amount_minor FROM payment_attempts WHERE id = $1",
        )
        .bind(&attempt_id)
        .fetch_one(&mut *conn)
        .await?;
        crate::splits::reverse_for_refund(
            conn,
            tenant_id,
            refund_id,
            &intent_id,
            refunded_total,
            payment_amount,
            currency,
        )
        .await?;
        // Linked invoice goes back to partially paid / open: a reversal row, never an edit.
        if let Some(inv) = sqlx::query_scalar::<_, Option<String>>(
            "SELECT invoice_id FROM payment_intents WHERE id = $1",
        )
        .bind(&intent_id)
        .fetch_one(&mut *conn)
        .await?
        {
            sqlx::query("INSERT INTO invoice_allocations (tenant_id, invoice_id, intent_id, amount_minor, reference) VALUES ($1, $2, $3, $4, $5)")
                .bind(tenant_id)
                .bind(&inv)
                .bind(&intent_id)
                .bind(-amount)
                .bind(format!("refund:{refund_id}"))
                .execute(&mut *conn)
                .await?;
            refresh_invoice_status(conn, &inv).await?;
        }
        tenant::emit(conn, tenant_id, "refund.processed", json!({ "refund_id": refund_id, "intent_id": intent_id, "attempt_id": attempt_id, "amount_minor": amount })).await?;
    } else {
        sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor - $2, updated_at = now() WHERE id = $1").bind(&attempt_id).bind(amount).execute(&mut *conn).await?;
        tenant::emit(conn, tenant_id, "refund.failed", json!({ "refund_id": refund_id, "intent_id": intent_id, "amount_minor": amount, "reason": reason })).await?;
    }
    sqlx::query("UPDATE recon_cases SET status = 'resolved', resolution = 'resolved automatically', resolved_at = now() WHERE kind = 'unresolved_refund' AND subject_id = $1 AND status <> 'resolved'").bind(refund_id).execute(&mut *conn).await?;
    Ok(true)
}

/// Ask the provider about a refund whose outcome we don't know.
pub async fn refresh_refund_internal(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    refund_id: &str,
) -> Result<RefundRow, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let r = fetch_refund(&mut tx, refund_id).await?;
    let (provider_name, pref): (String, Option<String>) =
        sqlx::query_as("SELECT provider, provider_payment_ref FROM payment_attempts WHERE id = $1")
            .bind(&r.attempt_id)
            .fetch_one(&mut *tx)
            .await?;
    let provider = registry::load(s, &mut tx, tenant_id, env, &provider_name).await?;
    tx.commit().await?;
    if !matches!(r.status.as_str(), "unknown" | "processing") {
        return Ok(r);
    }
    match provider
        .fetch_refund(&r.attempt_id, &r.id, pref.as_deref().unwrap_or_default())
        .await
    {
        Ok(st) => {
            let mut tx = tenant::begin(&s.pool, tenant_id).await?;
            match st.status {
                RefundStatus::Processed => {
                    apply_refund_result(
                        &mut tx,
                        tenant_id,
                        refund_id,
                        true,
                        st.provider_refund_ref,
                        None,
                    )
                    .await?;
                }
                RefundStatus::Failed => {
                    apply_refund_result(
                        &mut tx,
                        tenant_id,
                        refund_id,
                        false,
                        st.provider_refund_ref,
                        st.failure_reason,
                    )
                    .await?;
                }
                RefundStatus::Pending => {}
            }
            let row = fetch_refund(&mut tx, refund_id).await?;
            tx.commit().await?;
            Ok(row)
        }
        Err(e) => {
            tracing::warn!(refund = refund_id, error = %e, "could not refresh refund");
            Ok(r)
        }
    }
}

pub async fn refresh_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<RefundRow>, ApiError> {
    auth.require("refunds:write")?;
    Ok(Json(
        refresh_refund_internal(&s, &auth.tenant_id, &auth.env, &id).await?,
    ))
}

pub async fn approve_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<RefundRow>, ApiError> {
    auth.require("refunds:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r: RefundRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM refunds WHERE id = $1 FOR UPDATE"
    )))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("refund"))?;
    if r.status != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("refund is {}", r.status),
        ));
    }
    if r.requested_by == auth.actor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "separation_of_duties",
            "the person who requested a refund cannot approve it",
        ));
    }
    sqlx::query("UPDATE refunds SET status = 'processing', approved_by = $2, updated_at = now() WHERE id = $1").bind(&id).bind(&auth.actor).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "refund.approved",
        Some(&id),
        json!({ "amount_minor": r.amount_minor }),
    )
    .await?;
    tx.commit().await?;
    run_provider_refund(&s, &auth.tenant_id, &auth.env, &id).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(fetch_refund(&mut tx, &id).await?))
}

pub async fn reject_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<RefundRow>, ApiError> {
    auth.require("refunds:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r: RefundRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM refunds WHERE id = $1 FOR UPDATE"
    )))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("refund"))?;
    if r.status != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("refund is {}", r.status),
        ));
    }
    sqlx::query("UPDATE refunds SET status = 'rejected', approved_by = $2, updated_at = now() WHERE id = $1").bind(&id).bind(&auth.actor).execute(&mut *tx).await?;
    sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor - $2, updated_at = now() WHERE id = $1").bind(&r.attempt_id).bind(r.amount_minor).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "refund.rejected",
        Some(&id),
        json!({}),
    )
    .await?;
    let row = fetch_refund(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(row))
}

/// Re-send a refund the provider definitively refused. Re-reserves the amount first.
pub async fn retry_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<RefundRow>, ApiError> {
    auth.require("refunds:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r: RefundRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM refunds WHERE id = $1 FOR UPDATE"
    )))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("refund"))?;
    if r.status != "failed" {
        return Err(ApiError::conflict(
            "not_failed",
            format!("refund is {}; only failed refunds can be retried", r.status),
        ));
    }
    let (total, refunded, reserved): (i64, i64, i64) = sqlx::query_as("SELECT amount_minor, refunded_minor, reserved_refund_minor FROM payment_attempts WHERE id = $1 FOR UPDATE").bind(&r.attempt_id).fetch_one(&mut *tx).await?;
    if r.amount_minor > total - refunded - reserved {
        return Err(ApiError::invalid(
            "refund_exceeds_refundable",
            "the refundable amount changed; create a new refund instead",
        ));
    }
    sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor + $2 WHERE id = $1").bind(&r.attempt_id).bind(r.amount_minor).execute(&mut *tx).await?;
    sqlx::query("UPDATE refunds SET status = 'processing', failure_reason = NULL, updated_at = now() WHERE id = $1").bind(&id).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "refund.retried",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    run_provider_refund(&s, &auth.tenant_id, &auth.env, &id).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(fetch_refund(&mut tx, &id).await?))
}

pub async fn get_refund(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<RefundRow>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(fetch_refund(&mut tx, &id).await?))
}

pub async fn list_refunds(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // `status=failed` is the failed-refund queue; `unknown` those awaiting resolution.
    let mut rows: Vec<RefundRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM refunds WHERE ($1::text IS NULL OR status = $1)
           AND ($2::text IS NULL OR (created_at, id) < (SELECT created_at, id FROM refunds WHERE id = $2))
         ORDER BY created_at DESC, id DESC LIMIT $3"
    )))
    .bind(&q.status)
    .bind(&q.starting_after)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    Ok(Json(json!({ "data": rows, "has_more": has_more })))
}
