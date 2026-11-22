// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Donations: campaigns with suggested amounts, a public page-less start endpoint a site or app can call, receipts as
//! PDF, a CSV export, and an optional recurring donor through a UPI AutoPay mandate (where the provider supports it).
//!
//! Receipt wording is the merchant's: a campaign carries free-text receipt fields (a registration line, an exemption
//! line, a note) that are printed as typed. Zyvor does not check, interpret or vouch for them, makes no statement about
//! tax benefits, and a donor can never set them. A donor may give a name, an email and a phone number, or none
//! (anonymous); a receipt needs a name. See docs/zyvor/VERTICALS.md.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, request::Parts},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::{Auth, hash_key},
    storefront::{clean_email, clean_phone, clean_text, throttle},
    tenant,
    verticals::{csv_line, csv_response, rupees},
};

const MAX_SUGGESTED: usize = 6;

#[derive(Deserialize, Default, Clone)]
pub struct DonationReceiptFields {
    registration_text: Option<String>,
    exemption_text: Option<String>,
    receipt_note: Option<String>,
}

fn clean_fields(f: &DonationReceiptFields) -> Result<Value, ApiError> {
    let mut out = serde_json::Map::new();
    for (k, v) in [
        ("registration_text", &f.registration_text),
        ("exemption_text", &f.exemption_text),
        ("receipt_note", &f.receipt_note),
    ] {
        if let Some(t) = v
            .as_deref()
            .map(|t| clean_text(t, k, 300, false, false))
            .transpose()?
            .filter(|t| !t.is_empty())
        {
            out.insert(k.to_string(), json!(t));
        }
    }
    Ok(Value::Object(out))
}

fn check_amounts(suggested: &[i64], min: i64, max: i64, goal: Option<i64>) -> Result<(), ApiError> {
    let ok = min > 0
        && min <= max
        && max <= zyvor_core::MAX_AMOUNT_MINOR
        && suggested.len() <= MAX_SUGGESTED
        && suggested.iter().all(|a| (min..=max).contains(a))
        && goal.is_none_or(|g| g > 0 && g <= zyvor_core::MAX_AMOUNT_MINOR);
    if ok {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "invalid_amounts",
            format!(
                "min_minor <= max_minor, at most {MAX_SUGGESTED} suggested amounts each between them, and a positive goal"
            ),
        ))
    }
}

#[derive(Deserialize)]
pub struct DonationCampaignIn {
    name: String,
    description: Option<String>,
    suggested_minor: Option<Vec<i64>>,
    min_minor: Option<i64>,
    max_minor: Option<i64>,
    goal_minor: Option<i64>,
    receipt_fields: Option<DonationReceiptFields>,
    recurring_enabled: Option<bool>,
}

type CampaignRow = (
    String,
    String,
    String,
    Vec<i64>,
    i64,
    i64,
    Option<i64>,
    Value,
    bool,
    String,
    DateTime<Utc>,
);
const CAMPAIGN_COLS: &str = "id, name, description, suggested_minor, min_minor, max_minor, goal_minor, receipt_fields, recurring_enabled, status, created_at";

fn campaign_json(r: CampaignRow) -> Value {
    json!({ "id": r.0, "name": r.1, "description": r.2, "suggested_minor": r.3, "min_minor": r.4, "max_minor": r.5, "goal_minor": r.6,
            "receipt_fields": r.7, "recurring_enabled": r.8, "status": r.9, "created_at": r.10 })
}

