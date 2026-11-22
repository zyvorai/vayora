// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! A hosted seller page ("link in bio"): one public page per tenant at a slug, with a short description, an optional
//! logo and a list of items. A visitor picks items and pays through the ordinary payment-intent checkout; the order is
//! recorded, the stock is taken out in the same transaction (so it can never go below zero, however many visitors
//! arrive at once) and the seller gets an order list with fulfilment and refund actions.
//!
//! Safety rules in one place:
//! - Text is stored as typed (control characters and bidi overrides refused, lengths capped) and escaped where it is
//!   shown; there are no user-supplied URLs, so an item image is an uploaded PNG/JPEG/GIF/WebP, recognised by its
//!   bytes and served with `nosniff` and a sandboxing policy.
//! - Stock is held for `HOLD_MINUTES` while the buyer pays. A hold that lapses with no payment in flight is given
//!   back (the intent is closed first, so nobody can pay for stock that has gone back). A payment that arrives for an
//!   order whose stock was already given back wins: the order is marked paid and flagged for the seller to look at.
//! - Buyer contact details are kept only on the order, never put in the audit trail or in logs.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection};
use std::time::Instant;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::{Auth, hash_key},
    ratelimit::{Bucket, client_ip, vouched_client_ip},
    tenant,
};

/// How long the stock of an unpaid order is kept for the buyer.
pub const HOLD_MINUTES: i64 = 30;
const MAX_ITEMS: i64 = 100;
const MAX_IMAGES: i64 = 60;
const MAX_IMAGE_BYTES: usize = 200 * 1024;
const MAX_LINES: usize = 20;
const MAX_QTY: i32 = 20;
const MAX_STOCK: i64 = 1_000_000;
/// The most unpaid orders one page may have open at once, so stock cannot be held hostage without limit.
const MAX_PENDING: i64 = 200;

const RESERVED: &[&str] = &[
    "admin",
    "api",
    "app",
    "apps",
    "assets",
    "auth",
    "billing",
    "blog",
    "cdn",
    "checkout",
    "console",
    "dashboard",
    "docs",
    "donate",
    "help",
    "home",
    "login",
    "logout",
    "mail",
    "new",
    "null",
    "orders",
    "pay",
    "payment",
    "payments",
    "pricing",
    "privacy",
    "refund",
    "refunds",
    "root",
    "security",
    "settings",
    "shop",
    "signup",
    "static",
    "status",
    "store",
    "stores",
    "support",
    "terms",
    "test",
    "undefined",
    "v1",
    "vayora",
    "webhook",
    "webhooks",
    "www",
    "zyvor",
];
/// Names of payment brands and authorities that a page must not borrow.
const BORROWED: &[&str] = &[
    "razorpay",
    "cashfree",
    "payu",
    "paytm",
    "phonepe",
    "npci",
    "bhim",
    "googlepay",
    "zyvor",
    "vayora",
];

// ---------- text rules ----------

fn bad_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}' | '\u{FEFF}')
}

/// Trim, refuse control characters (a line break is allowed only when `multiline`) and cap the length in characters.
pub(crate) fn clean_text(
    raw: &str,
    field: &'static str,
    max: usize,
    multiline: bool,
    required: bool,
) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() && required {
        return Err(ApiError::invalid(
            "invalid_text",
            format!("{field} is required"),
        ));
    }
    if t.chars().count() > max {
        return Err(ApiError::invalid(
            "text_too_long",
            format!("{field} is at most {max} characters"),
        ));
    }
    if t.chars().any(|c| bad_char(c) && !(multiline && c == '\n')) {
        return Err(ApiError::invalid(
            "invalid_text",
            format!("{field} has characters that are not allowed"),
        ));
    }
    Ok(t.to_string())
}

/// A phone number kept as digits with an optional leading plus; 10 to 15 digits.
pub(crate) fn clean_phone(raw: &str) -> Result<String, ApiError> {
    let t: String = raw
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')'))
        .collect();
    let digits = t.strip_prefix('+').unwrap_or(&t);
    if !(10..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ApiError::invalid(
            "invalid_phone",
            "enter a phone number of 10 to 15 digits",
        ));
    }
    Ok(t)
}

pub(crate) fn clean_email(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    let ok = t.len() <= 120
        && !t
            .chars()
            .any(|c| c.is_whitespace() || bad_char(c) || matches!(c, '<' | '>' | ',' | ';' | '"'))
        && t.split_once('@').is_some_and(|(l, d)| {
            !l.is_empty()
                && d.contains('.')
                && !d.contains('@')
                && !d.starts_with('.')
                && !d.ends_with('.')
        });
    if ok {
        Ok(t.to_string())
    } else {
        Err(ApiError::invalid(
            "invalid_email",
            "enter a valid email address",
        ))
    }
}

