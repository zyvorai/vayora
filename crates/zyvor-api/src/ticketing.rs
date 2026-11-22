// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Events and ticketing, on top of payment intents, coupons, refunds and splits.
//!
//! * **Seats are never oversold.** `seats_held` (unpaid orders inside their hold time) and `seats_sold` (paid tickets)
//!   live on the event and on each tier. They change only while the event row is locked (`FOR UPDATE`), and the
//!   database refuses any row where held + sold would pass the capacity, so a bug here fails loudly instead of selling
//!   a seat twice.
//! * **Tickets exist only after a verified capture.** `on_payment_captured` is called from the one place a provider
//!   result is applied to a payment (`payments::apply_provider_state`), once per real transition, and is idempotent.
//! * **Unpaid orders hold seats briefly.** The worker releases an order whose hold ran out (never while an attempt's
//!   outcome is unknown or authorised); a payment that still arrives afterwards takes the seats back if they are free,
//!   otherwise it is queued for a refund.
//! * **Check-in** takes a signed QR payload or the bare code, and a second scan of the same ticket is reported, not
//!   accepted.
//! * **Cancellation** refunds through the ordinary refund flow (`refunds::reserve_refund`), so the over-refund
//!   protection and the approval threshold apply to every one.
//!
//! The pieces that need no database (pricing windows, signing, form validation, CSV) are plain functions with unit tests.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{FromRow, PgConnection};
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, refunds, tenant};

pub const METHODS: [&str; 4] = ["upi", "card", "netbanking", "wallet"];
/// Characters a ticket code is made of: no 0/O, 1/I/L, so it can be read out over a phone.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 12;
/// Bytes of the HMAC kept in a ticket's QR payload (24 hex characters).
const SIG_BYTES: usize = 12;

// ---------------------------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------------------------

fn random_bytes() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

/// A fresh ticket code: 12 characters from a 31-letter alphabet (about 59 bits), drawn without modulo bias.
pub fn new_code() -> String {
    let mut out = String::with_capacity(CODE_LEN);
    while out.len() < CODE_LEN {
        for b in uuid::Uuid::new_v4().as_bytes() {
            // 248 = 31 * 8: bytes at or above it would favour the first letters
            if *b < 248 && out.len() < CODE_LEN {
                out.push(ALPHABET[(*b as usize) % ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// What a person types or a scanner reads, cleaned up: no spaces or dashes, upper case.
pub fn normalise_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_uppercase()
}

fn mac(secret: &[u8], domain: &str, parts: &[&str]) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    m.update(domain.as_bytes());
    for p in parts {
        m.update(b"|");
        m.update(p.as_bytes());
    }
    m
}

fn ticket_sig(secret: &[u8], event_id: &str, code: &str) -> String {
    hex::encode(
        &mac(secret, "zyvor-ticket-v1", &[event_id, code])
            .finalize()
            .into_bytes()[..SIG_BYTES],
    )
}

/// The text inside a ticket's QR code: `ZT1.<code>.<signature>`. The signature binds the code to its event, so a
/// payload cannot be made up from a guessed code and cannot be moved to another event.
pub fn qr_payload(secret: &[u8], event_id: &str, code: &str) -> String {
    format!("ZT1.{code}.{}", ticket_sig(secret, event_id, code))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Scanned {
    /// A QR payload: its code and the signature it carried.
    Signed { code: String, sig: String },
    /// A code typed by hand.
    Bare(String),
}

pub fn parse_scan(raw: &str) -> Option<Scanned> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 200 {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("ZT1.") {
        let (code, sig) = rest.split_once('.')?;
        let code = normalise_code(code);
        if code.len() != CODE_LEN || sig.len() != 24 || !sig.chars().all(|c| c.is_ascii_hexdigit())
        {
            return None;
        }
        return Some(Scanned::Signed {
            code,
            sig: sig.to_lowercase(),
        });
    }
    let code = normalise_code(raw);
    (code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b)))
        .then_some(Scanned::Bare(code))
}

pub fn verify_sig(secret: &[u8], event_id: &str, code: &str, sig: &str) -> bool {
    let Ok(bytes) = hex::decode(sig) else {
        return false;
    };
    // exactly the 12 bytes we issue: a shorter prefix of the right tag must not pass
    if bytes.len() != SIG_BYTES {
        return false;
    }
    // constant-time comparison of the leading 12 bytes
    mac(secret, "zyvor-ticket-v1", &[event_id, code])
        .verify_truncated_left(&bytes)
        .is_ok()
}

/// The buyer's access token for an order page: derived, so nothing needs storing and an emailed link stays valid.
pub fn order_token(secret: &[u8], order_id: &str) -> String {
    format!(
        "eot_{}",
        hex::encode(
            &mac(secret, "zyvor-order-v1", &[order_id])
                .finalize()
                .into_bytes()[..16]
        )
    )
}

pub fn order_token_ok(secret: &[u8], order_id: &str, token: &str) -> bool {
    use subtle::ConstantTimeEq;
    order_token(secret, order_id)
        .as_bytes()
        .ct_eq(token.as_bytes())
        .into()
}

/// Where a tier stands in its sale window.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sale {
    Upcoming,
    Open,
    Ended,
}

#[derive(Debug, Clone)]
pub struct TierWindow {
    pub price_minor: i64,
    pub early_price_minor: Option<i64>,
    pub early_starts_at: Option<DateTime<Utc>>,
    pub early_ends_at: Option<DateTime<Utc>>,
    pub sale_starts_at: Option<DateTime<Utc>>,
    pub sale_ends_at: Option<DateTime<Utc>>,
}

impl TierWindow {
    pub fn sale(&self, now: DateTime<Utc>) -> Sale {
        if self.sale_starts_at.is_some_and(|s| now < s) {
            Sale::Upcoming
        } else if self.sale_ends_at.is_some_and(|e| now >= e) {
            Sale::Ended
        } else {
            Sale::Open
        }
    }

    /// The unit price at `now` and which price it is: the early-bird price inside its window, otherwise the regular one.
    pub fn price_at(&self, now: DateTime<Utc>) -> (i64, &'static str) {
        match (self.early_price_minor, self.early_ends_at) {
            (Some(p), Some(end)) if now < end && self.early_starts_at.is_none_or(|s| now >= s) => {
                (p, "early_bird")
            }
            _ => (self.price_minor, "regular"),
        }
    }
}

/// Split an order total between vendors by basis points. Shares are floored, so they never add up to more than the total.
pub fn split_by_bp(total: i64, shares: &[(String, i64)]) -> Vec<crate::splits::SplitIn> {
    shares
        .iter()
        .filter_map(|(vendor, bp)| {
            let amount = (total as i128 * *bp as i128 / 10_000) as i64;
            (amount > 0).then(|| crate::splits::SplitIn {
                vendor_id: vendor.clone(),
                amount_minor: amount,
                ..Default::default()
            })
        })
        .collect()
}

pub fn valid_email(e: &str) -> bool {
    let e = e.trim();
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    e.len() <= 254
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains('@')
        && !e.chars().any(|c| c.is_whitespace() || c.is_control())
}

pub fn valid_phone(p: &str) -> bool {
    let digits = p.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits)
        && p.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewField {
    pub key: String,
    pub label: String,
    /// text, email, phone, select or checkbox.
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    /// The choices of a select.
    pub options: Option<Vec<String>>,
    pub min_len: Option<i32>,
    pub max_len: Option<i32>,
}

pub fn validate_field_defs(fields: &[NewField]) -> Result<(), ApiError> {
    let bad = |m: String| ApiError::invalid("invalid_form_field", m);
    if fields.len() > 30 {
        return Err(bad("at most 30 form fields".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for f in fields {
        let key_ok = !f.key.is_empty()
            && f.key.len() <= 40
            && f.key.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && f.key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !key_ok || !seen.insert(f.key.clone()) {
            return Err(bad(format!(
                "field key {:?} must be unique, lower case letters, digits and _, starting with a letter",
                f.key
            )));
        }
        if f.label.trim().is_empty() || f.label.len() > 200 {
            return Err(bad(format!(
                "field {} needs a label of up to 200 characters",
                f.key
            )));
        }
        match f.kind.as_str() {
            "text" | "email" | "phone" | "checkbox" => {
                if f.options.is_some() {
                    return Err(bad(format!("field {} takes no options", f.key)));
                }
            }
            "select" => {
                let o = f.options.as_deref().unwrap_or_default();
                if o.is_empty()
                    || o.len() > 50
                    || o.iter().any(|x| x.trim().is_empty() || x.len() > 100)
                {
                    return Err(bad(format!(
                        "select {} needs 1-50 non-empty options",
                        f.key
                    )));
                }
            }
            _ => {
                return Err(bad(format!(
                    "field {} has an unknown kind {:?}",
                    f.key, f.kind
                )));
            }
        }
        if f.min_len.is_some_and(|m| m < 0)
            || f.max_len.is_some_and(|m| !(1..=2000).contains(&m))
            || f.min_len.zip(f.max_len).is_some_and(|(a, b)| a > b)
        {
            return Err(bad(format!(
                "field {} has an impossible length range",
                f.key
            )));
        }
    }
    Ok(())
}

/// Check a buyer's answers against the event's form and return them in storage shape. Unknown keys are refused, so
/// nothing but what the form asks for is kept.
pub fn validate_answers(fields: &[NewField], answers: &Value) -> Result<Value, ApiError> {
    let obj = match answers {
        Value::Null => serde_json::Map::new(),
        Value::Object(o) => o.clone(),
        _ => {
            return Err(ApiError::invalid(
                "invalid_answers",
                "answers must be an object of field key to value",
            ));
        }
    };
    if let Some(k) = obj.keys().find(|k| !fields.iter().any(|f| &f.key == *k)) {
        return Err(ApiError::invalid(
            "invalid_answers",
            format!("the form has no field {k:?}"),
        ));
    }
    let mut out = serde_json::Map::new();
    for f in fields {
        let err = |m: &str| ApiError::invalid("invalid_answers", format!("{}: {m}", f.label));
        let v = obj.get(&f.key).filter(|v| !v.is_null());
        match f.kind.as_str() {
            "checkbox" => {
                let on = match v {
                    None => false,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err(err("must be true or false")),
                };
                if f.required && !on {
                    return Err(err("must be ticked"));
                }
                out.insert(f.key.clone(), Value::Bool(on));
            }
            kind => {
                let text = match v {
                    None => String::new(),
                    Some(Value::String(s)) => s.trim().to_string(),
                    Some(_) => return Err(err("must be text")),
                };
                if text.is_empty() {
                    if f.required {
                        return Err(err("is required"));
                    }
                    continue;
                }
                if text.chars().any(|c| c.is_control()) {
                    return Err(err("contains characters that are not allowed"));
                }
                let len = text.chars().count() as i32;
                let (lo, hi) = (f.min_len.unwrap_or(0), f.max_len.unwrap_or(500));
                if len < lo || len > hi {
                    return Err(err(&format!("must be {lo} to {hi} characters")));
                }
                let ok = match kind {
                    "email" => valid_email(&text),
                    "phone" => valid_phone(&text),
                    "select" => f.options.as_deref().unwrap_or_default().contains(&text),
                    _ => true,
                };
                if !ok {
                    return Err(err(match kind {
                        "email" => "is not a valid email address",
                        "phone" => "is not a valid phone number",
                        _ => "is not one of the choices",
                    }));
                }
                out.insert(f.key.clone(), Value::String(text));
            }
        }
    }
    Ok(Value::Object(out))
}

/// One CSV cell. A value a spreadsheet could read as a formula is prefixed with `'`, so a name like `=HYPERLINK(...)`
/// typed into a registration form cannot run when the organiser opens the export.
pub fn csv_cell(s: &str) -> String {
    let mut v = s.to_string();
    if v.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        v.insert(0, '\'');
    }
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

pub fn csv_row(cells: &[String]) -> String {
    let mut line = cells
        .iter()
        .map(|c| csv_cell(c))
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

fn csv_response(name: &str, body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, FromRow)]
pub struct EventRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    pub venue: Option<String>,
    pub online_url: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub status: String,
    pub currency: String,
    pub capacity: i32,
    pub hold_minutes: i32,
    pub seats_held: i32,
    pub seats_sold: i32,
    pub splits: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

pub const EVENT_COLS: &str = "id, name, description, mode, venue, online_url, starts_at, ends_at, status, currency, capacity, hold_minutes, seats_held, seats_sold, splits, created_at, cancelled_at";

#[derive(Debug, FromRow)]
pub struct TierRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_minor: i64,
    pub quantity: i32,
    pub per_order_max: i32,
    pub sale_starts_at: Option<DateTime<Utc>>,
    pub sale_ends_at: Option<DateTime<Utc>>,
    pub early_bird_price_minor: Option<i64>,
    pub early_bird_starts_at: Option<DateTime<Utc>>,
    pub early_bird_ends_at: Option<DateTime<Utc>>,
    pub allowed_methods: Option<Vec<String>>,
    pub seats_held: i32,
    pub seats_sold: i32,
    pub archived_at: Option<DateTime<Utc>>,
}

pub const TIER_COLS: &str = "id, name, description, price_minor, quantity, per_order_max, sale_starts_at, sale_ends_at, early_bird_price_minor, early_bird_starts_at, early_bird_ends_at, allowed_methods, seats_held, seats_sold, archived_at";

impl TierRow {
    pub fn window(&self) -> TierWindow {
        TierWindow {
            price_minor: self.price_minor,
            early_price_minor: self.early_bird_price_minor,
            early_starts_at: self.early_bird_starts_at,
            early_ends_at: self.early_bird_ends_at,
            sale_starts_at: self.sale_starts_at,
            sale_ends_at: self.sale_ends_at,
        }
    }
    pub fn remaining(&self) -> i32 {
        (self.quantity - self.seats_held - self.seats_sold).max(0)
    }
}

#[derive(Debug, FromRow)]
pub struct FieldRow {
    pub key: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub options: Option<Value>,
    pub min_len: Option<i32>,
    pub max_len: Option<i32>,
}

impl FieldRow {
    pub fn def(&self) -> NewField {
        NewField {
            key: self.key.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            required: self.required,
            options: self
                .options
                .as_ref()
                .and_then(|o| serde_json::from_value(o.clone()).ok()),
            min_len: self.min_len,
            max_len: self.max_len,
        }
    }
}

pub async fn load_fields(
    conn: &mut PgConnection,
    event_id: &str,
) -> Result<Vec<FieldRow>, ApiError> {
    Ok(sqlx::query_as(
        "SELECT key, label, kind, required, options, min_len, max_len FROM event_form_fields WHERE event_id = $1 ORDER BY position, key",
    )
    .bind(event_id)
    .fetch_all(conn)
    .await?)
}

pub async fn load_tiers(conn: &mut PgConnection, event_id: &str) -> Result<Vec<TierRow>, ApiError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {TIER_COLS} FROM ticket_tiers WHERE event_id = $1 ORDER BY position, id"
    )))
    .bind(event_id)
    .fetch_all(conn)
    .await?)
}

