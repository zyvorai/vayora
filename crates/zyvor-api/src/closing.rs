//! Cash entries and the end-of-day closing. The system adds up the cash entries for a branch's IST day
//! and the digital payments captured that day; a person counts the till; the difference is recorded
//! and a second person signs it off. Nothing here moves money or posts to the ledger.

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

#[derive(Deserialize)]
pub struct NewCash {
    branch_id: String,
    counter_id: Option<String>,
    /// `sale`, `refund` (cash handed back) or `float` (cash put in the till to start).
    kind: String,
    amount_minor: i64,
    note: Option<String>,
}

pub async fn add_cash(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCash>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    if !matches!(b.kind.as_str(), "sale" | "refund" | "float")
        || b.amount_minor <= 0
        || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR
    {
        return Err(ApiError::invalid(
            "invalid_cash_entry",
            "kind is sale, refund or float, and amount_minor a positive integer within the limit",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (branch, counter) = crate::counters::resolve(
        &mut tx,
        auth.pin_branch(Some(b.branch_id.clone()))?,
        b.counter_id.as_deref(),
    )
    .await?;
    let id = new_id("cash");
    sqlx::query("INSERT INTO cash_entries (id, tenant_id, branch_id, counter_id, kind, amount_minor, note, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&id).bind(&auth.tenant_id).bind(&branch).bind(&counter).bind(&b.kind).bind(b.amount_minor).bind(&b.note).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_foreign_key_violation() => ApiError::not_found("branch"),
            e => e.into(),
        })?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

#[derive(Deserialize)]
pub struct NewClosing {
    branch_id: String,
    /// The IST day being closed, `YYYY-MM-DD`; not a future day.
    day: NaiveDate,
    counted_cash_minor: i64,
    note: Option<String>,
}

pub async fn create_closing(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewClosing>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    if b.counted_cash_minor < 0 || b.counted_cash_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "counted_cash_minor must be zero or more, within the limit",
        ));
    }
    let ist_today = (chrono::Utc::now() + chrono::Duration::minutes(330)).date_naive();
    if b.day > ist_today {
        return Err(ApiError::invalid(
            "future_day",
            "a day can be closed once it has started",
        ));
    }
    auth.pin_branch(Some(b.branch_id.clone()))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM branches WHERE id = $1")
        .bind(&b.branch_id)
        .fetch_optional(&mut *tx)
        .await?;
    exists.ok_or_else(|| ApiError::not_found("branch"))?;
    let expected: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(CASE kind WHEN 'refund' THEN -amount_minor ELSE amount_minor END), 0)::BIGINT
         FROM cash_entries WHERE branch_id = $1 AND (created_at AT TIME ZONE 'Asia/Kolkata')::date = $2",
    ).bind(&b.branch_id).bind(b.day).fetch_one(&mut *tx).await?;
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT a.method, count(*)::BIGINT, COALESCE(sum(a.amount_minor), 0)::BIGINT, COALESCE(sum(a.refunded_minor), 0)::BIGINT
         FROM payment_attempts a JOIN payment_intents i ON i.id = a.intent_id
         WHERE i.branch_id = $1 AND a.status = 'captured' AND a.currency = 'INR'
           AND (a.created_at AT TIME ZONE 'Asia/Kolkata')::date = $2
         GROUP BY 1 ORDER BY 1",
    ).bind(&b.branch_id).bind(b.day).fetch_all(&mut *tx).await?;
    let digital: Vec<Value> = rows.into_iter().map(|(m, n, g, r)| json!({ "method": m, "payments": n, "gross_minor": g, "refunded_minor": r, "net_minor": g - r })).collect();
    let diff = b.counted_cash_minor - expected;
    let id = new_id("clo");
    sqlx::query("INSERT INTO closings (id, tenant_id, branch_id, day, expected_cash_minor, counted_cash_minor, difference_minor, digital, note, counted_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.branch_id).bind(b.day).bind(expected).bind(b.counted_cash_minor).bind(diff).bind(json!(digital)).bind(&b.note).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("already_closed", "that day is already closed for this branch"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "closing.created",
        Some(&id),
        json!({ "branch_id": b.branch_id, "day": b.day, "difference_minor": diff }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "expected_cash_minor": expected, "counted_cash_minor": b.counted_cash_minor, "difference_minor": diff, "digital": digital, "status": "pending_signoff" }),
        ),
    ))
}

pub async fn sign_off(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("closings:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, Option<String>, i64)> = sqlx::query_as(
        "SELECT counted_by, signed_off_by, difference_minor FROM closings WHERE id = $1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (counted_by, signed, diff) = row.ok_or_else(|| ApiError::not_found("closing"))?;
    if signed.is_some() {
        return Err(ApiError::conflict(
            "already_signed_off",
            "already signed off",
        ));
    }
    if counted_by == auth.actor {
        return Err(ApiError::forbidden(
            "a different person must sign off a closing".to_string(),
        ));
    }
    sqlx::query("UPDATE closings SET signed_off_by = $2, signed_off_at = now() WHERE id = $1")
        .bind(&id)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "closing.signed_off",
        Some(&id),
        json!({ "difference_minor": diff }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "closing.signed_off",
        json!({ "id": id, "difference_minor": diff }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "signed_off" })))
}

#[derive(Deserialize)]
pub struct ClosingQuery {
    branch_id: Option<String>,
    /// `pending_signoff` or `signed_off`.
    status: Option<String>,
}

pub async fn list_closings(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ClosingQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, NaiveDate, i64, i64, i64, Value, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT id, branch_id, day, expected_cash_minor, counted_cash_minor, difference_minor, digital, note, counted_by, signed_off_by
         FROM closings WHERE ($1::text IS NULL OR branch_id = $1)
           AND ($2::text IS NULL OR ($2 = 'signed_off') = (signed_off_by IS NOT NULL))
         ORDER BY day DESC, id LIMIT 200",
    ).bind(auth.pin_branch(q.branch_id.clone())?).bind(&q.status).fetch_all(&mut *tx).await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, branch_id, day, exp, cnt, diff, digital, note, by, signed)| {
        json!({ "id": id, "branch_id": branch_id, "day": day, "expected_cash_minor": exp, "counted_cash_minor": cnt, "difference_minor": diff, "digital": digital, "note": note, "counted_by": by, "signed_off_by": signed, "status": if signed.is_some() { "signed_off" } else { "pending_signoff" } })
    }).collect();
    Ok(Json(json!({ "data": data })))
}