/// Slug rules: 3 to 40 of a-z, 0-9 and single hyphens, not starting or ending with one, not all digits, not reserved.
pub fn check_slug(slug: &str) -> Result<(), ApiError> {
    let shape = (3..=40).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && !slug.bytes().all(|b| b.is_ascii_digit());
    if !shape {
        return Err(ApiError::invalid(
            "invalid_slug",
            "a slug is 3 to 40 characters: lowercase letters, digits and single hyphens, not all digits",
        ));
    }
    if RESERVED.contains(&slug) || BORROWED.iter().any(|b| slug.contains(b)) {
        return Err(ApiError::invalid(
            "reserved_slug",
            "this name is reserved; pick another",
        ));
    }
    Ok(())
}

fn double_opt<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(d)?))
}

// ---------- seller: the page ----------

#[derive(Deserialize)]
pub struct StorefrontIn {
    /// Needed the first time; it cannot change afterwards (printed links would break).
    slug: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(default, deserialize_with = "double_opt")]
    logo_image_id: Option<Option<String>>,
    /// Ask search engines not to list the page. Default true.
    noindex: Option<bool>,
    published: Option<bool>,
}

async fn page_json(conn: &mut PgConnection, tenant_id: &str) -> Result<Option<Value>, ApiError> {
    let row: Option<(String, String, String, String, Option<String>, bool, bool, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, slug, title, description, logo_image_id, noindex, published, updated_at FROM storefronts",
    )
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|(id, slug, title, description, logo, noindex, published, updated)| {
        json!({ "id": id, "slug": slug, "title": title, "description": description, "logo_image_id": logo, "noindex": noindex,
                "published": published, "path": format!("/s/{slug}"), "updated_at": updated, "tenant_id": tenant_id })
    }))
}

/// `PUT /v1/storefront`: create or update this tenant's page.
pub async fn put_storefront(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<StorefrontIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:write")?;
    let title = clean_text(&b.title, "title", 80, false, true)?;
    let description = b
        .description
        .as_deref()
        .map(|d| clean_text(d, "description", 500, true, false))
        .transpose()?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let existing: Option<(String, String)> = sqlx::query_as("SELECT id, slug FROM storefronts")
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(Some(img)) = &b.logo_image_id {
        let ok: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM storefront_images WHERE id = $1)")
                .bind(img)
                .fetch_one(&mut *tx)
                .await?;
        if !ok {
            return Err(ApiError::not_found("image"));
        }
    }
    match existing {
        None => {
            let slug = b
                .slug
                .as_deref()
                .ok_or_else(|| ApiError::invalid("slug_required", "choose a slug for the page"))?;
            check_slug(slug)?;
            sqlx::query("INSERT INTO storefront_directory (slug, tenant_id) VALUES ($1, $2)")
                .bind(slug)
                .bind(&auth.tenant_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(d) if d.is_unique_violation() => {
                        ApiError::conflict("slug_taken", "this slug is already in use")
                    }
                    e => e.into(),
                })?;
            sqlx::query("INSERT INTO storefronts (id, tenant_id, slug, title, description, logo_image_id, noindex, published) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(new_id("sf"))
                .bind(&auth.tenant_id)
                .bind(slug)
                .bind(&title)
                .bind(description.unwrap_or_default())
                .bind(b.logo_image_id.clone().flatten())
                .bind(b.noindex.unwrap_or(true))
                .bind(b.published.unwrap_or(false))
                .execute(&mut *tx)
                .await?;
        }
        Some((id, slug)) => {
            if b.slug.as_deref().is_some_and(|x| x != slug) {
                return Err(ApiError::conflict(
                    "slug_locked",
                    "the slug cannot be changed once the page exists",
                ));
            }
            sqlx::query(
                "UPDATE storefronts SET title = $2, description = COALESCE($3, description),
                    logo_image_id = CASE WHEN $4 THEN $5 ELSE logo_image_id END,
                    noindex = COALESCE($6, noindex), published = COALESCE($7, published), updated_at = now() WHERE id = $1",
            )
            .bind(&id)
            .bind(&title)
            .bind(description)
            .bind(b.logo_image_id.is_some())
            .bind(b.logo_image_id.clone().flatten())
            .bind(b.noindex)
            .bind(b.published)
            .execute(&mut *tx)
            .await?;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.saved",
        None,
        json!({ "published": b.published }),
    )
    .await?;
    let out = page_json(&mut tx, &auth.tenant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    tx.commit().await?;
    Ok(Json(out))
}

/// `GET /v1/storefront`
pub async fn get_storefront(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let page = page_json(&mut tx, &auth.tenant_id)
        .await?
        .ok_or_else(|| ApiError::not_found("storefront"))?;
    Ok(Json(page))
}

/// `POST /v1/storefront/images`: the body is the image itself (up to 200 KB; PNG, JPEG, GIF or WebP by its bytes).
pub async fn upload_image(
    State(s): State<AppState>,
    auth: Auth,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("storefront:write")?;
    if body.is_empty() || body.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::invalid(
            "invalid_image",
            "the image must be up to 200 KB",
        ));
    }
    let kind = crate::logo::sniff(&body)
        .ok_or_else(|| ApiError::invalid("invalid_image", "send a PNG, JPEG, GIF or WebP image"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM storefront_images")
        .fetch_one(&mut *tx)
        .await?;
    if n >= MAX_IMAGES {
        return Err(ApiError::conflict(
            "too_many_images",
            format!("at most {MAX_IMAGES} images"),
        ));
    }
    let id = new_id("sfi");
    sqlx::query("INSERT INTO storefront_images (id, tenant_id, content_type, size_bytes, data) VALUES ($1,$2,$3,$4,$5)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(kind)
        .bind(body.len() as i32)
        .bind(body.as_ref())
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.image_added",
        Some(&id),
        json!({ "bytes": body.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "content_type": kind, "size_bytes": body.len() })),
    ))
}

