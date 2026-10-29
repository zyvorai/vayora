//! Payment intents and attempts: routing, provider calls, and the one function that applies
//! provider-reported state (from API responses, webhooks or the resolver) exactly once.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use zyvor_core::{Currency, new_id};
use zyvor_ledger::{Entry, Transaction, accounts};
use zyvor_providers::{
    ClientAction, CreatePayment, Method, PaymentState, PaymentStatus, ProviderError, router,
};

use crate::{
    AppState,
    auth::Auth,
    cases::open_case,
    error::ApiError,
    idempotency::{self, Begin, fingerprint, replay_response},
    registry,
    state_machine::{Attempt, Transition, attempt_transition},
    tenant,
};

#[derive(Debug, Serialize, FromRow)]
pub struct IntentRow {
    pub id: String,
    pub customer_id: Option<String>,
    pub invoice_id: Option<String>,
    pub branch_id: Option<String>,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub capture_method: String,
    pub allowed_providers: Option<Vec<String>>,
    pub external_ref: Option<String>,
    pub description: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AttemptRow {
    pub id: String,
    pub intent_id: String,
    pub provider: String,
    pub method: String,
    pub status: String,
    pub amount_minor: i64,
    pub currency: String,
    pub provider_order_ref: Option<String>,
    pub provider_payment_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub route: Value,
    pub client_action: Option<Value>,
    pub refunded_minor: i64,
    pub reserved_refund_minor: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn fetch_attempt(conn: &mut PgConnection, id: &str) -> Result<AttemptRow, ApiError> {
    sqlx::query_as(
        "SELECT id, intent_id, provider, method, status, amount_minor, currency, provider_order_ref, provider_payment_ref, failure_reason,
                route, client_action, refunded_minor, reserved_refund_minor, created_at, updated_at FROM payment_attempts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("attempt"))
}

// ---------- intents ----------

#[derive(Deserialize)]
pub struct CreateIntent {
    amount_minor: i64,
    currency: String,
    customer_id: Option<String>,
    invoice_id: Option<String>,
    branch_id: Option<String>,
    capture_method: Option<String>,
    allowed_providers: Option<Vec<String>>,
    /// The business object being paid for (student fee, patient deposit, sales order...).
    external_ref: Option<String>,
    description: Option<String>,
    metadata: Option<Value>,
}

pub async fn create_intent(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Json(b): Json<CreateIntent>,
) -> Result<Response, ApiError> {
    auth.require("payments:write")?;
    let currency: Currency = b
        .currency
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if b.amount_minor <= 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive integer",
        ));
    }
    let capture = b.capture_method.as_deref().unwrap_or("automatic");
    if !matches!(capture, "automatic" | "manual") {
        return Err(ApiError::invalid(
            "invalid_capture_method",
            "capture_method must be automatic or manual",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let hash = fingerprint(&[
        &b.amount_minor.to_string(),
        currency.code(),
        b.customer_id.as_deref().unwrap_or(""),
        b.invoice_id.as_deref().unwrap_or(""),
        b.external_ref.as_deref().unwrap_or(""),
        capture,
    ]);
    let key = match idempotency::begin(&mut tx, &auth.tenant_id, "intents", &headers, &hash).await?
    {
        Begin::Replay { status, body, .. } => {
            return Ok(replay_response(status, body.unwrap_or(Value::Null)));
        }
        Begin::Proceed(k) => k,
    };
    if let Some(cid) = &b.customer_id
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
            .bind(cid)
            .fetch_one(&mut *tx)
            .await?
    {
        return Err(ApiError::not_found("customer"));
    }
    if let Some(inv) = &b.invoice_id {
        let row: Option<(String, i64, String, i64)> = sqlx::query_as(
            "SELECT currency, amount_minor, status, COALESCE((SELECT SUM(amount_minor) FROM invoice_allocations a WHERE a.invoice_id = invoices.id), 0)::BIGINT FROM invoices WHERE id = $1",
        )
        .bind(inv)
        .fetch_optional(&mut *tx)
        .await?;
        let (ccy, total, status, paid) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
        if ccy != currency.code() {
            return Err(ApiError::invalid(
                "currency_mismatch",
                "payment currency must match the invoice",
            ));
        }
        if status == "void" || status == "paid" {
            return Err(ApiError::conflict(
                "invoice_not_payable",
                format!("invoice is {status}"),
            ));
        }
        if b.amount_minor > total - paid {
            return Err(ApiError::invalid(
                "exceeds_outstanding",
                format!(
                    "outstanding on this invoice is {} minor units",
                    total - paid
                ),
            ));
        }
    }
    let client_secret = format!(
        "pis_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let intent: IntentRow = sqlx::query_as(
        "INSERT INTO payment_intents (id, tenant_id, customer_id, invoice_id, branch_id, amount_minor, currency, status, capture_method, allowed_providers, external_ref, description, metadata, client_secret_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'requires_payment_method', $8, $9, $10, $11, $12, $13) RETURNING id, customer_id, invoice_id, branch_id, amount_minor, currency, status, capture_method, allowed_providers, external_ref, description, metadata, created_at, updated_at",
    )
    .bind(new_id("pi"))
    .bind(&auth.tenant_id)
    .bind(&b.customer_id)
    .bind(&b.invoice_id)
    .bind(&b.branch_id)
    .bind(b.amount_minor)
    .bind(currency.code())
    .bind(capture)
    .bind(&b.allowed_providers)
    .bind(&b.external_ref)
    .bind(&b.description)
    .bind(b.metadata.unwrap_or_else(|| json!({})))
    .bind(crate::auth::hash_key(&client_secret))
    .fetch_one(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_intent.created",
        Some(&intent.id),
        json!({ "amount_minor": intent.amount_minor, "external_ref": intent.external_ref }),
    )
    .await?;
    // The secret is returned here, once, for handing to the browser. Only its hash is stored.
    let mut body = serde_json::to_value(&intent).expect("serialises");
    body["client_secret"] = json!(client_secret);
    let res = key.finish(&mut tx, StatusCode::CREATED, &body).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn get_intent(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let intent: IntentRow = sqlx::query_as(
        "SELECT id, customer_id, invoice_id, branch_id, amount_minor, currency, status, capture_method, allowed_providers, external_ref, description, metadata, created_at, updated_at FROM payment_intents WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("payment intent"))?;
    let attempts: Vec<AttemptRow> = sqlx::query_as(
        "SELECT id, intent_id, provider, method, status, amount_minor, currency, provider_order_ref, provider_payment_ref, failure_reason, route, client_action, refunded_minor, reserved_refund_minor, created_at, updated_at
         FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "intent": intent, "attempts": attempts })))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub starting_after: Option<String>,
    pub status: Option<String>,
}

pub async fn list_intents(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut rows: Vec<IntentRow> = sqlx::query_as(
        "SELECT id, customer_id, invoice_id, branch_id, amount_minor, currency, status, capture_method, allowed_providers, external_ref, description, metadata, created_at, updated_at
         FROM payment_intents WHERE ($1::text IS NULL OR status = $1)
           AND ($2::text IS NULL OR (created_at, id) < (SELECT created_at, id FROM payment_intents WHERE id = $2))
         ORDER BY created_at DESC, id DESC LIMIT $3",
    )
    .bind(&q.status)
    .bind(&q.starting_after)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    Ok(Json(json!({ "data": rows, "has_more": has_more })))
}

// ---------- attempts ----------

#[derive(Deserialize)]
pub struct CreateAttempt {
    pub method: String,
    pub vpa: Option<String>,
    pub return_url: Option<String>,
}

fn valid_vpa(v: &str) -> bool {
    v.split_once('@').is_some_and(|(h, p)| {
        (2..=64).contains(&h.len())
            && h.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            && (2..=32).contains(&p.len())
            && p.chars().all(|c| c.is_ascii_alphabetic())
    })
}

pub async fn create_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path(intent_id): Path<String>,
    headers: HeaderMap,
    Json(b): Json<CreateAttempt>,
) -> Result<Response, ApiError> {
    auth.require("payments:write")?;
    start_attempt(
        &s,
        &auth.tenant_id,
        &auth.env,
        &auth.actor,
        &intent_id,
        &headers,
        b,
    )
    .await
}