/// `POST /v1/donations/campaigns`
pub async fn create_campaign(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<DonationCampaignIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    let name = clean_text(&b.name, "name", 80, false, true)?;
    let description = clean_text(
        b.description.as_deref().unwrap_or(""),
        "description",
        500,
        true,
        false,
    )?;
    let suggested = b.suggested_minor.unwrap_or_default();
    let (min, max) = (
        b.min_minor.unwrap_or(100),
        b.max_minor.unwrap_or(100_000_000),
    );
    check_amounts(&suggested, min, max, b.goal_minor)?;
    let fields = clean_fields(&b.receipt_fields.unwrap_or_default())?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id = new_id("dcm");
    sqlx::query("INSERT INTO donation_campaigns (id, tenant_id, name, description, suggested_minor, min_minor, max_minor, goal_minor, receipt_fields, recurring_enabled) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&name)
        .bind(&description)
        .bind(&suggested)
        .bind(min)
        .bind(max)
        .bind(b.goal_minor)
        .bind(&fields)
        .bind(b.recurring_enabled.unwrap_or(false))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "donation.campaign_created",
        Some(&id),
        json!({}),
    )
    .await?;
    let row: CampaignRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLS} FROM donation_campaigns WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(campaign_json(row))))
}

/// `GET /v1/donations/campaigns`
pub async fn list_campaigns(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<CampaignRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLS} FROM donation_campaigns ORDER BY created_at DESC, id DESC LIMIT 200"
    )))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(campaign_json).collect::<Vec<_>>() }),
    ))
}

#[derive(Deserialize)]
pub struct DonationCampaignPatch {
    name: Option<String>,
    description: Option<String>,
    suggested_minor: Option<Vec<i64>>,
    min_minor: Option<i64>,
    max_minor: Option<i64>,
    goal_minor: Option<i64>,
    receipt_fields: Option<DonationReceiptFields>,
    recurring_enabled: Option<bool>,
    /// `active` or `closed`. A closed campaign takes no new donations.
    status: Option<String>,
}

/// `PATCH /v1/donations/campaigns/{id}`
pub async fn patch_campaign(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<DonationCampaignPatch>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:write")?;
    let name = b
        .name
        .as_deref()
        .map(|n| clean_text(n, "name", 80, false, true))
        .transpose()?;
    let description = b
        .description
        .as_deref()
        .map(|d| clean_text(d, "description", 500, true, false))
        .transpose()?;
    if b.status
        .as_deref()
        .is_some_and(|x| !matches!(x, "active" | "closed"))
    {
        return Err(ApiError::invalid(
            "invalid_status",
            "status is active or closed",
        ));
    }
    let fields = b.receipt_fields.as_ref().map(clean_fields).transpose()?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur: (Vec<i64>, i64, i64, Option<i64>) = sqlx::query_as("SELECT suggested_minor, min_minor, max_minor, goal_minor FROM donation_campaigns WHERE id = $1 FOR UPDATE").bind(&id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::not_found("campaign"))?;
    let suggested = b.suggested_minor.unwrap_or(cur.0);
    let (min, max, goal) = (
        b.min_minor.unwrap_or(cur.1),
        b.max_minor.unwrap_or(cur.2),
        b.goal_minor.or(cur.3),
    );
    check_amounts(&suggested, min, max, goal)?;
    sqlx::query(
        "UPDATE donation_campaigns SET name = COALESCE($2, name), description = COALESCE($3, description), suggested_minor = $4, min_minor = $5, max_minor = $6, goal_minor = $7,
            receipt_fields = COALESCE($8, receipt_fields), recurring_enabled = COALESCE($9, recurring_enabled), status = COALESCE($10, status), updated_at = now() WHERE id = $1",
    )
    .bind(&id)
    .bind(name)
    .bind(description)
    .bind(&suggested)
    .bind(min)
    .bind(max)
    .bind(goal)
    .bind(fields)
    .bind(b.recurring_enabled)
    .bind(&b.status)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "donation.campaign_changed",
        Some(&id),
        json!({ "status": b.status }),
    )
    .await?;
    let row: CampaignRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLS} FROM donation_campaigns WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(campaign_json(row)))
}