// ---------- seller: items ----------

#[derive(Deserialize)]
pub struct StoreItemIn {
    name: String,
    description: Option<String>,
    price_minor: i64,
    image_id: Option<String>,
    /// Omit for an item that is not counted.
    stock: Option<i64>,
    active: Option<bool>,
    position: Option<i32>,
}

#[derive(Deserialize)]
pub struct StoreItemPatch {
    name: Option<String>,
    description: Option<String>,
    price_minor: Option<i64>,
    #[serde(default, deserialize_with = "double_opt")]
    image_id: Option<Option<String>>,
    /// Set the count to this (null: stop counting). Prefer `stock_delta` while orders may arrive.
    #[serde(default, deserialize_with = "double_opt")]
    stock: Option<Option<i64>>,
    /// Add (or take off, when negative) from the count in one step; never goes below zero.
    stock_delta: Option<i64>,
    active: Option<bool>,
    position: Option<i32>,
}

type ItemRow = (
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<i32>,
    bool,
    i32,
);
const ITEM_COLS: &str = "id, name, description, price_minor, image_id, stock, active, position";

fn item_json(r: ItemRow) -> Value {
    json!({ "id": r.0, "name": r.1, "description": r.2, "price_minor": r.3, "image_id": r.4, "stock": r.5, "active": r.6, "position": r.7 })
}

fn check_price(p: i64) -> Result<(), ApiError> {
    if p <= 0 || p > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_price",
            "price_minor must be a positive whole number of paise",
        ));
    }
    Ok(())
}

fn check_stock(v: i64) -> Result<(), ApiError> {
    if !(0..=MAX_STOCK).contains(&v) {
        return Err(ApiError::invalid(
            "invalid_stock",
            format!("stock is a whole number from 0 to {MAX_STOCK}"),
        ));
    }
    Ok(())
}

async fn own_image(conn: &mut PgConnection, id: &str) -> Result<(), ApiError> {
    // foreign keys ignore row-level security, so look the image up as this tenant first
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM storefront_images WHERE id = $1)")
            .bind(id)
            .fetch_one(conn)
            .await?;
    if ok {
        Ok(())
    } else {
        Err(ApiError::not_found("image"))
    }
}

/// `POST /v1/storefront/items`
pub async fn create_item(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<StoreItemIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("storefront:write")?;
    let name = clean_text(&b.name, "name", 80, false, true)?;
    let description = clean_text(
        b.description.as_deref().unwrap_or(""),
        "description",
        300,
        true,
        false,
    )?;
    check_price(b.price_minor)?;
    if let Some(st) = b.stock {
        check_stock(st)?;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sf: String = sqlx::query_scalar("SELECT id FROM storefronts")
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            ApiError::conflict(
                "no_storefront",
                "create the page first (PUT /v1/storefront)",
            )
        })?;
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM storefront_items WHERE storefront_id = $1")
            .bind(&sf)
            .fetch_one(&mut *tx)
            .await?;
    if n >= MAX_ITEMS {
        return Err(ApiError::conflict(
            "too_many_items",
            format!("at most {MAX_ITEMS} items"),
        ));
    }
    if let Some(img) = &b.image_id {
        own_image(&mut tx, img).await?;
    }
    let id = new_id("sfit");
    sqlx::query("INSERT INTO storefront_items (id, tenant_id, storefront_id, name, description, price_minor, image_id, stock, active, position) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id)
        .bind(&auth.tenant_id)
        .bind(&sf)
        .bind(&name)
        .bind(&description)
        .bind(b.price_minor)
        .bind(&b.image_id)
        .bind(b.stock.map(|v| v as i32))
        .bind(b.active.unwrap_or(true))
        .bind(b.position.unwrap_or(0))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.item_added",
        Some(&id),
        json!({ "price_minor": b.price_minor }),
    )
    .await?;
    let row: ItemRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ITEM_COLS} FROM storefront_items WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(item_json(row))))
}

/// `GET /v1/storefront/items`
pub async fn list_items(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<ItemRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ITEM_COLS} FROM storefront_items ORDER BY position, created_at, id"
    )))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(item_json).collect::<Vec<_>>() }),
    ))
}

