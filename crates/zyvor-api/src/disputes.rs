//! Disputes (chargebacks): record the provider's notice, collect evidence, and book the outcome.
//! Zyvor tracks the case and the money; submitting evidence to the provider/issuer is done in the
//! provider's own dashboard or API until that adapter capability exists.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::{Currency, new_id};
use zyvor_ledger::{Entry, Transaction, accounts};

use crate::{ApiError, AppState, auth::Auth, payments::ListQuery, tenant};

#[derive(Deserialize)]
pub struct NewDispute {
    attempt_id: String,
    /// Defaults to the full payment.
    amount_minor: Option<i64>,
    reason: String,
    provider_ref: Option<String>,
    respond_by: Option<NaiveDate>,
}

pub async fn open_dispute(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewDispute>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("disputes:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i64, i64, i64, String)> = sqlx::query_as("SELECT intent_id, status, amount_minor, refunded_minor, reserved_refund_minor, currency FROM payment_attempts WHERE id = $1 FOR UPDATE").bind(&b.attempt_id).fetch_optional(&mut *tx).await?;
    let (intent_id, status, total, refunded, reserved, ccy) =
        row.ok_or_else(|| ApiError::not_found("payment attempt"))?;
    if status != "captured" {
        return Err(ApiError::conflict(
            "not_disputable",
            format!("only a captured payment can be disputed; this one is {status}"),
        ));
    }
    let amount = b.amount_minor.unwrap_or(total);
    if amount <= 0 || amount > total - refunded - reserved {
        return Err(ApiError::invalid(
            "invalid_amount",
            format!(
                "only {} of this payment is still free to dispute (the rest is refunded or reserved)",
                total - refunded - reserved
            ),
        ));
    }
    let id = new_id("dsp");
    sqlx::query("INSERT INTO disputes (id, tenant_id, attempt_id, intent_id, amount_minor, currency, reason, provider_ref, respond_by, opened_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.attempt_id).bind(&intent_id).bind(amount).bind(&ccy).bind(&b.reason).bind(&b.provider_ref).bind(b.respond_by).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("dispute_exists", "this payment already has an active dispute"),
            e => e.into(),
        })?;
    // hold the money: it can't be refunded while the issuer is clawing it back
    sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor + $2, updated_at = now() WHERE id = $1").bind(&b.attempt_id).bind(amount).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dispute.opened",
        Some(&id),
        json!({ "attempt_id": b.attempt_id, "amount_minor": amount }),
    )
    .await?;
    tenant::emit(&mut tx, &auth.tenant_id, "dispute.opened", json!({ "id": id, "attempt_id": b.attempt_id, "intent_id": intent_id, "amount_minor": amount, "reason": b.reason, "respond_by": b.respond_by })).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "open", "amount_minor": amount })),
    ))
}

pub async fn list_disputes(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, String, String, Option<NaiveDate>, Value)> = sqlx::query_as("SELECT id, attempt_id, amount_minor, reason, status, respond_by, evidence FROM disputes WHERE ($1::text IS NULL OR status = $1) ORDER BY respond_by NULLS LAST, created_at DESC LIMIT $2")
        .bind(&q.status).bind(q.limit.unwrap_or(50).clamp(1, 200)).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, att, amt, reason, st, by, ev)| json!({ "id": id, "attempt_id": att, "amount_minor": amt, "reason": reason, "status": st, "respond_by": by, "evidence_count": ev.as_array().map_or(0, Vec::len) })).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
pub struct NewEvidence {
    note: String,
    /// Where the document lives (the merchant's own storage); Zyvor stores the reference, not the file.
    url: Option<String>,
}