pub async fn fetch_event(
    conn: &mut PgConnection,
    id: &str,
    lock: bool,
) -> Result<EventRow, ApiError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {EVENT_COLS} FROM ticket_events WHERE id = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("event"))
}

/// The key that signs this event's tickets and order links.
pub async fn event_secret(
    s: &AppState,
    conn: &mut PgConnection,
    event_id: &str,
) -> Result<Vec<u8>, ApiError> {
    let blob: Vec<u8> = sqlx::query_scalar("SELECT qr_secret_enc FROM ticket_events WHERE id = $1")
        .bind(event_id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("event"))?;
    s.crypto.decrypt(&blob).ok_or_else(ApiError::internal)
}

fn tier_json(t: &TierRow, now: DateTime<Utc>) -> Value {
    let w = t.window();
    let (price, kind) = w.price_at(now);
    json!({
        "id": t.id, "name": t.name, "description": t.description,
        "price_minor": t.price_minor, "current_price_minor": price, "price_kind": kind,
        "quantity": t.quantity, "per_order_max": t.per_order_max,
        "seats_held": t.seats_held, "seats_sold": t.seats_sold, "remaining": t.remaining(),
        "sale": { "state": w.sale(now), "starts_at": t.sale_starts_at, "ends_at": t.sale_ends_at },
        "early_bird": t.early_bird_price_minor.map(|p| json!({ "price_minor": p, "starts_at": t.early_bird_starts_at, "ends_at": t.early_bird_ends_at })),
        "allowed_methods": t.allowed_methods, "archived": t.archived_at.is_some(),
    })
}

fn field_json(f: &FieldRow) -> Value {
    json!({ "key": f.key, "label": f.label, "kind": f.kind, "required": f.required, "options": f.options, "min_len": f.min_len, "max_len": f.max_len })
}