/// `PATCH /v1/storefront/items/{id}`
pub async fn patch_item(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<StoreItemPatch>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:write")?;
    let name = b
        .name
        .as_deref()
        .map(|n| clean_text(n, "name", 80, false, true))
        .transpose()?;
    let description = b
        .description
        .as_deref()
        .map(|d| clean_text(d, "description", 300, true, false))
        .transpose()?;
    if let Some(p) = b.price_minor {
        check_price(p)?;
    }
    if b.stock.is_some() && b.stock_delta.is_some() {
        return Err(ApiError::invalid(
            "invalid_stock",
            "give stock or stock_delta, not both",
        ));
    }
    if let Some(Some(v)) = b.stock {
        check_stock(v)?;
    }
    if b.stock_delta.is_some_and(|d| d.abs() > MAX_STOCK) {
        return Err(ApiError::invalid(
            "invalid_stock",
            "stock_delta is too large",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(Some(img)) = &b.image_id {
        own_image(&mut tx, img).await?;
    }
    let n = sqlx::query(
        "UPDATE storefront_items SET name = COALESCE($2, name), description = COALESCE($3, description), price_minor = COALESCE($4, price_minor),
            image_id = CASE WHEN $5 THEN $6 ELSE image_id END,
            stock = CASE WHEN $7 THEN $8 WHEN $9::int IS NOT NULL AND stock IS NOT NULL THEN stock + $9 ELSE stock END,
            active = COALESCE($10, active), position = COALESCE($11, position), updated_at = now()
         WHERE id = $1",
    )
    .bind(&id)
    .bind(name)
    .bind(description)
    .bind(b.price_minor)
    .bind(b.image_id.is_some())
    .bind(b.image_id.clone().flatten())
    .bind(b.stock.is_some())
    .bind(b.stock.flatten().map(|v| v as i32))
    .bind(b.stock_delta.map(|v| v as i32))
    .bind(b.active)
    .bind(b.position)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        // the count would have gone below zero
        sqlx::Error::Database(d) if d.is_check_violation() => ApiError::conflict("stock_below_zero", "that change would take the stock below zero"),
        e => e.into(),
    })?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("item"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.item_changed",
        Some(&id),
        json!({}),
    )
    .await?;
    let row: ItemRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ITEM_COLS} FROM storefront_items WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(item_json(row)))
}

// ---------- stock helpers ----------

/// Put an order's stock back on the shelf, once. Only items that are counted change.
async fn give_back(conn: &mut PgConnection, order_id: &str) -> Result<(), sqlx::Error> {
    let n =
        sqlx::query("UPDATE storefront_orders SET stock_held = false WHERE id = $1 AND stock_held")
            .bind(order_id)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    if n == 1 {
        sqlx::query("UPDATE storefront_items i SET stock = i.stock + o.quantity, updated_at = now() FROM storefront_order_items o WHERE o.order_id = $1 AND o.item_id = i.id AND i.stock IS NOT NULL")
            .bind(order_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Close an unpaid order's intent so it can never be paid, unless a payment is already in flight. Returns whether it closed.
async fn close_intent(conn: &mut PgConnection, intent_id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE payment_intents SET status = 'canceled', updated_at = now() WHERE id = $1 AND status = 'requires_payment_method'
           AND NOT EXISTS (SELECT 1 FROM payment_attempts WHERE intent_id = $1 AND status IN ('created', 'pending', 'authorized', 'unknown', 'captured'))",
    )
    .bind(intent_id)
    .execute(conn)
    .await?
    .rows_affected()
        == 1)
}

/// Give back the stock of unpaid orders whose hold has lapsed (for one page, or all of this tenant's). Returns how many.
pub async fn release_lapsed(
    conn: &mut PgConnection,
    storefront_id: Option<&str>,
) -> Result<usize, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, intent_id FROM storefront_orders WHERE status = 'pending' AND stock_held AND hold_until < now() AND ($1::text IS NULL OR storefront_id = $1)
         ORDER BY hold_until LIMIT 100 FOR UPDATE SKIP LOCKED",
    )
    .bind(storefront_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut n = 0;
    for (order, intent) in rows {
        if !close_intent(conn, &intent).await? {
            continue; // a payment is in flight: look again next time
        }
        give_back(conn, &order).await?;
        sqlx::query(
            "UPDATE storefront_orders SET status = 'expired', cancelled_at = now() WHERE id = $1",
        )
        .bind(&order)
        .execute(&mut *conn)
        .await?;
        n += 1;
    }
    Ok(n)
}

