// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Commission rules: what the merchant keeps out of what a vendor earned on a payment.
//!
//! A rule is a percentage (basis points) and/or a flat amount, with an optional floor and cap, for a vendor, a
//! payment link or an event reference, effective from a date to a date. No rate is built in: every number is typed
//! by a person, and with no rule that applies, a share given as `gross_minor` is refused rather than guessed.
//!
//! Rules work through the existing split mechanism (`splits`): when a payment (or a payment-link opening) names a
//! vendor's `gross_minor`, the rule in force today turns it into a fixed share (`gross - commission`) before the
//! split is stored. From then on the share accrues on capture and reverses in proportion on refunds exactly like
//! any other split, and the split row keeps the gross, the commission and the rule that produced it.
//!
//! Which rule applies: of the active rules in force today, in the payment's currency, whose scope fits (vendor,
//! payment link and event reference each either match or are left open), the most specific one: a link beats an
//! event, an event beats a vendor, and combinations add up. Two active rules with the same scope may not overlap in
//! time (refused when created), so the answer is never a tie. With none, the vendor's default rule is used if it is
//! in force. A rule with no scope at all is a shared rule: it applies only where a vendor names it as its default.
//!
//! Commission is `floor(gross * basis_points / 10000) + flat`, raised to the floor and cut to the cap when those
//! are set. The vendor's share is the gross less the commission and must stay positive.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};

use crate::{ApiError, AppState, auth::Auth, clock, splits::SplitIn, tenant};

/// The numbers of one rule (or of an inline rule in a preview).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Terms {
    pub basis_points: Option<i64>,
    pub flat_minor: Option<i64>,
    pub min_commission_minor: Option<i64>,
    pub cap_minor: Option<i64>,
}

impl Terms {
    pub fn validate(&self) -> Result<(), ApiError> {
        let bad = |m: &str| Err(ApiError::invalid("invalid_commission_rule", m.to_string()));
        if self.basis_points.is_none() && self.flat_minor.is_none() {
            return bad("give basis_points, flat_minor or both");
        }
        if self
            .basis_points
            .is_some_and(|b| !(0..=10_000).contains(&b))
        {
            return bad("basis_points must be between 0 and 10000 (100 = 1%)");
        }
        let money =
            |v: Option<i64>| v.is_none_or(|n| (0..=zyvor_core::MAX_AMOUNT_MINOR).contains(&n));
        if !money(self.flat_minor) || !money(self.min_commission_minor) || !money(self.cap_minor) {
            return bad(
                "flat_minor, min_commission_minor and cap_minor are whole paise, zero or more",
            );
        }
        if let (Some(min), Some(cap)) = (self.min_commission_minor, self.cap_minor)
            && cap < min
        {
            return bad("cap_minor cannot be below min_commission_minor");
        }
        Ok(())
    }

    /// The commission on `gross` paise.
    pub fn commission(&self, gross: i64) -> i64 {
        let pct = self
            .basis_points
            .map_or(0i128, |bp| gross as i128 * bp as i128 / 10_000);
        let mut c = pct + self.flat_minor.unwrap_or(0) as i128;
        if let Some(min) = self.min_commission_minor {
            c = c.max(min as i128);
        }
        if let Some(cap) = self.cap_minor {
            c = c.min(cap as i128);
        }
        c.clamp(0, i64::MAX as i128) as i64
    }
}

/// A stored rule.
#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub vendor_id: Option<String>,
    pub payment_link_id: Option<String>,
    pub event_ref: Option<String>,
    pub currency: String,
    pub terms: Terms,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub status: String,
}

