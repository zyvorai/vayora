//! Offers: merchant-funded discounts that do not depend on which provider takes the payment.
//!
//! An offer is keyed on the payment method, a card network or BIN range class (never a card number), a UPI app or
//! handle, a bank, an amount band, new or returning customer and a validity window, with caps (total redemptions,
//! per customer, total budget) and stacking rules. It is judged when a payment ATTEMPT is created (the method is
//! known only then): the attempt is for the discounted amount, the amount the payer is asked for. The same
//! evaluation backs `POST /v1/offers/preview`, so what is previewed is what is applied.
//!
//! Who pays: only the merchant. Every offer and every redemption says `funded_by: merchant`; a database constraint
//! keeps it so. An offer a bank or a provider funds is not modelled here, and nothing in this module claims one.
//!
//! Caps cannot be over-used: the offers an attempt could use are locked (`FOR UPDATE`, in id order) inside the
//! transaction that creates the attempt, and counted again under the lock. A redemption is `reserved` with the
//! attempt, `redeemed` when the payment is captured, `released` when the attempt fails or is cancelled (the cap is
//! given back), and `reversed` when the whole payment is refunded. A refund hands back the same share of the
//! discount as it hands back of the cash.
//!
//! Left out on purpose: payments on an invoice, a split (marketplace) payment, and a payment-link payment that
//! already carries a coupon. Their amounts feed other records (invoice allocations, vendor shares) that expect the
//! full price, so no offer is applied and the preview says why.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use zyvor_core::{Currency, new_id};
use zyvor_providers::Method;

use crate::{
    ApiError, AppState,
    auth::Auth,
    coupons::half_up,
    instrument_hint::{self, Instrument, InstrumentIn},
    tenant,
};

const COLS: &str = "id, name, description, code, status, kind, value, max_discount_minor, currency, methods, card_networks, bin_ranges, upi_apps, upi_handles, banks, min_amount_minor, max_amount_minor, segment, valid_from, valid_until, max_redemptions, max_per_customer, budget_minor, redemptions, spent_minor, stackable, stack_group, priority, funded_by, created_by, created_at, updated_at";

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct OfferRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub code: Option<String>,
    pub status: String,
    pub kind: String,
    pub value: i64,
    pub max_discount_minor: Option<i64>,
    pub currency: String,
    pub methods: Option<Vec<String>>,
    pub card_networks: Option<Vec<String>>,
    pub bin_ranges: Value,
    pub upi_apps: Option<Vec<String>>,
    pub upi_handles: Option<Vec<String>>,
    pub banks: Option<Vec<String>>,
    pub min_amount_minor: Option<i64>,
    pub max_amount_minor: Option<i64>,
    pub segment: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub max_redemptions: Option<i32>,
    pub max_per_customer: Option<i32>,
    pub budget_minor: Option<i64>,
    pub redemptions: i32,
    pub spent_minor: i64,
    pub stackable: bool,
    pub stack_group: Option<String>,
    pub priority: i32,
    pub funded_by: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// What a payment looks like to the offers.
pub struct Ctx<'a> {
    pub amount_minor: i64,
    pub currency: &'a str,
    pub method: &'a str,
    pub instrument: &'a Instrument,
    pub customer_id: Option<&'a str>,
    /// `new` or `returning`; `None` when there is no customer to judge by.
    pub segment: Option<&'static str>,
    pub now: DateTime<Utc>,
    /// Codes the payer typed (normalised).
    pub codes: &'a [String],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NotApplied {
    pub offer_id: String,
    pub name: String,
    pub reason_code: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Applied {
    pub offer_id: String,
    pub name: String,
    pub kind: String,
    pub value: i64,
    pub discount_minor: i64,
    /// Always `merchant`.
    pub funded_by: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub gross_minor: i64,
    pub discount_minor: i64,
    pub net_minor: i64,
    pub applied: Vec<Applied>,
    pub not_applied: Vec<NotApplied>,
}

impl Evaluation {
    fn none(gross: i64, not_applied: Vec<NotApplied>) -> Self {
        Self {
            gross_minor: gross,
            discount_minor: 0,
            net_minor: gross,
            applied: Vec::new(),
            not_applied,
        }
    }
    pub fn summary(&self) -> Value {
        json!({
            "gross_minor": self.gross_minor, "discount_minor": self.discount_minor, "net_minor": self.net_minor,
            "funded_by": "merchant",
            "applied": self.applied,
        })
    }
}

struct Skip {
    code: &'static str,
    text: String,
    /// Not shown in a preview: an offer that needs a code the payer did not type.
    hidden: bool,
}

fn skip(code: &'static str, text: impl Into<String>) -> Skip {
    Skip {
        code,
        text: text.into(),
        hidden: false,
    }
}

fn any_of(list: &Option<Vec<String>>, v: Option<&str>) -> Option<bool> {
    // None: the offer has no such condition; Some(true/false): it has one and it holds / does not
    match list {
        Some(l) if !l.is_empty() => Some(v.is_some_and(|v| l.iter().any(|x| x == v))),
        _ => None,
    }
}

fn in_bin_ranges(ranges: &Value, bin6: &str) -> bool {
    ranges.as_array().is_some_and(|a| {
        a.iter().any(|r| {
            matches!((r["from"].as_str(), r["to"].as_str()), (Some(f), Some(t)) if f <= bin6 && bin6 <= t)
        })
    })
}