async fn event_json(conn: &mut PgConnection, id: &str) -> Result<Value, ApiError> {
    let e = fetch_event(conn, id, false).await?;
    let now = Utc::now();
    let tiers = load_tiers(conn, id).await?;
    let fields = load_fields(conn, id).await?;
    let tickets: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM tickets WHERE event_id = $1 GROUP BY status")
            .bind(id)
            .fetch_all(&mut *conn)
            .await?;
    let orders: Vec<(String, i64)> = sqlx::query_as(
        "SELECT status, count(*) FROM event_orders WHERE event_id = $1 GROUP BY status",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let count = |v: &[(String, i64)]| -> Value {
        v.iter()
            .map(|(k, n)| (k.clone(), json!(n)))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    Ok(json!({
        "id": e.id, "name": e.name, "description": e.description, "mode": e.mode, "venue": e.venue, "online_url": e.online_url,
        "starts_at": e.starts_at, "ends_at": e.ends_at, "status": e.status, "currency": e.currency,
        "capacity": e.capacity, "hold_minutes": e.hold_minutes, "seats_held": e.seats_held, "seats_sold": e.seats_sold,
        "seats_available": (e.capacity - e.seats_held - e.seats_sold).max(0),
        "splits": e.splits, "created_at": e.created_at, "cancelled_at": e.cancelled_at,
        "tiers": tiers.iter().map(|t| tier_json(t, now)).collect::<Vec<_>>(),
        "form_fields": fields.iter().map(field_json).collect::<Vec<_>>(),
        "tickets": count(&tickets), "orders": count(&orders),
    }))
}

// ---------------------------------------------------------------------------------------------------------------
// Create and edit events
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct NewTier {
    pub name: String,
    pub description: Option<String>,
    pub price_minor: i64,
    pub quantity: i32,
    pub per_order_max: Option<i32>,
    pub sale_starts_at: Option<DateTime<Utc>>,
    pub sale_ends_at: Option<DateTime<Utc>>,
    pub early_bird_price_minor: Option<i64>,
    pub early_bird_starts_at: Option<DateTime<Utc>>,
    pub early_bird_ends_at: Option<DateTime<Utc>>,
    /// Payment methods this tier may be paid with (upi, card, netbanking, wallet); omit for all.
    pub allowed_methods: Option<Vec<String>>,
}

pub fn validate_tier(t: &NewTier) -> Result<(), ApiError> {
    let bad = |m: &str| ApiError::invalid("invalid_tier", m);
    if t.name.trim().is_empty() || t.name.len() > 100 {
        return Err(bad("name is required, up to 100 characters"));
    }
    if !(0..=zyvor_core::MAX_AMOUNT_MINOR).contains(&t.price_minor) {
        return Err(bad(
            "price_minor must be a whole number of paise, 0 or more",
        ));
    }
    if !(1..=1_000_000).contains(&t.quantity)
        || t.per_order_max.is_some_and(|m| !(1..=100).contains(&m))
    {
        return Err(bad("quantity is 1 or more and per_order_max is 1 to 100"));
    }
    if t.sale_starts_at
        .zip(t.sale_ends_at)
        .is_some_and(|(a, b)| b <= a)
    {
        return Err(bad("sale_ends_at must be after sale_starts_at"));
    }
    match (t.early_bird_price_minor, t.early_bird_ends_at) {
        (None, None) if t.early_bird_starts_at.is_none() => {}
        (Some(p), Some(end)) => {
            if p < 0 || p >= t.price_minor {
                return Err(bad("the early-bird price must be below the regular price"));
            }
            if t.early_bird_starts_at.is_some_and(|s| end <= s) {
                return Err(bad("early_bird_ends_at must be after early_bird_starts_at"));
            }
        }
        _ => {
            return Err(bad(
                "an early bird needs both early_bird_price_minor and early_bird_ends_at",
            ));
        }
    }
    if let Some(m) = &t.allowed_methods
        && (m.is_empty() || m.iter().any(|x| !METHODS.contains(&x.as_str())))
    {
        return Err(bad(
            "allowed_methods must be a non-empty list of upi, card, netbanking, wallet",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ShareIn {
    pub vendor_id: String,
    pub share_bp: i64,
}

#[derive(Deserialize)]
pub struct NewEvent {
    name: String,
    description: Option<String>,
    /// `venue` (default) or `online`.
    mode: Option<String>,
    venue: Option<String>,
    online_url: Option<String>,
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
    /// Seats across all tiers; the hard ceiling.
    capacity: i32,
    /// Minutes an unpaid order keeps its seats (1-180, default 10).
    hold_minutes: Option<i32>,
    /// Shares of every paid order owed to vendors, in basis points of the order total.
    splits: Option<Vec<ShareIn>>,
    tiers: Option<Vec<NewTier>>,
    form_fields: Option<Vec<NewField>>,
}

#[allow(clippy::too_many_arguments)]
fn check_event_fields(
    name: &str,
    description: Option<&str>,
    mode: &str,
    venue: Option<&str>,
    online_url: Option<&str>,
    starts_at: DateTime<Utc>,
    ends_at: Option<DateTime<Utc>>,
    capacity: i32,
    hold: i32,
) -> Result<(), ApiError> {
    let bad = |m: &str| ApiError::invalid("invalid_event", m);
    if name.trim().is_empty() || name.len() > 200 {
        return Err(bad("name is required, up to 200 characters"));
    }
    if description.is_some_and(|d| d.len() > 5000) || venue.is_some_and(|v| v.len() > 300) {
        return Err(bad(
            "description is up to 5000 characters and venue up to 300",
        ));
    }
    if !matches!(mode, "venue" | "online") {
        return Err(bad("mode is venue or online"));
    }
    if mode == "online" && !online_url.is_some_and(|u| u.starts_with("https://") && u.len() <= 500)
    {
        return Err(bad("an online event needs an https online_url"));
    }
    if ends_at.is_some_and(|e| e < starts_at) {
        return Err(bad("ends_at must not be before starts_at"));
    }
    if !(1..=1_000_000).contains(&capacity) || !(1..=180).contains(&hold) {
        return Err(bad("capacity is 1 or more and hold_minutes is 1 to 180"));
    }
    Ok(())
}

async fn check_shares(conn: &mut PgConnection, shares: &[ShareIn]) -> Result<Value, ApiError> {
    if shares.is_empty() || shares.len() > 20 {
        return Err(ApiError::invalid(
            "invalid_splits",
            "give between 1 and 20 vendor shares",
        ));
    }
    let mut ids: Vec<&str> = shares.iter().map(|s| s.vendor_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != shares.len()
        || shares.iter().any(|s| !(1..=10_000).contains(&s.share_bp))
        || shares.iter().map(|s| s.share_bp).sum::<i64>() > 10_000
    {
        return Err(ApiError::invalid(
            "invalid_splits",
            "each vendor once, each share 1-10000 basis points, together at most 10000",
        ));
    }
    for s in shares {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM vendors WHERE id = $1 AND archived_at IS NULL")
                .bind(&s.vendor_id)
                .fetch_optional(&mut *conn)
                .await?;
        match status.as_deref() {
            None => return Err(ApiError::not_found("vendor")),
            // orders would fail at payment time otherwise: only an active vendor can be given a share
            Some("active") => {}
            Some(st) => {
                return Err(ApiError::conflict(
                    "vendor_not_active",
                    format!(
                        "vendor {} is {st}; only an active vendor can be given a share",
                        s.vendor_id
                    ),
                ));
            }
        }
    }
    Ok(json!(
        shares
            .iter()
            .map(|s| json!({ "vendor_id": s.vendor_id, "share_bp": s.share_bp }))
            .collect::<Vec<_>>()
    ))
}

async fn insert_tier(
    conn: &mut PgConnection,
    tenant_id: &str,
    event_id: &str,
    t: &NewTier,
    position: i32,
) -> Result<String, ApiError> {
    let id = new_id("tt");
    sqlx::query(
        "INSERT INTO ticket_tiers (id, tenant_id, event_id, name, description, price_minor, quantity, per_order_max, sale_starts_at, sale_ends_at, early_bird_price_minor, early_bird_starts_at, early_bird_ends_at, allowed_methods, position)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(&id)
    .bind(tenant_id)
    .bind(event_id)
    .bind(t.name.trim())
    .bind(t.description.as_deref().unwrap_or(""))
    .bind(t.price_minor)
    .bind(t.quantity)
    .bind(t.per_order_max.unwrap_or(10))
    .bind(t.sale_starts_at)
    .bind(t.sale_ends_at)
    .bind(t.early_bird_price_minor)
    .bind(t.early_bird_starts_at)
    .bind(t.early_bird_ends_at)
    .bind(&t.allowed_methods)
    .bind(position)
    .execute(conn)
    .await?;
    Ok(id)
}

async fn replace_fields(
    conn: &mut PgConnection,
    tenant_id: &str,
    event_id: &str,
    fields: &[NewField],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM event_form_fields WHERE event_id = $1")
        .bind(event_id)
        .execute(&mut *conn)
        .await?;
    for (i, f) in fields.iter().enumerate() {
        sqlx::query("INSERT INTO event_form_fields (id, tenant_id, event_id, key, label, kind, required, options, min_len, max_len, position) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(new_id("eff"))
            .bind(tenant_id)
            .bind(event_id)
            .bind(&f.key)
            .bind(f.label.trim())
            .bind(&f.kind)
            .bind(f.required)
            .bind(f.options.as_ref().map(|o| json!(o)))
            .bind(f.min_len)
            .bind(f.max_len)
            .bind(i as i32)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `POST /v1/events`
pub async fn create_event(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewEvent>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("events:write")?;
    let mode = b.mode.as_deref().unwrap_or("venue");
    let hold = b.hold_minutes.unwrap_or(10);
    check_event_fields(
        &b.name,
        b.description.as_deref(),
        mode,
        b.venue.as_deref(),
        b.online_url.as_deref(),
        b.starts_at,
        b.ends_at,
        b.capacity,
        hold,
    )?;
    for t in b.tiers.iter().flatten() {
        validate_tier(t)?;
    }
    validate_field_defs(b.form_fields.as_deref().unwrap_or_default())?;
    let id = new_id("evt_tk");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let splits = match &b.splits {
        Some(sh) => Some(check_shares(&mut tx, sh).await?),
        None => None,
    };
    sqlx::query(
        "INSERT INTO ticket_events (id, tenant_id, name, description, mode, venue, online_url, starts_at, ends_at, capacity, hold_minutes, splits, qr_secret_enc, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(b.name.trim())
    .bind(b.description.as_deref().unwrap_or(""))
    .bind(mode)
    .bind(&b.venue)
    .bind(&b.online_url)
    .bind(b.starts_at)
    .bind(b.ends_at)
    .bind(b.capacity)
    .bind(hold)
    .bind(&splits)
    .bind(s.crypto.encrypt(&random_bytes()))
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    for (i, t) in b.tiers.iter().flatten().enumerate() {
        insert_tier(&mut tx, &auth.tenant_id, &id, t, i as i32).await?;
    }
    replace_fields(
        &mut tx,
        &auth.tenant_id,
        &id,
        b.form_fields.as_deref().unwrap_or_default(),
    )
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.created",
        Some(&id),
        json!({ "name": b.name, "capacity": b.capacity }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(body)))
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
    starting_after: Option<String>,
}

/// `GET /v1/events`
pub async fn list_events(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut rows: Vec<EventRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {EVENT_COLS} FROM ticket_events WHERE ($1::text IS NULL OR status = $1) AND ($2::text IS NULL OR id < $2) ORDER BY id DESC LIMIT $3"
    )))
    .bind(&q.status)
    .bind(&q.starting_after)
    .bind(limit + 1)
    .fetch_all(&mut *tx)
    .await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let data: Vec<Value> = rows
        .iter()
        .map(|e| json!({ "id": e.id, "name": e.name, "mode": e.mode, "venue": e.venue, "starts_at": e.starts_at, "ends_at": e.ends_at, "status": e.status, "capacity": e.capacity, "seats_held": e.seats_held, "seats_sold": e.seats_sold, "seats_available": (e.capacity - e.seats_held - e.seats_sold).max(0), "created_at": e.created_at }))
        .collect();
    Ok(Json(json!({ "data": data, "has_more": has_more })))
}

/// `GET /v1/events/{id}`
pub async fn get_event(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(event_json(&mut tx, &id).await?))
}

#[derive(Deserialize)]
pub struct EditEvent {
    name: Option<String>,
    description: Option<String>,
    venue: Option<String>,
    online_url: Option<String>,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
    capacity: Option<i32>,
    hold_minutes: Option<i32>,
}

/// `PUT /v1/events/{id}`: change the fields given. Capacity cannot drop below the seats already held or sold.
pub async fn edit_event(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<EditEvent>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    if e.status == "cancelled" {
        return Err(ApiError::conflict(
            "event_cancelled",
            "a cancelled event cannot be changed",
        ));
    }
    let name = b.name.unwrap_or(e.name);
    let description = b.description.unwrap_or(e.description);
    let venue = b.venue.or(e.venue);
    let online_url = b.online_url.or(e.online_url);
    let starts_at = b.starts_at.unwrap_or(e.starts_at);
    let ends_at = b.ends_at.or(e.ends_at);
    let capacity = b.capacity.unwrap_or(e.capacity);
    let hold = b.hold_minutes.unwrap_or(e.hold_minutes);
    check_event_fields(
        &name,
        Some(&description),
        &e.mode,
        venue.as_deref(),
        online_url.as_deref(),
        starts_at,
        ends_at,
        capacity,
        hold,
    )?;
    if capacity < e.seats_held + e.seats_sold {
        return Err(ApiError::conflict(
            "capacity_below_sold",
            format!(
                "{} seats are held or sold; capacity cannot be lower",
                e.seats_held + e.seats_sold
            ),
        ));
    }
    sqlx::query("UPDATE ticket_events SET name=$2, description=$3, venue=$4, online_url=$5, starts_at=$6, ends_at=$7, capacity=$8, hold_minutes=$9, updated_at=now() WHERE id=$1")
        .bind(&id).bind(name.trim()).bind(&description).bind(&venue).bind(&online_url).bind(starts_at).bind(ends_at).bind(capacity).bind(hold)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.updated",
        Some(&id),
        json!({ "capacity": capacity }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(body))
}

#[derive(Deserialize)]
pub struct StatusBody {
    /// `on_sale` or `closed`.
    status: String,
}

/// `POST /v1/events/{id}/status`: open sales (draft or closed to on_sale) or close them (on_sale to closed).
pub async fn set_status(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<StatusBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    let ok = matches!(
        (e.status.as_str(), b.status.as_str()),
        ("draft" | "closed", "on_sale") | ("on_sale", "closed")
    );
    if !ok {
        return Err(ApiError::conflict(
            "invalid_transition",
            format!("an event that is {} cannot move to {}", e.status, b.status),
        ));
    }
    if b.status == "on_sale" {
        let tiers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM ticket_tiers WHERE event_id = $1 AND archived_at IS NULL",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        if tiers == 0 {
            return Err(ApiError::conflict(
                "no_tiers",
                "add at least one ticket tier before opening sales",
            ));
        }
    }
    sqlx::query("UPDATE ticket_events SET status = $2, updated_at = now() WHERE id = $1")
        .bind(&id)
        .bind(&b.status)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.status_changed",
        Some(&id),
        json!({ "from": e.status, "to": b.status }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(body))
}

/// `POST /v1/events/{id}/tiers`
pub async fn add_tier(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<NewTier>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("events:write")?;
    validate_tier(&b)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    if e.status == "cancelled" {
        return Err(ApiError::conflict(
            "event_cancelled",
            "a cancelled event cannot be changed",
        ));
    }
    let pos: i32 = sqlx::query_scalar(
        "SELECT COALESCE(max(position) + 1, 0) FROM ticket_tiers WHERE event_id = $1",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    let tier_id = insert_tier(&mut tx, &auth.tenant_id, &id, &b, pos).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.tier_added",
        Some(&id),
        json!({ "tier_id": tier_id }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(body)))
}

/// `PUT /v1/events/{id}/tiers/{tier_id}`: replace a tier's settings. Orders already made keep the price they were made
/// at. Quantity cannot drop below the seats already held or sold.
pub async fn edit_tier(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, tier_id)): Path<(String, String)>,
    Json(b): Json<NewTier>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    validate_tier(&b)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    if e.status == "cancelled" {
        return Err(ApiError::conflict(
            "event_cancelled",
            "a cancelled event cannot be changed",
        ));
    }
    let taken: Option<i32> = sqlx::query_scalar("SELECT seats_held + seats_sold FROM ticket_tiers WHERE id = $1 AND event_id = $2 FOR UPDATE")
        .bind(&tier_id)
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let taken = taken.ok_or_else(|| ApiError::not_found("tier"))?;
    if b.quantity < taken {
        return Err(ApiError::conflict(
            "quantity_below_sold",
            format!("{taken} seats of this tier are held or sold"),
        ));
    }
    sqlx::query(
        "UPDATE ticket_tiers SET name=$2, description=$3, price_minor=$4, quantity=$5, per_order_max=$6, sale_starts_at=$7, sale_ends_at=$8, early_bird_price_minor=$9, early_bird_starts_at=$10, early_bird_ends_at=$11, allowed_methods=$12 WHERE id=$1",
    )
    .bind(&tier_id)
    .bind(b.name.trim())
    .bind(b.description.as_deref().unwrap_or(""))
    .bind(b.price_minor)
    .bind(b.quantity)
    .bind(b.per_order_max.unwrap_or(10))
    .bind(b.sale_starts_at)
    .bind(b.sale_ends_at)
    .bind(b.early_bird_price_minor)
    .bind(b.early_bird_starts_at)
    .bind(b.early_bird_ends_at)
    .bind(&b.allowed_methods)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.tier_updated",
        Some(&id),
        json!({ "tier_id": tier_id }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(body))
}

/// `POST /v1/events/{id}/tiers/{tier_id}/archive`: stop selling a tier. Tickets already sold stay valid.
pub async fn archive_tier(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, tier_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, true).await?;
    let n = sqlx::query("UPDATE ticket_tiers SET archived_at = now() WHERE id = $1 AND event_id = $2 AND archived_at IS NULL")
        .bind(&tier_id)
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("active tier"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.tier_archived",
        Some(&id),
        json!({ "tier_id": tier_id }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(body))
}

#[derive(Deserialize)]
pub struct FieldsBody {
    fields: Vec<NewField>,
}

/// `PUT /v1/events/{id}/form_fields`: replace the registration form. Answers already given are kept as they were.
pub async fn put_form_fields(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<FieldsBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    validate_field_defs(&b.fields)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    if e.status == "cancelled" {
        return Err(ApiError::conflict(
            "event_cancelled",
            "a cancelled event cannot be changed",
        ));
    }
    replace_fields(&mut tx, &auth.tenant_id, &id, &b.fields).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event.form_updated",
        Some(&id),
        json!({ "fields": b.fields.len() }),
    )
    .await?;
    let body = event_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(body))
}

// ---------------------------------------------------------------------------------------------------------------
// Seats: release, confirm, issue
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, FromRow)]
pub struct OrderCore {
    pub id: String,
    pub event_id: String,
    pub status: String,
    pub quantity: i32,
    pub total_minor: i64,
    pub coupon_id: Option<String>,
    pub intent_id: Option<String>,
    pub buyer_name: String,
}

