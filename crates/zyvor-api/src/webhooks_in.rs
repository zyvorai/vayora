//! Inbound provider webhooks: verify, de-duplicate on (tenant, provider, event id), then apply
//! through the same state machine as every other source. The inbox row and the state change commit
//! together, so a failure leaves nothing behind and the provider's retry is processed normally.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::{Value, json};
use zyvor_providers::{EventKind, Headers};

use crate::{
    AppState,
    cases::open_case,
    error::ApiError,
    payments::{Incoming, apply_provider_state},
    refunds::apply_refund_result,
    registry,
    state_machine::Attempt,
    tenant,
};

pub async fn receive(
    State(s): State<AppState>,
    Path((provider, tenant_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    // One undifferentiated 401 for unknown tenant, unknown provider and bad signature: no enumeration.
    let reject = || ApiError::unauthorized("invalid webhook");
    let env: Option<String> = sqlx::query_scalar("SELECT env FROM tenants WHERE id = $1")
        .bind(&tenant_id)
        .fetch_optional(&s.pool)
        .await?;
    let env = env.ok_or_else(reject)?;

    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let secret = registry::webhook_secret(&s, &mut tx, &tenant_id, &provider)
        .await
        .map_err(|_| reject())?;
    let adapter = registry::load(&s, &mut tx, &tenant_id, &env, &provider)
        .await
        .map_err(|_| reject())?;
    let hdrs: Headers = headers
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_lowercase(), v.to_str().ok()?.to_string())))
        .collect();
    let ev = adapter
        .verify_webhook(&secret, &hdrs, &body, chrono::Utc::now().timestamp())
        .map_err(|e| {
            tracing::warn!(provider, tenant = %tenant_id, error = %e, "webhook rejected");
            reject()
        })?;

    let fresh = sqlx::query("INSERT INTO webhook_inbox (tenant_id, provider, event_id, kind, payload) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
        .bind(&tenant_id)
        .bind(&provider)
        .bind(&ev.event_id)
        .bind(format!("{:?}", ev.kind))
        .bind(&ev.raw)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    if !fresh {
        return Ok(Json(json!({ "status": "duplicate" })));
    }

    let outcome = match ev.kind {
        EventKind::PaymentAuthorized | EventKind::PaymentCaptured | EventKind::PaymentFailed => {
            let attempt: Option<String> = sqlx::query_scalar(
                "SELECT id FROM payment_attempts WHERE provider = $1 AND (provider_order_ref = $2 OR id = $2 OR (provider_payment_ref IS NOT NULL AND provider_payment_ref = $3))",
            )
            .bind(&provider)
            .bind(ev.order_ref.as_deref().unwrap_or(""))
            .bind(ev.provider_payment_ref.as_deref().unwrap_or(""))
            .fetch_optional(&mut *tx)
            .await?;
            match attempt {
                None => {
                    open_case(&mut tx, &tenant_id, "unmatched_webhook", "webhook", &ev.event_id, None, ev.amount_minor, &format!("Provider {provider} reported a payment for order {:?} that matches none of our attempts.", ev.order_ref), json!({ "provider": provider })).await?;
                    "unmatched".to_string()
                }
                Some(id) => {
                    let inc = Incoming {
                        status: match ev.kind {
                            EventKind::PaymentCaptured => Attempt::Captured,
                            EventKind::PaymentAuthorized => Attempt::Authorized,
                            _ => Attempt::Failed,
                        },
                        provider_payment_ref: ev.provider_payment_ref.clone(),
                        failure_reason: ev.failure_reason.clone(),
                        amount_minor: ev.amount_minor,
                        raw: Some(ev.raw.clone()),
                    };
                    format!(
                        "{:?}",
                        apply_provider_state(&mut tx, &tenant_id, &id, &inc, "webhook").await?
                    )
                }
            }
        }
        EventKind::RefundProcessed | EventKind::RefundFailed => {
            let refund: Option<String> = sqlx::query_scalar("SELECT id FROM refunds WHERE id = $1 OR (provider_refund_ref IS NOT NULL AND provider_refund_ref = $2)")
                .bind(ev.refund_ref.as_deref().unwrap_or(""))
                .bind(ev.provider_refund_ref.as_deref().unwrap_or(""))
                .fetch_optional(&mut *tx)
                .await?;
            match refund {
                None => {
                    open_case(
                        &mut tx,
                        &tenant_id,
                        "unmatched_webhook",
                        "webhook",
                        &ev.event_id,
                        None,
                        ev.amount_minor,
                        "Provider reported a refund we have no record of.",
                        json!({ "provider": provider }),
                    )
                    .await?;
                    "unmatched".to_string()
                }
                Some(id) => {
                    let done = apply_refund_result(
                        &mut tx,
                        &tenant_id,
                        &id,
                        ev.kind == EventKind::RefundProcessed,
                        ev.provider_refund_ref.clone(),
                        ev.failure_reason.clone(),
                    )
                    .await?;
                    if done { "applied" } else { "noop" }.to_string()
                }
            }
        }
        EventKind::Other => "ignored".to_string(),
    };
    sqlx::query("UPDATE webhook_inbox SET outcome = $4 WHERE tenant_id = $1 AND provider = $2 AND event_id = $3").bind(&tenant_id).bind(&provider).bind(&ev.event_id).bind(&outcome).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "status": "processed", "outcome": outcome })))
}
