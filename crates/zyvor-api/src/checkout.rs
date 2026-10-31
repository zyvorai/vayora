//! Public checkout API. Authorised by a per-intent client secret (not an API key), so a browser
//! can pay one specific intent and nothing else. Sessions are resumable: everything needed to
//! continue (status, last attempt, next action) is re-readable from the intent id and secret.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use zyvor_providers::Method;

use crate::{
    AppState,
    auth::hash_key,
    error::ApiError,
    payments::{AttemptRow, CreateAttempt, start_attempt},
    registry, tenant,
};

#[derive(Deserialize)]
pub struct SecretQuery {
    secret: String,
}

/// Verify the secret and return the tenant env. Unknown tenant, unknown intent and wrong secret are
/// indistinguishable to the caller.
async fn authorise(
    s: &AppState,
    tenant_id: &str,
    intent_id: &str,
    secret: &str,
) -> Result<String, ApiError> {
    let deny = || ApiError::not_found("checkout session");
    let env: String = sqlx::query_scalar("SELECT env FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(deny)?;
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT client_secret_hash FROM payment_intents WHERE id = $1")
            .bind(intent_id)
            .fetch_optional(&mut *tx)
            .await?;
    let stored = stored.flatten().ok_or_else(deny)?;
    if stored.as_bytes().ct_eq(hash_key(secret).as_bytes()).into() {
        Ok(env)
    } else {
        Err(deny())
    }
}

pub async fn session(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    Query(q): Query<SecretQuery>,
) -> Result<Json<Value>, ApiError> {
    let env = authorise(&s, &tenant_id, &intent_id, &q.secret).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let (amount, currency, status, description, allowed): (i64, String, String, Option<String>, Option<Vec<String>>) =
        sqlx::query_as("SELECT amount_minor, currency, status, description, allowed_providers FROM payment_intents WHERE id = $1").bind(&intent_id).fetch_one(&mut *tx).await?;
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&tenant_id).fetch_optional(&mut *tx).await?;

    // Which methods can this payment actually use? The union over enabled providers' capabilities.
    let routing = registry::routing(&mut tx, &tenant_id).await?;
    let candidates = registry::candidates(&s, &mut tx, &tenant_id, &env, &routing).await?;
    let ccy: zyvor_core::Currency = currency.parse().map_err(|_| ApiError::internal())?;
    let mut methods: Vec<&str> = [
        Method::Upi,
        Method::Card,
        Method::Netbanking,
        Method::Wallet,
    ]
    .into_iter()
    .filter(|m| {
        candidates.iter().any(|c| {
            c.enabled
                && allowed.as_ref().is_none_or(|a| a.contains(&c.provider))
                && c.caps.supports(*m, ccy).is_ok()
        })
    })
    .map(Method::as_str)
    .collect();
    methods.sort_unstable();
    // How should a UPI payment work? Prefer a partner-issued QR / intent (any UPI app). Asking the payer to
    // type a UPI ID ("collect") is only the fallback when no eligible provider can issue one.
    let upi_flow = if candidates.iter().any(|c| {
        c.enabled
            && allowed.as_ref().is_none_or(|a| a.contains(&c.provider))
            && c.caps.supports(Method::Upi, ccy).is_ok()
            && c.caps.upi_intent
    }) {
        "intent"
    } else {
        "collect"
    };

    // The latest attempt tells the page what to do next (and makes the session resumable).
    let attempt: Option<AttemptRow> = sqlx::query_as(
        "SELECT id, intent_id, provider, method, status, amount_minor, currency, provider_order_ref, provider_payment_ref, failure_reason, route, client_action, refunded_minor, reserved_refund_minor, created_at, updated_at
         FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&intent_id)
    .fetch_optional(&mut *tx)
    .await?;
    let phase = match (status.as_str(), attempt.as_ref().map(|a| a.status.as_str())) {
        ("succeeded", _) => "paid",
        (_, Some("unknown")) => "confirming",
        (_, Some("pending" | "created" | "authorized")) => "awaiting_payment",
        (_, Some("failed")) => "failed",
        _ => "choose_method",
    };
    Ok(Json(json!({
        "merchant": merchant, "amount_minor": amount, "currency": currency, "description": description,
        "status": status, "phase": phase, "methods": methods, "upi_flow": upi_flow,
        // Only what the page needs. No provider refs, no routing internals.
        "attempt": attempt.map(|a| json!({ "id": a.id, "method": a.method, "status": a.status, "failure_reason": a.failure_reason.filter(|_| a.status == "failed"), "action": a.client_action })),
    })))
}

#[derive(Deserialize)]
pub struct PayRequest {
    secret: String,
    method: String,
    vpa: Option<String>,
    return_url: Option<String>,
    /// `"intent"` when the session's `upi_flow` said so.
    upi_flow: Option<String>,
}

pub async fn pay(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(b): Json<PayRequest>,
) -> Result<Response, ApiError> {
    let env = authorise(&s, &tenant_id, &intent_id, &b.secret).await?;
    start_attempt(
        &s,
        &tenant_id,
        &env,
        "checkout",
        &intent_id,
        &headers,
        CreateAttempt {
            method: b.method,
            vpa: b.vpa,
            return_url: b.return_url,
            upi_flow: b.upi_flow,
        },
    )
    .await
}

/// `GET /v1/checkout/{tenant}/{intent}/upi-qr.svg?secret=`: the QR of the latest attempt's partner-issued
/// UPI payload, for a desktop or counter screen. The payload is encoded exactly as the partner issued it;
/// a QR is only a way to present it, never proof of payment.
pub async fn upi_qr(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    Query(q): Query<SecretQuery>,
) -> Result<Response, ApiError> {
    use axum::response::IntoResponse;
    authorise(&s, &tenant_id, &intent_id, &q.secret).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let action: Option<Value> = sqlx::query_scalar(
        "SELECT client_action FROM payment_attempts WHERE intent_id = $1 AND status IN ('pending', 'authorized', 'created') ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let uri = action
        .filter(|a| a["type"] == "upi_intent")
        .and_then(|a| a["uri"].as_str().map(str::to_string))
        .ok_or_else(|| ApiError::not_found("UPI payment"))?;
    let svg = qrcode::QrCode::new(uri.as_bytes())
        .map_err(|_| ApiError::internal())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build();
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        svg,
    )
        .into_response())
}