const ORDER_CORE: &str =
    "id, event_id, status, quantity, total_minor, coupon_id, intent_id, buyer_name";

async fn adjust_counters(
    conn: &mut PgConnection,
    event_id: &str,
    order_id: &str,
    held: i32,
    sold: i32,
) -> Result<(), ApiError> {
    // `held` and `sold` are signed deltas per seat; the database refuses a result outside 0..=capacity
    let lines: Vec<(String, i32)> = sqlx::query_as(
        "SELECT tier_id, quantity FROM event_order_lines WHERE order_id = $1 ORDER BY tier_id",
    )
    .bind(order_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut total = 0;
    for (tier, q) in lines {
        total += q;
        sqlx::query("UPDATE ticket_tiers SET seats_held = seats_held + $2, seats_sold = seats_sold + $3 WHERE id = $1")
            .bind(&tier)
            .bind(held * q)
            .bind(sold * q)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("UPDATE ticket_events SET seats_held = seats_held + $2, seats_sold = seats_sold + $3 WHERE id = $1")
        .bind(event_id)
        .bind(held * total)
        .bind(sold * total)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Issue the tickets of a paid order. One row per seat, keyed by (order, seq), so a repeat issues nothing new.
pub async fn issue_tickets(
    conn: &mut PgConnection,
    tenant_id: &str,
    o: &OrderCore,
) -> Result<u64, ApiError> {
    let lines: Vec<(String, i32)> = sqlx::query_as(
        "SELECT tier_id, quantity FROM event_order_lines WHERE order_id = $1 ORDER BY tier_id",
    )
    .bind(&o.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut seq = 0;
    let mut made = 0;
    for (tier, q) in lines {
        for _ in 0..q {
            seq += 1;
            made += sqlx::query("INSERT INTO tickets (id, tenant_id, event_id, order_id, tier_id, seq, code, holder_name) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (order_id, seq) DO NOTHING")
                .bind(new_id("tk"))
                .bind(tenant_id)
                .bind(&o.event_id)
                .bind(&o.id)
                .bind(&tier)
                .bind(seq)
                .bind(new_code())
                .bind(&o.buyer_name)
                .execute(&mut *conn)
                .await?
                .rows_affected();
        }
    }
    Ok(made)
}

/// Turn a held order into a paid one and issue its tickets. The caller holds the event lock and the order row lock.
pub async fn mark_paid_locked(
    conn: &mut PgConnection,
    tenant_id: &str,
    o: &OrderCore,
    was: &str,
) -> Result<(), ApiError> {
    // from `pending` the seats move held -> sold; from an expired/cancelled order they were free and are taken again
    if was == "pending" {
        adjust_counters(conn, &o.event_id, &o.id, -1, 1).await?;
    } else {
        adjust_counters(conn, &o.event_id, &o.id, 0, 1).await?;
    }
    sqlx::query("UPDATE event_orders SET status = 'paid', paid_at = now(), hold_expires_at = NULL, cancelled_at = NULL, cancel_reason = NULL, updated_at = now() WHERE id = $1")
        .bind(&o.id)
        .execute(&mut *conn)
        .await?;
    issue_tickets(conn, tenant_id, o).await?;
    tenant::emit(conn, tenant_id, "ticket_order.paid", json!({ "order_id": o.id, "event_id": o.event_id, "quantity": o.quantity, "total_minor": o.total_minor, "intent_id": o.intent_id })).await?;
    Ok(())
}

/// Free the seats of an unpaid order (hold ran out, buyer cancelled, or the event was cancelled) and give its coupon use
/// back. The caller holds the event lock.
pub async fn release_order(
    conn: &mut PgConnection,
    tenant_id: &str,
    o: &OrderCore,
    status: &str,
    reason: &str,
) -> Result<(), ApiError> {
    adjust_counters(conn, &o.event_id, &o.id, -1, 0).await?;
    sqlx::query("UPDATE event_orders SET status = $2, cancelled_at = now(), cancel_reason = $3, hold_expires_at = NULL, updated_at = now() WHERE id = $1")
        .bind(&o.id)
        .bind(status)
        .bind(reason)
        .execute(&mut *conn)
        .await?;
    if let Some(c) = &o.coupon_id {
        sqlx::query("UPDATE coupons SET uses = uses - 1 WHERE id = $1 AND uses > 0")
            .bind(c)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(i) = &o.intent_id {
        // no new payment can start on it. SKIP LOCKED: a payment being applied right now holds the row; leave it, the
        // late-payment path below deals with it.
        sqlx::query("UPDATE payment_intents SET status = 'canceled', updated_at = now() WHERE id = (SELECT id FROM payment_intents WHERE id = $1 AND status = 'requires_payment_method' FOR UPDATE SKIP LOCKED)")
            .bind(i)
            .execute(&mut *conn)
            .await?;
    }
    if status == "expired" {
        tenant::emit(
            conn,
            tenant_id,
            "ticket_order.expired",
            json!({ "order_id": o.id, "event_id": o.event_id, "quantity": o.quantity }),
        )
        .await?;
    }
    Ok(())
}

/// Called once a provider-verified capture has been applied to an intent (inside the same transaction). Issues the
/// tickets of the order that intent belongs to. Safe to call again: a paid order is left alone.
pub async fn on_payment_captured(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
) -> Result<(), ApiError> {
    let ev: Option<String> =
        sqlx::query_scalar("SELECT event_id FROM event_orders WHERE intent_id = $1")
            .bind(intent_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(event_id) = ev else { return Ok(()) };
    // lock order: event first, then the order
    let e = fetch_event(conn, &event_id, true).await?;
    let o: OrderCore = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_CORE} FROM event_orders WHERE intent_id = $1 FOR UPDATE"
    )))
    .bind(intent_id)
    .fetch_one(&mut *conn)
    .await?;
    match o.status.as_str() {
        "pending" => {
            mark_paid_locked(conn, tenant_id, &o, "pending").await?;
        }
        "expired" | "cancelled" if e.status != "cancelled" => {
            // the hold ran out (or the buyer cancelled) before the money arrived: take the seats back if they are free
            let fits: bool = sqlx::query_scalar(
                "SELECT e.capacity - e.seats_held - e.seats_sold >= $2
                    AND NOT EXISTS (SELECT 1 FROM event_order_lines l JOIN ticket_tiers t ON t.id = l.tier_id
                                    WHERE l.order_id = $3 AND t.quantity - t.seats_held - t.seats_sold < l.quantity)
                   FROM ticket_events e WHERE e.id = $1",
            )
            .bind(&o.event_id)
            .bind(o.quantity)
            .bind(&o.id)
            .fetch_one(&mut *conn)
            .await?;
            if fits {
                mark_paid_locked(conn, tenant_id, &o, &o.status).await?;
                tenant::audit(
                    conn,
                    tenant_id,
                    "system:tickets",
                    "ticket_order.late_payment_seated",
                    Some(&o.id),
                    json!({ "was": o.status }),
                )
                .await?;
            } else {
                unfulfilled(conn, tenant_id, &o).await?;
            }
        }
        "expired" | "cancelled" => unfulfilled(conn, tenant_id, &o).await?,
        _ => {}
    }
    Ok(())
}

/// Money arrived for an order that can no longer have seats: queue its refund and tell a person.
async fn unfulfilled(
    conn: &mut PgConnection,
    tenant_id: &str,
    o: &OrderCore,
) -> Result<(), ApiError> {
    let Some(intent) = &o.intent_id else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE event_orders SET status = 'needs_refund', updated_at = now() WHERE id = $1",
    )
    .bind(&o.id)
    .execute(&mut *conn)
    .await?;
    let batch = new_id("erb");
    sqlx::query("INSERT INTO event_refund_batches (id, tenant_id, event_id, kind, reason, requested_by) VALUES ($1,$2,$3,'late_payment','Payment arrived after the seats were released','system:tickets')")
        .bind(&batch)
        .bind(tenant_id)
        .bind(&o.event_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO event_refund_items (id, tenant_id, batch_id, event_id, order_id, intent_id, amount_minor) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (order_id) DO NOTHING")
        .bind(new_id("eri"))
        .bind(tenant_id)
        .bind(&batch)
        .bind(&o.event_id)
        .bind(&o.id)
        .bind(intent)
        .bind(o.total_minor)
        .execute(&mut *conn)
        .await?;
    crate::cases::open_case(
        conn,
        tenant_id,
        "ticket_order_unfulfilled",
        "event_order",
        &o.id,
        Some(0),
        Some(o.total_minor),
        "A payment was captured for an order whose seats had already been released and are no longer free. It is queued for a refund.",
        json!({ "event_id": o.event_id, "intent_id": intent }),
    )
    .await?;
    tenant::emit(
        conn,
        tenant_id,
        "ticket_order.needs_refund",
        json!({ "order_id": o.id, "event_id": o.event_id, "total_minor": o.total_minor }),
    )
    .await?;
    Ok(())
}

/// Called when a refund is processed. A fully refunded order stops being valid: its tickets become `refunded` and, if
/// they still held seats, the seats are freed. A partial refund changes nothing.
pub async fn on_refund_processed(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
) -> Result<(), ApiError> {
    let ev: Option<String> =
        sqlx::query_scalar("SELECT event_id FROM event_orders WHERE intent_id = $1")
            .bind(intent_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(event_id) = ev else { return Ok(()) };
    let full: bool = sqlx::query_scalar("SELECT COALESCE(bool_or(refunded_minor >= amount_minor), false) FROM payment_attempts WHERE intent_id = $1 AND status = 'captured'")
        .bind(intent_id)
        .fetch_one(&mut *conn)
        .await?;
    if !full {
        return Ok(());
    }
    fetch_event(conn, &event_id, true).await?;
    let o: OrderCore = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_CORE} FROM event_orders WHERE intent_id = $1 FOR UPDATE"
    )))
    .bind(intent_id)
    .fetch_one(&mut *conn)
    .await?;
    match o.status.as_str() {
        "paid" => {
            // refunded through the ordinary refund endpoint: the seats go back on sale
            let live: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets WHERE order_id = $1 AND status IN ('issued', 'checked_in')")
                .bind(&o.id)
                .fetch_one(&mut *conn)
                .await?;
            if live > 0 {
                adjust_counters(conn, &o.event_id, &o.id, 0, -1).await?;
            }
        }
        "cancelled" | "needs_refund" => {}
        _ => return Ok(()),
    }
    sqlx::query("UPDATE event_orders SET status = 'refunded', updated_at = now() WHERE id = $1")
        .bind(&o.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE tickets SET status = 'refunded' WHERE order_id = $1 AND status IN ('issued', 'checked_in', 'void')")
        .bind(&o.id)
        .execute(&mut *conn)
        .await?;
    tenant::audit(
        conn,
        tenant_id,
        "system:tickets",
        "ticket_order.refunded",
        Some(&o.id),
        json!({ "intent_id": intent_id }),
    )
    .await?;
    sqlx::query("UPDATE recon_cases SET status = 'resolved', resolution = 'refunded', resolved_at = now() WHERE kind = 'ticket_order_unfulfilled' AND subject_id = $1 AND status <> 'resolved'")
        .bind(&o.id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The worker's pass over one tenant's holds: release every unpaid order whose hold has run out. An order whose payment
/// is authorised, unknown, or was started in the last five minutes keeps its seats until that is settled. Returns how
/// many orders were released.
pub async fn expire_holds(conn: &mut PgConnection, tenant_id: &str) -> Result<usize, ApiError> {
    let events: Vec<String> = sqlx::query_scalar("SELECT DISTINCT event_id FROM event_orders WHERE status = 'pending' AND hold_expires_at <= now() LIMIT 100")
        .fetch_all(&mut *conn)
        .await?;
    let mut released = 0;
    for ev in events {
        fetch_event(conn, &ev, true).await?;
        let due: Vec<OrderCore> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {ORDER_CORE} FROM event_orders o WHERE event_id = $1 AND status = 'pending' AND hold_expires_at <= now()
               AND NOT EXISTS (SELECT 1 FROM payment_attempts a WHERE a.intent_id = o.intent_id
                               AND (a.status IN ('unknown', 'authorized') OR (a.status IN ('created', 'pending') AND a.updated_at > now() - interval '5 minutes')))
             ORDER BY id FOR UPDATE"
        )))
        .bind(&ev)
        .fetch_all(&mut *conn)
        .await?;
        for o in due {
            release_order(conn, tenant_id, &o, "expired", "hold expired").await?;
            released += 1;
        }
    }
    Ok(released)
}

