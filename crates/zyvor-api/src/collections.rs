//! Collections workspace: ageing of unpaid invoices, promises to pay and reminder history.
//! Zyvor records and reports; the merchant's own channel (email/SMS/WhatsApp) does the sending,
//! triggered by the `invoice.reminder` event.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

pub(crate) const OUTSTANDING: &str = "(i.amount_minor - invoice_settled(i.id))::BIGINT";

#[derive(Deserialize)]
pub struct AgeingQuery {
    /// Report date; defaults to today. Lets finance reproduce last month-end.
    as_of: Option<NaiveDate>,
}

/// Outstanding amounts by days past due, per currency, plus the invoices in each bucket.
pub async fn ageing(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<AgeingQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let as_of = q.as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        String,
        String,
        String,
        Option<String>,
        Option<NaiveDate>,
        i64,
        i64,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, i.number, i.currency, i.customer_id, i.due_date, {OUTSTANDING} AS outstanding,
                COALESCE(($1::date - i.due_date), 0)::BIGINT AS days_over
         FROM invoices i WHERE i.status IN ('open', 'partially_paid') AND i.created_at::date <= $1
         ORDER BY i.due_date NULLS LAST, i.number"
    )))
    .bind(as_of)
    .fetch_all(&mut *tx)
    .await?;
    let bucket = |days: i64, due: Option<NaiveDate>| match (due, days) {
        (None, _) => "no_due_date",
        (_, d) if d <= 0 => "current",
        (_, 1..=30) => "1_30",
        (_, 31..=60) => "31_60",
        (_, 61..=90) => "61_90",
        _ => "over_90",
    };
    let mut buckets: std::collections::BTreeMap<(String, &str), (i64, Vec<Value>)> =
        Default::default();
    for (id, number, ccy, customer, due, outstanding, days) in rows {
        if outstanding <= 0 {
            continue;
        }
        let e = buckets.entry((ccy.clone(), bucket(days, due))).or_default();
        e.0 += outstanding;
        e.1.push(json!({ "id": id, "number": number, "customer_id": customer, "due_date": due, "outstanding_minor": outstanding, "days_past_due": days.max(0) }));
    }
    // foreign amounts also shown in INR at the latest rate you recorded on or before the report date
    let usd = crate::fx::rate_on(&mut tx, "USD", as_of).await?;
    let mut total_inr: Option<i64> = Some(0);
    let data: Vec<Value> = buckets
        .into_iter()
        .map(|((ccy, b), (total, invs))| {
            let inr = match (ccy.as_str(), usd) {
                ("INR", _) => Some(total),
                ("USD", Some((micro, _))) => Some(crate::fx::usd_to_inr(total, micro)),
                _ => None,
            };
            total_inr = total_inr.zip(inr).map(|(a, b)| a + b);
            json!({ "currency": ccy, "bucket": b, "outstanding_minor": total, "outstanding_inr_minor": inr, "count": invs.len(), "invoices": invs })
        })
        .collect();
    Ok(Json(
        json!({ "as_of": as_of, "data": data, "total_outstanding_inr_minor": total_inr, "usd_rate_date": usd.map(|u| u.1) }),
    ))
}

#[derive(Deserialize)]
pub struct NewPromise {
    promised_minor: i64,
    promised_date: NaiveDate,
    note: Option<String>,
}

pub async fn create_promise(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    Json(b): Json<NewPromise>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.status, {OUTSTANDING} FROM invoices i WHERE i.id = $1 FOR UPDATE"
    )))
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, outstanding) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if !matches!(status.as_str(), "open" | "partially_paid") {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {status}"),
        ));
    }
    if b.promised_minor <= 0 || b.promised_minor > outstanding {
        return Err(ApiError::invalid(
            "invalid_amount",
            format!("promised amount must be between 1 and the outstanding {outstanding}"),
        ));
    }
    let id = new_id("prm");
    sqlx::query("INSERT INTO payment_promises (id, tenant_id, invoice_id, promised_minor, promised_date, note, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(b.promised_minor).bind(b.promised_date).bind(&b.note).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "promise.created",
        Some(&invoice_id),
        json!({ "promise_id": id, "date": b.promised_date }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "invoice_id": invoice_id, "promised_minor": b.promised_minor, "promised_date": b.promised_date, "status": "open" }),
        ),
    ))
}

