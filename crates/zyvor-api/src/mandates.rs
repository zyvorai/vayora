//! UPI AutoPay mandates: the payer approves once ("this merchant may debit up to ₹X, this often, until this
//! date"), the merchant then schedules debits, and each debit is announced in advance.
//!
//! A debit is an ordinary payment attempt, so the ledger, the `payment.*` events, invoice paying and the
//! resolution of unknown outcomes all work as they do for any payment. This module holds the agreement and
//! the schedule. What is guaranteed here:
//! - a mandate is never debited twice for the same day (`UNIQUE (mandate_id, due_date)`), and two debits
//!   never fall in the same period of its frequency;
//! - no debit above the mandate's cap, in the wrong currency, or outside its dates;
//! - no debit unless the payer was told at least the notice window ahead (`ZYVOR_MANDATE_NOTICE_HOURS`);
//! - a paused, revoked or expired mandate never debits;
//! - the provider is called outside any transaction, after the attempt is saved; a lost answer becomes
//!   `unknown` and is resolved by asking, never by sending again.
//!
//! Zyvor records and emits the pre-debit notice (`mandate.debit_notified`); it does not message the payer.
//! The merchant or the provider must.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Days, FixedOffset, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};
use zyvor_providers::{
    CreateMandate, DebitRequest, Frequency, MandateState, MandateStatus, Provider, ProviderError,
};

use crate::{
    ApiError, AppState,
    auth::Auth,
    cases::open_case,
    idempotency::{self, Begin, fingerprint, replay_response},
    payments::{Incoming, apply_provider_state, incoming},
    registry,
    state_machine::Attempt,
    tenant,
};

macro_rules! mandate_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, customer_id, currency, max_amount_minor, frequency, start_date, end_date, status, provider, provider_mandate_ref, auth_request, failure_reason, revoked_by, external_ref, created_at, updated_at FROM mandates ",
            $tail
        )
    };
}
macro_rules! debit_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, mandate_id, amount_minor, currency, due_date, notified_at, invoice_id, state, attempt_id, retry_no, failure_reason, created_at FROM mandate_debits ",
            $tail
        )
    };
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct MandateRow {
    pub id: String,
    pub customer_id: String,
    pub currency: String,
    pub max_amount_minor: i64,
    pub frequency: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub status: String,
    pub provider: String,
    pub provider_mandate_ref: Option<String>,
    #[sqlx(rename = "auth_request")]
    pub authorization: Option<Value>,
    pub failure_reason: Option<String>,
    pub revoked_by: Option<String>,
    pub external_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct DebitRow {
    pub id: String,
    pub mandate_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub due_date: NaiveDate,
    pub notified_at: Option<DateTime<Utc>>,
    pub invoice_id: Option<String>,
    pub state: String,
    pub attempt_id: Option<String>,
    pub retry_no: i32,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn ist() -> FixedOffset {
    FixedOffset::east_opt(19_800).expect("IST")
}

/// The first due date a debit scheduled right now may have, so the payer is told at least `notice_hours`
/// ahead of it. A date, not a moment: due dates start at midnight India time.
pub fn earliest_due(notice_hours: i64) -> NaiveDate {
    if notice_hours <= 0 {
        return crate::ist_today();
    }
    (Utc::now() + chrono::Duration::hours(notice_hours))
        .with_timezone(&ist())
        .date_naive()
        .checked_add_days(Days::new(1))
        .expect("date in range")
}

/// Would two debits on these dates fall in the same period of the frequency?
pub fn same_period(f: Frequency, a: NaiveDate, b: NaiveDate) -> bool {
    let gap = (a - b).num_days().abs();
    match f {
        Frequency::AsPresented | Frequency::Daily => a == b,
        Frequency::Weekly => gap < 7,
        Frequency::Fortnightly => gap < 14,
        _ => f.period_end(a) == f.period_end(b),
    }
}

async fn mandate_row(conn: &mut PgConnection, id: &str) -> Result<MandateRow, ApiError> {
    sqlx::query_as::<_, MandateRow>(mandate_select!("WHERE id = $1"))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("mandate"))
}

async fn debit_row(conn: &mut PgConnection, id: &str) -> Result<DebitRow, ApiError> {
    sqlx::query_as::<_, DebitRow>(debit_select!("WHERE id = $1"))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("debit"))
}

async fn pick_provider(
    s: &AppState,
    conn: &mut PgConnection,
    tenant_id: &str,
    env: &str,
    want: Option<&str>,
) -> Result<(String, Arc<dyn Provider>), ApiError> {
    let names: Vec<String> = match want {
        Some(w) => vec![w.to_string()],
        None => sqlx::query_scalar(
            "SELECT provider FROM provider_accounts WHERE tenant_id = $1 AND enabled ORDER BY provider",
        )
        .bind(tenant_id)
        .fetch_all(&mut *conn)
        .await?,
    };
    for n in names {
        let p = registry::load(s, conn, tenant_id, env, &n).await?;
        if p.capabilities().mandates {
            return Ok((n, p));
        }
    }
    Err(ApiError::invalid(
        "no_route",
        "no connected provider supports UPI AutoPay mandates",
    ))
}

