//! Builds provider adapters from a tenant's stored (encrypted) provider accounts, and gathers
//! the routing inputs: capabilities, enablement, health and configured fees.

use std::{collections::BTreeMap, sync::Arc};

use serde::Deserialize;
use serde_json::Value;
use sqlx::PgConnection;
use zyvor_providers::{
    Method, Provider,
    cashfree::Cashfree,
    custom::Custom,
    razorpay::Razorpay,
    router::{Candidate, Fee, Health, Rule},
};

use crate::{AppState, error::ApiError};

#[derive(Debug, Deserialize, Default)]
pub struct Routing {
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// provider -> method -> fee
    #[serde(default)]
    pub fees: BTreeMap<String, BTreeMap<Method, Fee>>,
}

pub async fn routing(conn: &mut PgConnection, tenant_id: &str) -> Result<Routing, ApiError> {
    let v: Option<Value> =
        sqlx::query_scalar("SELECT routing FROM tenant_settings WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(conn)
            .await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub async fn load(
    state: &AppState,
    conn: &mut PgConnection,
    tenant_id: &str,
    env: &str,
    provider: &str,
) -> Result<Arc<dyn Provider>, ApiError> {
    let row: Option<(Option<String>, Vec<u8>)> = sqlx::query_as("SELECT base_url, credentials_enc FROM provider_accounts WHERE tenant_id = $1 AND provider = $2")
        .bind(tenant_id)
        .bind(provider)
        .fetch_optional(&mut *conn)
        .await?;
    let (base_url, enc) = row.ok_or_else(|| ApiError::not_found("provider account"))?;
    if provider == "sim" {
        return Ok(state.sim.clone());
    }
    let creds: Value = state
        .crypto
        .decrypt(&enc)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| {
            tracing::error!(provider, "could not decrypt provider credentials");
            ApiError::internal()
        })?;
    let s = |k: &str| creds[k].as_str().unwrap_or_default().to_string();
    Ok(match provider {
        "razorpay" => Arc::new(Razorpay::new(
            base_url.unwrap_or_else(|| "https://api.razorpay.com".into()),
            s("key_id"),
            s("key_secret"),
            state.provider_timeout,
        )),
        "cashfree" => Arc::new(
            Cashfree::new(
                base_url.unwrap_or_else(|| {
                    if env == "live" {
                        "https://api.cashfree.com".into()
                    } else {
                        "https://sandbox.cashfree.com".into()
                    }
                }),
                s("client_id"),
                s("client_secret"),
                state.provider_timeout,
            )
            // Native UPI intent / QR is opt-in per account: set `"upi_intent": true` in the credentials once
            // Cashfree has enabled it for that account.
            .with_upi_intent(creds["upi_intent"].as_bool().unwrap_or(false)),
        ),
        "custom" => {
            // Unlike the two hosted providers there is no default endpoint: it is your own gateway.
            let base = base_url.ok_or_else(ApiError::internal)?;
            crate::ssrf::check_url(&base, state.allow_private_targets).map_err(|e| {
                tracing::error!(provider, error = %e, "custom provider base_url refused");
                ApiError::internal()
            })?;
            Arc::new(Custom::new(base, s("api_key"), state.provider_timeout))
        }
        other => {
            return Err(ApiError::internal())
                .inspect_err(|_| tracing::error!(other, "unknown provider"));
        }
    })
}

pub async fn webhook_secret(
    state: &AppState,
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
) -> Result<Vec<u8>, ApiError> {
    let enc: Option<Vec<u8>> = sqlx::query_scalar("SELECT webhook_secret_enc FROM provider_accounts WHERE tenant_id = $1 AND provider = $2 AND enabled")
        .bind(tenant_id)
        .bind(provider)
        .fetch_optional(conn)
        .await?;
    let enc = enc.ok_or_else(|| ApiError::not_found("provider account"))?;
    state.crypto.decrypt(&enc).ok_or_else(ApiError::internal)
}

pub async fn candidates(
    state: &AppState,
    conn: &mut PgConnection,
    tenant_id: &str,
    env: &str,
    routing: &Routing,
) -> Result<Vec<Candidate>, ApiError> {
    let accounts: Vec<(String, bool)> = sqlx::query_as(
        "SELECT provider, enabled FROM provider_accounts WHERE tenant_id = $1 ORDER BY provider",
    )
    .bind(tenant_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for (provider, enabled) in accounts {
        let p = load(state, conn, tenant_id, env, &provider).await?;
        // Health from the last 50 attempts; any attempt stuck in `unknown` counts against the provider.
        let (attempts, failures, unresolved): (i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE status = 'failed'), COUNT(*) FILTER (WHERE status = 'unknown')
             FROM (SELECT status FROM payment_attempts WHERE tenant_id = $1 AND provider = $2 ORDER BY created_at DESC LIMIT 50) r",
        )
        .bind(tenant_id)
        .bind(&provider)
        .fetch_one(&mut *conn)
        .await?;
        out.push(Candidate {
            fees: routing.fees.get(&provider).cloned().unwrap_or_default(),
            caps: p.capabilities(),
            enabled,
            health: Health {
                recent_attempts: attempts as u32,
                recent_failures: failures as u32,
                unresolved: unresolved as u32,
            },
            provider,
        });
    }
    Ok(out)
}
