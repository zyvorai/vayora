//! PA-grade acquire KYC/AML gate on the organisation.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, money_mode, tenant};

pub async fn get_status(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let org = money_mode::organisation_id(&mut tx, &auth.tenant_id).await?;
    let (status, approved_at): (String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT acquire_kyc_status, acquire_kyc_approved_at FROM organisations WHERE id = $1",
    )
    .bind(&org)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "organisation_id": org,
        "acquire_kyc_status": status,
        "acquire_kyc_approved_at": approved_at,
    })))
}

#[derive(Deserialize)]
pub struct SetStatus {
    status: String,
}

pub async fn set_status(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SetStatus>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    if !matches!(
        b.status.as_str(),
        "not_started" | "pending" | "approved" | "rejected" | "re_kyc_due"
    ) {
        return Err(ApiError::invalid(
            "invalid_status",
            "status must be not_started, pending, approved, rejected or re_kyc_due",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let org = money_mode::organisation_id(&mut tx, &auth.tenant_id).await?;
    if b.status == "approved" {
        sqlx::query(
            "UPDATE organisations SET acquire_kyc_status = $2, acquire_kyc_approved_at = now() WHERE id = $1",
        )
        .bind(&org)
        .bind(&b.status)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "UPDATE organisations SET acquire_kyc_status = $2, acquire_kyc_approved_at = NULL WHERE id = $1",
        )
        .bind(&org)
        .bind(&b.status)
        .execute(&mut *tx)
        .await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "acquire_kyc.set",
        Some(&org),
        json!({ "status": b.status }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "organisation_id": org, "acquire_kyc_status": b.status })))
}
