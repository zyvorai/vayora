//! Read-side endpoints for the dashboard: keyset-paginated lists, webhook retry, stats.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, PgPool, postgres::PgRow};
use vayora_ledger::accounts;

use crate::{
    AppState,
    error::ApiError,
    routes::{Order, Payment, Refund},
};

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
    /// id of the last item of the previous page
    starting_after: Option<String>,
    status: Option<String>,
}

#[derive(Serialize)]
pub struct Page<T> {
    data: Vec<T>,
    has_more: bool,
}

#[derive(Debug, Serialize, FromRow)]
pub struct WebhookEvent {
    pub id: String,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub event_type: String,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/// Newest first, stable under concurrent inserts (keyset on `(created_at, id)`).
/// `table` is always a literal from this file, never user input.
async fn list<T>(
    pool: &PgPool,
    table: &'static str,
    q: &ListQuery,
    statuses: &[&str],
) -> Result<Json<Page<T>>, ApiError>
where
    T: for<'r> FromRow<'r, PgRow> + Send + Unpin,
{
    if let Some(s) = &q.status
        && !statuses.contains(&s.as_str())
    {
        return Err(ApiError::invalid(
            "invalid_status",
            format!("status must be one of {statuses:?}"),
        ));
    }
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let sql = format!(
        "SELECT * FROM {table}
         WHERE ($1::text IS NULL OR status = $1)
           AND ($2::text IS NULL OR (created_at, id) < (SELECT created_at, id FROM {table} WHERE id = $2))
         ORDER BY created_at DESC, id DESC LIMIT $3"
    );
    let mut rows: Vec<T> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(&q.status)
        .bind(&q.starting_after)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    Ok(Json(Page {
        data: rows,
        has_more,
    }))
}

pub async fn list_orders(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<Order>>, ApiError> {
    list(&s.pool, "orders", &q, &["created", "paid"]).await
}
pub async fn list_payments(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<Payment>>, ApiError> {
    list(&s.pool, "payments", &q, &["captured", "failed"]).await
}
pub async fn list_refunds(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<Refund>>, ApiError> {
    list(&s.pool, "refunds", &q, &["processed"]).await
}
pub async fn list_events(
    State(s): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page<WebhookEvent>>, ApiError> {
    list(
        &s.pool,
        "webhook_events",
        &q,
        &["pending", "delivered", "dead"],
    )
    .await
}

/// Re-queue a webhook that is waiting or dead. Delivered events are final.
pub async fn retry_event(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WebhookEvent>, ApiError> {
    let ev: Option<WebhookEvent> = sqlx::query_as(
        "UPDATE webhook_events
         SET attempts = CASE WHEN status = 'dead' THEN 0 ELSE attempts END,
             status = 'pending', next_attempt_at = now()
         WHERE id = $1 AND status IN ('pending', 'dead') RETURNING *",
    )
    .bind(&id)
    .fetch_optional(&s.pool)
    .await?;
    if let Some(ev) = ev {
        return Ok(Json(ev));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM webhook_events WHERE id = $1)")
            .bind(&id)
            .fetch_one(&s.pool)
            .await?;
    Err(if exists {
        ApiError::conflict("already_delivered", "this event was already delivered")
    } else {
        ApiError::not_found("event")
    })
}

pub async fn stats(State(s): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let (captured, failed, gross, refunded): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE status = 'captured'),
                COUNT(*) FILTER (WHERE status = 'failed'),
                COALESCE(SUM(amount_paise) FILTER (WHERE status = 'captured'), 0)::BIGINT,
                COALESCE(SUM(refunded_paise), 0)::BIGINT
         FROM payments",
    )
    .fetch_one(&s.pool)
    .await?;

    // Calendar days in IST, zero-filled so the chart has no gaps.
    let daily: Vec<(String, i64, i64)> = sqlx::query_as(
        "WITH days AS (
           SELECT d::date AS day FROM generate_series(
             (now() AT TIME ZONE 'Asia/Kolkata')::date - 13,
             (now() AT TIME ZONE 'Asia/Kolkata')::date, interval '1 day') d)
         SELECT to_char(days.day, 'YYYY-MM-DD'),
                COALESCE(SUM(p.amount_paise), 0)::BIGINT,
                COUNT(p.id)::BIGINT
         FROM days LEFT JOIN payments p
           ON p.status = 'captured' AND (p.created_at AT TIME ZONE 'Asia/Kolkata')::date = days.day
         GROUP BY days.day ORDER BY days.day",
    )
    .fetch_all(&s.pool)
    .await?;

    let hooks: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM webhook_events GROUP BY status")
            .fetch_all(&s.pool)
            .await?;
    let hook = |name: &str| hooks.iter().find(|(s, _)| s == name).map_or(0, |(_, n)| *n);

    let mut conn = s.pool.acquire().await?;
    let (d, c) = vayora_ledger::totals(&mut conn, accounts::MERCHANT_PAYABLE).await?;

    let attempts = captured + failed;
    Ok(Json(json!({
        "captured_count": captured,
        "failed_count": failed,
        "success_rate": if attempts == 0 { None } else { Some(captured as f64 / attempts as f64) },
        "gross_paise": gross,
        "refunded_paise": refunded,
        "net_paise": gross - refunded,
        "merchant_payable_paise": c - d,
        "webhooks": { "pending": hook("pending"), "delivered": hook("delivered"), "dead": hook("dead") },
        "daily": daily.into_iter().map(|(date, gross_paise, count)| json!({ "date": date, "gross_paise": gross_paise, "count": count })).collect::<Vec<_>>(),
    })))
}