/// Shared by the authenticated API and the public checkout. Persists the attempt before calling the provider.
pub async fn start_attempt(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    actor: &str,
    intent_id: &str,
    headers: &HeaderMap,
    b: CreateAttempt,
) -> Result<Response, ApiError> {
    let method: Method = b
        .method
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_method", e))?;
    if let Some(v) = &b.vpa
        && !valid_vpa(v)
    {
        return Err(ApiError::invalid(
            "invalid_vpa",
            "vpa must look like name@psp",
        ));
    }

    // ---- tx1: claim the key, validate, route, and persist the attempt BEFORE calling the provider ----
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let hash = fingerprint(&[
        intent_id,
        method.as_str(),
        b.vpa.as_deref().unwrap_or(""),
        b.return_url.as_deref().unwrap_or(""),
    ]);
    let key = match idempotency::begin(&mut tx, tenant_id, "attempts", headers, &hash).await? {
        Begin::Replay {
            status,
            body,
            resource_id,
        } => {
            // Replays return the attempt as it is *now*, not a stale snapshot.
            let body = match resource_id {
                Some(id) => serde_json::to_value(fetch_attempt(&mut tx, &id).await?).unwrap(),
                None => body.unwrap_or(Value::Null),
            };
            return Ok(replay_response(status, body));
        }
        Begin::Proceed(k) => k,
    };
    let intent: (String, i64, String, String, String, Option<Vec<String>>, Option<String>) = sqlx::query_as(
        "SELECT status, amount_minor, currency, capture_method, id, allowed_providers, customer_id FROM payment_intents WHERE id = $1 FOR UPDATE",
    )
    .bind(intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("payment intent"))?;
    let (status, amount_minor, ccy, capture, _, allowed, customer_id) = intent;
    let currency: Currency = ccy.parse().map_err(|_| ApiError::internal())?;
    let live: Vec<(String, String)> = sqlx::query_as("SELECT id, status FROM payment_attempts WHERE intent_id = $1 AND status IN ('created', 'pending', 'authorized', 'unknown', 'captured')")
        .bind(intent_id)
        .fetch_all(&mut *tx)
        .await?;
    if let Some((id, st)) = live.iter().find(|(_, st)| st == "unknown") {
        // Never start another payment while we don't know whether the last one took the money.
        return Err(ApiError::conflict(
            "payment_outcome_unresolved",
            format!(
                "attempt {id} has an unknown outcome ({st}); it must be resolved before another attempt"
            ),
        ));
    }
    if let Some((id, st)) = live.first() {
        return Err(ApiError::conflict(
            "attempt_in_progress",
            format!("attempt {id} is already {st}"),
        ));
    }
    if status != "requires_payment_method" {
        return Err(ApiError::conflict(
            "intent_not_payable",
            format!("intent is {status}"),
        ));
    }

    let routing = registry::routing(&mut tx, tenant_id).await?;
    let candidates = registry::candidates(s, &mut tx, tenant_id, env, &routing).await?;
    let decision = router::choose(
        &router::Request {
            method,
            amount_minor,
            currency,
            allowed,
        },
        &candidates,
        &routing.rules,
    )
    .map_err(|e| ApiError::invalid("no_route", e.to_string()))?;
    let provider = registry::load(s, &mut tx, tenant_id, env, &decision.provider).await?;

    let attempt_id = new_id("att");
    sqlx::query(
        "INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route) VALUES ($1, $2, $3, $4, $5, 'created', $6, $7, $8)",
    )
    .bind(&attempt_id)
    .bind(tenant_id)
    .bind(intent_id)
    .bind(&decision.provider)
    .bind(method.as_str())
    .bind(amount_minor)
    .bind(currency.code())
    .bind(serde_json::to_value(&decision).unwrap())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE payment_intents SET status = 'processing', updated_at = now() WHERE id = $1",
    )
    .bind(intent_id)
    .execute(&mut *tx)
    .await?;
    key.set_resource(&mut tx, &attempt_id).await?;
    tenant::audit(
        &mut tx,
        tenant_id,
        actor,
        "payment_attempt.created",
        Some(&attempt_id),
        json!({ "provider": decision.provider, "reasons": decision.reasons }),
    )
    .await?;
    // Customer details some providers require.
    let phone: Option<String> = match &customer_id {
        Some(c) => {
            sqlx::query_scalar("SELECT phone FROM customers WHERE id = $1")
                .bind(c)
                .fetch_one(&mut *tx)
                .await?
        }
        None => None,
    };
    tx.commit().await?;

    // ---- provider call, outside any DB transaction ----
    let req = CreatePayment {
        attempt_id: attempt_id.clone(),
        amount_minor,
        currency,
        method,
        vpa: b.vpa.clone(),
        customer_id,
        customer_phone: phone,
        capture_automatic: capture == "automatic",
        return_url: b.return_url.clone(),
    };
    let result = provider.initiate(&req).await;

    // ---- tx2: record whatever happened, including "we don't know" ----
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    match result {
        Ok(init) => {
            if !matches!(init.action, ClientAction::None) {
                sqlx::query("UPDATE payment_attempts SET client_action = $2 WHERE id = $1")
                    .bind(&attempt_id)
                    .bind(serde_json::to_value(&init.action).unwrap())
                    .execute(&mut *tx)
                    .await?;
            }
            let order_ref = init.state.provider_order_ref.clone();
            if order_ref.is_some() {
                sqlx::query("UPDATE payment_attempts SET provider_order_ref = $2 WHERE id = $1")
                    .bind(&attempt_id)
                    .bind(order_ref)
                    .execute(&mut *tx)
                    .await?;
            }
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &incoming(&init.state),
                "api",
            )
            .await?;
        }
        Err(ProviderError::Uncertain(why)) => {
            tracing::warn!(attempt = %attempt_id, %why, "provider outcome unknown");
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &Incoming {
                    status: Attempt::Unknown,
                    failure_reason: Some(why.clone()),
                    ..Incoming::default()
                },
                "api",
            )
            .await?;
            open_case(&mut tx, tenant_id, "unresolved_payment", "payment_attempt", &attempt_id, None, None, &format!("The provider did not confirm the outcome ({why}). No second payment will be started until it is resolved."), json!({ "provider": decision.provider })).await?;
        }
        Err(ProviderError::Rejected(why) | ProviderError::Unsupported(why)) => {
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &Incoming {
                    status: Attempt::Failed,
                    failure_reason: Some(why),
                    ..Incoming::default()
                },
                "api",
            )
            .await?;
        }
    }
    let row = fetch_attempt(&mut tx, &attempt_id).await?;
    let res = key.finish(&mut tx, StatusCode::CREATED, &row).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn get_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<AttemptRow>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(fetch_attempt(&mut tx, &id).await?))
}