impl Rule {
    pub fn in_force(&self, on: NaiveDate) -> bool {
        self.status == "active"
            && self.effective_from <= on
            && self.effective_to.is_none_or(|t| t >= on)
    }
    fn scoped(&self) -> bool {
        self.vendor_id.is_some() || self.payment_link_id.is_some() || self.event_ref.is_some()
    }
    /// How specific the scope is, or `None` when the rule does not fit this payment.
    fn fit(&self, vendor: &str, link: Option<&str>, event: Option<&str>) -> Option<u8> {
        if !self.scoped() {
            return None;
        }
        let mut score = 0;
        if let Some(v) = &self.vendor_id {
            if v != vendor {
                return None;
            }
            score += 1;
        }
        if let Some(e) = &self.event_ref {
            if Some(e.as_str()) != event {
                return None;
            }
            score += 2;
        }
        if let Some(l) = &self.payment_link_id {
            if Some(l.as_str()) != link {
                return None;
            }
            score += 4;
        }
        Some(score)
    }
}

/// The rule that applies: the most specific one in force, else the vendor's default if in force.
pub fn pick<'a>(
    rules: &'a [Rule],
    default_rule_id: Option<&str>,
    vendor: &str,
    link: Option<&str>,
    event: Option<&str>,
    currency: &str,
    on: NaiveDate,
) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.currency == currency && r.in_force(on))
        .filter_map(|r| r.fit(vendor, link, event).map(|s| (s, r)))
        .max_by_key(|(s, r)| (*s, r.effective_from))
        .map(|(_, r)| r)
        .or_else(|| {
            let d = default_rule_id?;
            rules.iter().find(|r| {
                r.id == d
                    && r.currency == currency
                    && r.in_force(on)
                    && r.vendor_id.as_deref().is_none_or(|v| v == vendor)
            })
        })
}

const COLS: &str = "id, name, vendor_id, payment_link_id, event_ref, currency, basis_points, flat_minor, min_commission_minor, cap_minor, effective_from, effective_to, status";
type Row = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<i32>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    NaiveDate,
    Option<NaiveDate>,
    String,
);

fn rule_of(r: Row) -> Rule {
    Rule {
        id: r.0,
        name: r.1,
        vendor_id: r.2,
        payment_link_id: r.3,
        event_ref: r.4,
        currency: r.5,
        terms: Terms {
            basis_points: r.6.map(i64::from),
            flat_minor: r.7,
            min_commission_minor: r.8,
            cap_minor: r.9,
        },
        effective_from: r.10,
        effective_to: r.11,
        status: r.12,
    }
}

fn rule_json(r: &Rule) -> Value {
    json!({
        "id": r.id, "name": r.name, "vendor_id": r.vendor_id, "payment_link_id": r.payment_link_id,
        "event_ref": r.event_ref, "currency": r.currency,
        "basis_points": r.terms.basis_points, "flat_minor": r.terms.flat_minor,
        "min_commission_minor": r.terms.min_commission_minor, "cap_minor": r.terms.cap_minor,
        "effective_from": r.effective_from, "effective_to": r.effective_to, "status": r.status,
        "scope": if !r.scoped() { "shared" } else { "scoped" },
    })
}

async fn load_rules(
    conn: &mut PgConnection,
    vendor: Option<&str>,
    currency: &str,
) -> Result<Vec<Rule>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM commission_rules WHERE status = 'active' AND currency = $1 AND ($2::text IS NULL OR vendor_id = $2 OR vendor_id IS NULL)"
    )))
    .bind(currency)
    .bind(vendor)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(rule_of).collect())
}

/// What a payment is being split for.
pub struct Ctx<'a> {
    pub currency: &'a str,
    pub payment_link_id: Option<&'a str>,
    pub event_ref: Option<String>,
}