/// `GET /v1/donations/campaigns/{id}/summary`
pub async fn summary(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let goal: Option<Option<i64>> =
        sqlx::query_scalar("SELECT goal_minor FROM donation_campaigns WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let goal = goal.ok_or_else(|| ApiError::not_found("campaign"))?;
    let (paid_n, paid, pending_n): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'paid'), COALESCE(SUM(amount_minor) FILTER (WHERE status = 'paid'), 0)::BIGINT, count(*) FILTER (WHERE status = 'pending') FROM donations WHERE campaign_id = $1",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let refunded: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(r.amount_minor), 0)::BIGINT FROM refunds r JOIN donations d ON d.intent_id = r.intent_id WHERE d.campaign_id = $1 AND r.status = 'processed'").bind(&id).fetch_one(&mut *tx).await?;
    let pledges: i64 =
        sqlx::query_scalar("SELECT count(*) FROM donation_pledges WHERE campaign_id = $1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    Ok(Json(
        json!({ "campaign_id": id, "paid_count": paid_n, "paid_minor": paid, "refunded_minor": refunded, "net_minor": paid - refunded, "pending_count": pending_n, "goal_minor": goal, "recurring_pledges": pledges }),
    ))
}

// ---------- the donor's side (public) ----------

/// `GET /v1/donate/{tenant_id}/{campaign_id}`: what a donation page needs.
pub async fn public_campaign(
    State(s): State<AppState>,
    Path((tenant_id, id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let row: Option<(String, String, Vec<i64>, i64, i64, bool)> = sqlx::query_as("SELECT name, description, suggested_minor, min_minor, max_minor, recurring_enabled FROM donation_campaigns WHERE id = $1 AND status = 'active'").bind(&id).fetch_optional(&mut *tx).await?;
    let (name, description, suggested, min, max, recurring) =
        row.ok_or_else(|| ApiError::not_found("campaign"))?;
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&tenant_id).fetch_optional(&mut *tx).await?;
    Ok(Json(
        json!({ "merchant": merchant, "name": name, "description": description, "suggested_minor": suggested, "min_minor": min, "max_minor": max, "currency": "INR", "recurring_enabled": recurring }),
    ))
}

#[derive(Deserialize)]
pub struct DonationStartIn {
    amount_minor: i64,
    /// Leave all donor fields out, or set `anonymous`, to give without a name; a receipt then cannot be made.
    donor_name: Option<String>,
    donor_email: Option<String>,
    donor_phone: Option<String>,
    anonymous: Option<bool>,
}

/// `POST /v1/donate/{tenant_id}/{campaign_id}/start`: record the donation and open the donor's own payment intent.
pub async fn start(
    State(s): State<AppState>,
    Path((tenant_id, id)): Path<(String, String)>,
    parts: Parts,
    Json(b): Json<DonationStartIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    throttle(&s, &parts, &format!("donate:{id}"))?;
    let anonymous = b.anonymous.unwrap_or(false);
    let opt = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    let (name, email, phone) = if anonymous {
        (None, None, None)
    } else {
        (
            opt(&b.donor_name)
                .map(|n| clean_text(&n, "name", 80, false, true))
                .transpose()?,
            opt(&b.donor_email).map(|e| clean_email(&e)).transpose()?,
            opt(&b.donor_phone).map(|p| clean_phone(&p)).transpose()?,
        )
    };
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let c: Option<(String, i64, i64)> = sqlx::query_as("SELECT name, min_minor, max_minor FROM donation_campaigns WHERE id = $1 AND status = 'active'").bind(&id).fetch_optional(&mut *tx).await?;
    let (cname, min, max) = c.ok_or_else(|| ApiError::not_found("campaign"))?;
    if !(min..=max).contains(&b.amount_minor) {
        return Err(ApiError::invalid(
            "amount_out_of_range",
            format!("the amount must be between {min} and {max} paise"),
        ));
    }
    let (donation, intent) = (new_id("don"), new_id("pi"));
    let secret = format!(
        "pis_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method, description, metadata, client_secret_hash) VALUES ($1,$2,$3,'INR','requires_payment_method','automatic',$4,$5,$6)")
        .bind(&intent)
        .bind(&tenant_id)
        .bind(b.amount_minor)
        .bind(format!("Donation to {cname}"))
        .bind(json!({ "donation": donation, "campaign": id }))
        .bind(hash_key(&secret))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO donations (id, tenant_id, campaign_id, intent_id, donor_name, donor_email, donor_phone, anonymous, amount_minor, currency) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'INR')")
        .bind(&donation)
        .bind(&tenant_id)
        .bind(&id)
        .bind(&intent)
        .bind(&name)
        .bind(&email)
        .bind(&phone)
        .bind(anonymous)
        .bind(b.amount_minor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &tenant_id,
        &format!("donate:{id}"),
        "donation.started",
        Some(&donation),
        json!({ "amount_minor": b.amount_minor }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "donation_id": donation, "intent_id": intent, "client_secret": secret, "amount_minor": b.amount_minor, "currency": "INR", "path": format!("/pay/{tenant_id}/{intent}") }),
        ),
    ))
}