/// Ask the provider where the payment stands and apply it. Safe to call any time, any number of times.
pub async fn refresh_attempt_internal(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    attempt_id: &str,
) -> Result<AttemptRow, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let row = fetch_attempt(&mut tx, attempt_id).await?;
    let provider = registry::load(s, &mut tx, tenant_id, env, &row.provider).await?;
    tx.commit().await?;
    match provider.fetch_payment(attempt_id).await {
        Ok(state) => {
            let mut tx = tenant::begin(&s.pool, tenant_id).await?;
            apply_provider_state(
                &mut tx,
                tenant_id,
                attempt_id,
                &incoming(&state),
                "resolver",
            )
            .await?;
            let row = fetch_attempt(&mut tx, attempt_id).await?;
            tx.commit().await?;
            Ok(row)
        }
        Err(e) => {
            tracing::warn!(attempt = attempt_id, error = %e, "could not refresh attempt");
            Ok(row)
        }
    }
}

pub async fn refresh_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<AttemptRow>, ApiError> {
    auth.require("payments:write")?;
    Ok(Json(
        refresh_attempt_internal(&s, &auth.tenant_id, &auth.env, &id).await?,
    ))
}

/// Cancel a payment that has not completed. We first ask the provider; if it already succeeded we adopt that instead.
pub async fn cancel_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<AttemptRow>, ApiError> {
    auth.require("payments:write")?;
    let row = refresh_attempt_internal(&s, &auth.tenant_id, &auth.env, &id).await?;
    if !matches!(row.status.as_str(), "created" | "pending") {
        return Err(ApiError::conflict(
            "not_cancellable",
            format!("attempt is {}", row.status),
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    apply_provider_state(
        &mut tx,
        &auth.tenant_id,
        &id,
        &Incoming {
            status: Attempt::Failed,
            failure_reason: Some("canceled_by_merchant".into()),
            ..Incoming::default()
        },
        "cancel",
    )
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_attempt.canceled",
        Some(&id),
        json!({}),
    )
    .await?;
    let row = fetch_attempt(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(row))
}

/// Manual capture of an authorised payment.
pub async fn capture_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<AttemptRow>, ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row = fetch_attempt(&mut tx, &id).await?;
    if row.status != "authorized" {
        return Err(ApiError::conflict(
            "not_authorized",
            format!(
                "attempt is {}, only authorized payments can be captured",
                row.status
            ),
        ));
    }
    let provider = registry::load(&s, &mut tx, &auth.tenant_id, &auth.env, &row.provider).await?;
    let currency: Currency = row.currency.parse().map_err(|_| ApiError::internal())?;
    tx.commit().await?;
    match provider
        .capture(
            &id,
            row.provider_payment_ref.as_deref().unwrap_or_default(),
            row.amount_minor,
            currency,
        )
        .await
    {
        Ok(state) => {
            let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
            apply_provider_state(&mut tx, &auth.tenant_id, &id, &incoming(&state), "api").await?;
            tenant::audit(
                &mut tx,
                &auth.tenant_id,
                &auth.actor,
                "payment_attempt.captured_manually",
                Some(&id),
                json!({}),
            )
            .await?;
            let row = fetch_attempt(&mut tx, &id).await?;
            tx.commit().await?;
            Ok(Json(row))
        }
        Err(ProviderError::Uncertain(why)) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "capture_outcome_unknown",
            format!("capture outcome unknown ({why}); the worker will re-check it"),
        )),
        Err(ProviderError::Rejected(why) | ProviderError::Unsupported(why)) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "capture_rejected",
            why,
        )),
    }
}