pub async fn cancel_promise(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE payment_promises SET canceled_at = now() WHERE id = $1 AND canceled_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("promise"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "promise.canceled",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "canceled" })))
}

/// Promises with a derived status: `kept` once the invoice is cleared, `broken` once the date has
/// passed with money still owing, otherwise `open`.
pub async fn list_promises(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<crate::payments::ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i64, NaiveDate, Option<String>, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT p.id, p.invoice_id, i.number, p.promised_minor, p.promised_date, p.note,
                CASE WHEN p.canceled_at IS NOT NULL THEN 'canceled'
                     WHEN {OUTSTANDING} <= 0 THEN 'kept'
                     WHEN p.promised_date < CURRENT_DATE THEN 'broken'
                     ELSE 'open' END
         FROM payment_promises p JOIN invoices i ON i.id = p.invoice_id ORDER BY p.promised_date, p.id LIMIT $1"
    )))
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .filter(|r| q.status.as_deref().is_none_or(|st| st == r.6))
        .map(|(id, inv, number, amt, date, note, status)| json!({ "id": id, "invoice_id": inv, "invoice_number": number, "promised_minor": amt, "promised_date": date, "note": note, "status": status }))
        .collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize)]
pub struct NewReminder {
    channel: String,
    message: Option<String>,
}

/// Log a reminder and emit `invoice.reminder` so the merchant's own messaging sends it.
pub async fn create_reminder(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    Json(b): Json<NewReminder>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.status, i.number, {OUTSTANDING} FROM invoices i WHERE i.id = $1"
    )))
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, number, outstanding) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if !matches!(status.as_str(), "open" | "partially_paid") {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {status}"),
        ));
    }
    let id = new_id("rem");
    sqlx::query("INSERT INTO invoice_reminders (id, tenant_id, invoice_id, channel, message, sent_by) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(&b.channel).bind(&b.message).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_check_violation() => ApiError::invalid("invalid_channel", "channel must be email, sms, whatsapp, call or other"),
            e => e.into(),
        })?;
    tenant::emit(&mut tx, &auth.tenant_id, "invoice.reminder", json!({ "reminder_id": id, "invoice_id": invoice_id, "number": number, "channel": b.channel, "outstanding_minor": outstanding, "message": b.message })).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "reminder.logged",
        Some(&invoice_id),
        json!({ "channel": b.channel }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "invoice_id": invoice_id, "channel": b.channel })),
    ))
}

/// Reminder history for one invoice, newest first.
pub async fn list_reminders(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, Option<String>, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, channel, message, sent_by, created_at FROM invoice_reminders WHERE invoice_id = $1 ORDER BY created_at DESC, id DESC",
    )
    .bind(&invoice_id)
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, channel, message, sent_by, at)| json!({ "id": id, "channel": channel, "message": message, "sent_by": sent_by, "created_at": at }))
        .collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize, Default)]
pub struct DunningBody {
    /// Only invoices at least this many days past due (default 1).
    min_days_overdue: Option<i64>,
    /// Don't nudge the same invoice again within this many days (default 7).
    repeat_after_days: Option<i64>,
    channel: Option<String>,
}

/// Remind every overdue invoice that has no live promise to pay and wasn't reminded recently.
/// Safe to run on a schedule: the repeat window makes it idempotent, and the invoice rows are
/// locked while deciding so two concurrent runs can't both remind.
/// How a dunning run behaves.
#[derive(Clone, Copy)]
pub struct Policy<'a> {
    pub min_days: i64,
    pub repeat_days: i64,
    pub channel: &'a str,
    pub requires_approval: bool,
}

