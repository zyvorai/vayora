// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Public checkout API. Authorised by a per-intent client secret (not an API key), so a browser
//! can pay one specific intent and nothing else. Sessions are resumable: everything needed to
//! continue (status, last attempt, next action) is re-readable from the intent id and secret.
//! Live status also streams over SSE (`GET …/events`) so the page need not poll while a payment
//! is in flight.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::stream::Stream;
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

async fn session_body(
    s: &AppState,
    tenant_id: &str,
    intent_id: &str,
    env: &str,
) -> Result<Value, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let (amount, currency, status, description, allowed): (
        i64,
        String,
        String,
        Option<String>,
        Option<Vec<String>>,
    ) = sqlx::query_as(
        "SELECT amount_minor, currency, status, description, allowed_providers FROM payment_intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(&mut *tx)
    .await?;
    let merchant: Option<String> = sqlx::query_scalar(
        "SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let routing = registry::routing(&mut tx, tenant_id).await?;
    let candidates = registry::candidates(s, &mut tx, tenant_id, env, &routing).await?;
    let ccy: zyvor_core::Currency = currency.parse().map_err(|_| ApiError::internal())?;
    let only: Option<Vec<String>> = sqlx::query_scalar::<_, Option<Value>>(
        "SELECT metadata->'allowed_methods' FROM payment_intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(&mut *tx)
    .await?
    .and_then(|v| serde_json::from_value(v).ok());
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
    .filter(|m| only.as_ref().is_none_or(|o| o.iter().any(|x| x == m)))
    .collect();
    methods.sort_unstable();
    // a payment made through a payment link may be limited to some methods, and carries the link's wording
    let link = crate::payment_links::presentation(&mut tx, tenant_id, intent_id).await?;
    if let Some(allowed) = link.as_ref().and_then(|l| l["allowed_methods"].as_array()) {
        methods.retain(|m| allowed.iter().any(|a| a.as_str() == Some(m)));
    }
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

    let attempt: Option<AttemptRow> = sqlx::query_as(
        "SELECT id, intent_id, provider, method, status, amount_minor, currency, provider_order_ref, provider_payment_ref, failure_reason, route, client_action, refunded_minor, reserved_refund_minor, created_at, updated_at
         FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(intent_id)
    .fetch_optional(&mut *tx)
    .await?;
    // A definitive provider-side failure that a soft retry is about to answer with another provider. The payer is
    // told so; no second payment can exist until that retry is created, and it is created only after this attempt
    // has failed (see softretry.rs).
    let retrying = attempt.as_ref().is_some_and(|a| a.status == "failed")
        && status != "succeeded"
        && crate::softretry::pending(&mut tx, intent_id).await?;
    let retried = attempt
        .as_ref()
        .is_some_and(|a| a.route.get("soft_retry").is_some());
    let phase = match (status.as_str(), attempt.as_ref().map(|a| a.status.as_str())) {
        ("succeeded", _) => "paid",
        (_, Some("unknown")) => "confirming",
        (_, Some("pending" | "created" | "authorized")) => "awaiting_payment",
        (_, Some("failed")) if retrying => "trying_another_way",
        (_, Some("failed")) => "failed",
        _ => "choose_method",
    };
    Ok(json!({
        "merchant": merchant, "amount_minor": amount, "currency": currency, "description": description,
        "status": status, "phase": phase, "methods": methods, "upi_flow": upi_flow, "link": link,
        // true when the attempt shown is a second try on another provider after the first one failed definitively
        "retried_on_another_provider": retried,
        "attempt": attempt.map(|a| json!({ "id": a.id, "method": a.method, "status": a.status, "amount_minor": a.amount_minor, "failure_reason": a.failure_reason.filter(|_| a.status == "failed"), "action": a.client_action })),
    }))
}

pub async fn session(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    Query(q): Query<SecretQuery>,
) -> Result<Json<Value>, ApiError> {
    let env = authorise(&s, &tenant_id, &intent_id, &q.secret).await?;
    Ok(Json(session_body(&s, &tenant_id, &intent_id, &env).await?))
}

/// `GET /v1/checkout/{tenant}/{intent}/events?secret=`: Server-Sent Events of the same session JSON.
/// Emits on change; ends when the phase is terminal (`paid` / `failed` / `choose_method`).
pub async fn events(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    Query(q): Query<SecretQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let env = authorise(&s, &tenant_id, &intent_id, &q.secret).await?;
    let stream = async_stream::stream! {
        let mut last = String::new();
        // Cap the stream so a forgotten tab cannot hold a connection forever.
        for _ in 0..450 {
            match session_body(&s, &tenant_id, &intent_id, &env).await {
                Ok(body) => {
                    let phase = body["phase"].as_str().unwrap_or_default().to_string();
                    let data = body.to_string();
                    if data != last {
                        last = data.clone();
                        yield Ok(Event::default().event("session").data(data));
                    }
                    if matches!(phase.as_str(), "paid" | "failed" | "choose_method") {
                        break;
                    }
                }
                Err(_) => break,
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

#[derive(Deserialize)]
pub struct PayRequest {
    secret: String,
    method: String,
    vpa: Option<String>,
    return_url: Option<String>,
    /// `"intent"` when the session's `upi_flow` said so.
    upi_flow: Option<String>,
    /// What the payer chose (card network, UPI app, bank), never a card number. Not checked against the provider.
    #[serde(default)]
    instrument: Option<crate::instrument_hint::InstrumentIn>,
    /// Offer codes the payer typed.
    #[serde(default)]
    offer_codes: Option<Vec<String>>,
}

pub async fn pay(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(b): Json<PayRequest>,
) -> Result<Response, ApiError> {
    let env = authorise(&s, &tenant_id, &intent_id, &b.secret).await?;
    // a payment made through a link that accepts only some methods can only be paid with those
    {
        let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
        let allowed: Option<Vec<String>> = sqlx::query_scalar(
            "SELECT l.allowed_methods FROM payment_intents i JOIN payment_links l ON l.id = i.metadata->>'payment_link' WHERE i.id = $1",
        )
        .bind(&intent_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        if let Some(list) = allowed
            && !list.contains(&b.method.trim().to_lowercase())
        {
            return Err(ApiError::invalid(
                "method_not_allowed",
                "this payment link does not accept that method",
            ));
        }
    }
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
            card_token_id: None,
            instrument: b.instrument,
            offer_codes: b.offer_codes,
        },
    )
    .await
}

#[derive(Deserialize)]
pub struct OffersRequest {
    secret: String,
    method: String,
    vpa: Option<String>,
    #[serde(default)]
    instrument: Option<crate::instrument_hint::InstrumentIn>,
    #[serde(default)]
    offer_codes: Option<Vec<String>>,
}

/// `POST /v1/checkout/{tenant}/{intent}/offers`: the price the payer would be asked for with this method, after any
/// offers. Nothing is reserved; the offers are judged again, and the caps counted again, when the payment starts.
/// Only what the payer needs is returned: the offers that apply, not the ones that do not.
pub async fn offers(
    State(s): State<AppState>,
    Path((tenant_id, intent_id)): Path<(String, String)>,
    Json(b): Json<OffersRequest>,
) -> Result<Json<Value>, ApiError> {
    authorise(&s, &tenant_id, &intent_id, &b.secret).await?;
    let method: Method = b
        .method
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_method", e))?;
    let ins = crate::instrument_hint::build(b.instrument.as_ref(), b.vpa.as_deref(), None)?;
    let codes = crate::offers::clean_codes(b.offer_codes.as_ref());
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let (amount, currency, customer): (i64, String, Option<String>) = sqlx::query_as(
        "SELECT amount_minor, currency, customer_id FROM payment_intents WHERE id = $1",
    )
    .bind(&intent_id)
    .fetch_one(&mut *tx)
    .await?;
    let segment = crate::offers::segment(&mut tx, customer.as_deref(), Some(&intent_id)).await?;
    let ev = crate::offers::preview_for_attempt(
        &mut tx,
        &crate::offers::AttemptFacts {
            intent_id: &intent_id,
            amount_minor: amount,
            currency: &currency,
            method: method.as_str(),
            instrument: &ins,
            customer_id: customer.as_deref(),
            segment,
            codes: &codes,
        },
    )
    .await?;
    Ok(Json(json!({
        "gross_minor": ev.gross_minor, "discount_minor": ev.discount_minor, "net_minor": ev.net_minor,
        "currency": currency, "funded_by": "merchant",
        "offers": ev.applied.iter().map(|a| json!({ "name": a.name, "discount_minor": a.discount_minor })).collect::<Vec<_>>(),
    })))
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