/// Called when a payment is captured: the order becomes paid. Money that arrives for an order whose stock was already
/// given back wins; the stock is taken again if there is any, and the order is flagged when there is not.
pub(crate) async fn on_captured(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
) -> Result<(), ApiError> {
    let row: Option<(String, String, bool)> = sqlx::query_as(
        "SELECT id, status, stock_held FROM storefront_orders WHERE intent_id = $1 FOR UPDATE",
    )
    .bind(intent_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((order, status, held)) = row else {
        return Ok(());
    };
    if matches!(status.as_str(), "paid" | "fulfilled") {
        return Ok(());
    }
    let mut attention = false;
    if !held {
        let mut sp = conn.begin().await?;
        let lines: Vec<(String, i32)> = sqlx::query_as("SELECT item_id, quantity FROM storefront_order_items WHERE order_id = $1 ORDER BY item_id").bind(&order).fetch_all(&mut *sp).await?;
        let mut ok = true;
        for (item, qty) in lines {
            let n = sqlx::query("UPDATE storefront_items SET stock = CASE WHEN stock IS NULL THEN NULL ELSE stock - $2 END WHERE id = $1 AND (stock IS NULL OR stock >= $2)")
                .bind(&item)
                .bind(qty)
                .execute(&mut *sp)
                .await?
                .rows_affected();
            if n == 0 {
                ok = false;
                break;
            }
        }
        if ok {
            sp.commit().await?;
            sqlx::query("UPDATE storefront_orders SET stock_held = true WHERE id = $1")
                .bind(&order)
                .execute(&mut *conn)
                .await?;
        } else {
            sp.rollback().await?;
            attention = true;
        }
    }
    sqlx::query("UPDATE storefront_orders SET status = 'paid', paid_at = now(), cancelled_at = NULL, needs_attention = $2 WHERE id = $1").bind(&order).bind(attention).execute(&mut *conn).await?;
    tenant::audit(
        conn,
        tenant_id,
        "system:payments",
        "storefront.order_paid",
        Some(&order),
        json!({ "needs_attention": attention }),
    )
    .await?;
    Ok(())
}

// ---------- seller: orders ----------

#[derive(Deserialize)]
pub struct StoreOrderQuery {
    status: Option<String>,
    limit: Option<i64>,
}

type OrderRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
    String,
    bool,
    bool,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    String,
);
const ORDER_COLS: &str = "id, code, buyer_name, buyer_phone, buyer_email, delivery_note, total_minor, currency, status, stock_held, needs_attention, created_at, paid_at, fulfilled_at, cancelled_at, intent_id";

async fn order_json(conn: &mut PgConnection, r: OrderRow) -> Result<Value, ApiError> {
    let lines: Vec<(String, String, i64, i32)> = sqlx::query_as("SELECT item_id, name, unit_minor, quantity FROM storefront_order_items WHERE order_id = $1 ORDER BY id").bind(&r.0).fetch_all(&mut *conn).await?;
    let (refunded, refunding): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor) FILTER (WHERE status = 'processed'), 0)::BIGINT,
                COALESCE(SUM(amount_minor) FILTER (WHERE status IN ('pending_approval', 'processing', 'unknown')), 0)::BIGINT FROM refunds WHERE intent_id = $1",
    )
    .bind(&r.15)
    .fetch_one(&mut *conn)
    .await?;
    Ok(json!({
        "id": r.0, "code": r.1, "buyer": { "name": r.2, "phone": r.3, "email": r.4 }, "delivery_note": r.5,
        "total_minor": r.6, "currency": r.7, "status": r.8, "stock_held": r.9, "needs_attention": r.10,
        "created_at": r.11, "paid_at": r.12, "fulfilled_at": r.13, "cancelled_at": r.14, "intent_id": r.15,
        "refunded_minor": refunded, "refund_in_progress_minor": refunding,
        "items": lines.into_iter().map(|(id, name, unit, qty)| json!({ "item_id": id, "name": name, "unit_minor": unit, "quantity": qty })).collect::<Vec<_>>(),
    }))
}

/// `GET /v1/storefront/orders`
pub async fn list_orders(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<StoreOrderQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:read")?;
    if q.status.as_deref().is_some_and(|x| {
        !matches!(
            x,
            "pending" | "paid" | "fulfilled" | "cancelled" | "expired"
        )
    }) {
        return Err(ApiError::invalid(
            "invalid_status",
            "status is pending, paid, fulfilled, cancelled or expired",
        ));
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<OrderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ORDER_COLS} FROM storefront_orders WHERE ($1::text IS NULL OR status = $1) ORDER BY created_at DESC, id DESC LIMIT $2")))
        .bind(q.status)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    let mut data = Vec::new();
    for r in rows {
        data.push(order_json(&mut tx, r).await?);
    }
    Ok(Json(json!({ "data": data })))
}

async fn load_order(conn: &mut PgConnection, id: &str, lock: bool) -> Result<OrderRow, ApiError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_COLS} FROM storefront_orders WHERE id = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("order"))
}

/// `GET /v1/storefront/orders/{id}`
pub async fn get_order(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = load_order(&mut tx, &id, false).await?;
    Ok(Json(order_json(&mut tx, r).await?))
}