// ---------- applying provider state ----------

#[derive(Debug, Default, Clone)]
pub struct Incoming {
    pub status: Attempt,
    pub provider_payment_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub amount_minor: Option<i64>,
    pub raw: Option<Value>,
}

pub fn incoming(st: &PaymentState) -> Incoming {
    Incoming {
        status: match st.status {
            PaymentStatus::Pending => Attempt::Pending,
            PaymentStatus::Authorized => Attempt::Authorized,
            PaymentStatus::Captured => Attempt::Captured,
            PaymentStatus::Failed => Attempt::Failed,
        },
        provider_payment_ref: st.provider_payment_ref.clone(),
        failure_reason: st.failure_reason.clone(),
        amount_minor: None,
        raw: Some(st.raw.clone()),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Applied(Attempt),
    NoOp,
    Ignored,
    AmountMismatch,
}

/// The single place attempt state changes. Locks the attempt, applies the monotonic rules, and runs
/// side effects (ledger, intent, invoice, outbox) at most once per real transition.
pub async fn apply_provider_state(
    conn: &mut PgConnection,
    tenant_id: &str,
    attempt_id: &str,
    inc: &Incoming,
    source: &str,
) -> Result<Applied, ApiError> {
    let (intent_id, provider, current, cur_ref, amount, ccy): (String, String, String, Option<String>, i64, String) = sqlx::query_as(
        "SELECT intent_id, provider, status, provider_payment_ref, amount_minor, currency FROM payment_attempts WHERE id = $1 FOR UPDATE",
    )
    .bind(attempt_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| ApiError::not_found("attempt"))?;
    let current = Attempt::parse(&current).ok_or_else(ApiError::internal)?;
    let currency: Currency = ccy.parse().map_err(|_| ApiError::internal())?;

    if matches!(inc.status, Attempt::Captured | Attempt::Authorized)
        && let Some(a) = inc.amount_minor
        && a != amount
    {
        open_case(conn, tenant_id, "amount_mismatch", "payment_attempt", attempt_id, Some(amount), Some(a), &format!("The provider reported {a} minor units for a payment we created for {amount}. State was not changed."), json!({ "provider": provider, "source": source })).await?;
        return Ok(Applied::AmountMismatch);
    }
    // A second, different provider payment captured for the same attempt: money was taken twice.
    if current == Attempt::Captured
        && inc.status == Attempt::Captured
        && let (Some(a), Some(b)) = (&cur_ref, &inc.provider_payment_ref)
        && a != b
    {
        open_case(conn, tenant_id, "duplicate_payment", "payment_attempt", attempt_id, Some(amount), Some(amount * 2), &format!("A second payment {b} was captured for an attempt already paid by {a}. The customer was charged twice; refund one."), json!({ "first": a, "second": b, "provider": provider })).await?;
        return Ok(Applied::Ignored);
    }

    // A provider payment reference belongs to exactly one attempt. If a provider reports one we already
    // attached elsewhere, something is wrong on their side or ours: don't guess, open a case.
    if let Some(r) = &inc.provider_payment_ref
        && let Some(other) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM payment_attempts WHERE provider = $1 AND provider_payment_ref = $2 AND id <> $3",
        )
        .bind(&provider)
        .bind(r)
        .bind(attempt_id)
        .fetch_optional(&mut *conn)
        .await?
    {
        open_case(
            conn,
            tenant_id,
            "provider_reference_conflict",
            "payment_attempt",
            attempt_id,
            None,
            inc.amount_minor,
            &format!("{provider} reported payment {r} for this attempt, but that reference already belongs to attempt {other}. State was not changed."),
            json!({ "reference": r, "other_attempt": other, "source": source }),
        )
        .await?;
        return Ok(Applied::Ignored);
    }

    let same_payment = cur_ref.is_none()
        || inc.provider_payment_ref.is_none()
        || cur_ref == inc.provider_payment_ref;
    match attempt_transition(current, inc.status, same_payment) {
        Transition::NoOp => return Ok(Applied::NoOp),
        Transition::Ignore => {
            tracing::info!(
                attempt = attempt_id,
                from = current.as_str(),
                to = inc.status.as_str(),
                source,
                "ignored out-of-order provider state"
            );
            return Ok(Applied::Ignored);
        }
        Transition::Apply => {}
    }

    sqlx::query(
        "UPDATE payment_attempts SET status = $2, provider_payment_ref = COALESCE($3, provider_payment_ref),
                failure_reason = CASE WHEN $2 = 'failed' OR $2 = 'unknown' THEN COALESCE($4, failure_reason) ELSE NULL END,
                raw = raw || COALESCE($5, '{}'::jsonb), updated_at = now() WHERE id = $1",
    )
    .bind(attempt_id)
    .bind(inc.status.as_str())
    .bind(&inc.provider_payment_ref)
    .bind(&inc.failure_reason)
    .bind(inc.raw.as_ref().map(|r| json!({ source: r })))
    .execute(&mut *conn)
    .await?;

    if current == Attempt::Unknown {
        // The outcome is now known, so the task that asked someone to look into it closes itself.
        sqlx::query("UPDATE recon_cases SET status = 'resolved', resolution = $2, resolved_at = now() WHERE kind = 'unresolved_payment' AND subject_id = $1 AND status <> 'resolved'")
            .bind(attempt_id)
            .bind(format!("provider reported {} via {source}", inc.status.as_str()))
            .execute(&mut *conn)
            .await?;
    }
    let payload = json!({ "intent_id": intent_id, "attempt_id": attempt_id, "provider": provider, "amount_minor": amount, "currency": currency });
    match inc.status {
        Attempt::Captured => {
            if current == Attempt::Failed {
                tenant::audit(
                    conn,
                    tenant_id,
                    source,
                    "payment_attempt.late_capture",
                    Some(attempt_id),
                    json!({ "note": "captured after being marked failed; money wins" }),
                )
                .await?;
            }
            on_captured(
                conn, tenant_id, attempt_id, &intent_id, &provider, amount, currency,
            )
            .await?;
        }
        Attempt::Authorized => {
            sqlx::query("UPDATE payment_intents SET status = 'processing', updated_at = now() WHERE id = $1").bind(&intent_id).execute(&mut *conn).await?;
            tenant::emit(conn, tenant_id, "payment.authorized", payload).await?;
        }
        Attempt::Failed => {
            sqlx::query(
                "UPDATE payment_intents SET status = 'requires_payment_method', updated_at = now()
                 WHERE id = $1 AND status = 'processing' AND NOT EXISTS (SELECT 1 FROM payment_attempts WHERE intent_id = $1 AND status IN ('created', 'pending', 'authorized', 'unknown', 'captured'))",
            )
            .bind(&intent_id)
            .execute(&mut *conn)
            .await?;
            tenant::emit(conn, tenant_id, "payment.failed", json!({ "intent_id": intent_id, "attempt_id": attempt_id, "reason": inc.failure_reason })).await?;
        }
        Attempt::Unknown => {
            tenant::emit(conn, tenant_id, "payment.unresolved", payload).await?;
        }
        Attempt::Pending | Attempt::Created => {}
    }
    Ok(Applied::Applied(inc.status))
}