// ---------- create, read ----------

#[derive(Deserialize)]
pub struct NewMandate {
    customer_id: String,
    /// The most any single debit may take, in minor units (paise).
    max_amount_minor: i64,
    frequency: String,
    /// Defaults to today (India time).
    start_date: Option<NaiveDate>,
    end_date: NaiveDate,
    /// `INR` only: UPI.
    currency: Option<String>,
    provider: Option<String>,
    external_ref: Option<String>,
}

pub async fn create_mandate(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Json(b): Json<NewMandate>,
) -> Result<Response, ApiError> {
    auth.require("mandates:write")?;
    let currency: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|_| ApiError::invalid("invalid_currency", "unknown currency"))?;
    if currency != Currency::Inr {
        return Err(ApiError::invalid(
            "unsupported_currency",
            "UPI AutoPay mandates are in INR",
        ));
    }
    if b.max_amount_minor <= 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "max_amount_minor must be positive",
        ));
    }
    if b.max_amount_minor > s.mandate_max_minor {
        return Err(ApiError::invalid(
            "exceeds_mandate_ceiling",
            format!(
                "this server allows a mandate limit of at most {} minor units (ZYVOR_MANDATE_MAX_MINOR)",
                s.mandate_max_minor
            ),
        ));
    }
    let frequency: Frequency = b
        .frequency
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_frequency", e))?;
    let today = crate::ist_today();
    let start = b.start_date.unwrap_or(today);
    if start < today {
        return Err(ApiError::invalid(
            "invalid_dates",
            "start_date cannot be in the past",
        ));
    }
    if b.end_date <= start || b.end_date > start + Days::new(3650) {
        return Err(ApiError::invalid(
            "invalid_dates",
            "end_date must be after start_date, within ten years",
        ));
    }

    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let hash = fingerprint(&[
        &b.customer_id,
        &b.max_amount_minor.to_string(),
        frequency.as_str(),
        &start.to_string(),
        &b.end_date.to_string(),
        b.provider.as_deref().unwrap_or(""),
        b.external_ref.as_deref().unwrap_or(""),
    ]);
    let key =
        match idempotency::begin(&mut tx, &auth.tenant_id, "mandates", &headers, &hash).await? {
            Begin::Replay {
                status,
                body,
                resource_id,
            } => {
                let body = match resource_id {
                    Some(id) => serde_json::to_value(mandate_row(&mut tx, &id).await?).unwrap(),
                    None => body.unwrap_or(Value::Null),
                };
                return Ok(replay_response(status, body));
            }
            Begin::Proceed(k) => k,
        };
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
        .bind(&b.customer_id)
        .fetch_one(&mut *tx)
        .await?
    {
        return Err(ApiError::not_found("customer"));
    }
    let (provider_name, provider) = pick_provider(
        &s,
        &mut tx,
        &auth.tenant_id,
        &auth.env,
        b.provider.as_deref(),
    )
    .await?;
    let id = new_id("mnd");
    sqlx::query("INSERT INTO mandates (id, tenant_id, customer_id, currency, max_amount_minor, frequency, start_date, end_date, provider, external_ref, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.customer_id).bind(currency.code()).bind(b.max_amount_minor)
        .bind(frequency.as_str()).bind(start).bind(b.end_date).bind(&provider_name).bind(&b.external_ref).bind(&auth.actor)
        .execute(&mut *tx).await?;
    key.set_resource(&mut tx, &id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "mandate.created",
        Some(&id),
        json!({ "provider": provider_name, "max_amount_minor": b.max_amount_minor, "frequency": frequency.as_str() }),
    )
    .await?;
    let phone: Option<String> = sqlx::query_scalar("SELECT phone FROM customers WHERE id = $1")
        .bind(&b.customer_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;

    // ---- provider call, outside any DB transaction ----
    let result = provider
        .create_mandate(&CreateMandate {
            mandate_id: id.clone(),
            max_amount_minor: b.max_amount_minor,
            currency,
            frequency,
            start_date: start,
            end_date: b.end_date,
            customer_id: Some(b.customer_id.clone()),
            customer_phone: phone,
        })
        .await;

    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    match result {
        Ok(st) => apply_mandate_state(&mut tx, &auth.tenant_id, &id, &st, "api").await?,
        Err(ProviderError::Uncertain(why)) => {
            // It may exist at the provider. It stays pending; refresh resolves it by asking.
            tracing::warn!(mandate = %id, %why, "mandate creation outcome unknown");
            sqlx::query(
                "UPDATE mandates SET failure_reason = $2, updated_at = now() WHERE id = $1",
            )
            .bind(&id)
            .bind(format!(
                "the provider did not confirm ({why}); refresh to resolve"
            ))
            .execute(&mut *tx)
            .await?;
        }
        Err(ProviderError::Rejected(why) | ProviderError::Unsupported(why)) => {
            sqlx::query("UPDATE mandates SET status = 'failed', failure_reason = $2, updated_at = now() WHERE id = $1")
                .bind(&id).bind(&why).execute(&mut *tx).await?;
            tenant::emit(
                &mut tx,
                &auth.tenant_id,
                "mandate.failed",
                json!({ "id": id, "reason": why }),
            )
            .await?;
        }
    }
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "mandate.created",
        json!({ "id": id, "customer_id": b.customer_id, "max_amount_minor": b.max_amount_minor, "frequency": frequency.as_str() }),
    )
    .await?;
    let row = mandate_row(&mut tx, &id).await?;
    let res = key.finish(&mut tx, StatusCode::CREATED, &row).await?;
    tx.commit().await?;
    Ok(res)
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    customer_id: Option<String>,
}

