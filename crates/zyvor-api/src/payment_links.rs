//! Payment links: one URL, many payers. A link holds either a fixed amount or "what is still owed on
//! this invoice"; opening it mints a fresh payment intent, so every payer gets their own checkout
//! session, secret and attempts. Links can expire, be capped to N uses, or be canceled.

use axum::{
    Json,
    extract::{Path, Query, State},
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
    /// Tie the link to a branch, or to a counter within it (print its QR at that counter).
    branch_id: Option<String>,
    counter_id: Option<String>,
    /// Stop after this many checkouts have been started.
    max_uses: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
    /// Shares owed to vendors out of every payment made through this link.
    splits: Option<Vec<crate::splits::SplitIn>>,
    /// A fee the payer covers on top of the amount, in basis points (200 = 2%, at most 1000). Fixed-amount links only.
    convenience_fee_bp: Option<i32>,
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
    if b.convenience_fee_bp
        .is_some_and(|f| !(0..=1000).contains(&f))
        || (b.convenience_fee_bp.unwrap_or(0) > 0 && b.invoice_id.is_some())
    {
        return Err(ApiError::invalid(
            "invalid_fee",
            "convenience_fee_bp is 0-1000 and only for fixed-amount links",
        ));
    }
    if b.amount_minor
        .is_some_and(|a| a <= 0 || a > zyvor_core::MAX_AMOUNT_MINOR)
        || b.max_uses.is_some_and(|m| m <= 0)
    {
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
    if let (Some(sp), Some(total)) = (&b.splits, b.amount_minor) {
        crate::splits::validate(sp, total)?;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    for sp in b.splits.iter().flatten() {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM vendors WHERE id = $1 AND archived_at IS NULL)",
        )
        .bind(&sp.vendor_id)
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(ApiError::not_found("vendor"));
        }
    }
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
    let (branch, counter) =
        crate::counters::resolve(&mut tx, b.branch_id.clone(), b.counter_id.as_deref()).await?;
    sqlx::query("INSERT INTO payment_links (id, tenant_id, token, description, amount_minor, currency, invoice_id, customer_id, max_uses, expires_at, created_by, splits, convenience_fee_bp, branch_id, counter_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)")
        .bind(&id).bind(&auth.tenant_id).bind(&token).bind(&b.description).bind(b.amount_minor).bind(&currency)
        .bind(&b.invoice_id).bind(&b.customer_id).bind(b.max_uses).bind(b.expires_at).bind(&auth.actor).bind(b.splits.as_ref().map(|v| json!(v.iter().map(|x| json!({ "vendor_id": x.vendor_id, "amount_minor": x.amount_minor })).collect::<Vec<_>>())))
        .bind(b.convenience_fee_bp.unwrap_or(0))
        .bind(&branch).bind(&counter)
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
#[derive(Deserialize, Default)]
pub struct CouponQuery {
    coupon: Option<String>,
}

pub async fn link_info(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
    Query(q): Query<CouponQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let (link_id, description, amount, currency, invoice, _, max, uses, exp, canc) =
        load(&mut tx, &token, false).await?;
    let fee_bp: i32 =
        sqlx::query_scalar("SELECT convenience_fee_bp FROM payment_links WHERE id = $1")
            .bind(&link_id)
            .fetch_one(&mut *tx)
            .await?;
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
    // what the payer will actually be asked for, with any coupon they typed
    let code = q.coupon.as_deref().map(str::trim).filter(|c| !c.is_empty());
    let applied = match code {
        Some(c) if invoice.is_none() => Some(crate::coupons::find(&mut tx, c, false).await?),
        Some(_) => {
            return Err(ApiError::invalid(
                "coupon_not_applicable",
                "coupons apply to fixed-amount links",
            ));
        }
        None => None,
    };
    let p = crate::coupons::price(
        amount,
        applied.as_ref().map(|c| (c.kind.as_str(), c.value)),
        fee_bp as i64,
    );
    Ok(Json(
        json!({ "merchant": merchant, "description": description, "amount_minor": amount, "discount_minor": p.discount, "fee_minor": p.fee, "total_minor": p.total, "coupon_applied": applied.is_some(), "currency": currency, "status": st }),
    ))
}