pub async fn add_evidence(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<NewEvidence>,
) -> Result<Json<Value>, ApiError> {
    auth.require("disputes:write")?;
    if b.note.trim().is_empty() {
        return Err(ApiError::invalid(
            "note_required",
            "describe what this evidence shows",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let item =
        json!({ "note": b.note, "url": b.url, "added_by": auth.actor, "at": chrono::Utc::now() });
    let n = sqlx::query("UPDATE disputes SET evidence = evidence || $2::jsonb, status = 'under_review' WHERE id = $1 AND status IN ('open', 'under_review')")
        .bind(&id).bind(json!([item])).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::conflict(
            "dispute_closed",
            "this dispute is already resolved or does not exist",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dispute.evidence_added",
        Some(&id),
        json!({ "note": b.note }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "under_review" })))
}

#[derive(Deserialize)]
pub struct Resolve {
    /// `won` or `lost`.
    outcome: String,
}

/// Record the issuer's decision. Winning releases the hold. Losing books the loss on the ledger,
/// counts the amount as refunded money-out on the payment, and re-opens the invoice it had paid.
pub async fn resolve_dispute(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<Resolve>,
) -> Result<Json<Value>, ApiError> {
    auth.require("disputes:write")?;
    if !matches!(b.outcome.as_str(), "won" | "lost") {
        return Err(ApiError::invalid(
            "invalid_outcome",
            "outcome must be won or lost",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i64, String, String)> = sqlx::query_as("SELECT attempt_id, intent_id, amount_minor, currency, status FROM disputes WHERE id = $1 FOR UPDATE").bind(&id).fetch_optional(&mut *tx).await?;
    let (attempt_id, intent_id, amount, ccy, status) =
        row.ok_or_else(|| ApiError::not_found("dispute"))?;
    if !matches!(status.as_str(), "open" | "under_review") {
        return Err(ApiError::conflict(
            "dispute_closed",
            format!("dispute is already {status}"),
        ));
    }
    if b.outcome == "won" {
        sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor - $2, updated_at = now() WHERE id = $1").bind(&attempt_id).bind(amount).execute(&mut *tx).await?;
    } else {
        let provider: String =
            sqlx::query_scalar("SELECT provider FROM payment_attempts WHERE id = $1")
                .bind(&attempt_id)
                .fetch_one(&mut *tx)
                .await?;
        let currency: Currency = ccy
            .parse()
            .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
        Transaction::new(vec![
            Entry::debit(accounts::ADJUSTMENTS, amount, currency),
            Entry::credit(accounts::provider_receivable(&provider), amount, currency),
        ])
        .map_err(zyvor_ledger::StoreError::from)?
        .post(
            &mut tx,
            &auth.tenant_id,
            &format!("dispute_lost:{id}"),
            None,
        )
        .await?;
        sqlx::query("UPDATE payment_attempts SET reserved_refund_minor = reserved_refund_minor - $2, refunded_minor = refunded_minor + $2, updated_at = now() WHERE id = $1").bind(&attempt_id).bind(amount).execute(&mut *tx).await?;
        if let Some(inv) = sqlx::query_scalar::<_, Option<String>>(
            "SELECT invoice_id FROM payment_intents WHERE id = $1",
        )
        .bind(&intent_id)
        .fetch_one(&mut *tx)
        .await?
        {
            sqlx::query("INSERT INTO invoice_allocations (tenant_id, invoice_id, intent_id, amount_minor, reference) VALUES ($1,$2,$3,$4,$5)")
                .bind(&auth.tenant_id).bind(&inv).bind(&intent_id).bind(-amount).bind(format!("dispute:{id}")).execute(&mut *tx).await?;
            crate::payments::refresh_invoice_status(&mut tx, &inv).await?;
        }
    }
    sqlx::query("UPDATE disputes SET status = $2, resolved_at = now() WHERE id = $1")
        .bind(&id)
        .bind(&b.outcome)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        &format!("dispute.{}", b.outcome),
        Some(&id),
        json!({ "amount_minor": amount }),
    )
    .await?;
    tenant::emit(&mut tx, &auth.tenant_id, &format!("dispute.{}", b.outcome), json!({ "id": id, "attempt_id": attempt_id, "intent_id": intent_id, "amount_minor": amount })).await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": b.outcome })))
}
