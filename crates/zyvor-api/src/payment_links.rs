//! Payment links: one URL, many payers. A link holds either a fixed amount or "what is still owed on
//! this invoice"; opening it mints a fresh payment intent, so every payer gets their own checkout
//! session, secret and attempts. Links can expire, be capped to N uses, or be canceled.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::{Currency, new_id};

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Deserialize)]
pub struct NewLink {
    description: Option<String>,
    /// A fixed amount in minor units. Give this or `invoice_id`, not both.
    amount_minor: Option<i64>,
    currency: Option<String>,
    /// Collect whatever is still outstanding on this invoice.
    invoice_id: Option<String>,
    customer_id: Option<String>,
    /// Stop after this many checkouts have been started.
    max_uses: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
}

pub async fn create_link(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewLink>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    if b.amount_minor.is_some() == b.invoice_id.is_some() {
        return Err(ApiError::invalid(
            "invalid_link",
            "give exactly one of amount_minor or invoice_id",
        ));
    }
    if b.amount_minor.is_some_and(|a| a <= 0) || b.max_uses.is_some_and(|m| m <= 0) {
        return Err(ApiError::invalid(
            "invalid_link",
            "amount_minor and max_uses must be positive",
        ));
    }
    if b.expires_at.is_some_and(|e| e <= Utc::now()) {
        return Err(ApiError::invalid(
            "invalid_expiry",
            "expires_at must be in the future",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut currency = b.currency.clone().unwrap_or_else(|| "INR".into());
    if let Some(inv) = &b.invoice_id {
        // an invoice link takes the invoice's currency
        currency = sqlx::query_scalar("SELECT currency FROM invoices WHERE id = $1")
            .bind(inv)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::not_found("invoice"))?;
    }
    currency
        .parse::<Currency>()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let (id, token) = (
        new_id("plk"),
        format!(
            "plink_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    );
    sqlx::query("INSERT INTO payment_links (id, tenant_id, token, description, amount_minor, currency, invoice_id, customer_id, max_uses, expires_at, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(&id).bind(&auth.tenant_id).bind(&token).bind(&b.description).bind(b.amount_minor).bind(&currency)
        .bind(&b.invoice_id).bind(&b.customer_id).bind(b.max_uses).bind(b.expires_at).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_foreign_key_violation() => ApiError::not_found("customer"),
            e => e.into(),
        })?;
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "payment_link.created", Some(&id), json!({ "amount_minor": b.amount_minor, "invoice_id": b.invoice_id, "max_uses": b.max_uses })).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "token": token, "currency": currency, "path": format!("/pay/link/{}/{token}", auth.tenant_id), "max_uses": b.max_uses, "expires_at": b.expires_at }),
        ),
    ))
}

pub async fn list_links(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, Option<i64>, String, Option<String>, Option<i32>, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, token, description, amount_minor, currency, invoice_id, max_uses, uses, expires_at, canceled_at, created_at FROM payment_links ORDER BY created_at DESC, id DESC LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, token, description, amount, currency, invoice, max, uses, exp, canc, at)| {
        let status = state(canc, exp, max, uses);
        json!({ "id": id, "path": format!("/pay/link/{}/{token}", auth.tenant_id), "description": description, "amount_minor": amount, "currency": currency, "invoice_id": invoice, "max_uses": max, "uses": uses, "expires_at": exp, "status": status, "created_at": at })
    }).collect();
    Ok(Json(json!({ "data": data })))
}

pub async fn cancel_link(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE payment_links SET canceled_at = now() WHERE id = $1 AND canceled_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("active payment link"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_link.canceled",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "canceled" })))
}

fn state(
    canceled: Option<DateTime<Utc>>,
    exp: Option<DateTime<Utc>>,
    max: Option<i32>,
    uses: i32,
) -> &'static str {
    if canceled.is_some() {
        "canceled"
    } else if exp.is_some_and(|e| e <= Utc::now()) {
        "expired"
    } else if max.is_some_and(|m| uses >= m) {
        "used_up"
    } else {
        "active"
    }
}