/// Every condition that does not need the database. Pure.
fn static_check(o: &OfferRow, c: &Ctx) -> Result<(), Skip> {
    if o.currency != c.currency {
        return Err(skip(
            "currency_mismatch",
            format!("this offer is for {}", o.currency),
        ));
    }
    if o.valid_from.is_some_and(|f| f > c.now) {
        return Err(skip("not_started", "this offer has not started yet"));
    }
    if o.valid_until.is_some_and(|u| u <= c.now) {
        return Err(skip("expired", "this offer has ended"));
    }
    if let Some(code) = &o.code
        && !c.codes.iter().any(|x| x == code)
    {
        return Err(Skip {
            hidden: true,
            ..skip("code_required", "needs a code")
        });
    }
    if o.min_amount_minor.is_some_and(|m| c.amount_minor < m) {
        return Err(skip(
            "amount_below_min",
            "the amount is below this offer's minimum",
        ));
    }
    if o.max_amount_minor.is_some_and(|m| c.amount_minor > m) {
        return Err(skip(
            "amount_above_max",
            "the amount is above this offer's maximum",
        ));
    }
    if let Some(l) = &o.methods
        && !l.is_empty()
        && !l.iter().any(|m| m == c.method)
    {
        return Err(skip(
            "method_not_eligible",
            format!("this offer is not for {} payments", c.method),
        ));
    }
    let i = c.instrument;
    if any_of(&o.card_networks, i.card_network.as_deref()) == Some(false) {
        return Err(skip(
            if i.card_network.is_none() {
                "card_network_unknown"
            } else {
                "card_network_not_eligible"
            },
            "the card network does not match this offer",
        ));
    }
    if o.bin_ranges.as_array().is_some_and(|a| !a.is_empty()) {
        match i.bin6.as_deref() {
            None => {
                return Err(skip(
                    "card_bin_needed",
                    "this offer is for a card BIN range and no card_bin was given",
                ));
            }
            Some(b) if !in_bin_ranges(&o.bin_ranges, b) => {
                return Err(skip(
                    "card_bin_not_eligible",
                    "the card is outside this offer's BIN range",
                ));
            }
            _ => {}
        }
    }
    for (list, v, code, what) in [
        (&o.upi_apps, i.upi_app.as_deref(), "upi_app", "UPI app"),
        (
            &o.upi_handles,
            i.upi_handle.as_deref(),
            "upi_handle",
            "UPI handle",
        ),
        (&o.banks, i.bank.as_deref(), "bank", "bank"),
    ] {
        if any_of(list, v) == Some(false) {
            let code: &'static str = match (code, v.is_some()) {
                ("upi_app", true) => "upi_app_not_eligible",
                ("upi_app", false) => "upi_app_unknown",
                ("upi_handle", true) => "upi_handle_not_eligible",
                ("upi_handle", false) => "upi_handle_unknown",
                (_, true) => "bank_not_eligible",
                _ => "bank_unknown",
            };
            return Err(skip(code, format!("the {what} does not match this offer")));
        }
    }
    if let Some(seg) = &o.segment {
        match c.segment {
            None => {
                return Err(skip(
                    "customer_required",
                    "this offer is for new or returning customers and the payment has no customer",
                ));
            }
            Some(s) if s != seg => {
                return Err(skip(
                    "segment_mismatch",
                    format!("this offer is for {seg} customers"),
                ));
            }
            _ => {}
        }
    }
    if o.max_per_customer.is_some() && c.customer_id.is_none() {
        return Err(skip(
            "customer_required",
            "this offer has a per-customer limit and the payment has no customer",
        ));
    }
    Ok(())
}

/// The discount one offer gives on `amount`: never below zero, never the whole amount (one minor unit always stays).
pub fn discount_of(o: &OfferRow, amount: i64) -> i64 {
    if amount <= 1 {
        return 0;
    }
    let raw = if o.kind == "percent" {
        half_up(amount, o.value)
    } else {
        o.value
    };
    let capped = o.max_discount_minor.map_or(raw, |m| raw.min(m));
    capped.clamp(0, amount - 1)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Planned {
    pub index: usize,
    pub discount_minor: i64,
}

fn better(a: (i64, i32, &str), b: (i64, i32, &str)) -> bool {
    // more discount, then higher priority, then the smaller id (so the choice never depends on row order)
    (a.0, a.1, std::cmp::Reverse(a.2)) > (b.0, b.1, std::cmp::Reverse(b.2))
}

/// Which of the offers that all match apply, and for how much. Pure.
///
/// A non-stackable offer applies alone, and only the best one counts (most discount, then priority, then id).
/// Stackable offers may combine with each other: at most one per `stack_group` (the best), highest priority first,
/// each taking its share of what is left after the one before. The payer gets whichever of the two ways gives the
/// larger discount; a tie goes to the single offer.
pub fn plan(amount: i64, offers: &[&OfferRow]) -> Vec<Planned> {
    let mut single: Option<(usize, i64)> = None;
    for (i, o) in offers.iter().enumerate().filter(|(_, o)| !o.stackable) {
        let d = discount_of(o, amount);
        if d > 0
            && single.is_none_or(|(j, jd)| {
                better(
                    (d, o.priority, &o.id),
                    (jd, offers[j].priority, &offers[j].id),
                )
            })
        {
            single = Some((i, d));
        }
    }
    // stackable: one per group first
    let mut stackable: Vec<(usize, i64)> = Vec::new();
    for (i, o) in offers.iter().enumerate().filter(|(_, o)| o.stackable) {
        let d = discount_of(o, amount);
        if d == 0 {
            continue;
        }
        match o.stack_group.as_deref() {
            Some(g) => {
                if let Some(pos) = stackable
                    .iter()
                    .position(|(j, _)| offers[*j].stack_group.as_deref() == Some(g))
                {
                    let (j, jd) = stackable[pos];
                    if better(
                        (d, o.priority, &o.id),
                        (jd, offers[j].priority, &offers[j].id),
                    ) {
                        stackable[pos] = (i, d);
                    }
                } else {
                    stackable.push((i, d));
                }
            }
            None => stackable.push((i, d)),
        }
    }
    stackable.sort_by(|(a, _), (b, _)| {
        offers[*b]
            .priority
            .cmp(&offers[*a].priority)
            .then_with(|| offers[*a].id.cmp(&offers[*b].id))
    });
    let mut running = amount;
    let mut stacked = Vec::new();
    for (i, _) in stackable {
        let d = discount_of(offers[i], running);
        if d > 0 {
            stacked.push(Planned {
                index: i,
                discount_minor: d,
            });
            running -= d;
        }
    }
    let stacked_total: i64 = stacked.iter().map(|p| p.discount_minor).sum();
    match single {
        Some((i, d)) if d >= stacked_total => vec![Planned {
            index: i,
            discount_minor: d,
        }],
        _ => stacked,
    }
}

/// `new` when the customer has no earlier succeeded payment (other than `except_intent`), else `returning`.
pub async fn segment(
    conn: &mut PgConnection,
    customer_id: Option<&str>,
    except_intent: Option<&str>,
) -> Result<Option<&'static str>, ApiError> {
    let Some(cid) = customer_id else {
        return Ok(None);
    };
    let had: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM payment_intents WHERE customer_id = $1 AND status = 'succeeded' AND id <> COALESCE($2, ''))",
    )
    .bind(cid)
    .bind(except_intent)
    .fetch_one(conn)
    .await?;
    Ok(Some(if had { "returning" } else { "new" }))
}