// ---------------------------------------------------------------------------------------------------------------
// Orders (organiser view)
// ---------------------------------------------------------------------------------------------------------------

async fn order_json(conn: &mut PgConnection, id: &str) -> Result<Value, ApiError> {
    #[allow(clippy::type_complexity)]
    let o: Option<(String, String, String, String, String, Option<String>, Value, i64, i64, i64, i32, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>, DateTime<Utc>, Option<String>)> = sqlx::query_as(
        "SELECT id, event_id, status, buyer_name, buyer_email, buyer_phone, answers, subtotal_minor, discount_minor, total_minor, quantity, intent_id, hold_expires_at, paid_at, created_at, cancel_reason FROM event_orders WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let (
        oid,
        event_id,
        status,
        name,
        email,
        phone,
        answers,
        sub,
        disc,
        total,
        qty,
        intent,
        hold,
        paid,
        created,
        cancel_reason,
    ) = o.ok_or_else(|| ApiError::not_found("order"))?;
    let lines: Vec<(String, String, i32, i64, String)> = sqlx::query_as("SELECT l.tier_id, t.name, l.quantity, l.unit_price_minor, l.price_kind FROM event_order_lines l JOIN ticket_tiers t ON t.id = l.tier_id WHERE l.order_id = $1 ORDER BY l.tier_id")
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
    let tickets: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, code, status, tier_id FROM tickets WHERE order_id = $1 ORDER BY seq",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(json!({
        "id": oid, "event_id": event_id, "status": status,
        "buyer": { "name": name, "email": email, "phone": phone }, "answers": answers,
        "quantity": qty, "subtotal_minor": sub, "discount_minor": disc, "total_minor": total, "currency": "INR",
        "intent_id": intent, "hold_expires_at": hold, "paid_at": paid, "created_at": created, "cancel_reason": cancel_reason,
        "lines": lines.into_iter().map(|(t, n, q, p, k)| json!({ "tier_id": t, "tier_name": n, "quantity": q, "unit_price_minor": p, "price_kind": k })).collect::<Vec<_>>(),
        "tickets": tickets.into_iter().map(|(i, c, s, t)| json!({ "id": i, "code": c, "status": s, "tier_id": t })).collect::<Vec<_>>(),
    }))
}

/// `GET /v1/events/{id}/orders`
pub async fn list_orders(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, false).await?;
    let mut ids: Vec<String> = sqlx::query_scalar("SELECT id FROM event_orders WHERE event_id = $1 AND ($2::text IS NULL OR status = $2) AND ($3::text IS NULL OR id < $3) ORDER BY id DESC LIMIT $4")
        .bind(&id)
        .bind(&q.status)
        .bind(&q.starting_after)
        .bind(limit + 1)
        .fetch_all(&mut *tx)
        .await?;
    let has_more = ids.len() as i64 > limit;
    ids.truncate(limit as usize);
    let mut data = Vec::new();
    for oid in ids {
        data.push(order_json(&mut tx, &oid).await?);
    }
    Ok(Json(json!({ "data": data, "has_more": has_more })))
}

/// `GET /v1/events/{id}/orders/{order_id}`
pub async fn get_order(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, oid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let o = order_json(&mut tx, &oid).await?;
    if o["event_id"] != id {
        return Err(ApiError::not_found("order"));
    }
    Ok(Json(o))
}

// ---------------------------------------------------------------------------------------------------------------
// Attendees
// ---------------------------------------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct AttendeeQuery {
    /// Matches name, email, phone or ticket code.
    q: Option<String>,
    status: Option<String>,
    tier_id: Option<String>,
    limit: Option<i64>,
    starting_after: Option<String>,
}

#[derive(FromRow)]
struct Attendee {
    id: String,
    code: String,
    status: String,
    holder_name: String,
    tier_id: String,
    tier_name: String,
    order_id: String,
    buyer_email: String,
    buyer_phone: Option<String>,
    answers: Value,
    issued_at: DateTime<Utc>,
    checked_in_at: Option<DateTime<Utc>>,
    checked_in_by: Option<String>,
    paid_at: Option<DateTime<Utc>>,
}

const ATTENDEE_SQL: &str = "SELECT t.id, t.code, t.status, t.holder_name, t.tier_id, tt.name AS tier_name, t.order_id, o.buyer_email, o.buyer_phone, o.answers, t.issued_at, t.checked_in_at, t.checked_in_by, o.paid_at
  FROM tickets t JOIN event_orders o ON o.id = t.order_id JOIN ticket_tiers tt ON tt.id = t.tier_id
  WHERE t.event_id = $1
    AND ($2::text IS NULL OR t.holder_name ILIKE '%' || $2 || '%' OR o.buyer_email ILIKE '%' || $2 || '%' OR o.buyer_phone ILIKE '%' || $2 || '%' OR t.code = upper($2))
    AND ($3::text IS NULL OR t.status = $3) AND ($4::text IS NULL OR t.tier_id = $4)
    AND ($5::text IS NULL OR t.id > $5)
  ORDER BY t.id LIMIT $6";

fn escape_like(q: &str) -> String {
    q.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

async fn attendees(
    conn: &mut PgConnection,
    event_id: &str,
    q: &AttendeeQuery,
    limit: i64,
) -> Result<Vec<Attendee>, ApiError> {
    Ok(sqlx::query_as(ATTENDEE_SQL)
        .bind(event_id)
        .bind(
            q.q.as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(escape_like),
        )
        .bind(&q.status)
        .bind(&q.tier_id)
        .bind(&q.starting_after)
        .bind(limit)
        .fetch_all(conn)
        .await?)
}

/// `GET /v1/events/{id}/attendees`: one row per ticket, searchable.
pub async fn list_attendees(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<AttendeeQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, false).await?;
    let mut rows = attendees(&mut tx, &id, &q, limit + 1).await?;
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let data: Vec<Value> = rows
        .iter()
        .map(|a| json!({ "ticket_id": a.id, "code": a.code, "status": a.status, "holder_name": a.holder_name, "tier_id": a.tier_id, "tier_name": a.tier_name, "order_id": a.order_id, "email": a.buyer_email, "phone": a.buyer_phone, "answers": a.answers, "issued_at": a.issued_at, "checked_in_at": a.checked_in_at, "checked_in_by": a.checked_in_by }))
        .collect();
    Ok(Json(json!({ "data": data, "has_more": has_more })))
}

/// `GET /v1/events/{id}/attendees.csv`: every ticket (up to 20,000) with the answers to the registration form.
pub async fn attendees_csv(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<AttendeeQuery>,
) -> Result<Response, ApiError> {
    auth.require("events:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, false).await?;
    let fields = load_fields(&mut tx, &id).await?;
    let mut head: Vec<String> = [
        "ticket_code",
        "status",
        "tier",
        "holder_name",
        "email",
        "phone",
        "order_id",
        "paid_at",
        "checked_in_at",
        "checked_in_by",
    ]
    .map(String::from)
    .to_vec();
    head.extend(fields.iter().map(|f| f.key.clone()));
    let mut out = csv_row(&head);
    let mut query = AttendeeQuery {
        q: q.q.clone(),
        status: q.status.clone(),
        tier_id: q.tier_id.clone(),
        limit: None,
        starting_after: None,
    };
    let mut total = 0;
    loop {
        let rows = attendees(&mut tx, &id, &query, 1000).await?;
        let Some(last) = rows.last().map(|r| r.id.clone()) else {
            break;
        };
        total += rows.len();
        for a in &rows {
            let mut cells = vec![
                a.code.clone(),
                a.status.clone(),
                a.tier_name.clone(),
                a.holder_name.clone(),
                a.buyer_email.clone(),
                a.buyer_phone.clone().unwrap_or_default(),
                a.order_id.clone(),
                a.paid_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                a.checked_in_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                a.checked_in_by.clone().unwrap_or_default(),
            ];
            for f in &fields {
                cells.push(match &a.answers[&f.key] {
                    Value::String(s) => s.clone(),
                    Value::Bool(b) => b.to_string(),
                    _ => String::new(),
                });
            }
            out.push_str(&csv_row(&cells));
        }
        query.starting_after = Some(last);
        if total >= 20_000 {
            break;
        }
    }
    Ok(csv_response("attendees.csv", out))
}

// ---------------------------------------------------------------------------------------------------------------
// Check-in
// ---------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ScanBody {
    /// The QR payload (`ZT1.…`) or the ticket code typed by hand.
    code: String,
}

struct Outcome {
    result: &'static str,
    ticket: Option<(
        String,
        String,
        String,
        String,
        Option<DateTime<Utc>>,
        Option<String>,
    )>,
}

impl Outcome {
    fn json(&self, code: &str, manual: bool) -> Value {
        let valid = self.result == "ok";
        let (ticket, first_at, first_by) = match &self.ticket {
            Some((id, tier, holder, order, at, by)) => (
                json!({ "id": id, "tier": tier, "holder_name": holder, "order_id": order }),
                *at,
                by.clone(),
            ),
            None => (Value::Null, None, None),
        };
        json!({
            "result": self.result, "valid": valid, "code": code, "manual_entry": manual, "ticket": ticket,
            "checked_in_at": first_at, "checked_in_by": first_by,
            "message": match self.result {
                "ok" => "Welcome. Ticket checked in.",
                "duplicate" => "This ticket was already scanned.",
                "refunded" => "This ticket was refunded and is not valid.",
                "void" => "This ticket was cancelled and is not valid.",
                "wrong_event" => "This ticket is for another event.",
                _ => "Not a valid ticket for this event.",
            },
        })
    }
}