/// Public: open the link. Mints the payer's own payment intent and returns the pieces the checkout
/// page needs. Counts toward `max_uses` immediately, so a capped link can never over-sell.
#[derive(Deserialize, Default)]
pub struct StartBody {
    /// A coupon code the payer typed (fixed-amount links only).
    coupon: Option<String>,
}

pub async fn link_start(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
    body: Option<Json<StartBody>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let coupon_code = body
        .and_then(|b| b.0.coupon)
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
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
    // price: coupon (spent now, in this transaction) then the link's convenience fee
    let fee_bp: i32 =
        sqlx::query_scalar("SELECT convenience_fee_bp FROM payment_links WHERE id = $1")
            .bind(&link_id)
            .fetch_one(&mut *tx)
            .await?;
    let spent = match &coupon_code {
        Some(c) if invoice.is_none() => Some(crate::coupons::find(&mut tx, c, true).await?),
        Some(_) => {
            return Err(ApiError::invalid(
                "coupon_not_applicable",
                "coupons apply to fixed-amount links",
            ));
        }
        None => None,
    };
    let p = crate::coupons::price(
        amount,
        spent.as_ref().map(|c| (c.kind.as_str(), c.value)),
        fee_bp as i64,
    );
    let base_amount = amount;
    let amount = p.total;
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
    .bind(&description).bind(json!({ "payment_link": link_id, "base_minor": base_amount, "discount_minor": p.discount, "fee_minor": p.fee, "coupon_id": spent.as_ref().map(|c| c.id.clone()) }))
    .bind(crate::auth::hash_key(&secret))
    .execute(&mut *tx).await?;
    sqlx::query("UPDATE payment_intents i SET branch_id = l.branch_id, counter_id = l.counter_id FROM payment_links l WHERE l.id = $1 AND i.id = $2 AND l.branch_id IS NOT NULL")
        .bind(&link_id)
        .bind(&intent_id)
        .execute(&mut *tx)
        .await?;
    let stored: Option<Value> =
        sqlx::query_scalar("SELECT splits FROM payment_links WHERE id = $1")
            .bind(&link_id)
            .fetch_one(&mut *tx)
            .await?;
    if let Some(v) = stored {
        let sp: Vec<crate::splits::SplitIn> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| {
                Some(crate::splits::SplitIn {
                    vendor_id: x["vendor_id"].as_str()?.to_string(),
                    amount_minor: x["amount_minor"].as_i64()?,
                })
            })
            .collect();
        crate::splits::validate(&sp, amount - p.fee).map_err(|_| {
            ApiError::conflict(
                "link_unavailable",
                "this link's splits no longer fit the amount due",
            )
        })?;
        crate::splits::attach(&mut tx, &tenant_id, &intent_id, &sp).await?;
    }
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
        json!({ "amount_minor": amount, "discount_minor": p.discount, "fee_minor": p.fee }),
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

/// `GET /v1/payment_links/{id}/qr.svg`: a QR code of the link's checkout URL, for posters, counters and
/// printed fee notices. Needs `ZYVOR_CHECKOUT_PUBLIC_URL` so the code points at your public checkout.
pub async fn link_qr(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    auth.require("payments:read")?;
    let base = s.checkout_public_url.clone().ok_or_else(|| {
        ApiError::conflict(
            "checkout_url_not_configured",
            "set ZYVOR_CHECKOUT_PUBLIC_URL to your public checkout address",
        )
    })?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<i32>,
        i32,
    )> = sqlx::query_as(
        "SELECT token, canceled_at, expires_at, max_uses, uses FROM payment_links WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (token, canc, exp, max, uses) = row.ok_or_else(|| ApiError::not_found("payment link"))?;
    if state(canc, exp, max, uses) != "active" {
        return Err(ApiError::conflict(
            "link_unavailable",
            "this payment link is no longer active",
        ));
    }
    let url = format!(
        "{}/pay/link/{}/{token}",
        base.trim_end_matches('/'),
        auth.tenant_id
    );
    let svg = qrcode::QrCode::new(url.as_bytes())
        .map_err(|_| ApiError::internal())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .quiet_zone(true)
        .build();
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml"),
            (axum::http::header::CACHE_CONTROL, "private, max-age=300"),
        ],
        svg,
    )
        .into_response())
}