/// Judge the offers for one payment. With `lock`, the offers that could apply are locked first, so the caps are
/// counted under the lock and a capped offer cannot be over-used by concurrent attempts.
pub async fn evaluate(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    lock: bool,
) -> Result<Evaluation, ApiError> {
    let mut rows: Vec<OfferRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM offers WHERE status = 'active' ORDER BY id"
    )))
    .fetch_all(&mut *conn)
    .await?;
    let mut not_applied: Vec<NotApplied> = Vec::new();
    let note = |o: &OfferRow, s: Skip, out: &mut Vec<NotApplied>| {
        if !s.hidden {
            out.push(NotApplied {
                offer_id: o.id.clone(),
                name: o.name.clone(),
                reason_code: s.code,
                reason: s.text,
            });
        }
    };
    let mut live = Vec::new();
    for o in rows.drain(..) {
        match static_check(&o, ctx) {
            Ok(()) => live.push(o),
            Err(s) => note(&o, s, &mut not_applied),
        }
    }
    if lock && !live.is_empty() {
        let ids: Vec<String> = live.iter().map(|o| o.id.clone()).collect();
        // fresh rows under the lock: the counters, and a status that may just have changed
        let fresh: Vec<OfferRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLS} FROM offers WHERE id = ANY($1) AND status = 'active' ORDER BY id FOR UPDATE"
        )))
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
        live = fresh
            .into_iter()
            .filter(|o| static_check(o, ctx).is_ok())
            .collect();
    }
    // caps
    let mut candidates: Vec<OfferRow> = Vec::new();
    for o in live {
        if o.max_redemptions.is_some_and(|m| o.redemptions >= m) {
            note(
                &o,
                skip(
                    "redemption_limit_reached",
                    "this offer has been used as many times as it allows",
                ),
                &mut not_applied,
            );
            continue;
        }
        if let (Some(m), Some(cid)) = (o.max_per_customer, ctx.customer_id) {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM offer_redemptions WHERE offer_id = $1 AND customer_id = $2 AND status IN ('reserved', 'redeemed')",
            )
            .bind(&o.id)
            .bind(cid)
            .fetch_one(&mut *conn)
            .await?;
            if n >= i64::from(m) {
                note(
                    &o,
                    skip(
                        "customer_limit_reached",
                        "this customer has used this offer as many times as it allows",
                    ),
                    &mut not_applied,
                );
                continue;
            }
        }
        if o.budget_minor.is_some_and(|b| b - o.spent_minor <= 0) {
            note(
                &o,
                skip("budget_exhausted", "this offer's budget is used up"),
                &mut not_applied,
            );
            continue;
        }
        candidates.push(o);
    }
    // choose; an offer whose discount would overrun its remaining budget is set aside and the choice is made again
    let planned = loop {
        let refs: Vec<&OfferRow> = candidates.iter().collect();
        let p = plan(ctx.amount_minor, &refs);
        let over = p.iter().find(|x| {
            let o = &candidates[x.index];
            o.budget_minor
                .is_some_and(|b| o.spent_minor + x.discount_minor > b)
        });
        match over {
            None => break p,
            Some(x) => {
                let o = candidates.remove(x.index);
                note(
                    &o,
                    skip(
                        "budget_exhausted",
                        "this discount would go past the offer's budget",
                    ),
                    &mut not_applied,
                );
            }
        }
    };
    let mut applied = Vec::new();
    for p in &planned {
        let o = &candidates[p.index];
        applied.push(Applied {
            offer_id: o.id.clone(),
            name: o.name.clone(),
            kind: o.kind.clone(),
            value: o.value,
            discount_minor: p.discount_minor,
            funded_by: "merchant",
        });
    }
    for (i, o) in candidates.iter().enumerate() {
        if !planned.iter().any(|p| p.index == i) {
            let why = if o.stackable {
                "another offer or combination gives the payer more"
            } else {
                "another offer gives the payer more, and this one cannot be combined"
            };
            note(o, skip("outranked", why), &mut not_applied);
        }
    }
    let discount: i64 = applied.iter().map(|a| a.discount_minor).sum();
    Ok(Evaluation {
        gross_minor: ctx.amount_minor,
        discount_minor: discount,
        net_minor: ctx.amount_minor - discount,
        applied,
        not_applied,
    })
}

/// What an intent's own record says about whether offers may touch its price.
async fn intent_blocks_offers(
    conn: &mut PgConnection,
    intent_id: &str,
) -> Result<Option<NotApplied>, ApiError> {
    let (invoice, meta, splits): (Option<String>, Value, bool) = sqlx::query_as(
        "SELECT invoice_id, metadata, EXISTS (SELECT 1 FROM payment_splits WHERE intent_id = payment_intents.id) FROM payment_intents WHERE id = $1",
    )
    .bind(intent_id)
    .fetch_one(conn)
    .await?;
    let why = if invoice.is_some() {
        Some((
            "invoice_payment",
            "offers do not apply to a payment on an invoice",
        ))
    } else if splits {
        Some((
            "split_payment",
            "offers do not apply to a payment that is split between vendors",
        ))
    } else if meta["payment_link"].is_string() && !meta["coupon_id"].is_null() {
        Some((
            "coupon_already_applied",
            "this payment link price already includes a coupon",
        ))
    } else {
        None
    };
    Ok(why.map(|(code, text)| NotApplied {
        offer_id: String::new(),
        name: String::new(),
        reason_code: code,
        reason: text.to_string(),
    }))
}

pub struct AttemptFacts<'a> {
    pub intent_id: &'a str,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub method: &'a str,
    pub instrument: &'a Instrument,
    pub customer_id: Option<&'a str>,
    /// `new` or `returning` (None without a customer).
    pub segment: Option<&'static str>,
    pub codes: &'a [String],
}

/// Judge, and lock, the offers for an attempt about to be created. Call inside the attempt's transaction.
pub async fn plan_for_attempt(
    conn: &mut PgConnection,
    f: &AttemptFacts<'_>,
) -> Result<Evaluation, ApiError> {
    if let Some(b) = intent_blocks_offers(conn, f.intent_id).await? {
        return Ok(Evaluation::none(f.amount_minor, vec![b]));
    }
    evaluate(
        conn,
        &Ctx {
            amount_minor: f.amount_minor,
            currency: f.currency,
            method: f.method,
            instrument: f.instrument,
            customer_id: f.customer_id,
            segment: f.segment,
            now: Utc::now(),
            codes: f.codes,
        },
        true,
    )
    .await
}

/// The same judgement as `plan_for_attempt` without locking or reserving anything (for a checkout page that shows
/// the price before the payer pays).
pub async fn preview_for_attempt(
    conn: &mut PgConnection,
    f: &AttemptFacts<'_>,
) -> Result<Evaluation, ApiError> {
    if let Some(b) = intent_blocks_offers(conn, f.intent_id).await? {
        return Ok(Evaluation::none(f.amount_minor, vec![b]));
    }
    evaluate(
        conn,
        &Ctx {
            amount_minor: f.amount_minor,
            currency: f.currency,
            method: f.method,
            instrument: f.instrument,
            customer_id: f.customer_id,
            segment: f.segment,
            now: Utc::now(),
            codes: f.codes,
        },
        false,
    )
    .await
}

/// Record the offers the attempt was priced with and count them against the caps (the offers are already locked).
pub async fn reserve(
    conn: &mut PgConnection,
    tenant_id: &str,
    actor: &str,
    attempt_id: &str,
    f: &AttemptFacts<'_>,
    ev: &Evaluation,
) -> Result<(), ApiError> {
    for a in &ev.applied {
        sqlx::query(
            "INSERT INTO offer_redemptions (id, tenant_id, offer_id, attempt_id, intent_id, customer_id, status, gross_minor, discount_minor, charged_minor, method, instrument, evaluation)
             VALUES ($1,$2,$3,$4,$5,$6,'reserved',$7,$8,$9,$10,$11,$12)",
        )
        .bind(new_id("ofr"))
        .bind(tenant_id)
        .bind(&a.offer_id)
        .bind(attempt_id)
        .bind(f.intent_id)
        .bind(f.customer_id)
        .bind(ev.gross_minor)
        .bind(a.discount_minor)
        .bind(ev.net_minor)
        .bind(f.method)
        .bind(f.instrument.to_json())
        .bind(json!({ "kind": a.kind, "value": a.value, "stacked_with": ev.applied.len() - 1 }))
        .execute(&mut *conn)
        .await?;
        sqlx::query("UPDATE offers SET redemptions = redemptions + 1, spent_minor = spent_minor + $2, updated_at = now() WHERE id = $1")
            .bind(&a.offer_id)
            .bind(a.discount_minor)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            actor,
            "offer.applied",
            Some(&a.offer_id),
            json!({ "attempt_id": attempt_id, "intent_id": f.intent_id, "discount_minor": a.discount_minor, "gross_minor": ev.gross_minor, "net_minor": ev.net_minor, "funded_by": "merchant" }),
        )
        .await?;
    }
    Ok(())
}