/// Turn every share given as `gross_minor` into a fixed share: the gross less the commission of the rule in force
/// today. Shares with a fixed `amount_minor` are left alone.
pub async fn resolve_splits(
    conn: &mut PgConnection,
    splits: &mut [SplitIn],
    ctx: &Ctx<'_>,
) -> Result<(), ApiError> {
    if splits.iter().all(|s| s.gross_minor.is_none()) {
        return Ok(());
    }
    let today = clock::today(conn).await?;
    for s in splits
        .iter_mut()
        .filter(|s| s.gross_minor.is_some() && s.rule_id.is_none())
    {
        let gross = s.gross_minor.unwrap_or(0);
        if gross <= 0 {
            return Err(ApiError::invalid(
                "invalid_splits",
                "each split must be a positive amount",
            ));
        }
        let default: Option<Option<String>> =
            sqlx::query_scalar("SELECT default_rule_id FROM vendors WHERE id = $1")
                .bind(&s.vendor_id)
                .fetch_optional(&mut *conn)
                .await?;
        let default = default.ok_or_else(|| ApiError::not_found("vendor"))?;
        let rules = load_rules(conn, Some(&s.vendor_id), ctx.currency).await?;
        let rule = pick(
            &rules,
            default.as_deref(),
            &s.vendor_id,
            ctx.payment_link_id,
            ctx.event_ref.as_deref(),
            ctx.currency,
            today,
        )
        .ok_or_else(|| {
            ApiError::conflict(
                "no_commission_rule",
                format!(
                    "no commission rule is in force today for vendor {} (add one, set the vendor's default, or give a fixed amount_minor)",
                    s.vendor_id
                ),
            )
        })?;
        let commission = rule.terms.commission(gross);
        if commission >= gross {
            return Err(ApiError::invalid(
                "commission_exceeds_share",
                format!(
                    "the commission ({commission}) is not less than the amount for vendor {} ({gross})",
                    s.vendor_id
                ),
            ));
        }
        s.amount_minor = gross - commission;
        s.commission_minor = commission;
        s.rule_id = Some(rule.id.clone());
    }
    Ok(())
}

// ---------- endpoints ----------

#[derive(Deserialize)]
pub struct NewCommissionRule {
    name: String,
    vendor_id: Option<String>,
    payment_link_id: Option<String>,
    event_ref: Option<String>,
    currency: Option<String>,
    /// Percentage of what the vendor earned, in basis points (100 = 1%).
    basis_points: Option<i64>,
    /// A flat amount in paise, added to the percentage.
    flat_minor: Option<i64>,
    /// The commission is at least this many paise.
    min_commission_minor: Option<i64>,
    /// The commission is at most this many paise.
    cap_minor: Option<i64>,
    effective_from: NaiveDate,
    effective_to: Option<NaiveDate>,
}

/// `POST /v1/commission_rules`
pub async fn create_rule(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCommissionRule>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:approve")?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::invalid(
            "invalid_commission_rule",
            "name is required (up to 80 characters)",
        ));
    }
    let terms = Terms {
        basis_points: b.basis_points,
        flat_minor: b.flat_minor,
        min_commission_minor: b.min_commission_minor,
        cap_minor: b.cap_minor,
    };
    terms.validate()?;
    if b.effective_to.is_some_and(|t| t < b.effective_from) {
        return Err(ApiError::invalid(
            "invalid_commission_rule",
            "effective_to cannot be before effective_from",
        ));
    }
    let ccy: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let event_ref = b
        .event_ref
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty());
    if event_ref.is_some_and(|e| e.len() > 120) {
        return Err(ApiError::invalid(
            "invalid_commission_rule",
            "event_ref is up to 120 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // foreign keys ignore row-level security: make sure what is named belongs to this tenant
    if let Some(v) = &b.vendor_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM vendors WHERE id = $1)")
            .bind(v)
            .fetch_one(&mut *tx)
            .await?;
        if !ok {
            return Err(ApiError::not_found("vendor"));
        }
    }
    if let Some(l) = &b.payment_link_id {
        let ok: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM payment_links WHERE id = $1)")
                .bind(l)
                .fetch_one(&mut *tx)
                .await?;
        if !ok {
            return Err(ApiError::not_found("payment link"));
        }
    }
    let scoped = b.vendor_id.is_some() || b.payment_link_id.is_some() || event_ref.is_some();
    if scoped {
        // two active rules of the same scope may not both be in force on one day
        let clash: Option<String> = sqlx::query_scalar(
            "SELECT id FROM commission_rules
             WHERE status = 'active' AND currency = $1
               AND vendor_id IS NOT DISTINCT FROM $2 AND payment_link_id IS NOT DISTINCT FROM $3
               AND event_ref IS NOT DISTINCT FROM $4
               AND effective_from <= COALESCE($6::date, 'infinity'::date)
               AND COALESCE(effective_to, 'infinity'::date) >= $5
             LIMIT 1",
        )
        .bind(ccy.code())
        .bind(&b.vendor_id)
        .bind(&b.payment_link_id)
        .bind(event_ref)
        .bind(b.effective_from)
        .bind(b.effective_to)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(other) = clash {
            return Err(ApiError::conflict(
                "rule_overlaps",
                format!(
                    "rule {other} already covers this scope on some of these dates; retire it or end it first"
                ),
            ));
        }
    }
    let id = new_id("cmr");
    sqlx::query("INSERT INTO commission_rules (id, tenant_id, name, vendor_id, payment_link_id, event_ref, currency, basis_points, flat_minor, min_commission_minor, cap_minor, effective_from, effective_to, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
        .bind(&id).bind(&auth.tenant_id).bind(name).bind(&b.vendor_id).bind(&b.payment_link_id).bind(event_ref)
        .bind(ccy.code()).bind(terms.basis_points.map(|v| v as i32)).bind(terms.flat_minor)
        .bind(terms.min_commission_minor).bind(terms.cap_minor).bind(b.effective_from).bind(b.effective_to)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "commission_rule.created",
        Some(&id),
        json!({ "name": name, "vendor_id": b.vendor_id, "basis_points": terms.basis_points, "flat_minor": terms.flat_minor }),
    )
    .await?;
    let out = fetch(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(rule_json(&out))))
}

