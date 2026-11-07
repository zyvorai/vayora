//! Organisation money mode: orchestrate (default) vs acquire (custody under sponsor / PA).

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_ledger::MoneyMode;

use crate::{ApiError, AppState, auth::Auth, tenant};

pub async fn load_for_tenant(
    conn: &mut PgConnection,
    tenant_id: &str,
) -> Result<MoneyMode, ApiError> {
    let mode: String = sqlx::query_scalar(
        "SELECT o.money_mode FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_one(conn)
    .await?;
    mode.parse().map_err(|e: String| {
        tracing::error!(error = %e, "bad money_mode in db");
        ApiError::internal()
    })
}

pub async fn organisation_id(conn: &mut PgConnection, tenant_id: &str) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT organisation_id FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_one(conn)
        .await
        .map_err(Into::into)
}

/// Acquire mode may not take live payments until KYC is approved.
pub async fn acquire_live_allowed(
    conn: &mut PgConnection,
    tenant_id: &str,
) -> Result<(), ApiError> {
    let (mode, env, kyc): (String, String, String) = sqlx::query_as(
        "SELECT o.money_mode, t.env, o.acquire_kyc_status
         FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_one(conn)
    .await?;
    if mode != "acquire" || env != "live" {
        return Ok(());
    }
    if kyc != "approved" {
        return Err(ApiError::conflict(
            "acquire_kyc_required",
            "acquire mode live payments need approved merchant KYC/AML",
        ));
    }
    Ok(())
}

pub async fn get_money_mode(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mode = load_for_tenant(&mut tx, &auth.tenant_id).await?;
    let (kyc, delay): (String, i32) = sqlx::query_as(
        "SELECT o.acquire_kyc_status, COALESCE(ts.settlement_delay_days, 1)
         FROM tenants t
         JOIN organisations o ON o.id = t.organisation_id
         LEFT JOIN tenant_settings ts ON ts.tenant_id = t.id
         WHERE t.id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "money_mode": mode.as_str(),
        "acquire_kyc_status": kyc,
        "settlement_delay_days": delay,
    })))
}

#[derive(Deserialize)]
pub struct SetMoneyMode {
    money_mode: String,
    settlement_delay_days: Option<i32>,
}

pub async fn set_money_mode(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SetMoneyMode>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mode: MoneyMode = b
        .money_mode
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_money_mode", e))?;
    if let Some(d) = b.settlement_delay_days
        && !(0..=7).contains(&d)
    {
        return Err(ApiError::invalid(
            "invalid_settlement_delay",
            "settlement_delay_days must be 0-7",
        ));
    }
    if mode == MoneyMode::Acquire && auth.env == "live" {
        // Operational gate: live acquire needs an explicit env override while counsel/sponsor freeze is open.
        if std::env::var("ZYVOR_ALLOW_LIVE_ACQUIRE").ok().as_deref() != Some("1") {
            return Err(ApiError::conflict(
                "acquire_not_enabled",
                "live acquire requires ZYVOR_ALLOW_LIVE_ACQUIRE=1 after fund-flow freeze (see REGULATORY.md)",
            ));
        }
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let org = organisation_id(&mut tx, &auth.tenant_id).await?;
    sqlx::query("UPDATE organisations SET money_mode = $2 WHERE id = $1")
        .bind(&org)
        .bind(mode.as_str())
        .execute(&mut *tx)
        .await?;
    if let Some(d) = b.settlement_delay_days {
        sqlx::query(
            "INSERT INTO tenant_settings (tenant_id, settlement_delay_days) VALUES ($1, $2)
             ON CONFLICT (tenant_id) DO UPDATE SET settlement_delay_days = $2",
        )
        .bind(&auth.tenant_id)
        .bind(d)
        .execute(&mut *tx)
        .await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "money_mode.set",
        Some(&org),
        json!({ "money_mode": mode.as_str(), "settlement_delay_days": b.settlement_delay_days }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "money_mode": mode.as_str() })))
}