/// The attempt failed or was cancelled: give the caps back.
pub async fn release(
    conn: &mut PgConnection,
    tenant_id: &str,
    attempt_id: &str,
) -> Result<(), ApiError> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "UPDATE offer_redemptions SET status = 'released', updated_at = now() WHERE attempt_id = $1 AND status = 'reserved' RETURNING id, offer_id, discount_minor",
    )
    .bind(attempt_id)
    .fetch_all(&mut *conn)
    .await?;
    for (_, offer_id, discount) in rows {
        sqlx::query("UPDATE offers SET redemptions = redemptions - 1, spent_minor = spent_minor - $2, updated_at = now() WHERE id = $1")
            .bind(&offer_id)
            .bind(discount)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system",
            "offer.released",
            Some(&offer_id),
            json!({ "attempt_id": attempt_id, "discount_minor": discount }),
        )
        .await?;
    }
    Ok(())
}

/// The payment was captured: the discount is real now. A capture that arrives after the attempt was marked failed
/// (a late webhook) takes the offer back even if a cap has been reached since: the payer already paid the lower price.
pub async fn redeemed(
    conn: &mut PgConnection,
    tenant_id: &str,
    attempt_id: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE offer_redemptions SET status = 'redeemed', updated_at = now() WHERE attempt_id = $1 AND status = 'reserved'")
        .bind(attempt_id)
        .execute(&mut *conn)
        .await?;
    let late: Vec<(String, i64)> = sqlx::query_as(
        "UPDATE offer_redemptions SET status = 'redeemed', updated_at = now() WHERE attempt_id = $1 AND status = 'released' RETURNING offer_id, discount_minor",
    )
    .bind(attempt_id)
    .fetch_all(&mut *conn)
    .await?;
    for (offer_id, discount) in late {
        sqlx::query("UPDATE offers SET redemptions = redemptions + 1, spent_minor = spent_minor + $2, updated_at = now() WHERE id = $1")
            .bind(&offer_id)
            .bind(discount)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system",
            "offer.reclaimed_after_late_capture",
            Some(&offer_id),
            json!({ "attempt_id": attempt_id, "discount_minor": discount }),
        )
        .await?;
    }
    Ok(())
}

/// What a refund returns of one discount: the same share as of the cash. `cumulative_refunded` is all the cash
/// handed back so far on the attempt (this refund included) and `paid` what the attempt charged. Rounded half up on
/// the cumulative figure, so the pieces always add up to the whole discount when the payment is refunded in full.
pub fn discount_share(discount: i64, cumulative_refunded: i64, paid: i64) -> i64 {
    if paid <= 0 || cumulative_refunded <= 0 {
        return 0;
    }
    if cumulative_refunded >= paid {
        return discount;
    }
    let n = i128::from(discount) * i128::from(cumulative_refunded);
    let d = i128::from(paid);
    (((n * 2) + d) / (2 * d)) as i64
}