pub async fn list_mandates(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("mandates:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<MandateRow> = sqlx::query_as(mandate_select!(
        "WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR customer_id = $2) ORDER BY created_at DESC LIMIT 100"
    ))
    .bind(&q.status)
    .bind(&q.customer_id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

pub async fn get_mandate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("mandates:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let m = mandate_row(&mut tx, &id).await?;
    let debits: Vec<DebitRow> = sqlx::query_as(debit_select!(
        "WHERE mandate_id = $1 ORDER BY due_date DESC LIMIT 50"
    ))
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "mandate": m, "debits": debits })))
}

// ---------- lifecycle ----------

/// Move our record to what the provider reports. Provider `Active` only ever activates a pending mandate (a
/// merchant's pause is ours, and a provider lookup must not undo it); a revoke, expiry or failure always wins.
async fn apply_mandate_state(
    conn: &mut PgConnection,
    tenant_id: &str,
    id: &str,
    st: &MandateState,
    source: &str,
) -> Result<(), ApiError> {
    let m = mandate_row(conn, id).await?;
    if let Some(r) = &st.provider_mandate_ref
        && m.provider_mandate_ref.as_deref() != Some(r)
    {
        sqlx::query("UPDATE mandates SET provider_mandate_ref = $2 WHERE id = $1")
            .bind(id)
            .bind(r)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(a) = &st.authorization
        && m.authorization.as_ref() != Some(a)
    {
        sqlx::query("UPDATE mandates SET auth_request = $2 WHERE id = $1")
            .bind(id)
            .bind(a)
            .execute(&mut *conn)
            .await?;
    }
    let terminal = matches!(m.status.as_str(), "revoked" | "expired" | "failed");
    let next: Option<(&str, &str)> = match (&st.status, m.status.as_str()) {
        _ if terminal => None,
        (MandateStatus::Active, "pending_authorization") => Some(("active", "mandate.activated")),
        (MandateStatus::Failed, _) => Some(("failed", "mandate.failed")),
        (MandateStatus::Revoked, _) => Some(("revoked", "mandate.revoked")),
        (MandateStatus::Expired, _) => Some(("expired", "mandate.expired")),
        (MandateStatus::Paused, "active") => Some(("paused", "mandate.paused")),
        _ => None,
    };
    if let Some((status, event)) = next {
        sqlx::query("UPDATE mandates SET status = $2, failure_reason = COALESCE($3, failure_reason), revoked_by = CASE WHEN $2 = 'revoked' THEN COALESCE(revoked_by, 'payer') ELSE revoked_by END, updated_at = now() WHERE id = $1")
            .bind(id).bind(status).bind(&st.failure_reason).execute(&mut *conn).await?;
        if matches!(status, "revoked" | "expired" | "failed") {
            skip_unstarted_debits(conn, id, status).await?;
        }
        tenant::audit(
            conn,
            tenant_id,
            source,
            event,
            Some(id),
            json!({ "status": status }),
        )
        .await?;
        tenant::emit(
            conn,
            tenant_id,
            event,
            json!({ "id": id, "customer_id": m.customer_id, "reason": st.failure_reason }),
        )
        .await?;
    }
    Ok(())
}

/// Debits that have not been sent to the provider are cancelled with their mandate. One already in flight is
/// left to the provider to decide, and the ledger follows its answer.
async fn skip_unstarted_debits(
    conn: &mut PgConnection,
    mandate_id: &str,
    why: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mandate_debits SET state = 'skipped', failure_reason = $2, updated_at = now() WHERE mandate_id = $1 AND state IN ('scheduled', 'notified')")
        .bind(mandate_id)
        .bind(format!("mandate {why}"))
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn refresh_mandate_internal(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    id: &str,
    source: &str,
) -> Result<MandateRow, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let m = mandate_row(&mut tx, id).await?;
    if matches!(m.status.as_str(), "revoked" | "expired" | "failed") {
        return Ok(m);
    }
    let provider = registry::load(s, &mut tx, tenant_id, env, &m.provider).await?;
    tx.commit().await?;
    let st = provider.fetch_mandate(id).await.map_err(|e| match e {
        ProviderError::Uncertain(w) => {
            ApiError::new(StatusCode::BAD_GATEWAY, "provider_unavailable", w)
        }
        ProviderError::Rejected(w) | ProviderError::Unsupported(w) => {
            ApiError::conflict("provider_rejected", w)
        }
    })?;
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    apply_mandate_state(&mut tx, tenant_id, id, &st, source).await?;
    let row = mandate_row(&mut tx, id).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn refresh_mandate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<MandateRow>, ApiError> {
    auth.require("mandates:write")?;
    Ok(Json(
        refresh_mandate_internal(&s, &auth.tenant_id, &auth.env, &id, "api").await?,
    ))
}

async fn set_status(
    s: &AppState,
    auth: &Auth,
    id: &str,
    from: &[&str],
    to: &str,
    event: &str,
) -> Result<Json<MandateRow>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let m = mandate_row(&mut tx, id).await?;
    if !from.contains(&m.status.as_str()) {
        return Err(ApiError::conflict(
            "mandate_state",
            format!("a {} mandate cannot be made {to}", m.status),
        ));
    }
    sqlx::query("UPDATE mandates SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(to)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        event,
        Some(id),
        json!({ "from": m.status }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        event,
        json!({ "id": id, "customer_id": m.customer_id }),
    )
    .await?;
    let row = mandate_row(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(row))
}

/// A merchant's hold: no debit goes out while paused. The provider is not told; the payer's own mandate is unchanged.
pub async fn pause_mandate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<MandateRow>, ApiError> {
    auth.require("mandates:write")?;
    set_status(&s, &auth, &id, &["active"], "paused", "mandate.paused").await
}

pub async fn resume_mandate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<MandateRow>, ApiError> {
    auth.require("mandates:write")?;
    set_status(&s, &auth, &id, &["paused"], "active", "mandate.resumed").await
}

/// The merchant ends the mandate. Our record stops debiting **first**, so the payer's wish is honoured even if
/// the provider is unreachable; then the provider is asked, and a failure there is recorded, not hidden.
pub async fn revoke_mandate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<MandateRow>, ApiError> {
    auth.require("mandates:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let m = mandate_row(&mut tx, &id).await?;
    if matches!(m.status.as_str(), "revoked" | "expired" | "failed") {
        return Err(ApiError::conflict(
            "mandate_state",
            format!("a {} mandate cannot be revoked", m.status),
        ));
    }
    let provider = registry::load(&s, &mut tx, &auth.tenant_id, &auth.env, &m.provider).await?;
    sqlx::query(
        "UPDATE mandates SET status = 'revoked', revoked_by = $2, updated_at = now() WHERE id = $1",
    )
    .bind(&id)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    skip_unstarted_debits(&mut tx, &id, "revoked").await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "mandate.revoked",
        Some(&id),
        json!({ "by": auth.actor }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "mandate.revoked",
        json!({ "id": id, "customer_id": m.customer_id, "revoked_by": auth.actor }),
    )
    .await?;
    tx.commit().await?;

    if m.provider_mandate_ref.is_some()
        && let Err(e) = provider.revoke_mandate(&id).await
    {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        sqlx::query("UPDATE mandates SET failure_reason = $2 WHERE id = $1")
            .bind(&id)
            .bind(format!("the provider did not confirm the revoke: {e}"))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(mandate_row(&mut tx, &id).await?))
}

// ---------- debits ----------

#[derive(Deserialize)]
pub struct NewDebit {
    amount_minor: i64,
    /// The day the debit is taken (India time). It must be far enough ahead for the payer's notice.
    due_date: NaiveDate,
    /// Pay this invoice with it (the customer must be the mandate's customer).
    invoice_id: Option<String>,
}

/// What to debit and when.
pub struct DebitSpec<'a> {
    pub amount_minor: i64,
    pub due_date: NaiveDate,
    pub invoice_id: Option<&'a str>,
}

/// Validate and insert one scheduled debit. Used by the API and by auto-collect, so both obey the same rules.
pub async fn schedule_debit(
    conn: &mut PgConnection,
    tenant_id: &str,
    actor: &str,
    notice_hours: i64,
    m: &MandateRow,
    spec: &DebitSpec<'_>,
) -> Result<DebitRow, ApiError> {
    let DebitSpec {
        amount_minor,
        due_date,
        invoice_id,
    } = *spec;
    if m.status != "active" {
        return Err(ApiError::conflict(
            "mandate_not_active",
            format!("the mandate is {}", m.status),
        ));
    }
    if amount_minor <= 0 || amount_minor > m.max_amount_minor {
        return Err(ApiError::invalid(
            "exceeds_mandate_limit",
            format!(
                "a debit must be between 1 and {} minor units",
                m.max_amount_minor
            ),
        ));
    }
    if due_date < m.start_date || due_date > m.end_date {
        return Err(ApiError::invalid(
            "outside_mandate_dates",
            format!(
                "due_date must be within {} and {}",
                m.start_date, m.end_date
            ),
        ));
    }
    let earliest = earliest_due(notice_hours);
    if due_date < earliest {
        return Err(ApiError::invalid(
            "notice_too_short",
            format!(
                "the payer must be told {notice_hours} hours ahead: the earliest due date is {earliest}"
            ),
        ));
    }
    let freq: Frequency = m.frequency.parse().map_err(|_| ApiError::internal())?;
    let taken: Vec<NaiveDate> = sqlx::query_scalar("SELECT due_date FROM mandate_debits WHERE mandate_id = $1 AND state NOT IN ('skipped', 'failed')")
        .bind(&m.id).fetch_all(&mut *conn).await?;
    if taken.iter().any(|d| same_period(freq, *d, due_date)) {
        return Err(ApiError::conflict(
            "period_already_debited",
            format!(
                "this {} mandate already has a debit in that period",
                m.frequency
            ),
        ));
    }
    if let Some(inv) = invoice_id {
        let row: Option<(Option<String>, String, String)> =
            sqlx::query_as("SELECT customer_id, currency, status FROM invoices WHERE id = $1")
                .bind(inv)
                .fetch_optional(&mut *conn)
                .await?;
        let (cust, ccy, status) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
        if cust.as_deref() != Some(m.customer_id.as_str()) || ccy != m.currency {
            return Err(ApiError::invalid(
                "invoice_mismatch",
                "the invoice must belong to the mandate's customer and be in its currency",
            ));
        }
        if !matches!(status.as_str(), "open" | "partially_paid") {
            return Err(ApiError::conflict(
                "invoice_not_payable",
                format!("the invoice is {status}"),
            ));
        }
    }
    let id = new_id("mdb");
    let row = sqlx::query_as::<_, DebitRow>(concat!(
        "INSERT INTO mandate_debits (id, tenant_id, mandate_id, amount_minor, currency, due_date, invoice_id, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ",
        "ON CONFLICT (mandate_id, due_date) DO NOTHING ",
        "RETURNING id, mandate_id, amount_minor, currency, due_date, notified_at, invoice_id, state, attempt_id, retry_no, failure_reason, created_at"
    ))
    .bind(&id).bind(tenant_id).bind(&m.id).bind(amount_minor).bind(&m.currency).bind(due_date).bind(invoice_id).bind(actor)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| ApiError::conflict("debit_exists", "that mandate already has a debit on that day"))?;
    tenant::audit(
        conn,
        tenant_id,
        actor,
        "mandate.debit_scheduled",
        Some(&id),
        json!({ "mandate_id": m.id, "amount_minor": amount_minor, "due_date": due_date }),
    )
    .await?;
    tenant::emit(conn, tenant_id, "mandate.debit_scheduled", json!({ "id": id, "mandate_id": m.id, "amount_minor": amount_minor, "currency": m.currency, "due_date": due_date, "invoice_id": invoice_id })).await?;
    Ok(row)
}

pub async fn create_debit(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<NewDebit>,
) -> Result<Response, ApiError> {
    auth.require("mandates:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let hash = fingerprint(&[
        &id,
        &b.amount_minor.to_string(),
        &b.due_date.to_string(),
        b.invoice_id.as_deref().unwrap_or(""),
    ]);
    let key = match idempotency::begin(&mut tx, &auth.tenant_id, "mandate_debits", &headers, &hash)
        .await?
    {
        Begin::Replay {
            status,
            body,
            resource_id,
        } => {
            let body = match resource_id {
                Some(d) => serde_json::to_value(debit_row(&mut tx, &d).await?).unwrap(),
                None => body.unwrap_or(Value::Null),
            };
            return Ok(replay_response(status, body));
        }
        Begin::Proceed(k) => k,
    };
    let m = mandate_row(&mut tx, &id).await?;
    let row = schedule_debit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        s.mandate_notice_hours,
        &m,
        &DebitSpec {
            amount_minor: b.amount_minor,
            due_date: b.due_date,
            invoice_id: b.invoice_id.as_deref(),
        },
    )
    .await?;
    key.set_resource(&mut tx, &row.id).await?;
    let res = key.finish(&mut tx, StatusCode::CREATED, &row).await?;
    tx.commit().await?;
    Ok(res)
}