pub async fn run_dunning_tx(
    tx: &mut sqlx::PgConnection,
    tenant_id: &str,
    actor: &str,
    today: NaiveDate,
    policy: &Policy<'_>,
) -> Result<Vec<String>, ApiError> {
    let Policy {
        min_days,
        repeat_days,
        channel,
        requires_approval,
    } = *policy;
    let due: Vec<(String, String, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, i.number, {OUTSTANDING}, ($1::date - i.due_date)::BIGINT
         FROM invoices i
         WHERE i.status IN ('open', 'partially_paid') AND i.due_date IS NOT NULL
           AND ($1::date - i.due_date) >= $2
           AND NOT EXISTS (SELECT 1 FROM payment_promises p WHERE p.invoice_id = i.id
                           AND p.canceled_at IS NULL AND p.promised_date >= $1::date)
           AND NOT EXISTS (SELECT 1 FROM invoice_reminders r WHERE r.invoice_id = i.id
                           AND r.created_at > now() - make_interval(days => $3::int))
           AND NOT EXISTS (SELECT 1 FROM reminder_drafts d WHERE d.invoice_id = i.id AND d.status = 'pending')
         ORDER BY i.due_date, i.number FOR UPDATE OF i"
    )))
    .bind(today)
    .bind(min_days)
    .bind(repeat_days as i32)
    .fetch_all(&mut *tx)
    .await?;
    let mut ids = Vec::new();
    for (invoice_id, number, outstanding, days) in due {
        if outstanding <= 0 {
            continue;
        }
        if requires_approval {
            // a person reviews the wording first; nothing is sent until they approve
            let f = crate::reminders::load_facts(&mut *tx, tenant_id, &invoice_id).await?;
            let id = new_id("rdf");
            sqlx::query("INSERT INTO reminder_drafts (id, tenant_id, invoice_id, channel, message, source, created_by) VALUES ($1, $2, $3, $4, $5, 'template', $6)")
                .bind(&id).bind(tenant_id).bind(&invoice_id).bind(channel).bind(crate::ai::template(&f.as_ai(channel))).bind(actor)
                .execute(&mut *tx).await?;
            ids.push(id);
            continue;
        }
        let id = new_id("rem");
        sqlx::query("INSERT INTO invoice_reminders (id, tenant_id, invoice_id, channel, message, sent_by) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(&id).bind(tenant_id).bind(&invoice_id).bind(channel).bind(format!("{days} days overdue")).bind(actor)
            .execute(&mut *tx).await?;
        tenant::emit(tx, tenant_id, "invoice.reminder", json!({ "reminder_id": id, "invoice_id": invoice_id, "number": number, "channel": channel, "outstanding_minor": outstanding, "days_overdue": days, "automated": true })).await?;
        ids.push(id);
    }
    Ok(ids)
}

/// `POST /v1/collections/dunning/run`
pub async fn run_dunning(
    State(s): State<AppState>,
    auth: Auth,
    body: Option<Json<DunningBody>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("collections:write")?;
    let b = body.map(|j| j.0).unwrap_or_default();
    let channel = b.channel.unwrap_or_else(|| "email".into());
    if !["email", "sms", "whatsapp", "call", "other"].contains(&channel.as_str()) {
        return Err(ApiError::invalid(
            "invalid_channel",
            "channel must be email, sms, whatsapp, call or other",
        ));
    }
    let (min, rep) = (
        b.min_days_overdue.unwrap_or(1).max(0),
        b.repeat_after_days.unwrap_or(7).clamp(0, 365),
    );
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let approval: bool = sqlx::query_scalar(
        "SELECT dunning_requires_approval FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    let ids = run_dunning_tx(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        chrono::Utc::now().date_naive(),
        &Policy {
            min_days: min,
            repeat_days: rep,
            channel: &channel,
            requires_approval: approval,
        },
    )
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "dunning.run",
        None,
        json!({ "reminded": ids.len(), "drafts": approval }),
    )
    .await?;
    tx.commit().await?;
    if approval {
        return Ok(Json(
            json!({ "reminded": 0, "drafts_created": ids.len(), "draft_ids": ids }),
        ));
    }
    Ok(Json(json!({ "reminded": ids.len(), "reminder_ids": ids })))
}