/// One refund finished (called inside the refund's transaction, after the attempt's `refunded_minor` was updated).
pub async fn on_refund(
    conn: &mut PgConnection,
    tenant_id: &str,
    attempt_id: &str,
    refund_id: &str,
    cash_minor: i64,
) -> Result<(), ApiError> {
    let (paid, refunded): (i64, i64) =
        sqlx::query_as("SELECT amount_minor, refunded_minor FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&mut *conn)
            .await?;
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, offer_id, discount_minor, discount_returned_minor FROM offer_redemptions WHERE attempt_id = $1 AND status = 'redeemed' ORDER BY id FOR UPDATE",
    )
    .bind(attempt_id)
    .fetch_all(&mut *conn)
    .await?;
    for (rid, offer_id, discount, returned) in rows {
        let target = discount_share(discount, refunded, paid);
        let delta = (target - returned).max(0);
        let n = sqlx::query("INSERT INTO offer_refund_adjustments (id, tenant_id, redemption_id, refund_id, cash_minor, discount_returned_minor) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (redemption_id, refund_id) DO NOTHING")
            .bind(new_id("ora"))
            .bind(tenant_id)
            .bind(&rid)
            .bind(refund_id)
            .bind(cash_minor)
            .bind(delta)
            .execute(&mut *conn)
            .await?
            .rows_affected();
        if n == 0 {
            continue; // this refund was applied already
        }
        let full = refunded >= paid;
        sqlx::query("UPDATE offer_redemptions SET refunded_cash_minor = refunded_cash_minor + $2, discount_returned_minor = discount_returned_minor + $3, status = CASE WHEN $4 THEN 'reversed' ELSE status END, updated_at = now() WHERE id = $1")
            .bind(&rid)
            .bind(cash_minor)
            .bind(delta)
            .bind(full)
            .execute(&mut *conn)
            .await?;
        // the budget gets back what was returned; a fully refunded payment also frees its place under the caps
        sqlx::query("UPDATE offers SET spent_minor = spent_minor - $2, redemptions = redemptions - CASE WHEN $3 THEN 1 ELSE 0 END, updated_at = now() WHERE id = $1")
            .bind(&offer_id)
            .bind(delta)
            .bind(full)
            .execute(&mut *conn)
            .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system",
            "offer.refund_adjusted",
            Some(&offer_id),
            json!({ "attempt_id": attempt_id, "refund_id": refund_id, "cash_minor": cash_minor, "discount_returned_minor": delta, "fully_refunded": full }),
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------- API

#[derive(Deserialize)]
pub struct BinRange {
    from: String,
    to: String,
}

#[derive(Deserialize)]
pub struct NewOffer {
    name: String,
    #[serde(default)]
    description: Option<String>,
    /// Optional: a code the payer types. Leave out for an offer that applies by itself.
    #[serde(default)]
    code: Option<String>,
    /// `percent` (value in basis points, 1000 = 10%) or `flat` (value in minor units).
    kind: String,
    value: i64,
    #[serde(default)]
    max_discount_minor: Option<i64>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    methods: Option<Vec<String>>,
    #[serde(default)]
    card_networks: Option<Vec<String>>,
    #[serde(default)]
    bin_ranges: Option<Vec<BinRange>>,
    #[serde(default)]
    upi_apps: Option<Vec<String>>,
    #[serde(default)]
    upi_handles: Option<Vec<String>>,
    #[serde(default)]
    banks: Option<Vec<String>>,
    #[serde(default)]
    min_amount_minor: Option<i64>,
    #[serde(default)]
    max_amount_minor: Option<i64>,
    /// `new` or `returning`.
    #[serde(default)]
    segment: Option<String>,
    #[serde(default)]
    valid_from: Option<DateTime<Utc>>,
    #[serde(default)]
    valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    max_redemptions: Option<i32>,
    #[serde(default)]
    max_per_customer: Option<i32>,
    #[serde(default)]
    budget_minor: Option<i64>,
    #[serde(default)]
    stackable: Option<bool>,
    #[serde(default)]
    stack_group: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::invalid("invalid_offer", m)
}

fn words(
    v: &Option<Vec<String>>,
    what: &str,
    handle: bool,
) -> Result<Option<Vec<String>>, ApiError> {
    let Some(v) = v.as_ref().filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.len() > 30 {
        return Err(bad(format!("{what}: at most 30 values")));
    }
    let mut out: Vec<String> = Vec::new();
    for x in v {
        let mut w = x.trim().to_lowercase();
        if handle && !w.starts_with('@') {
            w = format!("@{w}");
        }
        let body = w.trim_start_matches('@');
        if body.is_empty()
            || w.len() > 33
            || !body
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(bad(format!("{what}: {x:?} is not a plain word")));
        }
        if !out.contains(&w) {
            out.push(w);
        }
    }
    Ok(Some(out))
}

/// Check a new offer and turn it into what is stored. Pure apart from the clock.
fn validate(b: &NewOffer, now: DateTime<Utc>) -> Result<Validated, ApiError> {
    let name = b.name.trim().to_string();
    if name.is_empty() || name.len() > 80 {
        return Err(bad("name must be 1-80 characters"));
    }
    match b.kind.as_str() {
        "percent" if (1..=10_000).contains(&b.value) => {}
        "flat" if b.value > 0 && b.value < 1_000_000_000_000 => {}
        _ => {
            return Err(bad(
                "kind is percent (value in basis points, 1-10000) or flat (value in minor units)",
            ));
        }
    }
    if b.max_discount_minor.is_some_and(|m| m <= 0) {
        return Err(bad("max_discount_minor must be positive"));
    }
    let currency = match b.currency.as_deref() {
        None => "INR".to_string(),
        Some(c) => c.parse::<Currency>().map_err(bad)?.code().to_string(),
    };
    let code = match b.code.as_deref().map(crate::coupons::normalise) {
        None => None,
        Some(c)
            if (3..=24).contains(&c.len())
                && c.chars()
                    .all(|x| x.is_ascii_alphanumeric() || x == '-' || x == '_') =>
        {
            Some(c)
        }
        Some(_) => return Err(bad("code must be 3-24 letters, digits, - or _")),
    };
    let methods = match b.methods.as_ref().filter(|m| !m.is_empty()) {
        None => None,
        Some(m) => {
            let mut out: Vec<String> = Vec::new();
            for x in m {
                let parsed: Method = x
                    .trim()
                    .to_lowercase()
                    .parse()
                    .map_err(|e: String| bad(e))?;
                if !out.contains(&parsed.as_str().to_string()) {
                    out.push(parsed.as_str().to_string());
                }
            }
            Some(out)
        }
    };
    let mut networks = None;
    if let Some(n) = b.card_networks.as_ref().filter(|n| !n.is_empty()) {
        let mut out: Vec<String> = Vec::new();
        for x in n {
            let v = instrument_hint::normalise_network(x).map_err(|e| bad(e.message.clone()))?;
            if !out.contains(&v) {
                out.push(v);
            }
        }
        networks = Some(out);
    }
    let mut ranges = Vec::new();
    for r in b.bin_ranges.as_deref().unwrap_or_default() {
        let ok = |s: &str| s.len() == 6 && s.chars().all(|c| c.is_ascii_digit());
        if !ok(&r.from) || !ok(&r.to) || r.from > r.to {
            return Err(bad(
                "each bin_range needs 6-digit from and to, from not above to",
            ));
        }
        ranges.push(json!({ "from": r.from, "to": r.to }));
    }
    if ranges.len() > 20 {
        return Err(bad("at most 20 bin_ranges"));
    }
    let upi_apps = words(&b.upi_apps, "upi_apps", false)?;
    let upi_handles = words(&b.upi_handles, "upi_handles", true)?;
    let banks = words(&b.banks, "banks", false)?;
    // a condition that can never hold with the methods named is a mistake
    if let Some(m) = &methods {
        for (present, needs, what) in [
            (
                networks.is_some() || !ranges.is_empty(),
                "card",
                "card_networks / bin_ranges",
            ),
            (
                upi_apps.is_some() || upi_handles.is_some(),
                "upi",
                "upi_apps / upi_handles",
            ),
            (banks.is_some(), "netbanking", "banks"),
        ] {
            if present && !m.iter().any(|x| x == needs) {
                return Err(bad(format!("{what} need `{needs}` in methods")));
            }
        }
    }
    if b.min_amount_minor.is_some_and(|m| m < 0) || b.max_amount_minor.is_some_and(|m| m <= 0) {
        return Err(bad("the amount band must be positive"));
    }
    if let (Some(lo), Some(hi)) = (b.min_amount_minor, b.max_amount_minor)
        && lo > hi
    {
        return Err(bad("min_amount_minor is above max_amount_minor"));
    }
    if let Some(s) = &b.segment
        && !matches!(s.as_str(), "new" | "returning")
    {
        return Err(bad("segment is new or returning"));
    }
    if b.valid_until.is_some_and(|u| u <= now)
        || matches!((b.valid_from, b.valid_until), (Some(f), Some(u)) if f >= u)
    {
        return Err(bad(
            "valid_until must be in the future and after valid_from",
        ));
    }
    for v in [
        b.max_redemptions.map(i64::from),
        b.max_per_customer.map(i64::from),
        b.budget_minor,
    ] {
        if v.is_some_and(|v| v <= 0) {
            return Err(bad(
                "max_redemptions, max_per_customer and budget_minor must be positive",
            ));
        }
    }
    let stack_group = match b.stack_group.as_deref() {
        None => None,
        Some(g)
            if (1..=24).contains(&g.len())
                && g.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            Some(g.to_lowercase())
        }
        Some(_) => return Err(bad("stack_group must be 1-24 letters, digits, - or _")),
    };
    if stack_group.is_some() && !b.stackable.unwrap_or(false) {
        return Err(bad("stack_group only makes sense with stackable: true"));
    }
    if !(-1000..=1000).contains(&b.priority.unwrap_or(0)) {
        return Err(bad("priority must be between -1000 and 1000"));
    }
    Ok(Validated {
        name,
        code,
        currency,
        methods,
        networks,
        ranges: Value::Array(ranges),
        upi_apps,
        upi_handles,
        banks,
        stack_group,
    })
}

struct Validated {
    name: String,
    code: Option<String>,
    currency: String,
    methods: Option<Vec<String>>,
    networks: Option<Vec<String>>,
    ranges: Value,
    upi_apps: Option<Vec<String>>,
    upi_handles: Option<Vec<String>>,
    banks: Option<Vec<String>>,
    stack_group: Option<String>,
}

