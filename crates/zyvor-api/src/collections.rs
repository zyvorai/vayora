//! Collections workspace: ageing of unpaid invoices, promises to pay and reminder history.
//! Zyvor records and reports; the merchant's own channel (email/SMS/WhatsApp) does the sending,
//! triggered by the `invoice.reminder` event.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

const OUTSTANDING: &str = "(i.amount_minor - invoice_settled(i.id))::BIGINT";

#[derive(Deserialize)]
pub struct AgeingQuery {
    /// Report date; defaults to today. Lets finance reproduce last month-end.
    as_of: Option<NaiveDate>,
}

/// Outstanding amounts by days past due, per currency, plus the invoices in each bucket.
pub async fn ageing(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<AgeingQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let as_of = q.as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<NaiveDate>,
        i64,
        i64,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, i.number, i.currency, i.customer_id, i.due_date, {OUTSTANDING} AS outstanding,
                COALESCE(($1::date - i.due_date), 0)::BIGINT AS days_over
         FROM invoices i WHERE i.status IN ('open', 'partially_paid') AND i.created_at::date <= $1
         ORDER BY i.due_date NULLS LAST, i.number"
    )))
    .bind(as_of)
    .fetch_all(&mut *tx)
    .await?;
    let bucket = |days: i64, due: Option<NaiveDate>| match (due, days) {
        (None, _) => "no_due_date",
        (_, d) if d <= 0 => "current",
        (_, 1..=30) => "1_30",
        (_, 31..=60) => "31_60",
        (_, 61..=90) => "61_90",
        _ => "over_90",
    };
    let mut buckets: std::collections::BTreeMap<(String, &str), (i64, Vec<Value>)> =
        Default::default();
    for (id, number, ccy, customer, due, outstanding, days) in rows {
        if outstanding <= 0 {
            continue;
        }
        let e = buckets.entry((ccy.clone(), bucket(days, due))).or_default();
        e.0 += outstanding;
        e.1.push(json!({ "id": id, "number": number, "customer_id": customer, "due_date": due, "outstanding_minor": outstanding, "days_past_due": days.max(0) }));
    }
    let data: Vec<Value> = buckets
        .into_iter()
        .map(|((ccy, b), (total, invs))| json!({ "currency": ccy, "bucket": b, "outstanding_minor": total, "count": invs.len(), "invoices": invs }))
        .collect();
    Ok(Json(json!({ "as_of": as_of, "data": data })))
}

#[derive(Deserialize)]
pub struct NewPromise {
    promised_minor: i64,
    promised_date: NaiveDate,
    note: Option<String>,
}

pub async fn create_promise(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    Json(b): Json<NewPromise>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.status, {OUTSTANDING} FROM invoices i WHERE i.id = $1 FOR UPDATE"
    )))
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, outstanding) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if !matches!(status.as_str(), "open" | "partially_paid") {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {status}"),
        ));
    }
    if b.promised_minor <= 0 || b.promised_minor > outstanding {
        return Err(ApiError::invalid(
            "invalid_amount",
            format!("promised amount must be between 1 and the outstanding {outstanding}"),
        ));
    }
    let id = new_id("prm");
    sqlx::query("INSERT INTO payment_promises (id, tenant_id, invoice_id, promised_minor, promised_date, note, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(b.promised_minor).bind(b.promised_date).bind(&b.note).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "promise.created",
        Some(&invoice_id),
        json!({ "promise_id": id, "date": b.promised_date }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "invoice_id": invoice_id, "promised_minor": b.promised_minor, "promised_date": b.promised_date, "status": "open" }),
        ),
    ))
}

pub async fn cancel_promise(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE payment_promises SET canceled_at = now() WHERE id = $1 AND canceled_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("promise"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "promise.canceled",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "canceled" })))
}

/// Promises with a derived status: `kept` once the invoice is cleared, `broken` once the date has
/// passed with money still owing, otherwise `open`.
pub async fn list_promises(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<crate::payments::ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i64, NaiveDate, Option<String>, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT p.id, p.invoice_id, i.number, p.promised_minor, p.promised_date, p.note,
                CASE WHEN p.canceled_at IS NOT NULL THEN 'canceled'
                     WHEN {OUTSTANDING} <= 0 THEN 'kept'
                     WHEN p.promised_date < CURRENT_DATE THEN 'broken'
                     ELSE 'open' END
         FROM payment_promises p JOIN invoices i ON i.id = p.invoice_id ORDER BY p.promised_date, p.id LIMIT $1"
    )))
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .filter(|r| q.status.as_deref().is_none_or(|st| st == r.6))
        .map(|(id, inv, number, amt, date, note, status)| json!({ "id": id, "invoice_id": inv, "invoice_number": number, "promised_minor": amt, "promised_date": date, "note": note, "status": status }))
        .collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize)]
pub struct NewReminder {
    channel: String,
    message: Option<String>,
}

/// Log a reminder and emit `invoice.reminder` so the merchant's own messaging sends it.
pub async fn create_reminder(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    Json(b): Json<NewReminder>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.status, i.number, {OUTSTANDING} FROM invoices i WHERE i.id = $1"
    )))
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, number, outstanding) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if !matches!(status.as_str(), "open" | "partially_paid") {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {status}"),
        ));
    }
    let id = new_id("rem");
    sqlx::query("INSERT INTO invoice_reminders (id, tenant_id, invoice_id, channel, message, sent_by) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(&b.channel).bind(&b.message).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_check_violation() => ApiError::invalid("invalid_channel", "channel must be email, sms, whatsapp, call or other"),
            e => e.into(),
        })?;
    tenant::emit(&mut tx, &auth.tenant_id, "invoice.reminder", json!({ "reminder_id": id, "invoice_id": invoice_id, "number": number, "channel": b.channel, "outstanding_minor": outstanding, "message": b.message })).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "reminder.logged",
        Some(&invoice_id),
        json!({ "channel": b.channel }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "invoice_id": invoice_id, "channel": b.channel })),
    ))
}
