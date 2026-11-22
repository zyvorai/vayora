// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Payment links: one URL, many payers. A link holds either a fixed amount or "what is still owed on
//! this invoice"; opening it mints a fresh payment intent, so every payer gets their own checkout
//! session, secret and attempts. Links can expire, be capped to N uses, or be canceled.
//!
//! A link can also carry a short message and a thank-you, the business's logo and colour, a return address and the
//! payment methods it accepts; it can be paid in parts (never marked paid until the captured parts add up); and it
//! counts opens and starts per day without keeping anything about the visitor (`link_stats`). Many links can be made
//! from one CSV (`link_bulk`).

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

#[derive(Deserialize, Default, Clone)]
pub struct NewLink {
    pub description: Option<String>,
    /// A fixed amount in minor units. Give this or `invoice_id`, not both.
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    /// Collect whatever is still outstanding on this invoice.
    pub invoice_id: Option<String>,
    pub customer_id: Option<String>,
    /// Tie the link to a branch, or to a counter within it (print its QR at that counter).
    pub branch_id: Option<String>,
    pub counter_id: Option<String>,
    /// The payer types the amount (a printed counter QR). Give `min_amount_minor` and `max_amount_minor`
    /// to bound it; no amount, invoice, splits or fee on such a link.
    pub open_amount: Option<bool>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    /// Stop after this many checkouts have been started.
    pub max_uses: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Shares owed to vendors out of every payment made through this link.
    pub splits: Option<Vec<crate::splits::SplitIn>>,
    /// A fee the payer covers on top of the amount, in basis points (200 = 2%, at most 1000). Fixed-amount links only.
    pub convenience_fee_bp: Option<i32>,
    /// Email the link's customer once, in the last 24 hours before `expires_at`, if the link is still unused up
    /// and the invoice (if any) still owes money. Needs `customer_id` and `expires_at`.
    pub remind_before_expiry: Option<bool>,
    /// A short note shown to the payer on the link page and in the emailed link (plain text, up to 500 characters,
    /// no web addresses).
    pub message: Option<String>,
    /// Shown after a successful payment (plain text, up to 300 characters, no web addresses).
    pub thank_you: Option<String>,
    /// Where a "Back to the business" button on the paid page points. https only; when the business listed
    /// `return_url_origins` in its settings, the address must be on one of them.
    pub return_url: Option<String>,
    /// Show the business's logo and accent colour (from the email look in settings) on the payer's pages.
    pub use_branding: Option<bool>,
    /// Only these methods are offered for payments made through this link: `upi`, `card`, `netbanking`, `wallet`.
    pub allowed_methods: Option<Vec<String>>,
    /// Your own reference (up to 64 characters). Unique per business: creating again with the same reference
    /// returns the first link instead of a second one.
    pub external_ref: Option<String>,
    /// The payer may pay in parts. Needs `min_part_minor`; not with open amounts, `max_uses` or splits.
    pub allow_partial: Option<bool>,
    /// The smallest part a payer may pay (the last part may be smaller, being whatever is left).
    pub min_part_minor: Option<i64>,
    /// Remind the customer (email, and SMS when it is switched on) this many days after their last part, while a
    /// balance is left, up to three times. Needs `customer_id`.
    pub balance_reminder_days: Option<i32>,
}

pub const METHODS: [&str; 4] = ["upi", "card", "netbanking", "wallet"];

/// The business's own words for a link: control characters dropped, blank lines squeezed, length limited and no web
/// addresses (a payment page is a poor place for a link to somewhere else). It is stored as typed otherwise and
/// always shown as text, never as markup.
pub fn clean_text(raw: &str, max: usize, field: &'static str) -> Result<Option<String>, ApiError> {
    let t: String = raw
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    let mut out = String::new();
    let mut newlines = 0;
    for c in t.trim().chars() {
        if c == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(c);
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        return Ok(None);
    }
    let lower = out.to_lowercase();
    if lower.contains("://") || lower.contains("www.") || lower.contains("javascript:") {
        return Err(ApiError::invalid(
            "invalid_text",
            format!("{field} cannot contain web addresses"),
        ));
    }
    if out.chars().count() > max {
        return Err(ApiError::invalid(
            "invalid_text",
            format!("{field} is at most {max} characters"),
        ));
    }
    Ok(Some(out))
}

pub struct Created {
    pub id: String,
    pub token: String,
    pub currency: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// True when `external_ref` matched a link that already existed.
    pub existing: bool,
}