pub async fn list_debits(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("mandates:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    mandate_row(&mut tx, &id).await?;
    let rows: Vec<DebitRow> = sqlx::query_as(debit_select!(
        "WHERE mandate_id = $1 ORDER BY due_date DESC LIMIT 100"
    ))
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

// ---------- link a subscription (auto-collect) ----------

#[derive(Deserialize)]
pub struct LinkMandate {
    /// `null` unlinks.
    mandate_id: Option<String>,
}

pub async fn link_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path(sub_id): Path<String>,
    Json(b): Json<LinkMandate>,
) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    auth.require("mandates:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cust: String = sqlx::query_scalar("SELECT customer_id FROM subscriptions WHERE id = $1")
        .bind(&sub_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("subscription"))?;
    if let Some(mid) = &b.mandate_id {
        let m = mandate_row(&mut tx, mid).await?;
        if m.customer_id != cust {
            return Err(ApiError::invalid(
                "customer_mismatch",
                "the mandate belongs to a different customer",
            ));
        }
        if !matches!(
            m.status.as_str(),
            "pending_authorization" | "active" | "paused"
        ) {
            return Err(ApiError::conflict(
                "mandate_state",
                format!("a {} mandate cannot collect invoices", m.status),
            ));
        }
    }
    sqlx::query("UPDATE subscriptions SET mandate_id = $2 WHERE id = $1")
        .bind(&sub_id)
        .bind(&b.mandate_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "subscription.mandate_linked",
        Some(&sub_id),
        json!({ "mandate_id": b.mandate_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": sub_id, "mandate_id": b.mandate_id })))
}

/// Called by billing right after it raises an invoice: if the subscription has an active mandate and the
/// invoice fits it, schedule the debit. Never fails billing; a refusal is logged and the invoice stays open
/// for the usual reminders.
pub async fn auto_collect(
    conn: &mut PgConnection,
    tenant_id: &str,
    notice_hours: i64,
    sub_id: &str,
    invoice_id: &str,
    amount_minor: i64,
    invoice_due: Option<NaiveDate>,
) {
    let run = async {
        let mid: Option<String> =
            sqlx::query_scalar("SELECT mandate_id FROM subscriptions WHERE id = $1")
                .bind(sub_id)
                .fetch_one(&mut *conn)
                .await?;
        let Some(mid) = mid else {
            return Ok::<(), ApiError>(());
        };
        let m = mandate_row(conn, &mid).await?;
        if m.status != "active" {
            return Ok(());
        }
        // take it on the invoice's due date, or as soon as the notice allows if that is sooner
        let due = invoice_due
            .unwrap_or_else(crate::ist_today)
            .max(earliest_due(notice_hours));
        schedule_debit(
            conn,
            tenant_id,
            "auto-collect",
            notice_hours,
            &m,
            &DebitSpec {
                amount_minor,
                due_date: due,
                invoice_id: Some(invoice_id),
            },
        )
        .await?;
        Ok(())
    };
    if let Err(e) = run.await {
        tracing::warn!(sub = %sub_id, invoice = %invoice_id, error = ?e.message, "auto-collect did not schedule a debit");
    }
}

// ---------- the worker ----------

#[derive(Default, Debug, Serialize)]
pub struct Stats {
    pub expired: usize,
    pub notified: usize,
    pub initiated: usize,
}

/// Run one pass for the caller's tenant now, instead of waiting for the worker (a sandbox and test aid, like
/// `POST /v1/billing/run`). The in-process simulator only works when the API runs the pass itself.
pub async fn run_now(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("mandates:write")?;
    let st = run_cycle(&s, &auth.tenant_id, &auth.env).await?;
    Ok(Json(json!(st)))
}

/// One pass for one tenant: expire, notify, send the debits that are due, and settle the ones in flight.
pub async fn run_cycle(s: &AppState, tenant_id: &str, env: &str) -> Result<Stats, ApiError> {
    let mut st = Stats::default();
    let today = crate::ist_today();

    // 1. mandates past their end date
    {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM mandates WHERE status IN ('active', 'paused', 'pending_authorization') AND end_date < $1 FOR UPDATE SKIP LOCKED")
            .bind(today).fetch_all(&mut *tx).await?;
        for id in &ids {
            sqlx::query("UPDATE mandates SET status = 'expired', updated_at = now() WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            skip_unstarted_debits(&mut tx, id, "expired").await?;
            tenant::emit(&mut tx, tenant_id, "mandate.expired", json!({ "id": id })).await?;
        }
        st.expired = ids.len();
        tx.commit().await?;
    }

    // 2. tell the payer (emit the pre-debit notice) for every scheduled debit
    {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let due: Vec<DebitRow> = sqlx::query_as(debit_select!("WHERE state = 'scheduled' AND notify_at <= now() ORDER BY due_date FOR UPDATE SKIP LOCKED LIMIT 200"))
            .fetch_all(&mut *tx).await?;
        for d in &due {
            sqlx::query("UPDATE mandate_debits SET state = 'notified', notified_at = now(), updated_at = now() WHERE id = $1").bind(&d.id).execute(&mut *tx).await?;
            tenant::emit(&mut tx, tenant_id, "mandate.debit_notified", json!({ "id": d.id, "mandate_id": d.mandate_id, "amount_minor": d.amount_minor, "currency": d.currency, "due_date": d.due_date, "invoice_id": d.invoice_id })).await?;
        }
        st.notified = due.len();
        tx.commit().await?;
    }

    // 3. send the debits that are due, on an active mandate, whose notice was given long enough ago
    let ready: Vec<String> = {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        sqlx::query_scalar("SELECT d.id FROM mandate_debits d JOIN mandates m ON m.id = d.mandate_id WHERE d.state = 'notified' AND d.due_date <= $1 AND m.status = 'active' AND d.notified_at <= now() - make_interval(hours => $2::int) ORDER BY d.due_date LIMIT 50")
            .bind(today).bind(s.mandate_notice_hours as i32).fetch_all(&mut *tx).await?
    };
    for id in ready {
        match execute_debit(s, tenant_id, env, &id).await {
            Ok(true) => st.initiated += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(debit = %id, error = ?e.message, "debit could not be sent"),
        }
    }

    // 4. settle the debits in flight from their payment attempts (which the existing resolver keeps current)
    settle_in_flight(s, tenant_id, env).await?;
    Ok(st)
}

/// Claim one notified debit, save its payment attempt, call the provider outside the transaction, record
/// whatever happened. `Ok(false)` when it was not ours to send (already claimed, mandate no longer active).
async fn execute_debit(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    debit_id: &str,
) -> Result<bool, ApiError> {
    // ---- tx1: claim and persist before calling the provider ----
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let claim: Option<(String, i64, String, Option<String>, i32, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT d.mandate_id, d.amount_minor, d.currency, d.invoice_id, d.retry_no, m.customer_id, m.provider, m.provider_mandate_ref, m.status FROM mandate_debits d JOIN mandates m ON m.id = d.mandate_id WHERE d.id = $1 AND d.state = 'notified' FOR UPDATE OF d",
    )
    .bind(debit_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        mandate_id,
        amount,
        ccy,
        invoice_id,
        retry_no,
        customer_id,
        provider_name,
        pref,
        mstatus,
    )) = claim
    else {
        return Ok(false);
    };
    let (Some(pref), "active") = (pref, mstatus.as_str()) else {
        return Ok(false);
    };
    let currency: Currency = ccy.parse().map_err(|_| ApiError::internal())?;
    let provider = registry::load(s, &mut tx, tenant_id, env, &provider_name).await?;

    let intent_id = new_id("pi");
    let secret = format!(
        "pis_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, customer_id, invoice_id, amount_minor, currency, status, capture_method, external_ref, description, metadata, client_secret_hash) VALUES ($1,$2,$3,$4,$5,$6,'processing','automatic',$7,$8,$9,$10)")
        .bind(&intent_id).bind(tenant_id).bind(&customer_id).bind(&invoice_id).bind(amount).bind(currency.code())
        .bind(format!("mandate-debit:{debit_id}:{retry_no}")).bind("UPI AutoPay debit")
        .bind(json!({ "mandate_id": mandate_id, "mandate_debit_id": debit_id }))
        .bind(crate::auth::hash_key(&secret))
        .execute(&mut *tx).await?;
    let attempt_id = new_id("att");
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, mandate_id, mandate_debit_id) VALUES ($1,$2,$3,$4,'upi','created',$5,$6,$7,$8,$9)")
        .bind(&attempt_id).bind(tenant_id).bind(&intent_id).bind(&provider_name).bind(amount).bind(currency.code())
        .bind(json!({ "provider": provider_name, "reasons": ["UPI AutoPay mandate debit"], "mandate_id": mandate_id }))
        .bind(&mandate_id).bind(debit_id)
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE mandate_debits SET state = 'initiating', attempt_id = $2, updated_at = now() WHERE id = $1")
        .bind(debit_id).bind(&attempt_id).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        tenant_id,
        "worker",
        "mandate.debit_initiated",
        Some(debit_id),
        json!({ "attempt_id": attempt_id, "amount_minor": amount }),
    )
    .await?;
    tx.commit().await?;

    // ---- provider call, outside any DB transaction ----
    let result = provider
        .debit(&DebitRequest {
            attempt_id: attempt_id.clone(),
            mandate_id: mandate_id.clone(),
            provider_mandate_ref: pref,
            amount_minor: amount,
            currency,
        })
        .await;

    // ---- tx2: record whatever happened, including "we don't know" ----
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let mut mandate_gone = false;
    match result {
        Ok(init) => {
            if let Some(r) = init.state.provider_order_ref.clone() {
                sqlx::query("UPDATE payment_attempts SET provider_order_ref = $2 WHERE id = $1")
                    .bind(&attempt_id)
                    .bind(r)
                    .execute(&mut *tx)
                    .await?;
            }
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &incoming(&init.state),
                "worker",
            )
            .await?;
        }
        Err(ProviderError::Uncertain(why)) => {
            tracing::warn!(attempt = %attempt_id, %why, "debit outcome unknown");
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &Incoming {
                    status: Attempt::Unknown,
                    failure_reason: Some(why.clone()),
                    ..Incoming::default()
                },
                "worker",
            )
            .await?;
            open_case(&mut tx, tenant_id, "unresolved_payment", "payment_attempt", &attempt_id, None, None, &format!("The provider did not confirm a mandate debit ({why}). No further debit is sent for this mandate day until it resolves."), json!({ "mandate_id": mandate_id, "debit_id": debit_id })).await?;
        }
        Err(ProviderError::Rejected(why) | ProviderError::Unsupported(why)) => {
            mandate_gone = why.contains("mandate");
            apply_provider_state(
                &mut tx,
                tenant_id,
                &attempt_id,
                &Incoming {
                    status: Attempt::Failed,
                    failure_reason: Some(why),
                    ..Incoming::default()
                },
                "worker",
            )
            .await?;
        }
    }
    tx.commit().await?;
    sync_debit(s, tenant_id, debit_id).await?;
    if mandate_gone {
        // the provider says the mandate is not usable: find out what happened to it
        let _ = refresh_mandate_internal(s, tenant_id, env, &mandate_id, "worker").await;
    }
    Ok(true)
}