pub async fn create_offer(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewOffer>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("offers:write")?;
    let v = validate(&b, Utc::now())?;
    let id = new_id("ofx");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM offers WHERE status = 'active'")
        .fetch_one(&mut *tx)
        .await?;
    if n >= 200 {
        return Err(ApiError::conflict(
            "too_many_offers",
            "at most 200 active offers; pause or archive some first",
        ));
    }
    sqlx::query(
        "INSERT INTO offers (id, tenant_id, name, description, code, kind, value, max_discount_minor, currency, methods, card_networks, bin_ranges, upi_apps, upi_handles, banks,
            min_amount_minor, max_amount_minor, segment, valid_from, valid_until, max_redemptions, max_per_customer, budget_minor, stackable, stack_group, priority, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(&v.name)
    .bind(&b.description)
    .bind(&v.code)
    .bind(&b.kind)
    .bind(b.value)
    .bind(b.max_discount_minor)
    .bind(&v.currency)
    .bind(&v.methods)
    .bind(&v.networks)
    .bind(&v.ranges)
    .bind(&v.upi_apps)
    .bind(&v.upi_handles)
    .bind(&v.banks)
    .bind(b.min_amount_minor)
    .bind(b.max_amount_minor)
    .bind(&b.segment)
    .bind(b.valid_from)
    .bind(b.valid_until)
    .bind(b.max_redemptions)
    .bind(b.max_per_customer)
    .bind(b.budget_minor)
    .bind(b.stackable.unwrap_or(false))
    .bind(&v.stack_group)
    .bind(b.priority.unwrap_or(0))
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            ApiError::conflict("offer_code_exists", "that code is already used by another offer")
        }
        e => e.into(),
    })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "offer.created",
        Some(&id),
        json!({ "name": v.name, "kind": b.kind, "value": b.value, "funded_by": "merchant" }),
    )
    .await?;
    let row = fetch(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!(row))))
}

async fn fetch(conn: &mut PgConnection, id: &str) -> Result<OfferRow, ApiError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM offers WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("offer"))
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

pub async fn list_offers(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<OfferRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM offers WHERE ($1::text IS NULL OR status = $1) ORDER BY created_at DESC, id LIMIT 500"
    )))
    .bind(q.status)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

pub async fn get_offer(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let o = fetch(&mut tx, &id).await?;
    // what it has cost: discount given less what refunds took back
    let (given, returned, count): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(discount_minor) FILTER (WHERE status IN ('reserved','redeemed','reversed')), 0)::bigint,
                COALESCE(SUM(discount_returned_minor) FILTER (WHERE status IN ('reserved','redeemed','reversed')), 0)::bigint,
                COUNT(*) FILTER (WHERE status = 'redeemed')
         FROM offer_redemptions WHERE offer_id = $1",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    Ok(Json(json!({
        "offer": o,
        "totals": { "redeemed_payments": count, "discount_given_minor": given, "discount_returned_by_refunds_minor": returned, "net_cost_minor": given - returned, "funded_by": "merchant" },
    })))
}

#[derive(Deserialize)]
pub struct PatchOffer {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// `active`, `paused` or `archived`.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    valid_until: Option<DateTime<Utc>>,
    #[serde(default)]
    max_redemptions: Option<i32>,
    #[serde(default)]
    max_per_customer: Option<i32>,
    #[serde(default)]
    budget_minor: Option<i64>,
    #[serde(default)]
    priority: Option<i32>,
}

/// Change the parts of an offer that do not change its meaning. The conditions and the amount of the discount are
/// fixed: a different offer is a new offer, so what a past redemption means never changes.
pub async fn update_offer(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<PatchOffer>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:write")?;
    if let Some(st) = &b.status
        && !matches!(st.as_str(), "active" | "paused" | "archived")
    {
        return Err(bad("status is active, paused or archived"));
    }
    if b.name
        .as_deref()
        .is_some_and(|n| n.trim().is_empty() || n.len() > 80)
        || b.max_redemptions.is_some_and(|v| v <= 0)
        || b.max_per_customer.is_some_and(|v| v <= 0)
        || b.budget_minor.is_some_and(|v| v <= 0)
        || b.priority.is_some_and(|p| !(-1000..=1000).contains(&p))
    {
        return Err(bad("a value is out of range"));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur: OfferRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM offers WHERE id = $1 FOR UPDATE"
    )))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("offer"))?;
    if cur.status == "archived" {
        return Err(ApiError::conflict(
            "offer_archived",
            "an archived offer cannot be changed",
        ));
    }
    if b.budget_minor.is_some_and(|v| v < cur.spent_minor) {
        return Err(bad("budget_minor is below what has been spent already"));
    }
    if b.max_redemptions.is_some_and(|v| v < cur.redemptions) {
        return Err(bad("max_redemptions is below the redemptions so far"));
    }
    if b.valid_until.is_some_and(|u| u <= Utc::now()) {
        return Err(bad("valid_until must be in the future"));
    }
    sqlx::query(
        "UPDATE offers SET name = COALESCE($2, name), description = COALESCE($3, description), status = COALESCE($4, status),
            valid_until = COALESCE($5, valid_until), max_redemptions = COALESCE($6, max_redemptions),
            max_per_customer = COALESCE($7, max_per_customer), budget_minor = COALESCE($8, budget_minor),
            priority = COALESCE($9, priority), updated_at = now() WHERE id = $1",
    )
    .bind(&id)
    .bind(b.name.as_deref().map(str::trim))
    .bind(&b.description)
    .bind(&b.status)
    .bind(b.valid_until)
    .bind(b.max_redemptions)
    .bind(b.max_per_customer)
    .bind(b.budget_minor)
    .bind(b.priority)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "offer.updated",
        Some(&id),
        json!({ "status": b.status, "max_redemptions": b.max_redemptions, "max_per_customer": b.max_per_customer, "budget_minor": b.budget_minor, "valid_until": b.valid_until }),
    )
    .await?;
    let row = fetch(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(json!(row)))
}

#[derive(Deserialize)]
pub struct PreviewBody {
    amount_minor: i64,
    #[serde(default)]
    currency: Option<String>,
    method: String,
    #[serde(default)]
    customer_id: Option<String>,
    #[serde(default)]
    instrument: Option<InstrumentIn>,
    #[serde(default)]
    vpa: Option<String>,
    #[serde(default)]
    offer_codes: Option<Vec<String>>,
}

pub(crate) fn clean_codes(codes: Option<&Vec<String>>) -> Vec<String> {
    codes
        .map(|c| {
            c.iter()
                .map(|x| crate::coupons::normalise(x))
                .filter(|x| !x.is_empty())
                .take(10)
                .collect()
        })
        .unwrap_or_default()
}