async fn fetch(conn: &mut PgConnection, id: &str) -> Result<Rule, ApiError> {
    let r: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM commission_rules WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?;
    r.map(rule_of)
        .ok_or_else(|| ApiError::not_found("commission rule"))
}

#[derive(Deserialize)]
pub struct CommissionRuleListQuery {
    vendor_id: Option<String>,
    status: Option<String>,
}

/// `GET /v1/commission_rules`
pub async fn list_rules(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<CommissionRuleListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM commission_rules WHERE ($1::text IS NULL OR vendor_id = $1) AND ($2::text IS NULL OR status = $2) ORDER BY created_at DESC, id DESC LIMIT 500"
    )))
    .bind(&q.vendor_id)
    .bind(&q.status)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|r| rule_json(&rule_of(r))).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/commission_rules/{id}`
pub async fn get_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(rule_json(&fetch(&mut tx, &id).await?)))
}

/// `POST /v1/commission_rules/{id}/retire`: the rule stops applying to new payments. Shares already recorded keep
/// the commission they were given.
pub async fn retire_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r = fetch(&mut tx, &id).await?;
    if r.status == "retired" {
        return Err(ApiError::conflict("already_retired", "already retired"));
    }
    sqlx::query("UPDATE commission_rules SET status = 'retired', retired_by = $2, retired_at = now() WHERE id = $1")
        .bind(&id)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "commission_rule.retired",
        Some(&id),
        json!({}),
    )
    .await?;
    let out = fetch(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(rule_json(&out)))
}

#[derive(Deserialize)]
pub struct PreviewSplit {
    vendor_id: String,
    /// What the vendor earned on the sample payment. Left out with a single vendor: the whole payment.
    gross_minor: Option<i64>,
    /// Try a rule that is not saved.
    rule: Option<Terms>,
}

#[derive(Deserialize)]
pub struct CommissionPreviewIn {
    amount_minor: i64,
    currency: Option<String>,
    payment_link_id: Option<String>,
    event_ref: Option<String>,
    /// The day the payment is made; the organisation's today when left out.
    on: Option<NaiveDate>,
    splits: Vec<PreviewSplit>,
    /// A refund to show the effect of: each share is reversed in proportion.
    refund_minor: Option<i64>,
}

/// `POST /v1/commission_rules/preview`: exactly what each party would get on a sample amount. Stores nothing.
pub async fn preview(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<CommissionPreviewIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be positive and within the supported limit",
        ));
    }
    if b.splits.is_empty() || b.splits.len() > 20 {
        return Err(ApiError::invalid(
            "invalid_splits",
            "give between 1 and 20 splits",
        ));
    }
    let refund = b.refund_minor.unwrap_or(0);
    if !(0..=b.amount_minor).contains(&refund) {
        return Err(ApiError::invalid(
            "invalid_amount",
            "refund_minor must be between 0 and the payment amount",
        ));
    }
    let ccy: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    for sp in &b.splits {
        if let Some(t) = &sp.rule {
            t.validate()?;
        }
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = match b.on {
        Some(d) => d,
        None => clock::today(&mut tx).await?,
    };
    let mut parties = Vec::new();
    let (mut gross_sum, mut share_sum, mut commission_sum, mut reversed_sum) =
        (0i64, 0i64, 0i64, 0i64);
    let mut problems = Vec::new();
    for sp in &b.splits {
        let gross = sp
            .gross_minor
            .or((b.splits.len() == 1).then_some(b.amount_minor));
        let Some(gross) = gross.filter(|g| *g > 0) else {
            problems.push(format!(
                "vendor {}: give a positive gross_minor",
                sp.vendor_id
            ));
            continue;
        };
        let known: Option<Option<String>> =
            sqlx::query_scalar("SELECT default_rule_id FROM vendors WHERE id = $1")
                .bind(&sp.vendor_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(default) = known else {
            problems.push(format!("vendor {} not found", sp.vendor_id));
            continue;
        };
        let (terms, rule_json_v) = match &sp.rule {
            Some(t) => (
                t.clone(),
                json!({ "id": null, "name": "inline rule (not saved)", "basis_points": t.basis_points, "flat_minor": t.flat_minor, "min_commission_minor": t.min_commission_minor, "cap_minor": t.cap_minor }),
            ),
            None => {
                let rules = load_rules(&mut tx, Some(&sp.vendor_id), ccy.code()).await?;
                match pick(
                    &rules,
                    default.as_deref(),
                    &sp.vendor_id,
                    b.payment_link_id.as_deref(),
                    b.event_ref.as_deref(),
                    ccy.code(),
                    today,
                ) {
                    Some(r) => (r.terms.clone(), rule_json(r)),
                    None => {
                        problems.push(format!(
                            "vendor {}: no commission rule is in force on {today}",
                            sp.vendor_id
                        ));
                        continue;
                    }
                }
            }
        };
        let commission = terms.commission(gross);
        if commission >= gross {
            problems.push(format!(
                "vendor {}: the commission ({commission}) is not less than the amount ({gross})",
                sp.vendor_id
            ));
            continue;
        }
        let share = gross - commission;
        let reversed = ((share as i128 * refund as i128) / b.amount_minor as i128) as i64;
        gross_sum += gross;
        share_sum += share;
        commission_sum += commission;
        reversed_sum += reversed;
        parties.push(json!({
            "vendor_id": sp.vendor_id, "gross_minor": gross, "commission_minor": commission,
            "vendor_share_minor": share, "rule": rule_json_v,
            "reversed_on_refund_minor": reversed, "vendor_keeps_after_refund_minor": share - reversed,
        }));
    }
    if gross_sum > b.amount_minor {
        problems.push("the vendors' amounts add up to more than the payment".to_string());
    }
    Ok(Json(json!({
        "ok": problems.is_empty(),
        "problems": problems,
        "on": today, "currency": ccy.code(), "amount_minor": b.amount_minor,
        "parties": parties,
        "vendors_total_minor": share_sum,
        "commission_total_minor": commission_sum,
        // the payment less what the vendors are owed: the commission plus any part not given to a vendor
        "merchant_keeps_minor": b.amount_minor - share_sum,
        "refund_minor": refund,
        "merchant_keeps_after_refund_minor": b.amount_minor - refund - (share_sum - reversed_sum),
        "rounding": "commission is rounded down to the paisa; each refund reverses a share in proportion, rounded down",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn rule(id: &str, vendor: Option<&str>, link: Option<&str>, event: Option<&str>) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            vendor_id: vendor.map(Into::into),
            payment_link_id: link.map(Into::into),
            event_ref: event.map(Into::into),
            currency: "INR".into(),
            terms: Terms {
                basis_points: Some(1000),
                ..Default::default()
            },
            effective_from: d("2026-01-01"),
            effective_to: None,
            status: "active".into(),
        }
    }

    #[test]
    fn commission_is_percentage_plus_flat_within_floor_and_cap() {
        let t = |bp, flat, min, cap| Terms {
            basis_points: bp,
            flat_minor: flat,
            min_commission_minor: min,
            cap_minor: cap,
        };
        assert_eq!(t(Some(1000), None, None, None).commission(10_000), 1_000);
        assert_eq!(
            t(Some(1000), Some(300), None, None).commission(10_000),
            1_300
        );
        assert_eq!(t(None, Some(500), None, None).commission(10_000), 500);
        // rounds down to the paisa
        assert_eq!(t(Some(333), None, None, None).commission(1_001), 33);
        assert_eq!(t(Some(1000), None, None, Some(700)).commission(10_000), 700);
        assert_eq!(t(Some(100), None, Some(250), None).commission(10_000), 250);
        // huge amounts do not overflow
        assert_eq!(
            t(Some(10_000), None, None, None).commission(zyvor_core::MAX_AMOUNT_MINOR),
            zyvor_core::MAX_AMOUNT_MINOR
        );
    }

    #[test]
    fn terms_are_validated() {
        assert!(Terms::default().validate().is_err(), "a rule needs a rate");
        let ok = Terms {
            basis_points: Some(10_000),
            ..Default::default()
        };
        assert!(ok.validate().is_ok());
        assert!(
            Terms {
                basis_points: Some(10_001),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Terms {
                flat_minor: Some(-1),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Terms {
                basis_points: Some(1),
                min_commission_minor: Some(500),
                cap_minor: Some(100),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn the_most_specific_rule_in_force_wins() {
        let rules = vec![
            rule("vendor", Some("v1"), None, None),
            rule("event", None, None, Some("fest")),
            rule("link", None, Some("plk_1"), None),
            rule("vendor_link", Some("v1"), Some("plk_1"), None),
            rule("shared", None, None, None),
        ];
        let on = d("2026-06-01");
        let pick_id =
            |link, event| pick(&rules, None, "v1", link, event, "INR", on).map(|r| r.id.as_str());
        assert_eq!(pick_id(None, None), Some("vendor"));
        assert_eq!(pick_id(None, Some("fest")), Some("event"));
        assert_eq!(pick_id(Some("plk_1"), Some("fest")), Some("vendor_link"));
        assert_eq!(pick_id(Some("plk_other"), None), Some("vendor"));
        // another vendor: only the event and link rules, which are open to any vendor
        assert_eq!(
            pick(&rules, None, "v2", None, None, "INR", on).map(|r| r.id.as_str()),
            None,
            "a shared rule is not applied unless it is the vendor's default"
        );
        assert_eq!(
            pick(&rules, Some("shared"), "v2", None, None, "INR", on).map(|r| r.id.as_str()),
            Some("shared")
        );
    }

    #[test]
    fn effective_dates_status_and_currency_are_respected() {
        let mut r = rule("a", Some("v1"), None, None);
        r.effective_from = d("2026-03-01");
        r.effective_to = Some(d("2026-03-31"));
        let rules = vec![r.clone()];
        let p = |on: &str, ccy: &str| pick(&rules, None, "v1", None, None, ccy, d(on)).is_some();
        assert!(!p("2026-02-28", "INR"));
        assert!(p("2026-03-01", "INR") && p("2026-03-31", "INR"));
        assert!(!p("2026-04-01", "INR"));
        assert!(!p("2026-03-15", "USD"));
        r.status = "retired".into();
        assert!(pick(&[r], None, "v1", None, None, "INR", d("2026-03-15")).is_none());
    }
}