/// Bring one debit's state in line with its payment attempt.
async fn sync_debit(s: &AppState, tenant_id: &str, debit_id: &str) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT d.state, a.status, a.failure_reason FROM mandate_debits d JOIN payment_attempts a ON a.id = d.attempt_id WHERE d.id = $1 AND d.state IN ('initiating', 'initiated') FOR UPDATE OF d",
    )
    .bind(debit_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((state, attempt, reason)) = row else {
        return Ok(());
    };
    match attempt.as_str() {
        "captured" => {
            sqlx::query(
                "UPDATE mandate_debits SET state = 'captured', updated_at = now() WHERE id = $1",
            )
            .bind(debit_id)
            .execute(&mut *tx)
            .await?;
        }
        "failed" => {
            failed(
                &mut tx,
                tenant_id,
                debit_id,
                reason.as_deref().unwrap_or("declined"),
            )
            .await?
        }
        _ if state == "initiating" => {
            sqlx::query(
                "UPDATE mandate_debits SET state = 'initiated', updated_at = now() WHERE id = $1",
            )
            .bind(debit_id)
            .execute(&mut *tx)
            .await?;
        }
        _ => {}
    }
    tx.commit().await?;
    Ok(())
}

/// A debit failed: try once more the next day (the notice already given covers it), then give up and say so.
async fn failed(
    conn: &mut PgConnection,
    tenant_id: &str,
    debit_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    let d = debit_row(conn, debit_id).await?;
    let m = mandate_row(conn, &d.mandate_id).await?;
    let next = d.due_date.checked_add_days(Days::new(1));
    let can_retry = d.retry_no == 0
        && m.status == "active"
        && next.is_some_and(|n| n <= m.end_date)
        && !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM mandate_debits WHERE mandate_id = $1 AND due_date = $2)",
        )
        .bind(&d.mandate_id)
        .bind(next)
        .fetch_one(&mut *conn)
        .await?;
    if can_retry {
        sqlx::query("UPDATE mandate_debits SET state = 'notified', due_date = $2, retry_no = 1, attempt_id = NULL, failure_reason = $3, updated_at = now() WHERE id = $1")
            .bind(debit_id).bind(next).bind(reason).execute(&mut *conn).await?;
        return Ok(());
    }
    sqlx::query("UPDATE mandate_debits SET state = 'failed', failure_reason = $2, updated_at = now() WHERE id = $1")
        .bind(debit_id).bind(reason).execute(&mut *conn).await?;
    tenant::emit(conn, tenant_id, "mandate.debit_failed", json!({ "id": debit_id, "mandate_id": d.mandate_id, "amount_minor": d.amount_minor, "invoice_id": d.invoice_id, "reason": reason })).await?;
    Ok(())
}