/// Check one ticket in. `at` is when it was really scanned (an offline device syncing later); it is never in the future.
async fn scan(
    conn: &mut PgConnection,
    event_id: &str,
    secret: &[u8],
    raw: &str,
    actor: &str,
    at: Option<DateTime<Utc>>,
    offline: bool,
) -> Result<(Outcome, String, bool), ApiError> {
    let parsed = parse_scan(raw);
    let manual = matches!(parsed, Some(Scanned::Bare(_)));
    let (outcome, code) = match parsed {
        None => (
            Outcome {
                result: "invalid",
                ticket: None,
            },
            raw.chars().take(40).collect::<String>(),
        ),
        Some(Scanned::Signed { code, sig }) if !verify_sig(secret, event_id, &code, &sig) => {
            // a code with the wrong signature may still be a real ticket of another event: do not say which
            (
                Outcome {
                    result: "invalid",
                    ticket: None,
                },
                code,
            )
        }
        Some(Scanned::Signed { code, .. } | Scanned::Bare(code)) => {
            let row: Option<(String, String, String, String, String, String, Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
                "SELECT t.id, t.status, tt.name, t.holder_name, t.order_id, t.event_id, t.checked_in_at, t.checked_in_by FROM tickets t JOIN ticket_tiers tt ON tt.id = t.tier_id WHERE t.code = $1 FOR UPDATE OF t",
            )
            .bind(&code)
            .fetch_optional(&mut *conn)
            .await?;
            match row {
                None => (
                    Outcome {
                        result: "invalid",
                        ticket: None,
                    },
                    code,
                ),
                Some((_, _, _, _, _, ev, _, _)) if ev != event_id => (
                    Outcome {
                        result: "wrong_event",
                        ticket: None,
                    },
                    code,
                ),
                Some((id, status, tier, holder, order, _, first_at, first_by)) => {
                    let t = |at, by| {
                        Some((
                            id.clone(),
                            tier.clone(),
                            holder.clone(),
                            order.clone(),
                            at,
                            by,
                        ))
                    };
                    match status.as_str() {
                        "issued" => {
                            let when = at.map_or_else(Utc::now, |a| a.min(Utc::now()));
                            sqlx::query("UPDATE tickets SET status = 'checked_in', checked_in_at = $2, checked_in_by = $3 WHERE id = $1 AND status = 'issued'")
                                .bind(&id)
                                .bind(when)
                                .bind(actor)
                                .execute(&mut *conn)
                                .await?;
                            (
                                Outcome {
                                    result: "ok",
                                    ticket: t(Some(when), Some(actor.to_string())),
                                },
                                code,
                            )
                        }
                        "checked_in" => (
                            Outcome {
                                result: "duplicate",
                                ticket: t(first_at, first_by),
                            },
                            code,
                        ),
                        "refunded" => (
                            Outcome {
                                result: "refunded",
                                ticket: t(None, None),
                            },
                            code,
                        ),
                        _ => (
                            Outcome {
                                result: "void",
                                ticket: t(None, None),
                            },
                            code,
                        ),
                    }
                }
            }
        }
    };
    let ticket_id = outcome.ticket.as_ref().map(|t| t.0.clone());
    sqlx::query("INSERT INTO ticket_scans (tenant_id, event_id, ticket_id, code, result, actor, scanned_at, offline) SELECT tenant_id, id, $2, $3, $4, $5, COALESCE($6, now()), $7 FROM ticket_events WHERE id = $1")
        .bind(event_id)
        .bind(&ticket_id)
        .bind(&code)
        .bind(outcome.result)
        .bind(actor)
        .bind(at.map(|a| a.min(Utc::now())))
        .bind(offline)
        .execute(&mut *conn)
        .await?;
    Ok((outcome, code, manual))
}

/// `POST /v1/events/{id}/checkin`: check a ticket in. Always answers 200 with `result`: `ok`, `duplicate` (with who
/// scanned it first and when), `refunded`, `void`, `wrong_event` or `invalid`.
pub async fn checkin(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<ScanBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:checkin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let secret = event_secret(&s, &mut tx, &id).await?;
    let (out, code, manual) =
        scan(&mut tx, &id, &secret, &b.code, &auth.actor, None, false).await?;
    tx.commit().await?;
    Ok(Json(out.json(&code, manual)))
}

#[derive(Deserialize)]
pub struct SyncScan {
    code: String,
    /// When the door device scanned it.
    scanned_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct SyncBody {
    scans: Vec<SyncScan>,
}

/// `POST /v1/events/{id}/checkin/sync`: upload scans a device collected offline (up to 1000 at a time). They are applied
/// oldest first with their own scan times; a ticket already checked in is reported as a duplicate, never checked in twice.
pub async fn checkin_sync(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(mut b): Json<SyncBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:checkin")?;
    if b.scans.is_empty() || b.scans.len() > 1000 {
        return Err(ApiError::invalid("invalid_scans", "send 1 to 1000 scans"));
    }
    b.scans.sort_by_key(|x| x.scanned_at);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let secret = event_secret(&s, &mut tx, &id).await?;
    let mut results = Vec::new();
    let mut counts = std::collections::BTreeMap::<&str, i64>::new();
    for sc in &b.scans {
        let (out, code, manual) = scan(
            &mut tx,
            &id,
            &secret,
            &sc.code,
            &auth.actor,
            Some(sc.scanned_at),
            true,
        )
        .await?;
        *counts.entry(out.result).or_default() += 1;
        results.push(out.json(&code, manual));
    }
    tx.commit().await?;
    Ok(Json(json!({ "counts": counts, "results": results })))
}

#[derive(Deserialize, Default)]
pub struct ExportQuery {
    /// `json` (default) or `csv`.
    format: Option<String>,
}

/// `GET /v1/events/{id}/checkin/export`: the valid tickets of an event for a door device to keep offline. Refunded and
/// cancelled tickets are left out; checked-in ones are included with their status so the device can still flag a repeat.
/// The list is a snapshot: tickets refunded after it was taken are not in it until the next export.
pub async fn checkin_export(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    auth.require("events:checkin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, false).await?;
    let secret = event_secret(&s, &mut tx, &id).await?;
    let rows: Vec<(String, String, String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT t.code, t.status, tt.name, t.holder_name, t.checked_in_at FROM tickets t JOIN ticket_tiers tt ON tt.id = t.tier_id WHERE t.event_id = $1 AND t.status IN ('issued', 'checked_in') ORDER BY t.code",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    if q.format.as_deref() == Some("csv") {
        let mut out = csv_row(
            &[
                "code",
                "qr_payload",
                "status",
                "tier",
                "holder_name",
                "checked_in_at",
            ]
            .map(String::from),
        );
        for (code, status, tier, holder, at) in &rows {
            out.push_str(&csv_row(&[
                code.clone(),
                qr_payload(&secret, &id, code),
                status.clone(),
                tier.clone(),
                holder.clone(),
                at.map(|t| t.to_rfc3339()).unwrap_or_default(),
            ]));
        }
        return Ok(csv_response("valid-tickets.csv", out));
    }
    let tickets: Vec<Value> = rows
        .iter()
        .map(|(code, status, tier, holder, at)| json!({ "code": code, "qr_payload": qr_payload(&secret, &id, code), "status": status, "tier": tier, "holder_name": holder, "checked_in_at": at }))
        .collect();
    Ok(Json(json!({ "event": { "id": e.id, "name": e.name, "starts_at": e.starts_at }, "generated_at": Utc::now(), "count": tickets.len(), "tickets": tickets })).into_response())
}

/// `GET /v1/events/{id}/checkin/stats`: how many are in, how many to come, and the latest double scans.
pub async fn checkin_stats(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:checkin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, false).await?;
    let by: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM tickets WHERE event_id = $1 GROUP BY status")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
    let n = |k: &str| by.iter().find(|(s, _)| s == k).map_or(0, |(_, c)| *c);
    let dupes: Vec<(String, String, DateTime<Utc>)> = sqlx::query_as("SELECT code, actor, scanned_at FROM ticket_scans WHERE event_id = $1 AND result = 'duplicate' ORDER BY id DESC LIMIT 20")
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?;
    Ok(Json(json!({
        "checked_in": n("checked_in"), "expected": n("issued"), "refunded": n("refunded"), "void": n("void"),
        "recent_duplicates": dupes.into_iter().map(|(code, actor, at)| json!({ "code": code, "scanned_by": actor, "scanned_at": at })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------------------------------------------------------
// Cancellation and bulk refunds
// ---------------------------------------------------------------------------------------------------------------

async fn queue_refund(
    conn: &mut PgConnection,
    tenant_id: &str,
    batch: &str,
    event_id: &str,
    o: &OrderCore,
) -> Result<bool, ApiError> {
    let Some(intent) = &o.intent_id else {
        return Ok(false);
    };
    if o.total_minor <= 0 {
        return Ok(false);
    }
    let n = sqlx::query("INSERT INTO event_refund_items (id, tenant_id, batch_id, event_id, order_id, intent_id, amount_minor) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (order_id) DO NOTHING")
        .bind(new_id("eri"))
        .bind(tenant_id)
        .bind(batch)
        .bind(event_id)
        .bind(&o.id)
        .bind(intent)
        .bind(o.total_minor)
        .execute(conn)
        .await?
        .rows_affected();
    Ok(n == 1)
}

async fn new_batch(
    conn: &mut PgConnection,
    tenant_id: &str,
    event_id: &str,
    kind: &str,
    reason: Option<&str>,
    actor: &str,
) -> Result<String, ApiError> {
    let id = new_id("erb");
    sqlx::query("INSERT INTO event_refund_batches (id, tenant_id, event_id, kind, reason, requested_by) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&id)
        .bind(tenant_id)
        .bind(event_id)
        .bind(kind)
        .bind(reason)
        .bind(actor)
        .execute(conn)
        .await?;
    Ok(id)
}

/// Progress of one batch: how many orders, how far each refund has got, and the money in each state.
pub async fn batch_progress(conn: &mut PgConnection, batch_id: &str) -> Result<Value, ApiError> {
    let head: Option<(String, String, String, Option<String>, String, DateTime<Utc>)> = sqlx::query_as("SELECT id, event_id, kind, reason, requested_by, created_at FROM event_refund_batches WHERE id = $1")
        .bind(batch_id)
        .fetch_optional(&mut *conn)
        .await?;
    let (id, event_id, kind, reason, by, at) =
        head.ok_or_else(|| ApiError::not_found("refund batch"))?;
    let rows: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT i.state, r.status, count(*), COALESCE(sum(i.amount_minor), 0)::BIGINT FROM event_refund_items i LEFT JOIN refunds r ON r.id = i.refund_id WHERE i.batch_id = $1 GROUP BY 1, 2",
    )
    .bind(batch_id)
    .fetch_all(&mut *conn)
    .await?;
    let (mut total, mut queued, mut errors, mut requested) = (0, 0, 0, 0);
    let mut refunds = std::collections::BTreeMap::<String, i64>::new();
    let mut money = std::collections::BTreeMap::<String, i64>::new();
    for (state, rstatus, n, sum) in rows {
        total += n;
        match state.as_str() {
            "queued" => queued += n,
            "error" => errors += n,
            _ => requested += n,
        }
        if let Some(rs) = rstatus {
            *refunds.entry(rs.clone()).or_default() += n;
            *money.entry(rs).or_default() += sum;
        }
    }
    let open = queued
        + refunds.get("processing").copied().unwrap_or(0)
        + refunds.get("pending_approval").copied().unwrap_or(0)
        + refunds.get("unknown").copied().unwrap_or(0);
    Ok(json!({
        "id": id, "event_id": event_id, "kind": kind, "reason": reason, "requested_by": by, "created_at": at,
        "orders": total, "queued": queued, "requested": requested, "errors": errors,
        "refunds_by_status": refunds, "amount_minor_by_status": money,
        "finished": total > 0 && open == 0 && errors == 0,
        "needs_attention": errors > 0 || refunds.get("failed").copied().unwrap_or(0) > 0 || refunds.get("rejected").copied().unwrap_or(0) > 0,
    }))
}

/// Send up to `limit` queued refunds of a tenant (optionally one batch) through the refund flow, each in its own
/// transaction. A refund the over-refund check or the provider refuses becomes an `error` item with the reason;
/// one above the approval threshold waits for a second person like any other refund. Returns how many were handled.
pub async fn process_refund_items(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    batch: Option<&str>,
    limit: i64,
) -> Result<usize, ApiError> {
    let queued: Vec<(String, String, String, String, String, Option<String>)> = {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        sqlx::query_as(
            "SELECT i.id, i.intent_id, i.order_id, b.requested_by, b.kind, b.reason FROM event_refund_items i JOIN event_refund_batches b ON b.id = i.batch_id
             WHERE i.state = 'queued' AND ($1::text IS NULL OR i.batch_id = $1) ORDER BY i.id LIMIT $2",
        )
        .bind(batch)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?
    };
    let mut handled = 0;
    for (item, intent, order, actor, kind, reason) in queued {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let claimed: Option<String> = sqlx::query_scalar("SELECT id FROM event_refund_items WHERE id = $1 AND state = 'queued' FOR UPDATE SKIP LOCKED")
            .bind(&item)
            .fetch_optional(&mut *tx)
            .await?;
        if claimed.is_none() {
            continue;
        }
        let why = match kind.as_str() {
            "event_cancel" => "Event cancelled",
            "order_cancel" => "Order cancelled",
            _ => "Payment arrived after seats were released",
        };
        let text = match &reason {
            Some(r) if !r.is_empty() => format!("{why}: {r}"),
            _ => why.to_string(),
        };
        match refunds::reserve_refund(
            &mut tx,
            tenant_id,
            &actor,
            &intent,
            None,
            Some(&text),
            Some(&order),
        )
        .await
        {
            Ok(r) => {
                sqlx::query("UPDATE event_refund_items SET state = 'requested', refund_id = $2, error = NULL, updated_at = now() WHERE id = $1")
                    .bind(&item)
                    .bind(&r.refund_id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                if !r.needs_approval
                    && let Err(e) =
                        refunds::run_provider_refund(s, tenant_id, env, &r.refund_id).await
                {
                    tracing::warn!(refund = %r.refund_id, error = ?e.message, "bulk refund could not be sent; the resolver will pick it up");
                }
            }
            Err(e) => {
                drop(tx);
                let mut tx = tenant::begin(&s.pool, tenant_id).await?;
                sqlx::query("UPDATE event_refund_items SET state = 'error', error = $2, updated_at = now() WHERE id = $1")
                    .bind(&item)
                    .bind(e.message.chars().take(300).collect::<String>())
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
        }
        handled += 1;
    }
    Ok(handled)
}

#[derive(Deserialize, Default)]
pub struct CancelBody {
    reason: Option<String>,
    /// Single order only: cancel even if some of its tickets were already used at the door.
    force: Option<bool>,
}

/// `POST /v1/events/{id}/cancel`: cancel the event. Sales stop, unpaid orders are released, every paid ticket becomes
/// void and a refund for each paid order is queued as one batch (with progress) and sent through the ordinary refund
/// flow. Needs `events:write` and `refunds:write`. Calling it again returns the same batch.
pub async fn cancel_event(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<CancelBody>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    auth.require("refunds:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    if b.reason.as_deref().is_some_and(|r| r.len() > 500) {
        return Err(ApiError::invalid(
            "invalid_reason",
            "reason is up to 500 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = fetch_event(&mut tx, &id, true).await?;
    if e.status == "cancelled" {
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM event_refund_batches WHERE event_id = $1 AND kind = 'event_cancel' ORDER BY id LIMIT 1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
        let batch = match existing {
            Some(b) => Some(batch_progress(&mut tx, &b).await?),
            None => None,
        };
        return Ok(Json(
            json!({ "event_id": id, "status": "cancelled", "already_cancelled": true, "refund_batch": batch }),
        ));
    }
    sqlx::query("UPDATE ticket_events SET status = 'cancelled', cancelled_at = now(), updated_at = now() WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let batch = new_batch(
        &mut tx,
        &auth.tenant_id,
        &id,
        "event_cancel",
        b.reason.as_deref(),
        &auth.actor,
    )
    .await?;
    let orders: Vec<OrderCore> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ORDER_CORE} FROM event_orders WHERE event_id = $1 AND status IN ('pending', 'paid') ORDER BY id FOR UPDATE")))
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?;
    let (mut released, mut cancelled_paid, mut queued) = (0, 0, 0);
    for o in &orders {
        if o.status == "pending" {
            release_order(&mut tx, &auth.tenant_id, o, "cancelled", "event cancelled").await?;
            released += 1;
        } else {
            cancel_paid_order(&mut tx, o, "event cancelled").await?;
            cancelled_paid += 1;
            if queue_refund(&mut tx, &auth.tenant_id, &batch, &id, o).await? {
                queued += 1;
            }
        }
    }
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "event.cancelled", Some(&id), json!({ "reason": b.reason, "released_unpaid": released, "paid_orders": cancelled_paid, "refunds_queued": queued, "batch": batch })).await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "event.cancelled",
        json!({ "event_id": id, "refund_batch_id": batch, "refunds_queued": queued }),
    )
    .await?;
    tx.commit().await?;
    // the first slice now, so a small event is done when this returns; the worker takes the rest
    process_refund_items(&s, &auth.tenant_id, &auth.env, Some(&batch), 50).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let progress = batch_progress(&mut tx, &batch).await?;
    Ok(Json(
        json!({ "event_id": id, "status": "cancelled", "released_unpaid_orders": released, "paid_orders_cancelled": cancelled_paid, "refund_batch": progress }),
    ))
}

/// A paid order is cancelled: its tickets stop working now, its seats are freed, the refund follows.
async fn cancel_paid_order(
    conn: &mut PgConnection,
    o: &OrderCore,
    reason: &str,
) -> Result<(), ApiError> {
    adjust_counters(conn, &o.event_id, &o.id, 0, -1).await?;
    sqlx::query("UPDATE tickets SET status = 'void' WHERE order_id = $1 AND status IN ('issued', 'checked_in')")
        .bind(&o.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE event_orders SET status = 'cancelled', cancelled_at = now(), cancel_reason = $2, updated_at = now() WHERE id = $1")
        .bind(&o.id)
        .bind(reason)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `POST /v1/events/{id}/orders/{order_id}/cancel`: cancel one order. An unpaid order is released; a paid one has its
/// tickets voided and a refund queued through the ordinary refund flow. Refused when a ticket was already used at the
/// door unless `force` is set. Needs `events:write` and `refunds:write`.
pub async fn cancel_order(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, oid)): Path<(String, String)>,
    body: Option<Json<CancelBody>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:write")?;
    auth.require("refunds:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let reason = b
        .reason
        .clone()
        .unwrap_or_else(|| "cancelled by the organiser".into());
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, true).await?;
    let o: OrderCore = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_CORE} FROM event_orders WHERE id = $1 AND event_id = $2 FOR UPDATE"
    )))
    .bind(&oid)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("order"))?;
    let batch = match o.status.as_str() {
        "pending" => {
            release_order(&mut tx, &auth.tenant_id, &o, "cancelled", &reason).await?;
            None
        }
        "paid" => {
            let used: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM tickets WHERE order_id = $1 AND status = 'checked_in'",
            )
            .bind(&oid)
            .fetch_one(&mut *tx)
            .await?;
            if used > 0 && !b.force.unwrap_or(false) {
                return Err(ApiError::conflict(
                    "tickets_used",
                    format!(
                        "{used} ticket(s) of this order were already checked in; set force to cancel anyway"
                    ),
                ));
            }
            cancel_paid_order(&mut tx, &o, &reason).await?;
            let batch = new_batch(
                &mut tx,
                &auth.tenant_id,
                &id,
                "order_cancel",
                Some(&reason),
                &auth.actor,
            )
            .await?;
            queue_refund(&mut tx, &auth.tenant_id, &batch, &id, &o).await?;
            Some(batch)
        }
        other => {
            return Err(ApiError::conflict(
                "order_not_cancellable",
                format!("the order is {other}"),
            ));
        }
    };
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "event_order.cancelled",
        Some(&oid),
        json!({ "reason": reason, "batch": batch }),
    )
    .await?;
    tx.commit().await?;
    if let Some(bid) = &batch {
        process_refund_items(&s, &auth.tenant_id, &auth.env, Some(bid), 5).await?;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let progress = match &batch {
        Some(bid) => Some(batch_progress(&mut tx, bid).await?),
        None => None,
    };
    Ok(Json(
        json!({ "order": order_json(&mut tx, &oid).await?, "refund_batch": progress }),
    ))
}

#[derive(Deserialize, Default)]
pub struct ProcessBody {
    /// Put items that ended in `error` back in the queue first.
    retry_errors: Option<bool>,
}

/// `POST /v1/events/{id}/refund_batches/{batch_id}/process`: send the next slice (50) of a batch's queued refunds now,
/// instead of waiting for the worker. Needs `refunds:write`.
pub async fn process_batch(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, bid)): Path<(String, String)>,
    body: Option<Json<ProcessBody>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("refunds:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM event_refund_batches WHERE id = $1 AND event_id = $2)",
        )
        .bind(&bid)
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(ApiError::not_found("refund batch"));
        }
        if b.retry_errors.unwrap_or(false) {
            sqlx::query("UPDATE event_refund_items SET state = 'queued', error = NULL, updated_at = now() WHERE batch_id = $1 AND state = 'error'")
                .bind(&bid)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    process_refund_items(&s, &auth.tenant_id, &auth.env, Some(&bid), 50).await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(batch_progress(&mut tx, &bid).await?))
}

