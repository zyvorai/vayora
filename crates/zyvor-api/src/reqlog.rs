//! Authenticated API request log for the Developers inspector. Bodies are never stored; only the
//! route template, status, actor and latency. Filled from the request middleware when Auth ran.

use std::sync::{Arc, OnceLock};

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::{AppState, auth::Auth, error::ApiError, tenant};

/// Filled by `Auth` when a handler extracts it; read by the request middleware after the response.
#[derive(Clone, Default)]
pub struct Slot(pub Arc<OnceLock<Meta>>);

#[derive(Clone, Debug)]
pub struct Meta {
    pub tenant_id: String,
    pub actor: String,
}

pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        i32,
        String,
        i32,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, actor, method, route, status, request_id, latency_ms, created_at
         FROM api_request_log ORDER BY created_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({
        "data": rows.into_iter().map(|(id, actor, method, route, status, request_id, latency_ms, at)| json!({
            "id": id, "actor": actor, "method": method, "route": route, "status": status,
            "request_id": request_id, "latency_ms": latency_ms, "created_at": at
        })).collect::<Vec<_>>()
    })))
}

/// Best-effort insert; failures are ignored so logging never breaks a request.
pub async fn record(
    pool: &sqlx::PgPool,
    meta: &Meta,
    method: &str,
    route: &str,
    status: u16,
    request_id: &str,
    latency_ms: u64,
) {
    let Ok(mut tx) = tenant::begin(pool, &meta.tenant_id).await else {
        return;
    };
    let _ = sqlx::query(
        "INSERT INTO api_request_log (tenant_id, actor, method, route, status, request_id, latency_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&meta.tenant_id)
    .bind(&meta.actor)
    .bind(method)
    .bind(route)
    .bind(status as i32)
    .bind(request_id)
    .bind(latency_ms.min(i32::MAX as u64) as i32)
    .execute(&mut *tx)
    .await;
    // Cap history so the table cannot grow without bound.
    let _ = sqlx::query(
        "DELETE FROM api_request_log WHERE tenant_id = $1 AND id NOT IN (
            SELECT id FROM api_request_log WHERE tenant_id = $1 ORDER BY created_at DESC, id DESC LIMIT 500
         )",
    )
    .bind(&meta.tenant_id)
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
}
