// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Retry advice for failed payments (read-only).
//!
//! - `GET /v1/decline_advice?reason=..&provider=..`: stateless; what to do about a decline reason.
//! - `GET /v1/payment_attempts/{id}/retry_advice`: the same for a stored failed attempt (its provider, reason and
//!   recorded class); an attempt that did not fail answers `not_failed`.
//!
//! The rules are in `zyvor_providers::decline::retry_advice`. Nothing is retried or written: this only advises.

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_providers::decline::{self, MAX_RETRIES, RetryAdvice};

use crate::{ApiError, AppState, auth::Auth, payments::fetch_attempt, tenant};

#[derive(Debug, Deserialize)]
pub struct AdviceQuery {
    pub reason: Option<String>,
    pub provider: Option<String>,
}

fn advice_json(a: &RetryAdvice) -> Value {
    json!({
        "decline_class": a.class.as_str(),
        "hard_decline": a.hard_decline,
        "action": a.action.as_str(),
        "retry_after_hours": a.retry_after_hours,
        "max_retries": MAX_RETRIES,
    })
}

pub async fn advice(auth: Auth, Query(q): Query<AdviceQuery>) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let reason = q.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    if reason.is_some_and(|r| r.len() > 200) {
        return Err(ApiError::bad_request(
            "reason_too_long",
            "reason is at most 200 characters",
        ));
    }
    let provider = q.provider.as_deref().unwrap_or("sim");
    let a = decline::retry_advice(provider, reason, &Value::Null);
    let mut out = advice_json(&a);
    out["reason"] = json!(reason);
    out["provider"] = json!(provider);
    Ok(Json(out))
}

pub async fn attempt_advice(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row = fetch_attempt(&mut tx, &id).await?;
    let class: Option<String> =
        sqlx::query_scalar("SELECT failure_class FROM payment_attempts WHERE id = $1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    if row.status != "failed" {
        return Ok(Json(json!({
            "attempt_id": id, "status": row.status, "action": "not_failed",
            "retry_after_hours": [], "max_retries": MAX_RETRIES,
        })));
    }
    // a cancelled attempt is the merchant's own doing: nothing to retry
    if class.as_deref() == Some("cancelled") {
        return Ok(Json(json!({
            "attempt_id": id, "status": row.status, "failure_class": class, "action": "cancelled_by_merchant",
            "retry_after_hours": [], "max_retries": MAX_RETRIES,
        })));
    }
    let a = decline::retry_advice(&row.provider, row.failure_reason.as_deref(), &Value::Null);
    let mut out = advice_json(&a);
    out["attempt_id"] = json!(id);
    out["status"] = json!(row.status);
    out["provider"] = json!(row.provider);
    out["reason"] = json!(row.failure_reason);
    out["failure_class"] = json!(class);
    Ok(Json(out))
}