/// Check a request and insert the link in the caller's transaction. Nothing is committed here, so a bulk upload can
/// make hundreds of links and still be all or nothing.
pub async fn create_in_tx(
    s: &AppState,
    tx: &mut sqlx::PgConnection,
    auth: &Auth,
    b: &NewLink,
) -> Result<Created, ApiError> {
    let open = b.open_amount.unwrap_or(false);
    if open {
        let limit = 1..=zyvor_core::MAX_AMOUNT_MINOR;
        let (lo, hi) = (b.min_amount_minor, b.max_amount_minor);
        if b.amount_minor.is_some()
            || b.invoice_id.is_some()
            || b.splits.is_some()
            || b.convenience_fee_bp.unwrap_or(0) != 0
            || lo.is_some_and(|v| !limit.contains(&v))
            || hi.is_some_and(|v| !limit.contains(&v))
            || lo.zip(hi).is_some_and(|(l, h)| l > h)
        {
            return Err(ApiError::invalid(
                "invalid_link",
                "an open-amount link takes only optional min and max amounts (min not above max), and no amount, invoice, splits or fee",
            ));
        }
    } else if b.amount_minor.is_some() == b.invoice_id.is_some()
        || b.min_amount_minor.is_some()
        || b.max_amount_minor.is_some()
    {
        return Err(ApiError::invalid(
            "invalid_link",
            "give exactly one of amount_minor or invoice_id (or open_amount)",
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
    if b.remind_before_expiry.unwrap_or(false)
        && (b.customer_id.is_none() || b.expires_at.is_none())
    {
        return Err(ApiError::invalid(
            "invalid_link",
            "remind_before_expiry needs both customer_id and expires_at",
        ));
    }
    if let (Some(sp), Some(total)) = (&b.splits, b.amount_minor) {
        crate::splits::validate(sp, total)?;
    }
    // partial payments
    let partial = b.allow_partial.unwrap_or(false);
    if partial {
        let min = b.min_part_minor.ok_or_else(|| {
            ApiError::invalid(
                "invalid_partial",
                "a link that can be paid in parts needs min_part_minor",
            )
        })?;
        if open || b.max_uses.is_some() || b.splits.is_some() {
            return Err(ApiError::invalid(
                "invalid_partial",
                "paying in parts does not combine with open amounts, max_uses or splits",
            ));
        }
        if min <= 0 || b.amount_minor.is_some_and(|a| min > a) {
            return Err(ApiError::invalid(
                "invalid_partial",
                "min_part_minor must be positive and not above the amount",
            ));
        }
        if b.balance_reminder_days
            .is_some_and(|d| !(1..=60).contains(&d))
            || (b.balance_reminder_days.is_some() && b.customer_id.is_none())
        {
            return Err(ApiError::invalid(
                "invalid_partial",
                "balance_reminder_days is 1-60 and needs customer_id",
            ));
        }
    } else if b.min_part_minor.is_some() || b.balance_reminder_days.is_some() {
        return Err(ApiError::invalid(
            "invalid_partial",
            "min_part_minor and balance_reminder_days need allow_partial",
        ));
    }
    // presentation
    let message = match &b.message {
        Some(m) => clean_text(m, 500, "message")?,
        None => None,
    };
    let thank_you = match &b.thank_you {
        Some(m) => clean_text(m, 300, "thank_you")?,
        None => None,
    };
    let return_url = b
        .return_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty());
    if let Some(u) = return_url {
        if !u.starts_with("https://") || u.chars().count() > 500 {
            return Err(ApiError::invalid(
                "invalid_return_url",
                "return_url must be an https address of at most 500 characters",
            ));
        }
        crate::ssrf::check_url(u, s.allow_private_targets)
            .map_err(|e| ApiError::invalid("invalid_return_url", e))?;
        crate::payments::check_return_origin(&mut *tx, &auth.tenant_id, u).await?;
    }
    let methods: Option<Vec<String>> = match &b.allowed_methods {
        None => None,
        Some(v) => {
            let mut out: Vec<String> = Vec::new();
            for m in v {
                let m = m.trim().to_lowercase();
                if !METHODS.contains(&m.as_str()) {
                    return Err(ApiError::invalid(
                        "invalid_methods",
                        "allowed_methods are upi, card, netbanking, wallet",
                    ));
                }
                if !out.contains(&m) {
                    out.push(m);
                }
            }
            if out.is_empty() {
                return Err(ApiError::invalid(
                    "invalid_methods",
                    "allowed_methods cannot be empty; leave it out to allow every method",
                ));
            }
            Some(out)
        }
    };
    let external_ref = b
        .external_ref
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if external_ref.is_some_and(|r| {
        r.chars().count() > 64 || !r.chars().all(|c| c.is_ascii_graphic() && c != '"')
    }) {
        return Err(ApiError::invalid(
            "invalid_external_ref",
            "external_ref is up to 64 visible characters",
        ));
    }

    if let Some(r) = external_ref {
        let found: Option<(String, String, String, Option<DateTime<Utc>>, Option<i64>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, token, currency, expires_at, amount_minor, invoice_id, customer_id, description FROM payment_links WHERE external_ref = $1",
        )
        .bind(r)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((id, token, currency, expires_at, amount, invoice, customer, description)) =
            found
        {
            let desc = b
                .description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            if amount != b.amount_minor
                || invoice != b.invoice_id
                || customer != b.customer_id
                || description.as_deref() != desc
            {
                return Err(ApiError::conflict(
                    "external_ref_conflict",
                    "a link with this external_ref already exists with a different amount, description or customer",
                ));
            }
            return Ok(Created {
                id,
                token,
                currency,
                expires_at,
                existing: true,
            });
        }
    }
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
        if partial
            && let Some(total) =
                sqlx::query_scalar::<_, i64>("SELECT amount_minor FROM invoices WHERE id = $1")
                    .bind(inv)
                    .fetch_optional(&mut *tx)
                    .await?
            && b.min_part_minor.is_some_and(|m| m > total)
        {
            return Err(ApiError::invalid(
                "invalid_partial",
                "min_part_minor must not be above the invoice amount",
            ));
        }
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
    if let Some(c) = &b.customer_id {
        // foreign keys ignore row-level security, so make sure the customer is this tenant's
        let seen: Option<String> = sqlx::query_scalar("SELECT id FROM customers WHERE id = $1")
            .bind(c)
            .fetch_optional(&mut *tx)
            .await?;
        seen.ok_or_else(|| ApiError::not_found("customer"))?;
    }
    let (branch, counter) = crate::counters::resolve(
        &mut *tx,
        auth.pin_branch(b.branch_id.clone())?,
        b.counter_id.as_deref(),
    )
    .await?;
    sqlx::query("INSERT INTO payment_links (id, tenant_id, token, description, amount_minor, currency, invoice_id, customer_id, max_uses, expires_at, created_by, splits, convenience_fee_bp, branch_id, counter_id, open_amount, min_amount_minor, max_amount_minor, remind_before_expiry, message, thank_you, return_url, use_branding, allowed_methods, external_ref, allow_partial, min_part_minor, balance_reminder_days) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28)")
        .bind(&id).bind(&auth.tenant_id).bind(&token).bind(&b.description).bind(b.amount_minor).bind(&currency)
        .bind(&b.invoice_id).bind(&b.customer_id).bind(b.max_uses).bind(b.expires_at).bind(&auth.actor).bind(b.splits.as_ref().map(|v| json!(v.iter().map(crate::splits::SplitIn::to_json).collect::<Vec<_>>())))
        .bind(b.convenience_fee_bp.unwrap_or(0))
        .bind(&branch).bind(&counter).bind(open).bind(b.min_amount_minor).bind(b.max_amount_minor).bind(b.remind_before_expiry.unwrap_or(false))
        .bind(&message).bind(&thank_you).bind(return_url).bind(b.use_branding.unwrap_or(false)).bind(&methods).bind(external_ref)
        .bind(partial).bind(b.min_part_minor).bind(b.balance_reminder_days)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_foreign_key_violation() => ApiError::not_found("customer"),
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("external_ref_conflict", "a link with this external_ref was created at the same moment; send the request again"),
            e => e.into(),
        })?;
    tenant::audit(&mut *tx, &auth.tenant_id, &auth.actor, "payment_link.created", Some(&id), json!({ "amount_minor": b.amount_minor, "invoice_id": b.invoice_id, "max_uses": b.max_uses, "partial": partial, "external_ref": external_ref })).await?;
    Ok(Created {
        id,
        token,
        currency,
        expires_at: b.expires_at,
        existing: false,
    })
}