/// Called when a payment is captured: the donation is paid and gets its receipt number (date plus a short id, not a
/// gap-free series).
pub(crate) async fn on_captured(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
) -> Result<(), ApiError> {
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let today = Utc::now().with_timezone(&ist).format("%Y%m%d");
    let row: Option<String> = sqlx::query_scalar("UPDATE donations SET status = 'paid', paid_at = now(), receipt_no = 'DN' || $2 || '-' || upper(right(id, 6)) WHERE intent_id = $1 AND status = 'pending' RETURNING id")
        .bind(intent_id)
        .bind(today.to_string())
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = row {
        tenant::audit(
            conn,
            tenant_id,
            "system:payments",
            "donation.paid",
            Some(&id),
            json!({}),
        )
        .await?;
    }
    Ok(())
}

// ---------- records, receipts, export ----------

#[derive(Deserialize)]
pub struct DonationListQuery {
    campaign_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
}

type DonationRow = (
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    i64,
);

async fn donation_rows(
    conn: &mut PgConnection,
    q: &DonationListQuery,
    limit: i64,
) -> Result<Vec<DonationRow>, ApiError> {
    if q.status
        .as_deref()
        .is_some_and(|x| !matches!(x, "pending" | "paid"))
    {
        return Err(ApiError::invalid(
            "invalid_status",
            "status is pending or paid",
        ));
    }
    Ok(sqlx::query_as(
        "SELECT d.id, d.campaign_id, d.intent_id, d.amount_minor, d.status, d.donor_name, d.donor_email, d.donor_phone, d.anonymous, d.receipt_no, d.created_at, d.paid_at,
                COALESCE((SELECT SUM(r.amount_minor) FROM refunds r WHERE r.intent_id = d.intent_id AND r.status = 'processed'), 0)::BIGINT
           FROM donations d WHERE ($1::text IS NULL OR d.campaign_id = $1) AND ($2::text IS NULL OR d.status = $2) ORDER BY d.created_at DESC, d.id DESC LIMIT $3",
    )
    .bind(&q.campaign_id)
    .bind(&q.status)
    .bind(limit)
    .fetch_all(conn)
    .await?)
}