type LinkRow = (
    String,
    Option<String>,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    Option<i32>,
    i32,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

async fn load(tx: &mut sqlx::PgConnection, token: &str, lock: bool) -> Result<LinkRow, ApiError> {
    let sql = format!(
        "SELECT id, description, amount_minor, currency, invoice_id, customer_id, max_uses, uses, expires_at, canceled_at FROM payment_links WHERE token = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(token)
        .fetch_optional(tx)
        .await?
        .ok_or_else(|| ApiError::not_found("payment link"))
}

/// Public: what the page shows before the payer commits to anything.
pub async fn link_info(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let (_, description, amount, currency, invoice, _, max, uses, exp, canc) =
        load(&mut tx, &token, false).await?;
    let amount =
        match (amount, &invoice) {
            (Some(a), _) => a,
            (None, Some(inv)) => sqlx::query_scalar(
                "SELECT (amount_minor - invoice_settled(id))::BIGINT FROM invoices WHERE id = $1",
            )
            .bind(inv)
            .fetch_one(&mut *tx)
            .await?,
            _ => 0,
        };
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&tenant_id).fetch_optional(&mut *tx).await?;
    let mut st = state(canc, exp, max, uses);
    if st == "active" && amount <= 0 {
        st = "paid";
    }
    Ok(Json(
        json!({ "merchant": merchant, "description": description, "amount_minor": amount, "currency": currency, "status": st }),
    ))
}

/// Public: open the link. Mints the payer's own payment intent and returns the pieces the checkout
/// page needs. Counts toward `max_uses` immediately, so a capped link can never over-sell.
pub async fn link_start(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let (link_id, description, amount, currency, invoice, customer, max, uses, exp, canc) =
        load(&mut tx, &token, true).await?;
    match state(canc, exp, max, uses) {
        "active" => {}
        other => {
            return Err(ApiError::conflict(
                "link_unavailable",
                format!("this payment link is {other}"),
            ));
        }
    }
    let amount = match (amount, &invoice) {
        (Some(a), _) => a,
        (None, Some(inv)) => {
            let (status, owed): (String, i64) = sqlx::query_as("SELECT status, (amount_minor - invoice_settled(id))::BIGINT FROM invoices WHERE id = $1 FOR SHARE")
                .bind(inv)
                .fetch_one(&mut *tx)
                .await?;
            if matches!(status.as_str(), "void" | "paid") || owed <= 0 {
                return Err(ApiError::conflict(
                    "link_unavailable",
                    "this invoice has nothing left to pay",
                ));
            }
            owed
        }
        _ => return Err(ApiError::internal()),
    };
    let secret = format!(
        "pis_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let intent_id = new_id("pi");
    sqlx::query(
        "INSERT INTO payment_intents (id, tenant_id, customer_id, invoice_id, amount_minor, currency, status, capture_method, description, metadata, client_secret_hash)
         VALUES ($1, $2, $3, $4, $5, $6, 'requires_payment_method', 'automatic', $7, $8, $9)",
    )
    .bind(&intent_id).bind(&tenant_id).bind(&customer).bind(&invoice).bind(amount).bind(&currency)
    .bind(&description).bind(json!({ "payment_link": link_id }))
    .bind(crate::auth::hash_key(&secret))
    .execute(&mut *tx).await?;
    sqlx::query("UPDATE payment_links SET uses = uses + 1 WHERE id = $1")
        .bind(&link_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &tenant_id,
        &format!("link:{link_id}"),
        "payment_link.opened",
        Some(&intent_id),
        json!({ "amount_minor": amount }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "intent_id": intent_id, "client_secret": secret, "amount_minor": amount, "currency": currency, "path": format!("/pay/{tenant_id}/{intent_id}") }),
        ),
    ))
}