/// `POST /v1/storefront/orders/{id}/fulfill`: the goods were handed over or sent. Paid orders only.
pub async fn fulfill_order(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE storefront_orders SET status = 'fulfilled', fulfilled_at = now() WHERE id = $1 AND status = 'paid'").bind(&id).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        load_order(&mut tx, &id, false).await?;
        return Err(ApiError::conflict(
            "not_paid",
            "only a paid order can be marked fulfilled",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.order_fulfilled",
        Some(&id),
        json!({}),
    )
    .await?;
    let r = load_order(&mut tx, &id, false).await?;
    let out = order_json(&mut tx, r).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// `POST /v1/storefront/orders/{id}/cancel`: stop an order that has not been paid; its stock goes back. A paid order is
/// not cancelled here: refund it (which can put the stock back too).
pub async fn cancel_order(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("storefront:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = load_order(&mut tx, &id, true).await?;
    if r.8 != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            "only an order that is waiting for payment can be cancelled; refund a paid one",
        ));
    }
    if !close_intent(&mut tx, &r.15).await? {
        return Err(ApiError::conflict(
            "payment_in_progress",
            "a payment for this order is in progress; wait for it to finish, then refund it if needed",
        ));
    }
    give_back(&mut tx, &id).await?;
    sqlx::query(
        "UPDATE storefront_orders SET status = 'cancelled', cancelled_at = now() WHERE id = $1",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "storefront.order_cancelled",
        Some(&id),
        json!({}),
    )
    .await?;
    let r = load_order(&mut tx, &id, false).await?;
    let out = order_json(&mut tx, r).await?;
    tx.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
pub struct StoreRefundIn {
    /// Omit to refund what is still refundable.
    amount_minor: Option<i64>,
    reason: Option<String>,
    /// Put the stock back and mark the order cancelled. Only with a full refund (no amount).
    restock: Option<bool>,
}

/// `POST /v1/storefront/orders/{id}/refund`: refund a paid order through the ordinary refund flow (limits, approval
/// thresholds and the unknown-outcome rules all apply). Needs an `Idempotency-Key`.
pub async fn refund_order(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<StoreRefundIn>>,
) -> Result<Response, ApiError> {
    auth.require("storefront:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    if b.restock.unwrap_or(false) && b.amount_minor.is_some() {
        return Err(ApiError::invalid(
            "invalid_refund",
            "restock goes with a full refund: leave amount_minor out",
        ));
    }
    let reason = b
        .reason
        .as_deref()
        .map(|r| clean_text(r, "reason", 200, false, false))
        .transpose()?;
    let intent_id = {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let r = load_order(&mut tx, &id, false).await?;
        if !matches!(r.8.as_str(), "paid" | "fulfilled") {
            return Err(ApiError::conflict(
                "not_paid",
                "only a paid order can be refunded",
            ));
        }
        r.15
    };
    let res = crate::refunds::create_refund(
        State(s.clone()),
        auth.clone(),
        Path(intent_id),
        headers,
        Some(Json(crate::refunds::CreateRefund {
            amount_minor: b.amount_minor,
            reason,
            external_ref: Some(format!("storefront_order:{id}")),
        })),
    )
    .await?;
    if res.status().is_success() && b.restock.unwrap_or(false) {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let r = load_order(&mut tx, &id, true).await?;
        if r.9 {
            give_back(&mut tx, &id).await?;
            sqlx::query("UPDATE storefront_orders SET status = 'cancelled', cancelled_at = now() WHERE id = $1").bind(&id).execute(&mut *tx).await?;
        }
        tenant::audit(
            &mut tx,
            &auth.tenant_id,
            &auth.actor,
            "storefront.order_refunded",
            Some(&id),
            json!({ "restock": true }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(res)
}

// ---------- public ----------

async fn tenant_of(s: &AppState, slug: &str) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT tenant_id FROM storefront_directory WHERE slug = $1")
        .bind(slug)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("page"))
}

/// `GET /v1/store/{slug}`: what the page shows. Only a published page exists.
pub async fn public_page(
    State(s): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let tenant_id = tenant_of(&s, &slug).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let sf: Option<(String, String, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, title, description, logo_image_id, noindex FROM storefronts WHERE published",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let (sf_id, title, description, logo, noindex) =
        sf.ok_or_else(|| ApiError::not_found("page"))?;
    release_lapsed(&mut tx, Some(&sf_id)).await?;
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1").bind(&tenant_id).fetch_optional(&mut *tx).await?;
    let rows: Vec<ItemRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ITEM_COLS} FROM storefront_items WHERE storefront_id = $1 AND active ORDER BY position, created_at, id"))).bind(&sf_id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let img = |id: &Option<String>| id.as_ref().map(|i| format!("/api/store/{slug}/images/{i}"));
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| json!({ "id": r.0, "name": r.1, "description": r.2, "price_minor": r.3, "image_url": img(&r.4), "available": r.5, "sold_out": r.5 == Some(0) }))
        .collect();
    let body = json!({ "slug": slug, "title": title, "description": description, "merchant": merchant, "currency": "INR", "logo_url": img(&logo), "noindex": noindex, "items": items });
    Ok(([(header::CACHE_CONTROL, "no-cache")], Json(body)).into_response())
}

/// `GET /v1/store/{slug}/images/{id}`: an item or logo image of a published page.
pub async fn public_image(
    State(s): State<AppState>,
    Path((slug, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let tenant_id = tenant_of(&s, &slug).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let published: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM storefronts WHERE published)")
            .fetch_one(&mut *tx)
            .await?;
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT content_type, data FROM storefront_images WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let (kind, data) = row
        .filter(|_| published)
        .ok_or_else(|| ApiError::not_found("image"))?;
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        kind.parse().map_err(|_| ApiError::internal())?,
    );
    h.insert(
        header::CACHE_CONTROL,
        "public, max-age=86400, immutable".parse().expect("static"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; sandbox".parse().expect("static"),
    );
    Ok((h, data).into_response())
}

#[derive(Deserialize)]
pub struct StoreLineIn {
    item_id: String,
    quantity: i32,
}

#[derive(Deserialize)]
pub struct PublicOrderIn {
    buyer_name: String,
    buyer_phone: String,
    buyer_email: Option<String>,
    delivery_note: Option<String>,
    items: Vec<StoreLineIn>,
}

fn rate_limited(retry: u64) -> ApiError {
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        format!("too many orders from here; try again in {retry}s"),
    )
}

/// Limits for public pages that create something: a few per visitor address, and a ceiling per page or campaign
/// (`scope`) so a flood cannot empty its stock or fill its books.
pub(crate) fn throttle(s: &AppState, parts: &Parts, scope: &str) -> Result<(), ApiError> {
    let now = Instant::now();
    if let Some(ip) = vouched_client_ip(s.actor_secret.as_deref(), &parts.headers)
        .or_else(|| client_ip(&parts.headers, &parts.extensions, s.trusted_hops))
    {
        s.limiter
            .check(
                format!("pubcreate:ip:{ip}"),
                Bucket {
                    burst: 10.0,
                    per_sec: 0.05,
                },
                now,
            )
            .map_err(rate_limited)?;
    }
    s.limiter
        .check(
            format!("pubcreate:{scope}"),
            Bucket {
                burst: 120.0,
                per_sec: 2.0,
            },
            now,
        )
        .map_err(rate_limited)
}

fn order_code() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_uppercase()
}