/// `GET /v1/donations`
pub async fn list_donations(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<DonationListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verticals:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows = donation_rows(&mut tx, &q, q.limit.unwrap_or(100).clamp(1, 500)).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|r| json!({ "id": r.0, "campaign_id": r.1, "intent_id": r.2, "amount_minor": r.3, "status": r.4,
        "donor": if r.8 { Value::Null } else { json!({ "name": r.5, "email": r.6, "phone": r.7 }) }, "anonymous": r.8, "receipt_no": r.9, "created_at": r.10, "paid_at": r.11, "refunded_minor": r.12 })).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/donations.csv`: paid donations for the books. Not a tax statement.
pub async fn export_csv(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<DonationListQuery>,
) -> Result<axum::response::Response, ApiError> {
    auth.require("verticals:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let q = DonationListQuery {
        status: Some("paid".into()),
        ..q
    };
    let rows = donation_rows(&mut tx, &q, 20_000).await?;
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let mut out = csv_line(
        &[
            "receipt_no",
            "paid_on",
            "campaign_id",
            "amount",
            "refunded",
            "donor_name",
            "donor_email",
            "donor_phone",
        ]
        .map(String::from),
    );
    for r in rows {
        out.push_str(&csv_line(&[
            r.9.unwrap_or_default(),
            r.11.map(|t| t.with_timezone(&ist).date_naive().to_string())
                .unwrap_or_default(),
            r.1,
            rupees(r.3),
            rupees(r.12),
            r.5.unwrap_or_default(),
            r.6.unwrap_or_default(),
            r.7.unwrap_or_default(),
        ]));
    }
    csv_response("donations.csv", out)
}

/// `GET /v1/donations/{id}/receipt.pdf`: a receipt for a paid donation that has a donor name. The registration,
/// exemption and note lines are the merchant's own words, printed as typed.
pub async fn receipt_pdf(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    auth.require("verticals:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, i64, String, Option<String>, Option<String>, Option<DateTime<Utc>>, String, Value)> = sqlx::query_as(
        "SELECT d.status, d.amount_minor, c.name, d.donor_name, d.receipt_no, d.paid_at, c.id, c.receipt_fields FROM donations d JOIN donation_campaigns c ON c.id = d.campaign_id WHERE d.id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, amount, campaign, donor, receipt_no, paid_at, _, fields) =
        row.ok_or_else(|| ApiError::not_found("donation"))?;
    let (Some(donor), Some(receipt_no), "paid") = (donor, receipt_no, status.as_str()) else {
        return Err(ApiError::conflict(
            "no_receipt",
            "a receipt needs a paid donation with the donor's name",
        ));
    };
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&auth.tenant_id).fetch_optional(&mut *tx).await?.unwrap_or_default();
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let date: NaiveDate = paid_at
        .map(|t| t.with_timezone(&ist).date_naive())
        .unwrap_or_else(crate::ist_today);
    let mut doc = vec![
        format!("Received by: {merchant}"),
        format!("Receipt number: {receipt_no}"),
        format!("Date: {date}"),
        String::new(),
        format!("Received with thanks from: {donor}"),
        format!("Towards: {campaign}"),
        format!("Amount: {}", crate::ai::money(amount, "INR")),
    ];
    let mut own = Vec::new();
    for (k, label) in [
        ("registration_text", ""),
        ("exemption_text", ""),
        ("receipt_note", ""),
    ] {
        if let Some(t) = fields[k].as_str() {
            own.push(format!("{label}{t}"));
        }
    }
    if !own.is_empty() {
        doc.push(String::new());
        doc.extend(own);
    }
    doc.push(String::new());
    doc.push("This receipt records a payment received. Any registration or exemption wording above was written by the organisation; Zyvor does not check it and gives no tax advice.".to_string());
    let pdf = crate::pdf::text_pdf("Donation receipt", &doc);
    crate::uploads::download_response(&format!("{receipt_no}.pdf"), "application/pdf", pdf)
}

// ---------- recurring donors ----------

#[derive(Deserialize)]
pub struct DonationPledgeIn {
    donor_name: String,
    donor_email: Option<String>,
    /// The donor's mobile number; the UPI app uses it.
    donor_phone: String,
    /// Each debit is at most this; it must be inside the campaign's range.
    amount_minor: i64,
    /// `weekly`, `monthly`, ... as `POST /v1/mandates` accepts.
    frequency: String,
    start_date: Option<NaiveDate>,
    end_date: NaiveDate,
    provider: Option<String>,
}

/// `POST /v1/donations/campaigns/{id}/pledges`: a recurring donor. Makes the donor a customer and creates a UPI AutoPay
/// mandate through the ordinary mandate flow (the donor approves it in their UPI app; the limits, notice and revoke
/// rules of mandates apply). Debits are then scheduled with `POST /v1/mandates/{id}/debits`; they are not yet listed
/// under the campaign. Needs the `mandates:write` scope as well, and an `Idempotency-Key`.
pub async fn create_pledge(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(b): Json<DonationPledgeIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("verticals:write")?;
    auth.require("mandates:write")?;
    let name = clean_text(&b.donor_name, "donor_name", 80, false, true)?;
    let phone = clean_phone(&b.donor_phone)?;
    let email = b
        .donor_email
        .as_deref()
        .filter(|e| !e.trim().is_empty())
        .map(clean_email)
        .transpose()?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty() && k.len() <= 200)
        .ok_or_else(|| {
            ApiError::bad_request(
                "idempotency_key_required",
                "Idempotency-Key header is required",
            )
        })?
        .to_string();
    let pledge = new_id("dpl");
    let customer = {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let c: Option<(bool, i64, i64)> = sqlx::query_as("SELECT recurring_enabled, min_minor, max_minor FROM donation_campaigns WHERE id = $1 AND status = 'active'").bind(&id).fetch_optional(&mut *tx).await?;
        let (recurring, min, max) = c.ok_or_else(|| ApiError::not_found("campaign"))?;
        if !recurring {
            return Err(ApiError::conflict(
                "recurring_not_enabled",
                "this campaign does not take recurring donors",
            ));
        }
        if !(min..=max).contains(&b.amount_minor) {
            return Err(ApiError::invalid(
                "amount_out_of_range",
                "the amount is outside the campaign's range",
            ));
        }
        let customer = new_id("cus");
        sqlx::query("INSERT INTO customers (id, tenant_id, name, email, phone, external_ref) VALUES ($1,$2,$3,$4,$5,$6)").bind(&customer).bind(&auth.tenant_id).bind(&name).bind(&email).bind(&phone).bind(format!("donor:{pledge}")).execute(&mut *tx).await?;
        tx.commit().await?;
        customer
    };
    let mut mh = HeaderMap::new();
    mh.insert(
        "idempotency-key",
        format!("pledge:{key}")
            .parse()
            .map_err(|_| ApiError::internal())?,
    );
    let res = crate::mandates::create_mandate(
        State(s.clone()),
        auth.clone(),
        mh,
        Json(crate::mandates::NewMandate {
            customer_id: customer.clone(),
            max_amount_minor: b.amount_minor,
            frequency: b.frequency.clone(),
            start_date: b.start_date,
            end_date: b.end_date,
            currency: Some("INR".into()),
            provider: b.provider.clone(),
            external_ref: Some(format!("pledge:{pledge}")),
        }),
    )
    .await?;
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .map_err(|_| ApiError::internal())?;
    let mandate: Value = serde_json::from_slice(&bytes).map_err(|_| ApiError::internal())?;
    let mandate_id = mandate["id"]
        .as_str()
        .ok_or_else(ApiError::internal)?
        .to_string();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO donation_pledges (id, tenant_id, campaign_id, customer_id, mandate_id, amount_minor, frequency, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&pledge)
        .bind(&auth.tenant_id)
        .bind(&id)
        .bind(&customer)
        .bind(&mandate_id)
        .bind(b.amount_minor)
        .bind(&b.frequency)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "donation.pledge_created",
        Some(&pledge),
        json!({ "mandate_id": mandate_id }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": pledge, "customer_id": customer, "mandate": mandate })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_checked_together() {
        assert!(check_amounts(&[10_000, 50_000], 100, 1_000_000, Some(5_000_000)).is_ok());
        assert!(
            check_amounts(&[50], 100, 1_000_000, None).is_err(),
            "suggestion below the minimum"
        );
        assert!(check_amounts(&[], 500, 100, None).is_err(), "min above max");
        assert!(
            check_amounts(&[100; 7], 100, 1_000, None).is_err(),
            "too many suggestions"
        );
        assert!(check_amounts(&[], 100, 1_000, Some(0)).is_err());
    }

    #[test]
    fn receipt_fields_keep_only_known_keys_and_cap_length() {
        let f = DonationReceiptFields {
            registration_text: Some(" Reg. No. 123 ".into()),
            exemption_text: Some("".into()),
            receipt_note: None,
        };
        assert_eq!(
            clean_fields(&f).unwrap(),
            json!({ "registration_text": "Reg. No. 123" })
        );
        let long = DonationReceiptFields {
            receipt_note: Some("x".repeat(301)),
            ..Default::default()
        };
        assert!(clean_fields(&long).is_err());
    }
}