/// What a payment of this size, method and payer would be offered. Nothing is reserved or counted.
pub async fn preview(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<PreviewBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let currency = b.currency.as_deref().unwrap_or("INR");
    currency
        .parse::<Currency>()
        .map_err(|e| ApiError::invalid("invalid_currency", e))?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive integer within the supported limit",
        ));
    }
    let method: Method = b
        .method
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_method", e))?;
    let ins = instrument_hint::build(b.instrument.as_ref(), b.vpa.as_deref(), None)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(c) = &b.customer_id
        && !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
            .bind(c)
            .fetch_one(&mut *tx)
            .await?
    {
        return Err(ApiError::not_found("customer"));
    }
    let seg = segment(&mut tx, b.customer_id.as_deref(), None).await?;
    let codes = clean_codes(b.offer_codes.as_ref());
    let ev = evaluate(
        &mut tx,
        &Ctx {
            amount_minor: b.amount_minor,
            currency,
            method: method.as_str(),
            instrument: &ins,
            customer_id: b.customer_id.as_deref(),
            segment: seg,
            now: Utc::now(),
            codes: &codes,
        },
        false,
    )
    .await?;
    Ok(Json(json!({
        "preview": true,
        "note": "Nothing is reserved. The caps are checked again when the payment attempt is created.",
        "customer_segment": seg,
        "evaluation": ev,
        "funded_by": "merchant",
    })))
}

#[derive(Deserialize)]
pub struct RefundPreviewBody {
    intent_id: String,
    /// Cash to refund; omit for everything still refundable.
    #[serde(default)]
    amount_minor: Option<i64>,
}

/// What a refund of this much would hand back of the discount. Nothing is changed.
pub async fn refund_preview(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<RefundPreviewBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (attempt, paid, refunded, reserved): (String, i64, i64, i64) = sqlx::query_as(
        "SELECT id, amount_minor, refunded_minor, reserved_refund_minor FROM payment_attempts WHERE intent_id = $1 AND status = 'captured'",
    )
    .bind(&b.intent_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::conflict("payment_not_captured", "only captured payments can be refunded"))?;
    let cash = b.amount_minor.unwrap_or(paid - refunded - reserved);
    if cash <= 0 || cash > paid - refunded - reserved {
        return Err(ApiError::invalid(
            "refund_exceeds_refundable",
            "that is more than is still refundable",
        ));
    }
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, offer_id, discount_minor, discount_returned_minor FROM offer_redemptions WHERE attempt_id = $1 AND status IN ('redeemed', 'reserved') ORDER BY id",
    )
    .bind(&attempt)
    .fetch_all(&mut *tx)
    .await?;
    let after = refunded + cash;
    let lines: Vec<Value> = rows
        .iter()
        .map(|(rid, oid, d, ret)| {
            let target = discount_share(*d, after, paid);
            json!({ "redemption_id": rid, "offer_id": oid, "discount_minor": d, "already_returned_minor": ret, "returned_by_this_refund_minor": (target - ret).max(0) })
        })
        .collect();
    let total: i64 = lines
        .iter()
        .filter_map(|l| l["returned_by_this_refund_minor"].as_i64())
        .sum();
    Ok(Json(json!({
        "intent_id": b.intent_id, "attempt_id": attempt, "paid_minor": paid, "refund_minor": cash,
        "discount_returned_minor": total, "redemptions": lines,
        "rule": "A refund hands back the same share of each discount as it hands back of the cash paid, rounded half up on the running total, so refunding everything returns the whole discount.",
    })))
}

#[derive(Deserialize)]
pub struct RedemptionQuery {
    status: Option<String>,
    limit: Option<i64>,
}

pub async fn list_redemptions(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<RedemptionQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    fetch(&mut tx, &id).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) - 'tenant_id' FROM offer_redemptions r WHERE offer_id = $1 AND ($2::text IS NULL OR status = $2) ORDER BY created_at DESC, id LIMIT $3",
    )
    .bind(&id)
    .bind(q.status)
    .bind(q.limit.unwrap_or(100).clamp(1, 500))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