/// `GET /v1/events/{id}/refund_batches/{batch_id}`
pub async fn get_batch(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, bid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let p = batch_progress(&mut tx, &bid).await?;
    if p["event_id"] != id {
        return Err(ApiError::not_found("refund batch"));
    }
    Ok(Json(p))
}

/// `GET /v1/events/{id}/refund_report`: everything refunded (or being refunded) for an event: money by refund status,
/// the batches, and one line per order with its refund.
pub async fn refund_report(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("events:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch_event(&mut tx, &id, false).await?;
    let batches: Vec<String> =
        sqlx::query_scalar("SELECT id FROM event_refund_batches WHERE event_id = $1 ORDER BY id")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await?;
    let mut progress = Vec::new();
    for b in &batches {
        progress.push(batch_progress(&mut tx, b).await?);
    }
    let items: Vec<(String, String, String, i64, String, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT i.order_id, o.buyer_name, o.buyer_email, i.amount_minor, i.state, i.refund_id, r.status, i.error, r.failure_reason
           FROM event_refund_items i JOIN event_orders o ON o.id = i.order_id LEFT JOIN refunds r ON r.id = i.refund_id
          WHERE i.event_id = $1 ORDER BY i.id LIMIT 1000",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    let sums: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT COALESCE(r.status, i.state), count(*), COALESCE(sum(i.amount_minor), 0)::BIGINT FROM event_refund_items i LEFT JOIN refunds r ON r.id = i.refund_id WHERE i.event_id = $1 GROUP BY 1",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    let get = |k: &str| {
        sums.iter()
            .find(|(s, _, _)| s == k)
            .map_or((0, 0), |(_, n, m)| (*n, *m))
    };
    let total_orders: i64 = sums.iter().map(|(_, n, _)| n).sum();
    let total_minor: i64 = sums.iter().map(|(_, _, m)| m).sum();
    Ok(Json(json!({
        "event_id": id, "currency": "INR",
        "orders_to_refund": total_orders, "amount_minor": total_minor,
        "processed": { "orders": get("processed").0, "amount_minor": get("processed").1 },
        "in_progress": { "orders": get("processing").0 + get("unknown").0, "amount_minor": get("processing").1 + get("unknown").1 },
        "awaiting_approval": { "orders": get("pending_approval").0, "amount_minor": get("pending_approval").1 },
        "failed_or_rejected": { "orders": get("failed").0 + get("rejected").0, "amount_minor": get("failed").1 + get("rejected").1 },
        "not_yet_sent": { "orders": get("queued").0, "amount_minor": get("queued").1 },
        "errors": { "orders": get("error").0, "amount_minor": get("error").1 },
        "batches": progress,
        "items": items.into_iter().map(|(order, name, email, amount, state, refund, rstatus, err, fail)| json!({ "order_id": order, "buyer_name": name, "buyer_email": email, "amount_minor": amount, "state": state, "refund_id": refund, "refund_status": rstatus, "error": err, "failure_reason": fail })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------------------------------------------

/// One tenant's ticketing jobs for a worker cycle: release expired holds, send queued bulk refunds, email tickets.
pub async fn run_worker(s: &AppState, tenant_id: &str, env: &str) -> Result<usize, ApiError> {
    let mut work = 0;
    {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let n = expire_holds(&mut tx, tenant_id).await?;
        tx.commit().await?;
        work += n;
    }
    work += process_refund_items(s, tenant_id, env, None, 50).await?;
    if let Some(base) = &s.checkout_public_url {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let n = email_tickets(s, &mut tx, tenant_id, base).await?;
        tx.commit().await?;
        work += n;
    }
    Ok(work)
}

/// Queue the "your tickets" email for paid orders that have not had one. The mail carries the link to the order page
/// (the token is derived from the order, not stored). Once per order.
async fn email_tickets(
    s: &AppState,
    conn: &mut PgConnection,
    tenant_id: &str,
    base: &str,
) -> Result<usize, ApiError> {
    let due: Vec<(String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT o.id, o.event_id, o.buyer_name, o.buyer_email, e.name, e.starts_at FROM event_orders o JOIN ticket_events e ON e.id = o.event_id
          WHERE o.status = 'paid' AND o.emailed_at IS NULL ORDER BY o.id LIMIT 50",
    )
    .fetch_all(&mut *conn)
    .await?;
    if due.is_empty() {
        return Ok(0);
    }
    let merchant: String = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or_else(|| "the organiser".into());
    let ist = chrono::FixedOffset::east_opt(19_800).expect("IST");
    for (oid, event_id, name, email, event_name, starts) in &due {
        let secret = event_secret(s, conn, event_id).await?;
        let url = format!(
            "{}/events/{tenant_id}/orders/{oid}?t={}",
            base.trim_end_matches('/'),
            order_token(&secret, oid)
        );
        let (subject, body) = crate::mail::ticket_order(
            &merchant,
            name,
            event_name,
            &starts
                .with_timezone(&ist)
                .format("%d %b %Y, %I:%M %p IST")
                .to_string(),
            &url,
        );
        crate::mail::enqueue_once(
            conn,
            tenant_id,
            &format!("ticket_order:{oid}"),
            email,
            &subject,
            &body,
        )
        .await?;
        sqlx::query("UPDATE event_orders SET emailed_at = now() WHERE id = $1")
            .bind(oid)
            .execute(&mut *conn)
            .await?;
    }
    Ok(due.len())
}

// ---------------------------------------------------------------------------------------------------------------
// Tests that need no database
// ---------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 1, h, 0, 0).unwrap()
    }

    fn tier() -> TierWindow {
        TierWindow {
            price_minor: 100_000,
            early_price_minor: Some(70_000),
            early_starts_at: Some(at(2)),
            early_ends_at: Some(at(6)),
            sale_starts_at: Some(at(1)),
            sale_ends_at: Some(at(20)),
        }
    }

    #[test]
    fn sale_window_edges() {
        let t = tier();
        assert_eq!(t.sale(at(0)), Sale::Upcoming);
        assert_eq!(t.sale(at(1)), Sale::Open);
        assert_eq!(t.sale(at(19)), Sale::Open);
        assert_eq!(t.sale(at(20)), Sale::Ended);
        let open = TierWindow {
            sale_starts_at: None,
            sale_ends_at: None,
            ..tier()
        };
        assert_eq!(open.sale(at(23)), Sale::Open);
    }

    #[test]
    fn early_bird_applies_only_inside_its_window() {
        let t = tier();
        assert_eq!(
            t.price_at(at(1)),
            (100_000, "regular"),
            "before the early bird starts"
        );
        assert_eq!(t.price_at(at(2)), (70_000, "early_bird"));
        assert_eq!(t.price_at(at(5)), (70_000, "early_bird"));
        assert_eq!(
            t.price_at(at(6)),
            (100_000, "regular"),
            "the end is exclusive"
        );
        let no_start = TierWindow {
            early_starts_at: None,
            ..tier()
        };
        assert_eq!(no_start.price_at(at(0)).1, "early_bird");
        let none = TierWindow {
            early_price_minor: None,
            early_ends_at: None,
            early_starts_at: None,
            ..tier()
        };
        assert_eq!(none.price_at(at(3)), (100_000, "regular"));
    }

    #[test]
    fn codes_are_unambiguous_and_random() {
        let a = new_code();
        assert_eq!(a.len(), CODE_LEN);
        assert!(a.bytes().all(|b| ALPHABET.contains(&b)));
        assert_ne!(a, new_code());
        assert_eq!(normalise_code(" ab-cd 23 "), "ABCD23");
    }

    #[test]
    fn qr_payload_is_signed_and_bound_to_its_event() {
        let secret = [7u8; 32];
        let code = new_code();
        let p = qr_payload(&secret, "evt_a", &code);
        let Some(Scanned::Signed { code: c, sig }) = parse_scan(&p) else {
            panic!("not a signed payload: {p}")
        };
        assert_eq!(c, code);
        assert!(verify_sig(&secret, "evt_a", &c, &sig));
        assert!(
            !verify_sig(&secret, "evt_b", &c, &sig),
            "moved to another event"
        );
        assert!(!verify_sig(&[8u8; 32], "evt_a", &c, &sig), "another key");
        assert!(
            !verify_sig(&secret, "evt_a", &new_code(), &sig),
            "another code"
        );
        assert!(!verify_sig(&secret, "evt_a", &c, "zz"), "not hex");
        assert!(!verify_sig(&secret, "evt_a", &c, &sig[..10]), "truncated");
    }

    #[test]
    fn scans_are_parsed_strictly() {
        assert!(
            matches!(parse_scan("abcd-efgh-jkmn"), Some(Scanned::Bare(c)) if c == "ABCDEFGHJKMN")
        );
        assert_eq!(parse_scan(""), None);
        assert_eq!(parse_scan("ZT1.SHORT.abc"), None);
        assert_eq!(parse_scan("ABCDEFGHJKM0"), None, "0 is not in the alphabet");
        assert_eq!(parse_scan(&"A".repeat(500)), None);
    }

    #[test]
    fn order_token_is_derived_and_checked() {
        let k = [1u8; 32];
        let t = order_token(&k, "evo_1");
        assert!(order_token_ok(&k, "evo_1", &t));
        assert!(!order_token_ok(&k, "evo_2", &t));
        assert!(!order_token_ok(&[2u8; 32], "evo_1", &t));
        assert!(!order_token_ok(&k, "evo_1", ""));
    }

    #[test]
    fn vendor_shares_never_exceed_the_total() {
        let shares = vec![
            ("a".to_string(), 3333),
            ("b".to_string(), 3333),
            ("c".to_string(), 3334),
        ];
        for total in [1, 7, 100, 99_999, 1_000_003] {
            let sum: i64 = split_by_bp(total, &shares)
                .iter()
                .map(|s| s.amount_minor)
                .sum();
            assert!(sum <= total, "{sum} > {total}");
        }
        assert_eq!(
            split_by_bp(100_000, &[("a".into(), 2_000)])[0].amount_minor,
            20_000
        );
        assert!(
            split_by_bp(3, &[("a".into(), 100)]).is_empty(),
            "a share under one paise is dropped"
        );
    }

    fn fields() -> Vec<NewField> {
        let f = |key: &str, kind: &str, required: bool| NewField {
            key: key.into(),
            label: key.into(),
            kind: kind.into(),
            required,
            options: None,
            min_len: None,
            max_len: None,
        };
        vec![
            f("company", "text", true),
            f("work_email", "email", false),
            f("mobile", "phone", false),
            NewField {
                options: Some(vec!["S".into(), "M".into()]),
                ..f("shirt", "select", false)
            },
            f("terms", "checkbox", true),
        ]
    }

    #[test]
    fn form_answers_are_validated_and_normalised() {
        let defs = fields();
        validate_field_defs(&defs).unwrap();
        let ok = validate_answers(&defs, &json!({ "company": "  Acme ", "work_email": "a@b.co", "mobile": "+91 98765 43210", "shirt": "M", "terms": true })).unwrap();
        assert_eq!(ok["company"], "Acme");
        assert_eq!(ok["terms"], true);
        let miss = |a: Value| validate_answers(&defs, &a).unwrap_err().message;
        assert!(miss(json!({ "terms": true })).contains("required"));
        assert!(miss(json!({ "company": "x" })).contains("ticked"));
        assert!(
            miss(json!({ "company": "x", "terms": true, "work_email": "nope" })).contains("email")
        );
        assert!(miss(json!({ "company": "x", "terms": true, "mobile": "12" })).contains("phone"));
        assert!(miss(json!({ "company": "x", "terms": true, "shirt": "XL" })).contains("choices"));
        assert!(miss(json!({ "company": "x", "terms": true, "other": "y" })).contains("no field"));
        assert!(miss(json!({ "company": "x\u{0007}", "terms": true })).contains("not allowed"));
        assert!(miss(json!("text")).contains("object"));
    }

    #[test]
    fn field_definitions_are_checked() {
        let mut f = fields();
        f.push(f[0].clone());
        assert!(validate_field_defs(&f).is_err(), "duplicate key");
        let bad = |kind: &str, opts: Option<Vec<String>>| {
            validate_field_defs(&[NewField {
                key: "k".into(),
                label: "K".into(),
                kind: kind.into(),
                required: false,
                options: opts,
                min_len: None,
                max_len: None,
            }])
        };
        assert!(bad("select", None).is_err());
        assert!(bad("text", Some(vec!["x".into()])).is_err());
        assert!(bad("date", None).is_err());
        assert!(bad("text", None).is_ok());
    }

    #[test]
    fn length_limits_apply_to_text() {
        let defs = vec![NewField {
            key: "bio".into(),
            label: "Bio".into(),
            kind: "text".into(),
            required: false,
            options: None,
            min_len: Some(3),
            max_len: Some(5),
        }];
        assert!(validate_answers(&defs, &json!({ "bio": "ab" })).is_err());
        assert!(validate_answers(&defs, &json!({ "bio": "abcdef" })).is_err());
        assert!(validate_answers(&defs, &json!({ "bio": "abcd" })).is_ok());
        assert_eq!(validate_answers(&defs, &json!({})).unwrap(), json!({}));
    }

    #[test]
    fn csv_cells_cannot_run_as_formulas() {
        assert_eq!(csv_cell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_cell("+91 98765"), "'+91 98765");
        assert_eq!(csv_cell("@sum"), "'@sum");
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_row(&["a".into(), "b c".into()]), "a,b c\n");
    }

    #[test]
    fn tiers_are_validated() {
        let ok = NewTier {
            name: "GA".into(),
            description: None,
            price_minor: 50_000,
            quantity: 10,
            per_order_max: Some(4),
            sale_starts_at: Some(at(1)),
            sale_ends_at: Some(at(5)),
            early_bird_price_minor: Some(40_000),
            early_bird_starts_at: Some(at(1)),
            early_bird_ends_at: Some(at(3)),
            allowed_methods: Some(vec!["upi".into()]),
        };
        assert!(validate_tier(&ok).is_ok());
        let no = |f: &dyn Fn(&mut NewTier)| {
            let mut t = ok.clone();
            f(&mut t);
            validate_tier(&t).is_err()
        };
        assert!(no(&|t| t.quantity = 0));
        assert!(no(&|t| t.price_minor = -1));
        assert!(
            no(&|t| t.early_bird_price_minor = Some(50_000)),
            "early bird must be cheaper"
        );
        assert!(
            no(&|t| t.early_bird_ends_at = None),
            "early bird needs both parts"
        );
        assert!(no(&|t| t.sale_ends_at = Some(at(1))));
        assert!(no(&|t| t.allowed_methods = Some(vec!["cheque".into()])));
        assert!(no(&|t| t.allowed_methods = Some(vec![])));
        assert!(no(&|t| t.per_order_max = Some(0)));
    }

    #[test]
    fn email_and_phone_checks() {
        assert!(valid_email("a@b.co"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("@b.co"));
        assert!(!valid_email("a b@c.co"));
        assert!(valid_phone("+91 98765-43210"));
        assert!(!valid_phone("98765abc10"));
        assert!(!valid_phone("123"));
    }
}
