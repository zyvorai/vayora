//! Reminder drafts: a person reads (and may edit) a reminder before it goes out. Drafts are made on
//! request (optionally worded by the AI assistant) or by the dunning run when the tenant requires
//! approval. Approving is what logs the reminder and emits `invoice.reminder`; nothing is sent
//! before that.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    ai::{self, ReminderFacts},
    auth::Auth,
    collections::OUTSTANDING,
    tenant,
};

const CHANNELS: [&str; 5] = ["email", "sms", "whatsapp", "call", "other"];

pub struct Facts {
    pub status: String,
    pub number: String,
    pub currency: String,
    pub outstanding: i64,
    pub due: Option<NaiveDate>,
    pub days: i64,
    pub earlier: i64,
    pub merchant: String,
}

pub async fn load_facts(
    tx: &mut sqlx::PgConnection,
    tenant_id: &str,
    invoice_id: &str,
) -> Result<Facts, ApiError> {
    let row: Option<(String, String, String, i64, Option<NaiveDate>, i64, i64)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT i.status, i.number, i.currency, {OUTSTANDING}, i.due_date,
                COALESCE((CURRENT_DATE - i.due_date), 0)::BIGINT,
                (SELECT count(*) FROM invoice_reminders r WHERE r.invoice_id = i.id)
         FROM invoices i WHERE i.id = $1"
        )))
        .bind(invoice_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (status, number, currency, outstanding, due, days, earlier) =
        row.ok_or_else(|| ApiError::not_found("invoice"))?;
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(Facts {
        status,
        number,
        currency,
        outstanding,
        due,
        days,
        earlier,
        merchant: merchant.unwrap_or_else(|| "us".into()),
    })
}

impl Facts {
    pub fn as_ai<'a>(&'a self, channel: &'a str) -> ReminderFacts<'a> {
        ReminderFacts {
            merchant: &self.merchant,
            number: &self.number,
            currency: &self.currency,
            outstanding_minor: self.outstanding,
            due_date: self.due,
            days_overdue: self.days.max(0),
            earlier_reminders: self.earlier,
            channel,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct DraftReq {
    channel: Option<String>,
    /// `friendly` (default) or `firm`; only used by the AI assistant.
    tone: Option<String>,
    /// Ask the AI assistant to word it (falls back to the standard text if it is unavailable or its answer is refused).
    ai: Option<bool>,
}

/// `POST /v1/invoices/{id}/reminders/draft`
pub async fn create_draft(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    body: Option<Json<DraftReq>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("collections:write")?;
    let b = body.map(|j| j.0).unwrap_or_default();
    let channel = b.channel.unwrap_or_else(|| "whatsapp".into());
    if !CHANNELS.contains(&channel.as_str()) {
        return Err(ApiError::invalid(
            "invalid_channel",
            "channel must be email, sms, whatsapp, call or other",
        ));
    }
    let want_ai = b.ai.unwrap_or(false);
    if want_ai && s.ai.is_none() {
        return Err(ApiError::invalid(
            "ai_not_configured",
            "the AI assistant is not enabled for this deployment",
        ));
    }
    // read first, call the model with no transaction or lock held, then write
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let f = load_facts(&mut tx, &auth.tenant_id, &invoice_id).await?;
    if !matches!(f.status.as_str(), "open" | "partially_paid") || f.outstanding <= 0 {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {}", f.status),
        ));
    }
    let used: i64 = if want_ai {
        sqlx::query_scalar("SELECT count(*) FROM reminder_drafts WHERE ai_requested AND created_at >= date_trunc('month', now())").fetch_one(&mut *tx).await?
    } else {
        0
    };
    drop(tx);
    let (mut message, mut source) = (ai::template(&f.as_ai(&channel)), "template");
    if want_ai {
        let cfg = s.ai.as_ref().expect("checked above");
        if used >= cfg.monthly_cap {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "ai_quota_exceeded",
                "this month's AI drafting allowance is used up; use the standard text or wait for next month",
            ));
        }
        if let Some(text) = ai::draft(
            cfg,
            &f.as_ai(&channel),
            b.tone.as_deref().unwrap_or("friendly"),
        )
        .await
        {
            (message, source) = (text, "ai");
        }
    }
    let id = new_id("rdf");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO reminder_drafts (id, tenant_id, invoice_id, channel, message, source, ai_requested, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(&channel).bind(&message).bind(source).bind(want_ai).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("draft_pending", "this invoice already has a draft waiting for a decision"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "reminder_draft.created",
        Some(&id),
        json!({ "invoice_id": invoice_id, "source": source, "ai_requested": want_ai }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "invoice_id": invoice_id, "channel": channel, "message": message, "source": source, "status": "pending" }),
        ),
    ))
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