async fn on_captured(
    conn: &mut PgConnection,
    tenant_id: &str,
    attempt_id: &str,
    intent_id: &str,
    provider: &str,
    amount: i64,
    currency: Currency,
) -> Result<(), ApiError> {
    // Accounting record only: the provider holds the funds until it settles.
    Transaction::new(vec![
        Entry::debit(accounts::provider_receivable(provider), amount, currency),
        Entry::credit(accounts::COLLECTIONS, amount, currency),
    ])
    .map_err(zyvor_ledger::StoreError::from)?
    .post(conn, tenant_id, &format!("capture:{attempt_id}"), None)
    .await?;
    let (invoice_id, external_ref): (Option<String>, Option<String>) = sqlx::query_as("UPDATE payment_intents SET status = 'succeeded', updated_at = now() WHERE id = $1 RETURNING invoice_id, external_ref")
        .bind(intent_id)
        .fetch_one(&mut *conn)
        .await?;
    if let Some(inv) = &invoice_id {
        sqlx::query("INSERT INTO invoice_allocations (tenant_id, invoice_id, intent_id, amount_minor, reference) VALUES ($1, $2, $3, $4, $5)")
            .bind(tenant_id)
            .bind(inv)
            .bind(intent_id)
            .bind(amount)
            .bind(format!("capture:{attempt_id}"))
            .execute(&mut *conn)
            .await?;
        refresh_invoice_status(conn, inv).await?;
    }
    tenant::emit(conn, tenant_id, "payment.captured", json!({ "intent_id": intent_id, "attempt_id": attempt_id, "provider": provider, "amount_minor": amount, "currency": currency, "invoice_id": invoice_id, "external_ref": external_ref })).await?;
    Ok(())
}

pub async fn refresh_invoice_status(
    conn: &mut PgConnection,
    invoice_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE invoices SET status = CASE
            WHEN COALESCE((SELECT SUM(amount_minor) FROM invoice_allocations WHERE invoice_id = $1), 0) >= amount_minor THEN 'paid'
            WHEN COALESCE((SELECT SUM(amount_minor) FROM invoice_allocations WHERE invoice_id = $1), 0) > 0 THEN 'partially_paid'
            ELSE 'open' END
         WHERE id = $1 AND status <> 'void'",
    )
    .bind(invoice_id)
    .execute(conn)
    .await?;
    Ok(())
}

impl IntoResponse for Applied {
    fn into_response(self) -> Response {
        Json(json!({ "result": format!("{self:?}") })).into_response()
    }
}