async fn settle_in_flight(s: &AppState, tenant_id: &str, _env: &str) -> Result<(), ApiError> {
    let ids: Vec<String> = {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        sqlx::query_scalar("SELECT id FROM mandate_debits WHERE state IN ('initiating', 'initiated') ORDER BY updated_at LIMIT 100")
            .fetch_all(&mut *tx)
            .await?
    };
    for id in ids {
        sync_debit(s, tenant_id, &id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }

    #[test]
    fn two_debits_in_one_period_conflict_and_across_periods_do_not() {
        assert!(same_period(
            Frequency::Monthly,
            d(2026, 11, 3),
            d(2026, 11, 28)
        ));
        assert!(!same_period(
            Frequency::Monthly,
            d(2026, 11, 30),
            d(2026, 12, 1)
        ));
        assert!(same_period(
            Frequency::Weekly,
            d(2026, 11, 3),
            d(2026, 11, 9)
        ));
        assert!(!same_period(
            Frequency::Weekly,
            d(2026, 11, 3),
            d(2026, 11, 10)
        ));
        assert!(same_period(
            Frequency::Daily,
            d(2026, 11, 3),
            d(2026, 11, 3)
        ));
        assert!(!same_period(
            Frequency::Daily,
            d(2026, 11, 3),
            d(2026, 11, 4)
        ));
        assert!(same_period(
            Frequency::Quarterly,
            d(2026, 10, 1),
            d(2026, 12, 31)
        ));
        assert!(!same_period(
            Frequency::Quarterly,
            d(2026, 12, 31),
            d(2027, 1, 1)
        ));
        assert!(!same_period(
            Frequency::AsPresented,
            d(2026, 11, 3),
            d(2026, 11, 4)
        ));
    }

    #[test]
    fn the_notice_window_pushes_the_earliest_due_date_forward() {
        let today = crate::ist_today();
        assert_eq!(earliest_due(0), today);
        // with a 24 hour notice a debit cannot be due today or tomorrow: it is at least the day after
        assert!(earliest_due(24) >= today.checked_add_days(Days::new(2)).unwrap());
        assert!(earliest_due(24) <= today.checked_add_days(Days::new(3)).unwrap());
        assert!(earliest_due(48) > earliest_due(24));
    }
}