pub async fn list_drafts(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT d.id, d.invoice_id, i.number, d.channel, d.message, d.source, d.status, d.created_by, d.created_at
         FROM reminder_drafts d JOIN invoices i ON i.id = d.invoice_id
         WHERE d.status = COALESCE($1, 'pending') ORDER BY d.created_at DESC LIMIT 200",
    )
    .bind(&q.status)
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, inv, number, channel, message, source, status, by, at)| json!({ "id": id, "invoice_id": inv, "invoice_number": number, "channel": channel, "message": message, "source": source, "status": status, "created_by": by, "created_at": at })).collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize, Default)]
pub struct Approve {
    /// Replace the drafted text before sending.
    message: Option<String>,
}

pub async fn approve_draft(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<Approve>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("collections:write")?;
    let edited = body.and_then(|j| j.0.message).map(|m| m.trim().to_string());
    if edited
        .as_ref()
        .is_some_and(|m| m.is_empty() || m.chars().count() > 1000)
    {
        return Err(ApiError::invalid(
            "invalid_message",
            "message must be 1-1000 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let d: Option<(String, String, String, String, String)> = sqlx::query_as("SELECT invoice_id, channel, message, source, status FROM reminder_drafts WHERE id = $1 FOR UPDATE")
        .bind(&id).fetch_optional(&mut *tx).await?;
    let (invoice_id, channel, drafted, source, status) =
        d.ok_or_else(|| ApiError::not_found("draft"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "draft_decided",
            format!("this draft was already {status}"),
        ));
    }
    let f = load_facts(&mut tx, &auth.tenant_id, &invoice_id).await?;
    if !matches!(f.status.as_str(), "open" | "partially_paid") || f.outstanding <= 0 {
        return Err(ApiError::conflict(
            "invoice_not_collectable",
            format!("invoice is {}; reject this draft instead", f.status),
        ));
    }
    let message = edited.clone().unwrap_or(drafted);
    let rem = new_id("rem");
    sqlx::query("INSERT INTO invoice_reminders (id, tenant_id, invoice_id, channel, message, sent_by) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(&rem).bind(&auth.tenant_id).bind(&invoice_id).bind(&channel).bind(&message).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::emit(&mut tx, &auth.tenant_id, "invoice.reminder", json!({ "reminder_id": rem, "invoice_id": invoice_id, "number": f.number, "channel": channel, "outstanding_minor": f.outstanding, "message": message, "approved_by": auth.actor, "drafted_by": source })).await?;
    sqlx::query("UPDATE reminder_drafts SET status = 'approved', message = $2, decided_by = $3, decided_at = now(), reminder_id = $4 WHERE id = $1")
        .bind(&id).bind(&message).bind(&auth.actor).bind(&rem).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "reminder_draft.approved",
        Some(&id),
        json!({ "edited": edited.is_some(), "reminder_id": rem }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "status": "approved", "reminder_id": rem }),
    ))
}

pub async fn reject_draft(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("collections:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE reminder_drafts SET status = 'rejected', decided_by = $2, decided_at = now() WHERE id = $1 AND status = 'pending'")
        .bind(&id).bind(&auth.actor).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("pending draft"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "reminder_draft.rejected",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "rejected" })))
}