pub async fn create_link(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewLink>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require_branch("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let c = create_in_tx(&s, &mut tx, &auth, &b).await?;
    tx.commit().await?;
    Ok((
        if c.existing {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(
            json!({ "id": c.id, "token": c.token, "currency": c.currency, "path": format!("/pay/link/{}/{}", auth.tenant_id, c.token), "max_uses": b.max_uses, "expires_at": c.expires_at, "existing": c.existing }),
        ),
    ))
}

pub async fn list_links(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, Option<i64>, String, Option<String>, Option<i32>, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, bool, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, token, description, amount_minor, currency, invoice_id, max_uses, uses, expires_at, canceled_at, created_at, COALESCE((SELECT c.email LIKE '%@%' FROM customers c WHERE c.id = payment_links.customer_id), false), external_ref, allow_partial FROM payment_links ORDER BY created_at DESC, id DESC LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, token, description, amount, currency, invoice, max, uses, exp, canc, at, can_email, ext, partial)| {
        let status = state(canc, exp, max, uses);
        json!({ "id": id, "path": format!("/pay/link/{}/{token}", auth.tenant_id), "description": description, "amount_minor": amount, "currency": currency, "invoice_id": invoice, "max_uses": max, "uses": uses, "expires_at": exp, "status": status, "created_at": at, "can_email": can_email, "external_ref": ext, "allow_partial": partial })
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

/// The token of an active payment link for an invoice (what is still owed on it), made on the spot when there is none:
/// what an emailed invoice's "Pay now" button points at. An existing link that is still usable is reused, so
/// every email for the invoice carries the same address.
pub async fn ensure_invoice_link(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    invoice_id: &str,
    customer_id: Option<&str>,
    currency: &str,
) -> Result<String, sqlx::Error> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT token FROM payment_links WHERE invoice_id = $1 AND canceled_at IS NULL
           AND (expires_at IS NULL OR expires_at > now()) AND (max_uses IS NULL OR uses < max_uses)
         ORDER BY created_at LIMIT 1",
    )
    .bind(invoice_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(t) = existing {
        return Ok(t);
    }
    let token = format!(
        "plink_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query("INSERT INTO payment_links (id, tenant_id, token, currency, invoice_id, customer_id, created_by) VALUES ($1, $2, $3, $4, $5, $6, 'system:invoice_email')")
        .bind(new_id("plk"))
        .bind(tenant_id)
        .bind(&token)
        .bind(currency)
        .bind(invoice_id)
        .bind(customer_id)
        .execute(&mut *conn)
        .await?;
    Ok(token)
}

/// What sharing a link needs: the link as the payer will meet it, its address and who it was made for.
struct Share {
    url: String,
    merchant: String,
    description: Option<String>,
    amount_text: Option<String>,
    min_part_text: Option<String>,
    message: Option<String>,
    expires_on: Option<chrono::NaiveDate>,
    customer_id: Option<String>,
    customer: Option<(String, Option<String>, Option<String>)>,
}

/// Load an active link of this business for sharing. 409 when it is canceled, expired, used up or already paid.
async fn load_share(
    s: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: &str,
    id: &str,
) -> Result<Share, ApiError> {
    let base = s.checkout_public_url.clone().ok_or_else(|| {
        ApiError::conflict(
            "checkout_url_not_configured",
            "set ZYVOR_CHECKOUT_PUBLIC_URL to your public checkout address",
        )
    })?;
    let row: Option<(String, Option<String>, Option<i64>, String, Option<String>, Option<i32>, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, bool, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT token, description, amount_minor, currency, customer_id, max_uses, uses, expires_at, canceled_at, message, allow_partial, min_part_minor, invoice_id FROM payment_links WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (
        token,
        description,
        amount,
        currency,
        customer_id,
        max,
        uses,
        exp,
        canc,
        message,
        partial,
        min_part,
        invoice,
    ) = row.ok_or_else(|| ApiError::not_found("payment link"))?;
    if state(canc, exp, max, uses) != "active" {
        return Err(ApiError::conflict(
            "link_unavailable",
            "this payment link is no longer active",
        ));
    }
    let cur: Currency = currency.parse().map_err(|_| ApiError::internal())?;
    let mut amount_text = amount.map(|a| crate::ai::money(a, cur.code()));
    if partial {
        let b = balance(&mut *tx, id, amount, invoice.as_deref()).await?;
        if b.owed() <= 0 {
            return Err(ApiError::conflict(
                "link_unavailable",
                "this payment link has been paid in full",
            ));
        }
        // what is left, not what the link started at
        amount_text = Some(crate::ai::money(b.owed(), cur.code()));
    }
    let customer: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT name, email, phone FROM customers WHERE id = $1")
            .bind(customer_id.as_deref())
            .fetch_optional(&mut *tx)
            .await?;
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_else(|| "the business".into());
    Ok(Share {
        url: format!(
            "{}/pay/link/{tenant_id}/{token}",
            base.trim_end_matches('/')
        ),
        merchant,
        description,
        amount_text,
        min_part_text: if partial {
            min_part.map(|m| crate::ai::money(m, cur.code()))
        } else {
            None
        },
        message,
        expires_on: exp.map(|e| {
            e.with_timezone(&chrono::FixedOffset::east_opt(19_800).expect("IST"))
                .date_naive()
        }),
        customer_id,
        customer,
    })
}

/// The words a person can paste into any chat or message: what it is, who from, the address, and a plain caution.
/// No urgency and no promises; the amount and expiry are stated only when the link has them.
pub fn share_text(
    sh_merchant: &str,
    description: Option<&str>,
    amount: Option<&str>,
    min_part: Option<&str>,
    expires_on: Option<chrono::NaiveDate>,
    url: &str,
) -> String {
    let mut lines = vec![match (amount, description) {
        (Some(a), Some(d)) => format!("{sh_merchant} is asking you to pay {a} for {d}."),
        (Some(a), None) => format!("{sh_merchant} is asking you to pay {a}."),
        (None, Some(d)) => format!("{sh_merchant} has sent you a payment link for {d}."),
        (None, None) => format!("{sh_merchant} has sent you a payment link."),
    }];
    if let Some(m) = min_part {
        lines.push(format!("You can pay in parts, at least {m} each time."));
    }
    lines.push(format!("Pay here: {url}"));
    if let Some(e) = expires_on {
        lines.push(format!("The link works until {e}."));
    }
    lines.push(format!(
        "Pay only if you were expecting this from {sh_merchant}."
    ));
    lines.join("\n")
}

/// A text-message version: one short paragraph. Always carries the address and the caution.
pub fn share_sms(sh_merchant: &str, amount: Option<&str>, url: &str) -> String {
    let merchant: String = sh_merchant.chars().take(40).collect();
    match amount {
        Some(a) => {
            format!("{merchant}: payment link for {a}. {url} Pay only if you expected this.")
        }
        None => format!("{merchant}: payment link. {url} Pay only if you expected this."),
    }
}

/// `GET /v1/payment_links/{id}/share`: the link's address and ready-to-paste wording (message, text message, email
/// subject). Copying the address is the console's job; this is the text around it.
pub async fn share_link(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sh = load_share(&s, &mut tx, &auth.tenant_id, &id).await?;
    Ok(Json(json!({
        "id": id, "url": sh.url,
        "text": share_text(&sh.merchant, sh.description.as_deref(), sh.amount_text.as_deref(), sh.min_part_text.as_deref(), sh.expires_on, &sh.url),
        "sms_text": share_sms(&sh.merchant, sh.amount_text.as_deref(), &sh.url),
        "email_subject": match &sh.amount_text { Some(a) => format!("{} asks you to pay {a}", sh.merchant), None => format!("{} sent you a payment link", sh.merchant) },
        "qr_path": format!("/v1/payment_links/{id}/qr.svg"),
        "can_email": sh.customer.as_ref().is_some_and(|c| c.1.as_deref().is_some_and(|e| e.contains('@'))),
        "can_sms": s.sms.is_on() && sh.customer.as_ref().is_some_and(|c| c.2.as_deref().and_then(crate::sms::normalise_phone).is_some()),
        "sms_mode": s.sms.name(),
    })))
}

/// `POST /v1/payment_links/{id}/send`: email the link to the customer it was made for. A person's action, not a
/// setting: it needs an active link, a customer with an email address and `ZYVOR_CHECKOUT_PUBLIC_URL`, and goes
/// once per link per day (a second try that day is a 409).
pub async fn send_link(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sh = load_share(&s, &mut tx, &auth.tenant_id, &id).await?;
    let (name, email) = match &sh.customer {
        Some((n, Some(e), _)) if e.contains('@') => (n.clone(), e.clone()),
        _ => {
            return Err(ApiError::conflict(
                "no_customer_email",
                "this link has no customer with an email address",
            ));
        }
    };
    let (subject, body) = crate::mail::payment_link(
        &sh.merchant,
        &name,
        sh.description.as_deref(),
        sh.amount_text.as_deref(),
        &sh.url,
        sh.expires_on,
        sh.message.as_deref(),
        sh.min_part_text.as_deref(),
    );
    let fresh = crate::mail::enqueue_once(
        &mut tx,
        &auth.tenant_id,
        &format!("link_send:{id}:{}", crate::ist_today()),
        &email,
        &subject,
        &body,
    )
    .await?;
    if !fresh {
        return Err(ApiError::conflict(
            "already_sent_today",
            "this link was already emailed today",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_link.sent",
        Some(&id),
        json!({ "customer_id": sh.customer_id }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": id, "queued": true })),
    ))
}

/// `POST /v1/payment_links/{id}/send_sms`: text the link to the customer's phone number, once per link per day. Needs
/// SMS switched on (`ZYVOR_SMS_MODE`): in `sandbox` the text is only logged, in `hook` it goes to the configured
/// hook. A 409 `sms_not_configured` when SMS is off.
pub async fn send_link_sms(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    if !s.sms.is_on() {
        return Err(ApiError::conflict(
            "sms_not_configured",
            "SMS is switched off on this server (ZYVOR_SMS_MODE)",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sh = load_share(&s, &mut tx, &auth.tenant_id, &id).await?;
    let phone = sh
        .customer
        .as_ref()
        .and_then(|c| c.2.as_deref())
        .and_then(crate::sms::normalise_phone)
        .ok_or_else(|| {
            ApiError::conflict(
                "no_customer_phone",
                "this link has no customer with a readable phone number",
            )
        })?;
    let text = share_sms(&sh.merchant, sh.amount_text.as_deref(), &sh.url);
    let q = crate::sms::enqueue(
        &mut tx,
        &s.sms,
        &auth.tenant_id,
        &phone,
        &text,
        &format!("link_sms:{id}:{}", crate::ist_today()),
    )
    .await?;
    if q == Some(crate::sms::Queued::Duplicate) {
        return Err(ApiError::conflict(
            "already_sent_today",
            "this link was already texted today",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_link.sms_sent",
        Some(&id),
        json!({ "customer_id": sh.customer_id, "mode": s.sms.name() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": id, "queued": true, "mode": s.sms.name() })),
    ))
}

/// The worker's pass over one tenant: email the customer of every link that asked for it and expires within a
/// day. Once per link (`expiry_reminded_at`, plus a unique mail key). Returns how many were queued.
/// `as_of` is the organisation's "now" (test clock when set); compared in Asia/Kolkata.
pub async fn remind_expiring(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    checkout_base: &str,
    as_of: chrono::NaiveDate,
) -> Result<usize, ApiError> {
    let as_of_ts = as_of
        .and_hms_opt(12, 0, 0)
        .expect("noon")
        .and_local_timezone(chrono::FixedOffset::east_opt(19_800).expect("IST"))
        .single()
        .ok_or_else(ApiError::internal)?
        .with_timezone(&Utc);
    #[allow(clippy::type_complexity)]
    let due: Vec<(String, String, Option<String>, Option<i64>, String, DateTime<Utc>, String, String)> = sqlx::query_as(
        "SELECT l.id, l.token, l.description, l.amount_minor, l.currency, l.expires_at, c.name, c.email
           FROM payment_links l JOIN customers c ON c.id = l.customer_id
          WHERE l.remind_before_expiry AND l.expiry_reminded_at IS NULL AND l.canceled_at IS NULL
            AND l.expires_at > $1 AND l.expires_at <= $1 + interval '24 hours'
            AND (l.max_uses IS NULL OR l.uses < l.max_uses)
            AND (l.invoice_id IS NULL OR invoice_settled(l.invoice_id) < (SELECT amount_minor FROM invoices WHERE id = l.invoice_id))
            AND c.email LIKE '%@%'
          ORDER BY l.expires_at LIMIT 100",
    )
    .bind(as_of_ts)
    .fetch_all(&mut *conn)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or_else(|| "the business".into());
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let mut n = 0;
    for (id, token, description, amount, currency, exp, name, email) in due {
        let cur: Currency = currency.parse().map_err(|_| ApiError::internal())?;
        let amount_text = amount.map(|a| crate::ai::money(a, cur.code()));
        let (subject, body) = crate::mail::payment_link_expiring(
            &merchant,
            &name,
            description.as_deref(),
            amount_text.as_deref(),
            &format!(
                "{}/pay/link/{tenant_id}/{token}",
                checkout_base.trim_end_matches('/')
            ),
            exp.with_timezone(&ist).date_naive(),
        );
        if crate::mail::enqueue_once_as(
            conn,
            tenant_id,
            &format!("link_expiry:{id}"),
            &email,
            &subject,
            &body,
            true,
        )
        .await?
        {
            n += 1;
        }
        sqlx::query("UPDATE payment_links SET expiry_reminded_at = now() WHERE id = $1")
            .bind(&id)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system:worker",
            "payment_link.expiry_reminded",
            Some(&id),
            json!({}),
        )
        .await?;
    }
    Ok(n)
}

/// The most balance reminders a link sends.
pub const MAX_BALANCE_REMINDERS: i32 = 3;

/// The worker's pass over one tenant: for links paid in parts that asked for it, remind the customer of what is still
/// owed `balance_reminder_days` after their last part (or the last reminder), by email and, when SMS is switched on,
/// by text. At most three reminders per link, none once the link is paid, canceled or expired, and none to a customer
/// who unsubscribed (email). `as_of` is the organisation's "now" (test clock when set), compared in Asia/Kolkata.
/// Returns how many links were reminded.
pub async fn remind_balances(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    checkout_base: &str,
    as_of: chrono::NaiveDate,
    sms: &crate::sms::Mode,
) -> Result<usize, ApiError> {
    let as_of_ts = as_of
        .and_hms_opt(12, 0, 0)
        .expect("noon")
        .and_local_timezone(chrono::FixedOffset::east_opt(19_800).expect("IST"))
        .single()
        .ok_or_else(ApiError::internal)?
        .with_timezone(&Utc);
    #[allow(clippy::type_complexity)]
    let due: Vec<(String, String, Option<String>, String, Option<DateTime<Utc>>, Option<i64>, Option<i64>, Option<String>, i32, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT l.id, l.token, l.description, l.currency, l.expires_at, l.min_part_minor, l.amount_minor, l.invoice_id, l.balance_reminders_sent, c.name, c.email, c.phone
           FROM payment_links l JOIN customers c ON c.id = l.customer_id
          WHERE l.allow_partial AND l.balance_reminder_days IS NOT NULL AND l.canceled_at IS NULL
            AND (l.expires_at IS NULL OR l.expires_at > $1) AND l.balance_reminders_sent < $2
            AND COALESCE(l.balance_reminded_at, (SELECT max(i.updated_at) FROM payment_intents i WHERE i.metadata->>'payment_link' = l.id AND i.status = 'succeeded'))
                + make_interval(days => l.balance_reminder_days) <= $1
          ORDER BY l.created_at LIMIT 100",
    )
    .bind(as_of_ts)
    .bind(MAX_BALANCE_REMINDERS)
    .fetch_all(&mut *conn)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or_else(|| "the business".into());
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    let mut n = 0;
    for (
        id,
        token,
        description,
        currency,
        exp,
        min_part,
        amount,
        invoice,
        sent,
        name,
        email,
        phone,
    ) in due
    {
        let bal = balance(&mut *conn, &id, amount, invoice.as_deref()).await?;
        if bal.owed() <= 0 || bal.paid <= 0 {
            // paid in full (or nothing paid yet): nothing to remind about; stop asking about this link
            sqlx::query("UPDATE payment_links SET balance_reminders_sent = $2 WHERE id = $1")
                .bind(&id)
                .bind(MAX_BALANCE_REMINDERS)
                .execute(&mut *conn)
                .await?;
            continue;
        }
        let cur: Currency = currency.parse().map_err(|_| ApiError::internal())?;
        let money = |v: i64| crate::ai::money(v, cur.code());
        let url = format!(
            "{}/pay/link/{tenant_id}/{token}",
            checkout_base.trim_end_matches('/')
        );
        let key = format!("link_balance:{id}:{}", sent + 1);
        let mut delivered = false;
        if let Some(e) = email.as_deref().filter(|e| e.contains('@')) {
            let (subject, body) = crate::mail::payment_link_balance(
                &merchant,
                &name,
                description.as_deref(),
                &money(bal.paid),
                &money(bal.owed()),
                &money(min_part.unwrap_or(1).min(bal.owed())),
                &url,
                exp.map(|e| e.with_timezone(&ist).date_naive()),
            );
            delivered |=
                crate::mail::enqueue_once_as(conn, tenant_id, &key, e, &subject, &body, true)
                    .await?;
        }
        if let Some(p) = phone.as_deref().and_then(crate::sms::normalise_phone) {
            let text = format!(
                "{}: {} paid, {} still left. Pay the rest: {url}",
                merchant.chars().take(40).collect::<String>(),
                money(bal.paid),
                money(bal.owed())
            );
            delivered |= matches!(
                crate::sms::enqueue(conn, sms, tenant_id, &p, &text, &key).await?,
                Some(crate::sms::Queued::Queued | crate::sms::Queued::Logged)
            );
        }
        sqlx::query("UPDATE payment_links SET balance_reminders_sent = balance_reminders_sent + 1, balance_reminded_at = $2 WHERE id = $1")
            .bind(&id)
            .bind(as_of_ts)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system:worker",
            "payment_link.balance_reminded",
            Some(&id),
            json!({ "reminder": sent + 1, "owed_minor": bal.owed(), "delivered": delivered }),
        )
        .await?;
        if delivered {
            n += 1;
        }
    }
    Ok(n)
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

#[derive(sqlx::FromRow)]
struct Link {
    id: String,
    description: Option<String>,
    amount_minor: Option<i64>,
    currency: String,
    invoice_id: Option<String>,
    customer_id: Option<String>,
    max_uses: Option<i32>,
    uses: i32,
    expires_at: Option<DateTime<Utc>>,
    canceled_at: Option<DateTime<Utc>>,
    open_amount: bool,
    min_amount_minor: Option<i64>,
    max_amount_minor: Option<i64>,
    convenience_fee_bp: i32,
    message: Option<String>,
    use_branding: bool,
    allowed_methods: Option<Vec<String>>,
    allow_partial: bool,
    min_part_minor: Option<i64>,
    splits: Option<Value>,
}

impl Link {
    fn state(&self) -> &'static str {
        state(self.canceled_at, self.expires_at, self.max_uses, self.uses)
    }
}

const LINK_COLS: &str = "id, description, amount_minor, currency, invoice_id, customer_id, max_uses, uses, expires_at, canceled_at, open_amount, min_amount_minor, max_amount_minor, convenience_fee_bp, message, use_branding, allowed_methods, allow_partial, min_part_minor, splits";

async fn load(tx: &mut sqlx::PgConnection, token: &str, lock: bool) -> Result<Link, ApiError> {
    let sql = format!(
        "SELECT {LINK_COLS} FROM payment_links WHERE token = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(token)
        .fetch_optional(tx)
        .await?
        .ok_or_else(|| ApiError::not_found("payment link"))
}

/// How long a started-but-unfinished checkout holds back its amount from other payers of a part-paid link.
const HOLD_MINUTES: i32 = 30;

/// Where a part-paid link stands. `total` is what the link is for, `paid` what verified provider results say was
/// captured through it, `held` what checkouts started in the last half hour might still capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Balance {
    pub total: i64,
    pub paid: i64,
    pub held: i64,
}

impl Balance {
    /// Still owed.
    pub fn owed(&self) -> i64 {
        (self.total - self.paid).max(0)
    }
    /// Owed and not already being paid by someone else.
    pub fn available(&self) -> i64 {
        (self.owed() - self.held).max(0)
    }
    /// Captured beyond the total (two payers paying the same balance); the business refunds the excess.
    pub fn overpaid(&self) -> i64 {
        (self.paid - self.total).max(0)
    }
}

/// The balance of a link: captured parts are summed from payments marked succeeded by the normal capture path (a
/// provider's verified result), never from what a browser reports. An invoice link takes what is settled on the
/// invoice, which also counts payments made by other means.
pub async fn balance(
    conn: &mut sqlx::PgConnection,
    link_id: &str,
    amount: Option<i64>,
    invoice: Option<&str>,
) -> Result<Balance, ApiError> {
    let (by_link, held): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(CASE WHEN status = 'succeeded' THEN (metadata->>'base_minor')::bigint END), 0)::BIGINT,
                COALESCE(SUM(CASE WHEN status IN ('requires_payment_method', 'processing') AND created_at > now() - make_interval(mins => $2) THEN (metadata->>'base_minor')::bigint END), 0)::BIGINT
           FROM payment_intents WHERE metadata->>'payment_link' = $1",
    )
    .bind(link_id)
    .bind(HOLD_MINUTES)
    .fetch_one(&mut *conn)
    .await?;
    let (total, paid) = match invoice {
        Some(inv) => {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT amount_minor, invoice_settled(id)::BIGINT FROM invoices WHERE id = $1",
            )
            .bind(inv)
            .fetch_one(&mut *conn)
            .await?
        }
        None => (amount.unwrap_or(0), by_link),
    };
    Ok(Balance { total, paid, held })
}

/// The business's logo and accent colour for the payer's pages, when it has set them. The colour is only used when it
/// can carry text and stand out from the white card (button text 4.5:1, fill against the card 3:1); otherwise the
/// page keeps its own colour. Text on the button is chosen here so the page never has to guess.
pub async fn brand(conn: &mut sqlx::PgConnection, tenant_id: &str) -> Option<Value> {
    let (accent, logo): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT email_accent, email_logo_url FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten()?;
    brand_from(accent.as_deref(), logo.as_deref())
}

pub fn brand_from(accent: Option<&str>, logo: Option<&str>) -> Option<Value> {
    let accent = accent
        .and_then(crate::mail::clean_accent)
        .filter(|a| crate::mail::contrast_ratio(a, "#ffffff") >= 3.0)
        .map(|a| {
            let on = crate::mail::text_on_accent(&a);
            json!({ "color": a, "on_color": on })
        })
        .filter(|v| {
            crate::mail::contrast_ratio(
                v["color"].as_str().unwrap_or("#000000"),
                v["on_color"].as_str().unwrap_or("#ffffff"),
            ) >= 4.5
        });
    let logo = logo.and_then(crate::mail::clean_logo_url);
    if accent.is_none() && logo.is_none() {
        return None;
    }
    Some(json!({ "accent": accent, "logo_url": logo }))
}

/// What the checkout page shows for a payment that was made through a link: the link's note, thank-you, return
/// address, brand and balance. `None` for a payment that did not come from a link.
pub async fn presentation(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    intent_id: &str,
) -> Result<Option<Value>, ApiError> {
    let link_id: Option<String> =
        sqlx::query_scalar("SELECT metadata->>'payment_link' FROM payment_intents WHERE id = $1")
            .bind(intent_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let Some(link_id) = link_id else {
        return Ok(None);
    };
    #[allow(clippy::type_complexity)]
    let row: Option<(Option<String>, Option<String>, Option<String>, bool, Option<Vec<String>>, bool, Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT message, thank_you, return_url, use_branding, allowed_methods, allow_partial, amount_minor, min_part_minor, invoice_id FROM payment_links WHERE id = $1",
    )
    .bind(&link_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((
        message,
        thank_you,
        return_url,
        branded,
        methods,
        partial,
        amount,
        min_part,
        invoice,
    )) = row
    else {
        return Ok(None);
    };
    let bal = if partial {
        let b = balance(&mut *conn, &link_id, amount, invoice.as_deref()).await?;
        Some(
            json!({ "total_minor": b.total, "paid_minor": b.paid, "balance_minor": b.owed(), "min_part_minor": min_part }),
        )
    } else {
        None
    };
    Ok(Some(json!({
        "message": message, "thank_you": thank_you, "return_url": return_url,
        "brand": if branded { brand(&mut *conn, tenant_id).await } else { None },
        "allowed_methods": methods, "partial": bal,
    })))
}

/// Public: what the page shows before the payer commits to anything.
#[derive(Deserialize, Default)]
pub struct CouponQuery {
    coupon: Option<String>,
}

pub async fn link_info(
    State(s): State<AppState>,
    parts: axum::http::request::Parts,
    Path((tenant_id, token)): Path<(String, String)>,
    Query(q): Query<CouponQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let l = load(&mut tx, &token, false).await?;
    let bal = if l.allow_partial {
        Some(balance(&mut tx, &l.id, l.amount_minor, l.invoice_id.as_deref()).await?)
    } else {
        None
    };
    let amount =
        match (&bal, l.amount_minor, &l.invoice_id) {
            (Some(b), _, _) => b.owed(),
            (None, Some(a), _) => a,
            (None, None, Some(inv)) => sqlx::query_scalar(
                "SELECT (amount_minor - invoice_settled(id))::BIGINT FROM invoices WHERE id = $1",
            )
            .bind(inv)
            .fetch_one(&mut *tx)
            .await?,
            _ => 0,
        };
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&tenant_id).fetch_optional(&mut *tx).await?;
    let mut st = l.state();
    if st == "active" && amount <= 0 && !l.open_amount {
        st = "paid";
    }
    if st == "active" && bal.is_some_and(|b| b.available() <= 0) {
        st = "in_progress";
    }
    // what the payer will actually be asked for, with any coupon they typed
    let code = q.coupon.as_deref().map(str::trim).filter(|c| !c.is_empty());
    if (l.open_amount || l.allow_partial) && code.is_some() {
        return Err(ApiError::invalid(
            "coupon_not_applicable",
            "coupons apply to fixed-amount links that are paid in one go",
        ));
    }
    let applied = match code {
        Some(c) if l.invoice_id.is_none() => Some(crate::coupons::find(&mut tx, c, false).await?),
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
        l.convenience_fee_bp as i64,
    );
    let brand = if l.use_branding {
        brand(&mut tx, &tenant_id).await
    } else {
        None
    };
    let partial = bal.map(|b| {
        let avail = b.available();
        json!({ "total_minor": b.total, "paid_minor": b.paid, "balance_minor": b.owed(), "available_minor": avail, "min_part_minor": l.min_part_minor.map(|m| m.min(avail.max(1))) })
    });
    let link_id = l.id.clone();
    drop(tx);
    // a visit with a coupon typed is the same visit, not a new one
    if code.is_none() {
        crate::link_stats::record(
            &s,
            &tenant_id,
            &link_id,
            crate::link_stats::Event::Opened,
            &parts,
        )
        .await;
    }
    Ok(Json(
        json!({ "merchant": merchant, "description": l.description, "amount_minor": amount, "discount_minor": p.discount, "fee_minor": p.fee, "total_minor": p.total, "coupon_applied": applied.is_some(), "currency": l.currency, "status": st, "open_amount": l.open_amount, "min_amount_minor": l.min_amount_minor, "max_amount_minor": l.max_amount_minor, "message": l.message, "brand": brand, "allowed_methods": l.allowed_methods, "partial": partial }),
    ))
}

/// Public: open the link. Mints the payer's own payment intent and returns the pieces the checkout
/// page needs. Counts toward `max_uses` immediately, so a capped link can never over-sell.
#[derive(Deserialize, Default)]
pub struct StartBody {
    /// A coupon code the payer typed (fixed-amount links only).
    coupon: Option<String>,
    /// What the payer chose to pay (open-amount links, and links that can be paid in parts), in minor units.
    amount_minor: Option<i64>,
}

pub async fn link_start(
    State(s): State<AppState>,
    parts: axum::http::request::Parts,
    Path((tenant_id, token)): Path<(String, String)>,
    body: Option<Json<StartBody>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let chosen = body.amount_minor;
    let coupon_code = body
        .coupon
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let l = load(&mut tx, &token, true).await?;
    let (link_id, description, amount, currency, invoice, customer) = (
        l.id.clone(),
        l.description.clone(),
        l.amount_minor,
        l.currency.clone(),
        l.invoice_id.clone(),
        l.customer_id.clone(),
    );
    match l.state() {
        "active" => {}
        other => {
            return Err(ApiError::conflict(
                "link_unavailable",
                format!("this payment link is {other}"),
            ));
        }
    }
    let (open, lo, hi) = (l.open_amount, l.min_amount_minor, l.max_amount_minor);
    if !open && !l.allow_partial && chosen.is_some() {
        return Err(ApiError::invalid(
            "amount_not_allowed",
            "this link has a fixed price",
        ));
    }
    if l.allow_partial && coupon_code.is_some() {
        return Err(ApiError::invalid(
            "coupon_not_applicable",
            "coupons apply to fixed-amount links that are paid in one go",
        ));
    }
    let amount = if l.allow_partial {
        // The link row is locked above, so two payers cannot both claim the same balance.
        if let Some(inv) = &invoice {
            let status: String =
                sqlx::query_scalar("SELECT status FROM invoices WHERE id = $1 FOR SHARE")
                    .bind(inv)
                    .fetch_one(&mut *tx)
                    .await?;
            if matches!(status.as_str(), "void" | "paid") {
                return Err(ApiError::conflict(
                    "link_unavailable",
                    "this invoice has nothing left to pay",
                ));
            }
        }
        let b = balance(&mut tx, &link_id, amount, invoice.as_deref()).await?;
        if b.owed() <= 0 {
            return Err(ApiError::conflict(
                "link_unavailable",
                "this link has been paid in full",
            ));
        }
        let avail = b.available();
        if avail <= 0 {
            return Err(ApiError::conflict(
                "link_busy",
                "other payments on this link are in progress; try again in a few minutes",
            ));
        }
        let min = l.min_part_minor.unwrap_or(1).min(avail);
        let a = chosen.unwrap_or(avail);
        if a < min || a > avail {
            return Err(ApiError::invalid(
                "amount_out_of_range",
                format!("the amount must be between {min} and {avail} minor units"),
            ));
        }
        a
    } else {
        match (amount, &invoice) {
            (Some(a), _) => a,
            (None, None) if open => {
                let a = chosen.ok_or_else(|| {
                    ApiError::invalid("amount_required", "enter the amount you want to pay")
                })?;
                let (min, max) = (lo.unwrap_or(1), hi.unwrap_or(zyvor_core::MAX_AMOUNT_MINOR));
                if a < min || a > max {
                    return Err(ApiError::invalid(
                        "amount_out_of_range",
                        format!("the amount must be between {min} and {max} minor units"),
                    ));
                }
                a
            }
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
        }
    };
    // price: coupon (spent now, in this transaction) then the link's convenience fee
    let fee_bp = l.convenience_fee_bp;
    if open && coupon_code.is_some() {
        return Err(ApiError::invalid(
            "coupon_not_applicable",
            "coupons apply to fixed-amount links",
        ));
    }
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
    .bind(&description).bind(json!({ "payment_link": link_id, "base_minor": base_amount, "discount_minor": p.discount, "fee_minor": p.fee, "coupon_id": spent.as_ref().map(|c| c.id.clone()), "part": l.allow_partial }))
    .bind(crate::auth::hash_key(&secret))
    .execute(&mut *tx).await?;
    sqlx::query("UPDATE payment_intents i SET branch_id = l.branch_id, counter_id = l.counter_id FROM payment_links l WHERE l.id = $1 AND i.id = $2 AND l.branch_id IS NOT NULL")
        .bind(&link_id)
        .bind(&intent_id)
        .execute(&mut *tx)
        .await?;
    if let Some(v) = l.splits {
        let mut sp: Vec<crate::splits::SplitIn> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(crate::splits::SplitIn::from_json)
            .collect();
        // a share given as gross_minor takes the commission rule in force today for this link
        crate::commission::resolve_splits(
            &mut tx,
            &mut sp,
            &crate::commission::Ctx {
                currency: &currency,
                payment_link_id: Some(&link_id),
                event_ref: None,
            },
        )
        .await?;
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
    crate::link_stats::record(
        &s,
        &tenant_id,
        &link_id,
        crate::link_stats::Event::Started,
        &parts,
    )
    .await;
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
