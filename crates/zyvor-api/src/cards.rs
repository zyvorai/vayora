//! Network-token card instruments (zyvor-cards). Never accepts PAN/CVV.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_cards::{NetworkToken, reject_raw_card_fields, validate_token_ref};
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Deserialize)]
pub struct StoreToken {
    customer_id: Option<String>,
    network_token_ref: String,
    last4: String,
    brand: Option<String>,
    exp_month: Option<u8>,
    exp_year: Option<u16>,
}

pub async fn store_token(
    State(s): State<AppState>,
    auth: Auth,
    Json(raw): Json<Value>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    reject_raw_card_fields(&raw)
        .map_err(|e| ApiError::invalid("raw_card_forbidden", e.to_string()))?;
    let b: StoreToken = serde_json::from_value(raw)
        .map_err(|e| ApiError::invalid("invalid_token", e.to_string()))?;
    let tok = NetworkToken {
        id: new_id("tok"),
        network_token_ref: b.network_token_ref.trim().to_string(),
        last4: b.last4,
        brand: b.brand,
        exp_month: b.exp_month,
        exp_year: b.exp_year,
        revoked: false,
    };
    validate_token_ref(&tok).map_err(|e| ApiError::invalid("invalid_token", e.to_string()))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(cid) = &b.customer_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM customers WHERE id = $1)")
            .bind(cid)
            .fetch_one(&mut *tx)
            .await?;
        if !ok {
            return Err(ApiError::not_found("customer"));
        }
    }
    sqlx::query(
        "INSERT INTO card_tokens (id, tenant_id, customer_id, network_token_ref, last4, brand, exp_month, exp_year)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(&tok.id)
    .bind(&auth.tenant_id)
    .bind(&b.customer_id)
    .bind(&tok.network_token_ref)
    .bind(&tok.last4)
    .bind(&tok.brand)
    .bind(tok.exp_month.map(i32::from))
    .bind(tok.exp_year.map(i32::from))
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            ApiError::conflict("token_exists", "network_token_ref already stored")
        }
        e => e.into(),
    })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "card_token.stored",
        Some(&tok.id),
        json!({ "last4": tok.last4, "brand": tok.brand }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": tok.id,
            "last4": tok.last4,
            "brand": tok.brand,
            "exp_month": tok.exp_month,
            "exp_year": tok.exp_year,
        })),
    ))
}

pub async fn list_tokens(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        String,
        Option<String>,
        String,
        Option<String>,
        Option<i32>,
        Option<i32>,
    )> = sqlx::query_as(
        "SELECT id, customer_id, last4, brand, exp_month, exp_year FROM card_tokens
         WHERE revoked_at IS NULL ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, customer_id, last4, brand, exp_month, exp_year)| {
            json!({
                "id": id,
                "customer_id": customer_id,
                "last4": last4,
                "brand": brand,
                "exp_month": exp_month,
                "exp_year": exp_year,
            })
        })
        .collect();
    Ok(Json(json!({ "tokens": items })))
}

pub async fn revoke_token(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE card_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("card token"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "card_token.revoked",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "revoked": true })))
}
