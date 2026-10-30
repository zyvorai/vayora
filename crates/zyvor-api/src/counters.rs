//! Counters within a branch, and per-branch daily summaries. A counter's QR is a payment link that
//! carries the counter, so payments made through it are attributed to the counter and branch.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

/// Work out the branch and counter for something being created. A counter implies its branch; if a
/// branch is also given it has to be the counter's own.
pub async fn resolve(
    conn: &mut PgConnection,
    branch_id: Option<String>,
    counter_id: Option<&str>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    // Foreign keys are checked without row-level security, so confirm the branch is this tenant's.
    if let Some(b) = &branch_id {
        let seen: Option<String> = sqlx::query_scalar("SELECT id FROM branches WHERE id = $1")
            .bind(b)
            .fetch_optional(&mut *conn)
            .await?;
        seen.ok_or_else(|| ApiError::not_found("branch"))?;
    }
    let Some(cid) = counter_id else {
        return Ok((branch_id, None));
    };
    let row: Option<(String, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as("SELECT branch_id, deactivated_at FROM counters WHERE id = $1")
            .bind(cid)
            .fetch_optional(&mut *conn)
            .await?;
    let (own, off) = row.ok_or_else(|| ApiError::not_found("counter"))?;
    if off.is_some() {
        return Err(ApiError::conflict(
            "counter_inactive",
            "that counter is switched off",
        ));
    }
    if branch_id.as_deref().is_some_and(|b| b != own) {
        return Err(ApiError::invalid(
            "counter_branch_mismatch",
            "that counter belongs to a different branch",
        ));
    }
    Ok((Some(own), Some(cid.to_string())))
}

#[derive(Deserialize)]
pub struct NewCounter {
    branch_id: String,
    name: String,
}

pub async fn create_counter(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCounter>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(ApiError::invalid(
            "invalid_counter",
            "name is 1-60 characters",
        ));
    }
    let id = new_id("ctr");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    resolve(&mut tx, Some(b.branch_id.clone()), None).await?;
    sqlx::query(
        "INSERT INTO counters (id, tenant_id, branch_id, name, created_by) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(&b.branch_id)
    .bind(name)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_foreign_key_violation() => ApiError::not_found("branch"),
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict(
            "counter_exists",
            "that branch already has a counter with this name",
        ),
        e => e.into(),
    })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "counter.created",
        Some(&id),
        json!({ "branch_id": b.branch_id }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "branch_id": b.branch_id, "name": name })),
    ))
}

#[derive(Deserialize)]
pub struct CounterQuery {
    branch_id: Option<String>,
}

pub async fn list_counters(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<CounterQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require_branch("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
        "SELECT id, branch_id, name, deactivated_at IS NULL FROM counters WHERE ($1::text IS NULL OR branch_id = $1) ORDER BY name",
    ).bind(auth.pin_branch(q.branch_id.clone())?).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, branch_id, name, active)| json!({ "id": id, "branch_id": branch_id, "name": name, "active": active })).collect::<Vec<_>>() }),
    ))
}

pub async fn deactivate_counter(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // switching off a counter also stops its QR links taking payments
    let n = sqlx::query(
        "UPDATE counters SET deactivated_at = now() WHERE id = $1 AND deactivated_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("active counter"));
    }
    sqlx::query("UPDATE payment_links SET canceled_at = now() WHERE counter_id = $1 AND canceled_at IS NULL")
        .bind(&id).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "counter.deactivated",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "active": false })))
}

#[derive(Deserialize)]
pub struct SummaryQuery {
    /// First day (IST), `YYYY-MM-DD`. Defaults to 30 days ago.
    from: Option<chrono::NaiveDate>,
    /// Last day (IST), inclusive. Defaults to today.
    to: Option<chrono::NaiveDate>,
}

/// Captured money per IST day, counter and method for one branch, net of refunds and lost disputes.
pub async fn branch_summary(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require_branch("payments:read")?;
    auth.pin_branch(Some(id.clone()))?;
    let to = q.to.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let from = q.from.unwrap_or(to - chrono::Duration::days(30));
    if from > to || (to - from).num_days() > 366 {
        return Err(ApiError::invalid(
            "invalid_range",
            "from must not be after to, and the range is at most a year",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let name = name.ok_or_else(|| ApiError::not_found("branch"))?;
    let rows: Vec<(String, Option<String>, String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT to_char((a.created_at AT TIME ZONE 'Asia/Kolkata')::date, 'YYYY-MM-DD') AS day, i.counter_id, a.method, a.currency,
                count(*)::BIGINT, COALESCE(sum(a.amount_minor), 0)::BIGINT, COALESCE(sum(a.refunded_minor), 0)::BIGINT
         FROM payment_attempts a JOIN payment_intents i ON i.id = a.intent_id
         WHERE i.branch_id = $1 AND a.status = 'captured'
           AND (a.created_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $2 AND $3
         GROUP BY 1, 2, 3, 4 ORDER BY 1 DESC, 2, 3",
    ).bind(&id).bind(from).bind(to).fetch_all(&mut *tx).await?;
    let data: Vec<Value> = rows.into_iter().map(|(day, counter_id, method, currency, n, gross, refunded)| {
        json!({ "day": day, "counter_id": counter_id, "method": method, "currency": currency, "payments": n, "gross_minor": gross, "refunded_minor": refunded, "net_minor": gross - refunded })
    }).collect();
    Ok(Json(
        json!({ "branch": { "id": id, "name": name }, "from": from, "to": to, "data": data }),
    ))
}
