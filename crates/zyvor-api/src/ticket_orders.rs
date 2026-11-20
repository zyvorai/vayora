// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! The public side of ticketing: the event page's data, placing an order (which holds seats and creates a payment intent
//! through the normal routing), the buyer's order and ticket page, and a QR image per ticket. None of it needs an API key.
//! The buyer's order page is protected by a token derived from the order and the event's secret (`ticketing::order_token`).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState, coupons,
    idempotency::{self, Begin, fingerprint, replay_response},
    tenant,
    ticketing::{self, EventRow, NewField, Sale, TierRow, fetch_event, load_fields, load_tiers},
};

async fn merchant_name(
    conn: &mut PgConnection,
    tenant_id: &str,
) -> Result<Option<String>, ApiError> {
    Ok(sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(conn)
        .await?)
}

/// Sales run while the event is on sale and has not ended (or, with no end time, started).
fn selling(e: &EventRow, now: DateTime<Utc>) -> bool {
    e.status == "on_sale" && now < e.ends_at.unwrap_or(e.starts_at)
}

/// `GET /v1/public/events/{tenant_id}/{event_id}`: what the event page shows. A draft event does not exist for the public.
pub async fn event_page(
    State(s): State<AppState>,
    Path((tenant_id, event_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let e = fetch_event(&mut tx, &event_id, false).await?;
    if e.status == "draft" {
        return Err(ApiError::not_found("event"));
    }
    let now = Utc::now();
    let tiers = load_tiers(&mut tx, &event_id).await?;
    let fields = load_fields(&mut tx, &event_id).await?;
    let event_left = (e.capacity - e.seats_held - e.seats_sold).max(0);
    let sellable = selling(&e, now);
    let tiers: Vec<Value> = tiers
        .iter()
        .filter(|t| t.archived_at.is_none())
        .map(|t: &TierRow| {
            let w = t.window();
            let (price, kind) = w.price_at(now);
            let available = t.remaining().min(event_left);
            json!({
                "id": t.id, "name": t.name, "description": t.description,
                "price_minor": price, "regular_price_minor": t.price_minor, "price_kind": kind,
                "early_bird": t.early_bird_price_minor.map(|p| json!({ "price_minor": p, "starts_at": t.early_bird_starts_at, "ends_at": t.early_bird_ends_at })),
                "sale": { "state": w.sale(now), "starts_at": t.sale_starts_at, "ends_at": t.sale_ends_at },
                "available": available, "sold_out": available == 0,
                "per_order_max": t.per_order_max.min(available.max(1)),
                "allowed_methods": t.allowed_methods,
                "can_buy": sellable && w.sale(now) == Sale::Open && available > 0,
            })
        })
        .collect();
    let merchant = merchant_name(&mut tx, &tenant_id).await?;
    Ok(Json(json!({
        "id": e.id, "merchant": merchant, "name": e.name, "description": e.description, "mode": e.mode,
        "venue": e.venue, "starts_at": e.starts_at, "ends_at": e.ends_at, "status": e.status, "currency": e.currency,
        "on_sale": sellable, "hold_minutes": e.hold_minutes, "tiers": tiers,
        "form_fields": fields.iter().map(|f| json!({ "key": f.key, "label": f.label, "kind": f.kind, "required": f.required, "options": f.options, "min_len": f.min_len, "max_len": f.max_len })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct Buyer {
    name: String,
    email: String,
    phone: Option<String>,
}

#[derive(Deserialize)]
pub struct LineIn {
    tier_id: String,
    quantity: i32,
}

#[derive(Deserialize)]
pub struct OrderBody {
    buyer: Buyer,
    lines: Vec<LineIn>,
    /// Answers to the registration form, by field key.
    #[serde(default)]
    answers: Value,
    /// A coupon code the buyer typed.
    coupon: Option<String>,
}

/// `POST /v1/public/events/{tenant_id}/{event_id}/orders`: hold the seats and start the payment. Send an
/// `Idempotency-Key` header and a repeated request returns the same order. The reply carries the checkout address and the
/// payment's client secret (shown once) and the buyer's order token. The seats are held for the event's hold time; if
/// the payment does not arrive in that time they are released.
pub async fn create_order(
    State(s): State<AppState>,
    Path((tenant_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(b): Json<OrderBody>,
) -> Result<Response, ApiError> {
    let bad = |m: &str| ApiError::invalid("invalid_order", m);
    let name = b.buyer.name.trim().to_string();
    let email = b.buyer.email.trim().to_string();
    let phone = b
        .buyer
        .phone
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from);
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(|c| c.is_control()) {
        return Err(bad("buyer name is required, up to 120 characters"));
    }
    if !ticketing::valid_email(&email) {
        return Err(bad("buyer email is not a valid address"));
    }
    if phone.as_deref().is_some_and(|p| !ticketing::valid_phone(p)) {
        return Err(bad("buyer phone is not a valid number"));
    }
    if b.lines.is_empty()
        || b.lines.len() > 10
        || b.lines.iter().any(|l| !(1..=100).contains(&l.quantity))
    {
        return Err(bad(
            "choose 1 to 10 tiers, each with a quantity of 1 to 100",
        ));
    }
    let mut ids: Vec<&str> = b.lines.iter().map(|l| l.tier_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != b.lines.len() {
        return Err(bad("each tier may appear once"));
    }
    let want: i32 = b.lines.iter().map(|l| l.quantity).sum();
    let coupon_code = b
        .coupon
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from);

    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let key = if headers.contains_key("idempotency-key") {
        let hash = fingerprint(&[
            &event_id,
            &email.to_lowercase(),
            &b.lines
                .iter()
                .map(|l| format!("{}x{}", l.tier_id, l.quantity))
                .collect::<Vec<_>>()
                .join(","),
            coupon_code.as_deref().unwrap_or(""),
        ]);
        match idempotency::begin(&mut tx, &tenant_id, "event_orders", &headers, &hash).await? {
            Begin::Replay { status, body, .. } => {
                return Ok(replay_response(status, body.unwrap_or(Value::Null)));
            }
            Begin::Proceed(k) => Some(k),
        }
    } else {
        None
    };

    // Everything that decides whether seats are free happens under this lock.
    let e = fetch_event(&mut tx, &event_id, true).await?;
    let now = Utc::now();
    if e.status == "draft" {
        return Err(ApiError::not_found("event"));
    }
    if !selling(&e, now) {
        return Err(ApiError::conflict(
            "event_not_on_sale",
            format!(
                "this event is {}",
                if e.status == "on_sale" {
                    "over"
                } else {
                    e.status.as_str()
                }
            ),
        ));
    }
    let pending_here: i64 = sqlx::query_scalar("SELECT count(*) FROM event_orders WHERE event_id = $1 AND lower(buyer_email) = lower($2) AND status = 'pending'")
        .bind(&event_id)
        .bind(&email)
        .fetch_one(&mut *tx)
        .await?;
    if pending_here >= 3 {
        return Err(ApiError::conflict(
            "too_many_pending_orders",
            "finish or cancel your open orders for this event first",
        ));
    }
    let tiers: Vec<TierRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM ticket_tiers WHERE event_id = $1 AND id = ANY($2) ORDER BY id FOR UPDATE",
        ticketing::TIER_COLS
    )))
    .bind(&event_id)
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    if tiers.len() != ids.len() {
        return Err(ApiError::not_found("ticket tier"));
    }
    let event_left = e.capacity - e.seats_held - e.seats_sold;
    if want > event_left {
        return Err(ApiError::conflict(
            "sold_out",
            format!("only {} seat(s) are left for this event", event_left.max(0)),
        ));
    }
    let mut priced: Vec<(&TierRow, i32, i64, &'static str)> = Vec::new();
    let mut subtotal: i64 = 0;
    for t in &tiers {
        let q = b
            .lines
            .iter()
            .find(|l| l.tier_id == t.id)
            .map_or(0, |l| l.quantity);
        if t.archived_at.is_some() {
            return Err(ApiError::conflict(
                "tier_not_on_sale",
                format!("{} is no longer sold", t.name),
            ));
        }
        match t.window().sale(now) {
            Sale::Open => {}
            Sale::Upcoming => {
                return Err(ApiError::conflict(
                    "tier_not_on_sale",
                    format!("sales of {} have not started", t.name),
                ));
            }
            Sale::Ended => {
                return Err(ApiError::conflict(
                    "tier_not_on_sale",
                    format!("sales of {} have ended", t.name),
                ));
            }
        }
        if q > t.per_order_max {
            return Err(ApiError::invalid(
                "too_many_tickets",
                format!("at most {} {} ticket(s) per order", t.per_order_max, t.name),
            ));
        }
        if q > t.remaining() {
            return Err(ApiError::conflict(
                "sold_out",
                format!("only {} {} ticket(s) are left", t.remaining(), t.name),
            ));
        }
        let (unit, kind) = t.window().price_at(now);
        subtotal += unit * q as i64;
        priced.push((t, q, unit, kind));
    }
    let fields = load_fields(&mut tx, &event_id).await?;
    let defs: Vec<NewField> = fields.iter().map(|f| f.def()).collect();
    let answers = ticketing::validate_answers(&defs, &b.answers)?;

    // methods this order may be paid with: what every chosen tier allows
    let mut methods: Option<Vec<String>> = None;
    for (t, ..) in &priced {
        if let Some(m) = &t.allowed_methods {
            methods = Some(match methods {
                None => m.clone(),
                Some(cur) => cur.into_iter().filter(|x| m.contains(x)).collect(),
            });
        }
    }
    if methods.as_ref().is_some_and(Vec::is_empty) {
        return Err(ApiError::conflict(
            "no_common_payment_method",
            "these tickets cannot be paid for together; buy them in separate orders",
        ));
    }

    // coupon: spent in this transaction, given back if the order is released unpaid
    let spent = match &coupon_code {
        Some(c) if subtotal > 0 => Some(coupons::find(&mut tx, c, true).await?),
        Some(_) => {
            return Err(ApiError::invalid(
                "coupon_not_applicable",
                "there is nothing to discount on a free order",
            ));
        }
        None => None,
    };
    let p = coupons::price(
        subtotal,
        spent.as_ref().map(|c| (c.kind.as_str(), c.value)),
        0,
    );
    let total = p.total;

    // buyer as a customer, so receipts and refund notices can reach them
    let customer: String = match sqlx::query_scalar::<_, String>(
        "SELECT id FROM customers WHERE lower(email) = lower($1) ORDER BY created_at LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(&mut *tx)
    .await?
    {
        Some(id) => id,
        None => {
            let id = new_id("cus");
            sqlx::query(
                "INSERT INTO customers (id, tenant_id, name, email, phone) VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(&id)
            .bind(&tenant_id)
            .bind(&name)
            .bind(&email)
            .bind(&phone)
            .execute(&mut *tx)
            .await?;
            id
        }
    };

    let order_id = new_id("evo");
    let (intent_id, secret) = if total > 0 {
        let intent_id = new_id("pi");
        let secret = format!(
            "pis_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut meta = json!({ "event_order": order_id, "event_id": event_id, "coupon_id": spent.as_ref().map(|c| c.id.clone()), "discount_minor": p.discount });
        if let Some(m) = &methods {
            meta["allowed_methods"] = json!(m);
        }
        let qty_text = if want == 1 {
            "1 ticket".to_string()
        } else {
            format!("{want} tickets")
        };
        sqlx::query(
            "INSERT INTO payment_intents (id, tenant_id, customer_id, amount_minor, currency, status, capture_method, description, metadata, client_secret_hash)
             VALUES ($1, $2, $3, $4, 'INR', 'requires_payment_method', 'automatic', $5, $6, $7)",
        )
        .bind(&intent_id)
        .bind(&tenant_id)
        .bind(&customer)
        .bind(total)
        .bind(format!("{} ({qty_text})", e.name))
        .bind(meta)
        .bind(crate::auth::hash_key(&secret))
        .execute(&mut *tx)
        .await?;
        if let Some(sp) = &e.splits {
            let shares: Vec<(String, i64)> = sp
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    Some((
                        x["vendor_id"].as_str()?.to_string(),
                        x["share_bp"].as_i64()?,
                    ))
                })
                .collect();
            let splits = ticketing::split_by_bp(total, &shares);
            if !splits.is_empty() {
                crate::splits::validate(&splits, total)?;
                crate::splits::attach(&mut tx, &tenant_id, &intent_id, &splits).await?;
            }
        }
        (Some(intent_id), Some(secret))
    } else {
        (None, None)
    };

    let hold_until = now + Duration::minutes(e.hold_minutes as i64);
    sqlx::query(
        "INSERT INTO event_orders (id, tenant_id, event_id, status, buyer_name, buyer_email, buyer_phone, answers, subtotal_minor, discount_minor, total_minor, coupon_id, quantity, intent_id, hold_expires_at)
         VALUES ($1,$2,$3,'pending',$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(&order_id)
    .bind(&tenant_id)
    .bind(&event_id)
    .bind(&name)
    .bind(&email)
    .bind(&phone)
    .bind(&answers)
    .bind(subtotal)
    .bind(p.discount)
    .bind(total)
    .bind(spent.as_ref().map(|c| c.id.clone()))
    .bind(want)
    .bind(&intent_id)
    .bind(hold_until)
    .execute(&mut *tx)
    .await?;
    for (t, q, unit, kind) in &priced {
        sqlx::query("INSERT INTO event_order_lines (id, tenant_id, order_id, tier_id, quantity, unit_price_minor, price_kind) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(new_id("eol"))
            .bind(&tenant_id)
            .bind(&order_id)
            .bind(&t.id)
            .bind(q)
            .bind(unit)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE ticket_tiers SET seats_held = seats_held + $2 WHERE id = $1")
            .bind(&t.id)
            .bind(q)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE ticket_events SET seats_held = seats_held + $2 WHERE id = $1")
        .bind(&event_id)
        .bind(want)
        .execute(&mut *tx)
        .await?;
    let mut status = "pending";
    if total == 0 {
        // a free order has no payment to wait for
        let core: ticketing::OrderCore = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM event_orders WHERE id = $1 FOR UPDATE",
            ORDER_CORE_COLS
        )))
        .bind(&order_id)
        .fetch_one(&mut *tx)
        .await?;
        ticketing::mark_paid_locked(&mut tx, &tenant_id, &core, "pending").await?;
        status = "paid";
    }
    tenant::audit(
        &mut tx,
        &tenant_id,
        "public:checkout",
        "event_order.created",
        Some(&order_id),
        json!({ "event_id": event_id, "quantity": want, "total_minor": total }),
    )
    .await?;
    let secret_key = ticketing::event_secret(&s, &mut tx, &event_id).await?;
    let body = json!({
        "order_id": order_id, "status": status, "quantity": want,
        "subtotal_minor": subtotal, "discount_minor": p.discount, "total_minor": total, "currency": "INR",
        "hold_expires_at": (status == "pending").then_some(hold_until),
        "order_token": ticketing::order_token(&secret_key, &order_id),
        "order_path": format!("/events/{tenant_id}/orders/{order_id}"),
        "payment": intent_id.as_ref().map(|i| json!({ "intent_id": i, "client_secret": secret, "path": format!("/pay/{tenant_id}/{i}") })),
    });
    let res = match key {
        Some(k) => {
            k.set_resource(&mut tx, &order_id).await?;
            k.finish(&mut tx, StatusCode::CREATED, &body).await?
        }
        None => (StatusCode::CREATED, Json(body)).into_response(),
    };
    tx.commit().await?;
    Ok(res)
}

const ORDER_CORE_COLS: &str =
    "id, event_id, status, quantity, total_minor, coupon_id, intent_id, buyer_name";

#[derive(Deserialize)]
pub struct TokenQuery {
    t: String,
}

/// Look the order up and check the buyer's token. A wrong token and an unknown order look the same.
async fn authorise(
    s: &AppState,
    conn: &mut PgConnection,
    order_id: &str,
    token: &str,
) -> Result<String, ApiError> {
    let event_id: Option<String> =
        sqlx::query_scalar("SELECT event_id FROM event_orders WHERE id = $1")
            .bind(order_id)
            .fetch_optional(&mut *conn)
            .await?;
    let deny = || ApiError::not_found("order");
    let event_id = event_id.ok_or_else(deny)?;
    let secret = ticketing::event_secret(s, conn, &event_id).await?;
    if ticketing::order_token_ok(&secret, order_id, token) {
        Ok(event_id)
    } else {
        Err(deny())
    }
}

/// `GET /v1/public/events/{tenant_id}/orders/{order_id}?t=`: the buyer's order and, once paid, their tickets with the QR
/// payloads. While the order is unpaid it says when the hold ends.
pub async fn order_page(
    State(s): State<AppState>,
    Path((tenant_id, order_id)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let event_id = authorise(&s, &mut tx, &order_id, &q.t).await?;
    let e = fetch_event(&mut tx, &event_id, false).await?;
    let secret = ticketing::event_secret(&s, &mut tx, &event_id).await?;
    #[allow(clippy::type_complexity)]
    let (status, name, email, qty, sub, disc, total, intent, hold, paid): (String, String, String, i32, i64, i64, i64, Option<String>, Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, buyer_name, buyer_email, quantity, subtotal_minor, discount_minor, total_minor, intent_id, hold_expires_at, paid_at FROM event_orders WHERE id = $1",
    )
    .bind(&order_id)
    .fetch_one(&mut *tx)
    .await?;
    let lines: Vec<(String, i32, i64)> = sqlx::query_as("SELECT t.name, l.quantity, l.unit_price_minor FROM event_order_lines l JOIN ticket_tiers t ON t.id = l.tier_id WHERE l.order_id = $1 ORDER BY l.tier_id")
        .bind(&order_id)
        .fetch_all(&mut *tx)
        .await?;
    let tickets: Vec<(String, String, String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT t.id, t.code, t.status, tt.name, t.checked_in_at FROM tickets t JOIN ticket_tiers tt ON tt.id = t.tier_id WHERE t.order_id = $1 ORDER BY t.seq",
    )
    .bind(&order_id)
    .fetch_all(&mut *tx)
    .await?;
    let merchant = merchant_name(&mut tx, &tenant_id).await?;
    let valid_view = matches!(status.as_str(), "paid");
    Ok(Json(json!({
        "order_id": order_id, "status": status, "merchant": merchant,
        "event": { "id": e.id, "name": e.name, "mode": e.mode, "venue": e.venue, "starts_at": e.starts_at, "ends_at": e.ends_at, "status": e.status,
                   "online_url": e.online_url.filter(|_| valid_view) },
        "buyer": { "name": name, "email": email },
        "quantity": qty, "subtotal_minor": sub, "discount_minor": disc, "total_minor": total, "currency": "INR",
        "intent_id": intent, "hold_expires_at": hold.filter(|_| status == "pending"), "paid_at": paid,
        "lines": lines.into_iter().map(|(n, q, p)| json!({ "tier": n, "quantity": q, "unit_price_minor": p })).collect::<Vec<_>>(),
        "tickets": tickets.into_iter().map(|(id, code, st, tier, at)| json!({
            "id": id, "code": code, "status": st, "tier": tier, "checked_in_at": at,
            "qr_payload": (st == "issued" || st == "checked_in").then(|| ticketing::qr_payload(&secret, &event_id, &code)),
            "qr_path": format!("/events/{tenant_id}/orders/{order_id}/tickets/{id}/qr.svg"),
        })).collect::<Vec<_>>(),
    })))
}

/// `POST /v1/public/events/{tenant_id}/orders/{order_id}/cancel?t=`: the buyer gives up an unpaid order and its seats go
/// back at once. A paid order is not cancelled here: that is the organiser's call (and refund).
pub async fn buyer_cancel(
    State(s): State<AppState>,
    Path((tenant_id, order_id)): Path<(String, String)>,
    Query(q): Query<TokenQuery>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let event_id = authorise(&s, &mut tx, &order_id, &q.t).await?;
    fetch_event(&mut tx, &event_id, true).await?;
    let o: ticketing::OrderCore = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_CORE_COLS} FROM event_orders WHERE id = $1 FOR UPDATE"
    )))
    .bind(&order_id)
    .fetch_one(&mut *tx)
    .await?;
    if o.status != "pending" {
        return Err(ApiError::conflict(
            "order_not_cancellable",
            format!(
                "the order is {}; ask the organiser if you need it cancelled",
                o.status
            ),
        ));
    }
    // a payment in flight must be allowed to finish: its outcome decides the order
    let live: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM payment_attempts WHERE intent_id = $1 AND status IN ('created', 'pending', 'authorized', 'unknown'))")
        .bind(&o.intent_id)
        .fetch_one(&mut *tx)
        .await?;
    if live {
        return Err(ApiError::conflict(
            "payment_in_progress",
            "a payment for this order is still being confirmed",
        ));
    }
    ticketing::release_order(
        &mut tx,
        &tenant_id,
        &o,
        "cancelled",
        "cancelled by the buyer",
    )
    .await?;
    tenant::audit(
        &mut tx,
        &tenant_id,
        "public:checkout",
        "event_order.buyer_cancelled",
        Some(&order_id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "order_id": order_id, "status": "cancelled" })))
}

/// `GET /v1/public/events/{tenant_id}/orders/{order_id}/tickets/{ticket_id}/qr.svg?t=`: the ticket's QR code. Only an
/// issued or checked-in ticket has one.
pub async fn ticket_qr(
    State(s): State<AppState>,
    Path((tenant_id, order_id, ticket_id)): Path<(String, String, String)>,
    Query(q): Query<TokenQuery>,
) -> Result<Response, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let event_id = authorise(&s, &mut tx, &order_id, &q.t).await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT code, status FROM tickets WHERE id = $1 AND order_id = $2")
            .bind(&ticket_id)
            .bind(&order_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (code, status) = row.ok_or_else(|| ApiError::not_found("ticket"))?;
    if !matches!(status.as_str(), "issued" | "checked_in") {
        return Err(ApiError::not_found("ticket"));
    }
    let secret = ticketing::event_secret(&s, &mut tx, &event_id).await?;
    let svg = qrcode::QrCode::new(ticketing::qr_payload(&secret, &event_id, &code).as_bytes())
        .map_err(|_| ApiError::internal())?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(280, 280)
        .quiet_zone(true)
        .build();
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        svg,
    )
        .into_response())
}