/// Which offers priced one payment, with the funding source.
pub async fn intent_offers(
    State(s): State<AppState>,
    auth: Auth,
    Path(intent_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("offers:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let ok: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM payment_intents WHERE id = $1)")
            .bind(&intent_id)
            .fetch_one(&mut *tx)
            .await?;
    if !ok {
        return Err(ApiError::not_found("payment intent"));
    }
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) - 'tenant_id' || jsonb_build_object('offer_name', o.name) FROM offer_redemptions r JOIN offers o ON o.id = r.offer_id WHERE r.intent_id = $1 ORDER BY r.created_at, r.id",
    )
    .bind(&intent_id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "intent_id": intent_id, "funded_by": "merchant", "data": rows }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(id: &str, kind: &str, value: i64) -> OfferRow {
        OfferRow {
            id: id.into(),
            name: id.into(),
            description: None,
            code: None,
            status: "active".into(),
            kind: kind.into(),
            value,
            max_discount_minor: None,
            currency: "INR".into(),
            methods: None,
            card_networks: None,
            bin_ranges: json!([]),
            upi_apps: None,
            upi_handles: None,
            banks: None,
            min_amount_minor: None,
            max_amount_minor: None,
            segment: None,
            valid_from: None,
            valid_until: None,
            max_redemptions: None,
            max_per_customer: None,
            budget_minor: None,
            redemptions: 0,
            spent_minor: 0,
            stackable: false,
            stack_group: None,
            priority: 0,
            funded_by: "merchant".into(),
            created_by: "t".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn ctx<'a>(ins: &'a Instrument, codes: &'a [String]) -> Ctx<'a> {
        Ctx {
            amount_minor: 100_000,
            currency: "INR",
            method: "card",
            instrument: ins,
            customer_id: Some("cus_1"),
            segment: Some("new"),
            now: Utc::now(),
            codes,
        }
    }

    #[test]
    fn discounts_are_capped_and_never_take_the_whole_price() {
        let mut o = offer("a", "percent", 1_000);
        assert_eq!(discount_of(&o, 100_000), 10_000);
        o.max_discount_minor = Some(4_000);
        assert_eq!(discount_of(&o, 100_000), 4_000);
        let f = offer("b", "flat", 9_999_999);
        assert_eq!(discount_of(&f, 5_000), 4_999);
        assert_eq!(discount_of(&f, 1), 0);
    }

    #[test]
    fn conditions_all_have_to_hold() {
        let codes = vec![];
        let visa = Instrument {
            card_network: Some("visa".into()),
            ..Default::default()
        };
        let mut o = offer("a", "percent", 500);
        o.methods = Some(vec!["card".into()]);
        o.card_networks = Some(vec!["visa".into()]);
        o.min_amount_minor = Some(50_000);
        assert!(static_check(&o, &ctx(&visa, &codes)).is_ok());

        let rupay = Instrument {
            card_network: Some("rupay".into()),
            ..Default::default()
        };
        assert_eq!(
            static_check(&o, &ctx(&rupay, &codes)).err().unwrap().code,
            "card_network_not_eligible"
        );
        let none = Instrument::default();
        assert_eq!(
            static_check(&o, &ctx(&none, &codes)).err().unwrap().code,
            "card_network_unknown"
        );
        let mut c = ctx(&visa, &codes);
        c.amount_minor = 49_999;
        assert_eq!(static_check(&o, &c).err().unwrap().code, "amount_below_min");
        let mut c = ctx(&visa, &codes);
        c.method = "upi";
        assert_eq!(
            static_check(&o, &c).err().unwrap().code,
            "method_not_eligible"
        );
    }

    #[test]
    fn bin_ranges_segments_windows_and_codes() {
        let codes = vec![];
        let mut ins = Instrument {
            bin6: Some("411150".into()),
            ..Default::default()
        };
        let mut o = offer("a", "flat", 1_000);
        o.bin_ranges = json!([{ "from": "411100", "to": "411199" }]);
        assert!(static_check(&o, &ctx(&ins, &codes)).is_ok());
        ins.bin6 = Some("411200".into());
        assert_eq!(
            static_check(&o, &ctx(&ins, &codes)).err().unwrap().code,
            "card_bin_not_eligible"
        );
        ins.bin6 = None;
        assert_eq!(
            static_check(&o, &ctx(&ins, &codes)).err().unwrap().code,
            "card_bin_needed"
        );

        let mut seg = offer("s", "flat", 1_000);
        seg.segment = Some("returning".into());
        let none = Instrument::default();
        assert_eq!(
            static_check(&seg, &ctx(&none, &codes)).err().unwrap().code,
            "segment_mismatch"
        );
        let mut c = ctx(&none, &codes);
        c.segment = None;
        assert_eq!(
            static_check(&seg, &c).err().unwrap().code,
            "customer_required"
        );

        let mut w = offer("w", "flat", 1_000);
        w.valid_from = Some(Utc::now() + chrono::Duration::hours(1));
        assert_eq!(
            static_check(&w, &ctx(&none, &codes)).err().unwrap().code,
            "not_started"
        );
        w.valid_from = None;
        w.valid_until = Some(Utc::now() - chrono::Duration::hours(1));
        assert_eq!(
            static_check(&w, &ctx(&none, &codes)).err().unwrap().code,
            "expired"
        );

        let mut code = offer("c", "flat", 1_000);
        code.code = Some("SAVE".into());
        let e = static_check(&code, &ctx(&none, &codes)).err().unwrap();
        assert!(e.hidden && e.code == "code_required");
        let typed = vec!["SAVE".to_string()];
        assert!(static_check(&code, &ctx(&none, &typed)).is_ok());
    }

    #[test]
    fn a_non_stackable_offer_applies_alone_and_the_best_wins() {
        let a = offer("a", "percent", 1_000); // 10,000
        let b = offer("b", "flat", 12_000); // 12,000
        let p = plan(100_000, &[&a, &b]);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].index, p[0].discount_minor), (1, 12_000));
        // an exact tie: higher priority, then the smaller id
        let mut c = offer("c", "flat", 10_000);
        c.priority = 5;
        let p = plan(100_000, &[&a, &c]);
        assert_eq!(p[0].index, 1);
        let d = offer("0", "flat", 10_000);
        let p = plan(100_000, &[&a, &d]);
        assert_eq!(
            p[0].index, 1,
            "tie on discount and priority goes to the smaller id"
        );
    }

    #[test]
    fn stackable_offers_combine_one_per_group_and_the_payer_gets_the_larger_way() {
        let mut s1 = offer("s1", "percent", 1_000);
        s1.stackable = true;
        let mut s2 = offer("s2", "flat", 5_000);
        s2.stackable = true;
        // 10% of 100,000 = 10,000, then 5,000 off the remaining: 15,000 in total
        let p = plan(100_000, &[&s1, &s2]);
        assert_eq!(p.iter().map(|x| x.discount_minor).sum::<i64>(), 15_000);
        // a big single offer beats the stack
        let big = offer("big", "flat", 20_000);
        let p = plan(100_000, &[&s1, &s2, &big]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].discount_minor, 20_000);
        // a tie goes to the single offer
        let tie = offer("tie", "flat", 15_000);
        let p = plan(100_000, &[&s1, &s2, &tie]);
        assert_eq!(p.len(), 1);
        // two in one group: only the better one
        let mut g1 = offer("g1", "flat", 3_000);
        g1.stackable = true;
        g1.stack_group = Some("festive".into());
        let mut g2 = offer("g2", "flat", 4_000);
        g2.stackable = true;
        g2.stack_group = Some("festive".into());
        let p = plan(100_000, &[&g1, &g2]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].index, 1);
        // stacking can never take the whole price
        let mut x = offer("x", "flat", 99_999);
        x.stackable = true;
        let mut y = offer("y", "flat", 99_999);
        y.stackable = true;
        let p = plan(100_000, &[&x, &y]);
        assert_eq!(p.iter().map(|x| x.discount_minor).sum::<i64>(), 99_999);
    }

    #[test]
    fn a_refund_returns_the_same_share_of_the_discount_as_of_the_cash() {
        // paid 90,000 for a 100,000 price (discount 10,000)
        assert_eq!(discount_share(10_000, 0, 90_000), 0);
        assert_eq!(discount_share(10_000, 45_000, 90_000), 5_000);
        assert_eq!(discount_share(10_000, 90_000, 90_000), 10_000);
        // the pieces of a refund in three parts add up to the whole, whatever the rounding
        let (d, paid) = (1_001, 9_999);
        let mut total = 0;
        let mut cum = 0;
        for part in [3_333, 3_333, 3_333] {
            cum += part;
            let t = discount_share(d, cum, paid);
            assert!(t >= total, "never goes backwards");
            total = t;
        }
        assert_eq!(total, discount_share(d, 9_999, paid));
        assert_eq!(discount_share(d, 9_999, paid), d);
    }

    #[test]
    fn validation_catches_mistakes() {
        let base = || NewOffer {
            name: "Festive".into(),
            description: None,
            code: None,
            kind: "percent".into(),
            value: 1_000,
            max_discount_minor: None,
            currency: None,
            methods: None,
            card_networks: None,
            bin_ranges: None,
            upi_apps: None,
            upi_handles: None,
            banks: None,
            min_amount_minor: None,
            max_amount_minor: None,
            segment: None,
            valid_from: None,
            valid_until: None,
            max_redemptions: None,
            max_per_customer: None,
            budget_minor: None,
            stackable: None,
            stack_group: None,
            priority: None,
        };
        let now = Utc::now();
        assert!(validate(&base(), now).is_ok());
        let mut b = base();
        b.value = 10_001;
        assert!(validate(&b, now).is_err());
        let mut b = base();
        b.methods = Some(vec!["upi".into()]);
        b.card_networks = Some(vec!["visa".into()]);
        assert!(
            validate(&b, now).is_err(),
            "a card network on a UPI-only offer"
        );
        let mut b = base();
        b.bin_ranges = Some(vec![BinRange {
            from: "411199".into(),
            to: "411100".into(),
        }]);
        assert!(validate(&b, now).is_err());
        let mut b = base();
        b.segment = Some("vip".into());
        assert!(validate(&b, now).is_err());
        let mut b = base();
        b.stack_group = Some("g".into());
        assert!(validate(&b, now).is_err());
        let mut b = base();
        b.upi_handles = Some(vec!["OkAxis".into()]);
        assert_eq!(
            validate(&b, now).unwrap().upi_handles,
            Some(vec!["@okaxis".to_string()])
        );
        let mut b = base();
        b.valid_until = Some(now - chrono::Duration::days(1));
        assert!(validate(&b, now).is_err());
    }
}