/// `POST /v1/store/{slug}/orders`: record an order, take its stock, and open the buyer's own payment intent.
/// Prices come from the page, never from the request.
pub async fn public_order(
    State(s): State<AppState>,
    Path(slug): Path<String>,
    parts: Parts,
    Json(b): Json<PublicOrderIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    throttle(&s, &parts, &format!("store:{slug}"))?;
    let name = clean_text(&b.buyer_name, "name", 80, false, true)?;
    let phone = clean_phone(&b.buyer_phone)?;
    let email = b
        .buyer_email
        .as_deref()
        .filter(|e| !e.trim().is_empty())
        .map(clean_email)
        .transpose()?;
    let note = clean_text(
        b.delivery_note.as_deref().unwrap_or(""),
        "delivery note",
        300,
        true,
        false,
    )?;
    if b.items.is_empty()
        || b.items.len() > MAX_LINES
        || b.items.iter().any(|l| !(1..=MAX_QTY).contains(&l.quantity))
    {
        return Err(ApiError::invalid(
            "invalid_items",
            format!("choose 1 to {MAX_LINES} items, each 1 to {MAX_QTY}"),
        ));
    }
    // one line per item, in a fixed order so two orders never wait on each other in a circle
    let mut lines: std::collections::BTreeMap<String, i32> = Default::default();
    for l in &b.items {
        *lines.entry(l.item_id.clone()).or_default() += l.quantity;
    }
    if lines.values().any(|q| *q > MAX_QTY) {
        return Err(ApiError::invalid(
            "invalid_items",
            format!("at most {MAX_QTY} of one item"),
        ));
    }
    let tenant_id = tenant_of(&s, &slug).await?;
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let sf: Option<(String, String)> =
        sqlx::query_as("SELECT id, title FROM storefronts WHERE published FOR SHARE")
            .fetch_optional(&mut *tx)
            .await?;
    let (sf_id, title) = sf.ok_or_else(|| ApiError::not_found("page"))?;
    release_lapsed(&mut tx, Some(&sf_id)).await?;
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM storefront_orders WHERE storefront_id = $1 AND status = 'pending'",
    )
    .bind(&sf_id)
    .fetch_one(&mut *tx)
    .await?;
    if open >= MAX_PENDING {
        return Err(ApiError::conflict(
            "store_busy",
            "this page has many orders waiting for payment; try again in a few minutes",
        ));
    }
    let mut total: i64 = 0;
    let mut taken: Vec<(String, String, i64, i32)> = Vec::new();
    for (item_id, qty) in &lines {
        // the one statement that decides: it takes stock only if enough is there, and the row lock makes it exact
        let got: Option<(String, i64)> = sqlx::query_as(
            "UPDATE storefront_items SET stock = CASE WHEN stock IS NULL THEN NULL ELSE stock - $3 END, updated_at = now()
              WHERE id = $1 AND storefront_id = $2 AND active AND (stock IS NULL OR stock >= $3) RETURNING name, price_minor",
        )
        .bind(item_id)
        .bind(&sf_id)
        .bind(qty)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((iname, price)) = got else {
            let why: Option<(String, bool, Option<i32>)> = sqlx::query_as("SELECT name, active, stock FROM storefront_items WHERE id = $1 AND storefront_id = $2").bind(item_id).bind(&sf_id).fetch_optional(&mut *tx).await?;
            return Err(match why {
                Some((n, true, Some(left))) => {
                    ApiError::conflict("out_of_stock", format!("only {left} of \"{n}\" left"))
                }
                _ => ApiError::conflict(
                    "item_unavailable",
                    "one of the items is no longer available",
                ),
            });
        };
        total = price
            .checked_mul(*qty as i64)
            .and_then(|v| total.checked_add(v))
            .filter(|t| *t <= zyvor_core::MAX_AMOUNT_MINOR)
            .ok_or_else(|| ApiError::invalid("invalid_items", "the order total is too large"))?;
        taken.push((item_id.clone(), iname, price, *qty));
    }
    let (order_id, intent_id, code) = (new_id("sfo"), new_id("pi"), order_code());
    let secret = format!(
        "pis_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status, capture_method, description, metadata, client_secret_hash) VALUES ($1,$2,$3,'INR','requires_payment_method','automatic',$4,$5,$6)")
        .bind(&intent_id)
        .bind(&tenant_id)
        .bind(total)
        .bind(format!("Order {code} from {title}"))
        .bind(json!({ "storefront_order": order_id }))
        .bind(hash_key(&secret))
        .execute(&mut *tx)
        .await?;
    let hold_until = Utc::now() + Duration::minutes(HOLD_MINUTES);
    sqlx::query("INSERT INTO storefront_orders (id, tenant_id, storefront_id, intent_id, code, buyer_name, buyer_phone, buyer_email, delivery_note, total_minor, currency, status, hold_until) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'INR','pending',$11)")
        .bind(&order_id)
        .bind(&tenant_id)
        .bind(&sf_id)
        .bind(&intent_id)
        .bind(&code)
        .bind(&name)
        .bind(&phone)
        .bind(&email)
        .bind(Some(note).filter(|n| !n.is_empty()))
        .bind(total)
        .bind(hold_until)
        .execute(&mut *tx)
        .await?;
    for (item_id, iname, price, qty) in &taken {
        sqlx::query("INSERT INTO storefront_order_items (id, tenant_id, order_id, item_id, name, unit_minor, quantity) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(new_id("sfl"))
            .bind(&tenant_id)
            .bind(&order_id)
            .bind(item_id)
            .bind(iname)
            .bind(price)
            .bind(qty)
            .execute(&mut *tx)
            .await?;
    }
    tenant::audit(
        &mut tx,
        &tenant_id,
        &format!("store:{sf_id}"),
        "storefront.order_created",
        Some(&order_id),
        json!({ "total_minor": total, "lines": taken.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "order_id": order_id, "code": code, "intent_id": intent_id, "client_secret": secret, "amount_minor": total, "currency": "INR", "tenant_id": tenant_id, "path": format!("/pay/{tenant_id}/{intent_id}"), "hold_minutes": HOLD_MINUTES }),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_the_rules_and_reserved_names_are_refused() {
        for ok in ["raju-sweets", "shop2go", "a1b", "meena-handloom-2026"] {
            assert!(check_slug(ok).is_ok(), "{ok}");
        }
        for bad in [
            "ab",
            "-abc",
            "abc-",
            "a--b",
            "Raju",
            "raju_sweets",
            "123456",
            "has space",
            "x.y.z",
            &"a".repeat(41),
            "admin",
            "zyvor",
            "my-razorpay-store",
            "paytm-pay",
            "vayora-help",
        ] {
            assert!(check_slug(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn text_is_trimmed_capped_and_free_of_control_and_bidi_characters() {
        assert_eq!(
            clean_text("  Hello  ", "x", 10, false, true).unwrap(),
            "Hello"
        );
        assert!(clean_text("", "x", 10, false, true).is_err());
        assert!(clean_text("", "x", 10, false, false).is_ok());
        assert!(clean_text("12345678901", "x", 10, false, false).is_err());
        assert!(clean_text("a\nb", "x", 10, false, false).is_err());
        assert!(clean_text("a\nb", "x", 10, true, false).is_ok());
        assert!(clean_text("a\u{202E}b", "x", 10, true, false).is_err());
        assert!(clean_text("a\u{0}b", "x", 10, true, false).is_err());
        // markup is not stripped: it is shown as text wherever it is displayed
        assert_eq!(
            clean_text("<b>x</b>", "x", 20, false, false).unwrap(),
            "<b>x</b>"
        );
    }

    #[test]
    fn phones_and_emails_are_checked() {
        assert_eq!(clean_phone("+91 98765-43210").unwrap(), "+919876543210");
        assert!(clean_phone("12345").is_err());
        assert!(clean_phone("98765abcde").is_err());
        assert!(clean_email("a@b.in").is_ok());
        for bad in [
            "a@b",
            "a b@c.in",
            "@c.in",
            "a@@c.in",
            "a@c.in,b@d.in",
            "<a>@c.in",
        ] {
            assert!(clean_email(bad).is_err(), "{bad}");
        }
    }
}
