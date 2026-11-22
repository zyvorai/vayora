// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Fee checks: what a provider actually deducted from a settlement, against the fee rules the merchant
//! entered from their own contract.
//!
//! Rates are data, never code: nothing here ships a default rate for any provider, method, network or tax.
//! A payment with no rule in force on its date is reported as "no expected fee configured" and is never flagged.
//! A variance above the configured tolerance becomes a finding (a row in `fee_check_lines` plus one reconciliation
//! case per settlement, with an owner when one is configured). Findings only explain and ask a person to review;
//! they never change money, the ledger or a payment, and both over- and under-deduction are reported.
//!
//! Nothing here has been checked against any provider's real statements.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;
use zyvor_providers::Method;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, tenant};

/// The case kind used for fee findings (one case per settlement).
pub const CASE_KIND: &str = "fee_check";

// ---------- pure calculation ----------

/// A fee rule as stored. All amounts are whole paise; rates are basis points.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub id: String,
    pub provider: String,
    pub method: String,
    pub bps: i64,
    pub flat_minor: i64,
    /// Most the fee (percentage plus flat, before GST) can come to for one payment.
    pub cap_minor: Option<i64>,
    /// GST charged on the fee, in basis points. `None` means the contract has none.
    pub gst_bps: Option<i64>,
    /// The provider gives the fee back on refunded value.
    pub refund_returns_fee: bool,
    pub effective_from: NaiveDate,
}

impl Rule {
    fn snapshot(&self, b: &Breakdown) -> Value {
        json!({
            "id": self.id, "provider": self.provider, "method": self.method,
            "bps": self.bps, "flat_minor": self.flat_minor, "cap_minor": self.cap_minor,
            "gst_bps": self.gst_bps, "refund_returns_fee": self.refund_returns_fee,
            "effective_from": self.effective_from,
            "breakdown": {
                "percentage_minor": b.percentage_minor, "flat_minor": b.flat_minor, "capped": b.capped,
                "before_tax_minor": b.before_tax_minor, "gst_minor": b.gst_minor, "total_minor": b.total_minor,
            },
        })
    }
}

/// How an expected fee was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Breakdown {
    pub percentage_minor: i64,
    pub flat_minor: i64,
    pub capped: bool,
    pub before_tax_minor: i64,
    pub gst_minor: i64,
    pub total_minor: i64,
}

/// `value * bps / 10_000`, rounded half up. Integer arithmetic only; wide enough not to overflow.
fn bps_of(value: i64, bps: i64) -> i64 {
    ((i128::from(value) * i128::from(bps) + 5_000) / 10_000) as i64
}

/// The fee the rule expects on a payment of `amount_minor`: percentage (half up), plus flat, limited by the cap,
/// then GST on that fee (half up). A payment of nothing costs nothing.
pub fn expected_fee(rule: &Rule, amount_minor: i64) -> Breakdown {
    if amount_minor <= 0 {
        return Breakdown::default();
    }
    let percentage = bps_of(amount_minor, rule.bps);
    let uncapped = percentage + rule.flat_minor;
    let (before_tax, capped) = match rule.cap_minor {
        Some(cap) if uncapped > cap => (cap, true),
        _ => (uncapped, false),
    };
    let gst = rule.gst_bps.map_or(0, |g| bps_of(before_tax, g));
    Breakdown {
        percentage_minor: percentage,
        flat_minor: rule.flat_minor,
        capped,
        before_tax_minor: before_tax,
        gst_minor: gst,
        total_minor: before_tax + gst,
    }
}

/// The rule in force on `on`: the one with the latest `effective_from` that is not after that date.
pub fn pick_rule<'a>(
    rules: &'a [Rule],
    provider: &str,
    method: &str,
    on: NaiveDate,
) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.provider == provider && r.method == method && r.effective_from <= on)
        .max_by_key(|r| r.effective_from)
}

/// What one payment (attempt) did inside one settlement.
#[derive(Debug, Clone)]
pub struct LineInput {
    pub attempt_id: String,
    pub provider: String,
    pub method: String,
    /// The day the payment was made (IST): picks the rule.
    pub paid_on: NaiveDate,
    /// What we recorded for the payment.
    pub attempt_amount_minor: i64,
    /// The payment line in this settlement, when there is one.
    pub payment_in_settlement_minor: Option<i64>,
    /// Refunds against this payment in this settlement (positive).
    pub refunded_minor: i64,
    /// Fee lines for this payment in this settlement, net: deducted is positive, returned is negative.
    pub actual_minor: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStatus {
    Ok,
    Flagged,
    NoRule,
}

impl LineStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LineStatus::Ok => "ok",
            LineStatus::Flagged => "flagged",
            LineStatus::NoRule => "no_rule",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LineResult {
    pub input: LineInput,
    pub status: LineStatus,
    pub expected_minor: Option<i64>,
    /// actual - expected. Positive: the provider deducted more than the rule says.
    pub variance_minor: Option<i64>,
    pub rule: Option<Value>,
    pub rule_id: Option<String>,
}

/// Check one payment's net fee against the rule in force on its date.
///
/// The expected net fee is the fee on the payment in this settlement, less the fee given back on refunded
/// value when the rule says refunds return the fee. Returning the fee means the fee on the payment amount minus
/// the fee on what is left after the refund (nothing is left after a full refund, so flat and GST come back too).
pub fn check_line(input: &LineInput, rules: &[Rule], tolerance_minor: i64) -> LineResult {
    let Some(rule) = pick_rule(rules, &input.provider, &input.method, input.paid_on) else {
        return LineResult {
            input: input.clone(),
            status: LineStatus::NoRule,
            expected_minor: None,
            variance_minor: None,
            rule: None,
            rule_id: None,
        };
    };
    let on_payment = input
        .payment_in_settlement_minor
        .map(|p| expected_fee(rule, p));
    let charged = on_payment.map_or(0, |b| b.total_minor);
    let returned = if rule.refund_returns_fee && input.refunded_minor > 0 {
        let basis = input
            .payment_in_settlement_minor
            .unwrap_or(input.attempt_amount_minor);
        let full = expected_fee(rule, basis).total_minor;
        let left = basis - input.refunded_minor;
        full - if left > 0 {
            expected_fee(rule, left).total_minor
        } else {
            0
        }
    } else {
        0
    };
    let expected = charged - returned;
    let variance = input.actual_minor - expected;
    // the breakdown shown is the one for the payment (or, for a refund-only line, for the payment amount)
    let shown = on_payment.unwrap_or_else(|| expected_fee(rule, input.attempt_amount_minor));
    LineResult {
        input: input.clone(),
        status: if variance.abs() > tolerance_minor {
            LineStatus::Flagged
        } else {
            LineStatus::Ok
        },
        expected_minor: Some(expected),
        variance_minor: Some(variance),
        rule: Some(rule.snapshot(&shown)),
        rule_id: Some(rule.id.clone()),
    }
}

fn money(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    format!("{sign}₹{}.{:02}", minor.abs() / 100, minor.abs() % 100)
}

/// Totals over the flagged lines of one settlement: (over, under) as positive paise, and the counts.
pub fn flagged_totals(lines: &[LineResult]) -> (i64, i64, usize, usize) {
    let (mut over, mut under, mut n_over, mut n_under) = (0, 0, 0, 0);
    for v in lines
        .iter()
        .filter(|l| l.status == LineStatus::Flagged)
        .filter_map(|l| l.variance_minor)
    {
        if v > 0 {
            over += v;
            n_over += 1;
        } else {
            under += -v;
            n_under += 1;
        }
    }
    (over, under, n_over, n_under)
}

/// The plain sentence for "settlement explained" (empty when nothing is flagged).
pub fn variance_sentence(lines: &[LineResult]) -> Option<String> {
    let (over, under, n_over, n_under) = flagged_totals(lines);
    if n_over + n_under == 0 {
        return None;
    }
    let detail = match (n_over, n_under) {
        (_, 0) => format!(
            "the provider deducted {} more than expected on {n_over} payment(s)",
            money(over)
        ),
        (0, _) => format!(
            "the provider deducted {} less than expected on {n_under} payment(s)",
            money(under)
        ),
        _ => format!(
            "{} more than expected on {n_over} payment(s) and {} less on {n_under}",
            money(over),
            money(under)
        ),
    };
    Some(format!(
        "Fee differs from your configured rate by {} ({detail}). A person should check it against the contract; nothing has been changed.",
        money(over + under)
    ))
}

// ---------- reading a settlement ----------

/// The result of checking one settlement. Nothing in it has been written yet.
#[derive(Debug)]
pub struct SettlementCheck {
    pub settlement_id: String,
    pub provider: String,
    pub currency: String,
    pub tolerance_minor: i64,
    pub rules_configured: usize,
    pub lines: Vec<LineResult>,
    /// Fee lines the provider did not tie to one of our payments: reported, never checked.
    pub unattributed_fee_minor: i64,
    /// Payment lines that are not in our records, so their method is unknown: not checked.
    pub unmatched_payment_lines: i64,
}

impl SettlementCheck {
    pub fn to_json(&self) -> Value {
        let ruled: Vec<&LineResult> = self
            .lines
            .iter()
            .filter(|l| l.expected_minor.is_some())
            .collect();
        let expected: i64 = ruled.iter().filter_map(|l| l.expected_minor).sum();
        let actual: i64 = ruled.iter().map(|l| l.input.actual_minor).sum();
        let no_rule = self
            .lines
            .iter()
            .filter(|l| l.status == LineStatus::NoRule)
            .count();
        let (over, under, n_over, n_under) = flagged_totals(&self.lines);
        let status = if self.currency != "INR" {
            "not_checked"
        } else if self.rules_configured == 0 {
            "no_rule"
        } else if n_over + n_under > 0 {
            "flagged"
        } else if ruled.is_empty() {
            "no_rule"
        } else {
            "ok"
        };
        let message = match status {
            "not_checked" => "Fee checks cover INR settlements only.".to_string(),
            "no_rule" => {
                "No expected fee configured for the payments in this settlement.".to_string()
            }
            _ => variance_sentence(&self.lines).unwrap_or_else(|| {
                "The fees deducted match your configured rates (within the tolerance).".to_string()
            }),
        };
        json!({
            "settlement_id": self.settlement_id,
            "status": status,
            "message": message,
            "tolerance_minor": self.tolerance_minor,
            "rules_configured": self.rules_configured,
            "checked_lines": ruled.len(),
            "no_rule_lines": no_rule,
            "expected_minor": expected,
            "actual_minor": actual,
            "variance_minor": actual - expected,
            "flagged": { "over_deducted_minor": over, "under_deducted_minor": under, "over_lines": n_over, "under_lines": n_under },
            "unattributed_fee_minor": self.unattributed_fee_minor,
            "unmatched_payment_lines": self.unmatched_payment_lines,
            "lines": self.lines.iter().map(|l| json!({
                "attempt_id": l.input.attempt_id,
                "provider": l.input.provider,
                "method": l.input.method,
                "status": l.status.as_str(),
                "payment_minor": l.input.payment_in_settlement_minor.unwrap_or(l.input.attempt_amount_minor),
                "refunded_minor": l.input.refunded_minor,
                "actual_minor": l.input.actual_minor,
                "expected_minor": l.expected_minor,
                "variance_minor": l.variance_minor,
                "rule": l.rule,
            })).collect::<Vec<_>>(),
        })
    }
}

pub(crate) async fn load_rules(conn: &mut PgConnection) -> Result<Vec<Rule>, sqlx::Error> {
    let rows: Vec<(String, String, String, i32, i64, Option<i64>, Option<i32>, bool, NaiveDate)> = sqlx::query_as(
        "SELECT id, provider, method, bps, flat_minor, cap_minor, gst_bps, refund_returns_fee, effective_from FROM fee_rules ORDER BY effective_from",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, provider, method, bps, flat, cap, gst, ret, from)| Rule {
                id,
                provider,
                method,
                bps: i64::from(bps),
                flat_minor: flat,
                cap_minor: cap,
                gst_bps: gst.map(i64::from),
                refund_returns_fee: ret,
                effective_from: from,
            },
        )
        .collect())
}

async fn load_tolerance(conn: &mut PgConnection) -> Result<(i64, Option<String>), sqlx::Error> {
    Ok(
        sqlx::query_as("SELECT tolerance_minor, owner FROM fee_check_settings")
            .fetch_optional(conn)
            .await?
            .unwrap_or((0, None)),
    )
}

/// Work out the check for one settlement from rows we already hold. Writes nothing.
pub async fn compute(
    conn: &mut PgConnection,
    settlement_id: &str,
) -> Result<SettlementCheck, ApiError> {
    let (provider, currency): (String, String) =
        sqlx::query_as("SELECT provider, currency FROM settlements WHERE id = $1")
            .bind(settlement_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| ApiError::not_found("settlement"))?;
    let rules = load_rules(conn).await?;
    let (tolerance, _) = load_tolerance(conn).await?;

    let items: Vec<(Option<String>, String, i64)> = sqlx::query_as(
        "SELECT attempt_id, kind, amount_minor FROM settlement_items WHERE settlement_id = $1 AND kind IN ('payment', 'refund', 'fee')",
    )
    .bind(settlement_id)
    .fetch_all(&mut *conn)
    .await?;
    #[derive(Default)]
    struct Acc {
        payment: Option<i64>,
        refunded: i64,
        fees: i64,
    }
    let mut by_attempt: BTreeMap<String, Acc> = BTreeMap::new();
    let (mut unattributed, mut unmatched) = (0i64, 0i64);
    for (attempt, kind, amount) in items {
        match (attempt, kind.as_str()) {
            (None, "fee") => unattributed += -amount,
            (None, "payment") => unmatched += 1,
            (None, _) => {}
            (Some(a), k) => {
                let acc = by_attempt.entry(a).or_default();
                match k {
                    "payment" => acc.payment = Some(acc.payment.unwrap_or(0) + amount),
                    "refund" => acc.refunded += -amount,
                    _ => acc.fees += -amount,
                }
            }
        }
    }
    let mut lines = Vec::new();
    if currency == "INR" {
        for (attempt_id, acc) in by_attempt {
            // a refund-only line is worth checking only when a fee came back with it or the rule says one should
            let row: Option<(String, String, i64, NaiveDate)> = sqlx::query_as(
                "SELECT provider, method, amount_minor, (created_at AT TIME ZONE 'Asia/Kolkata')::date FROM payment_attempts WHERE id = $1",
            )
            .bind(&attempt_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some((prov, method, amount, paid_on)) = row else {
                continue;
            };
            let input = LineInput {
                attempt_id,
                provider: prov,
                method,
                paid_on,
                attempt_amount_minor: amount,
                payment_in_settlement_minor: acc.payment,
                refunded_minor: acc.refunded,
                actual_minor: acc.fees,
            };
            let result = check_line(&input, &rules, tolerance);
            if acc.payment.is_none() && acc.fees == 0 && result.expected_minor.unwrap_or(0) == 0 {
                continue;
            }
            lines.push(result);
        }
    }
    Ok(SettlementCheck {
        settlement_id: settlement_id.to_string(),
        provider,
        currency,
        tolerance_minor: tolerance,
        rules_configured: rules.len(),
        lines,
        unattributed_fee_minor: unattributed,
        unmatched_payment_lines: unmatched,
    })
}

/// Check a settlement and record the result: lines, and one case per settlement when something is flagged.
/// Called when a settlement is ingested and when a person asks for a re-check. Does nothing for a business with
/// no fee rules at all. Never touches money or the ledger.
pub async fn run(
    conn: &mut PgConnection,
    tenant_id: &str,
    settlement_id: &str,
    actor: &str,
) -> Result<SettlementCheck, ApiError> {
    let check = compute(conn, settlement_id).await?;
    if check.currency != "INR" {
        return Ok(check);
    }
    let (_, owner) = load_tolerance(conn).await?;
    // with no rules at all there is nothing to record (and anything recorded earlier is now stale)
    let recorded: &[LineResult] = if check.rules_configured == 0 {
        &[]
    } else {
        &check.lines
    };
    let keep: Vec<&str> = recorded
        .iter()
        .map(|l| l.input.attempt_id.as_str())
        .collect();
    sqlx::query("DELETE FROM fee_check_lines WHERE settlement_id = $1 AND tenant_id = $2 AND attempt_id <> ALL($3)")
        .bind(settlement_id)
        .bind(tenant_id)
        .bind(&keep)
        .execute(&mut *conn)
        .await?;
    for l in recorded {
        sqlx::query(
            "INSERT INTO fee_check_lines (tenant_id, settlement_id, attempt_id, provider, method, status, payment_minor, refunded_minor, actual_minor, expected_minor, variance_minor, tolerance_minor, rule_id, rule, case_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, NULL)
             ON CONFLICT (tenant_id, settlement_id, attempt_id) DO UPDATE SET
               provider = EXCLUDED.provider, method = EXCLUDED.method, status = EXCLUDED.status, payment_minor = EXCLUDED.payment_minor,
               refunded_minor = EXCLUDED.refunded_minor, actual_minor = EXCLUDED.actual_minor, expected_minor = EXCLUDED.expected_minor,
               variance_minor = EXCLUDED.variance_minor, tolerance_minor = EXCLUDED.tolerance_minor, rule_id = EXCLUDED.rule_id,
               rule = EXCLUDED.rule, case_id = NULL, checked_at = now()",
        )
        .bind(tenant_id)
        .bind(settlement_id)
        .bind(&l.input.attempt_id)
        .bind(&l.input.provider)
        .bind(&l.input.method)
        .bind(l.status.as_str())
        .bind(l.input.payment_in_settlement_minor.unwrap_or(l.input.attempt_amount_minor))
        .bind(l.input.refunded_minor)
        .bind(l.input.actual_minor)
        .bind(l.expected_minor)
        .bind(l.variance_minor)
        .bind(check.tolerance_minor)
        .bind(&l.rule_id)
        .bind(&l.rule)
        .execute(&mut *conn)
        .await?;
    }
    let flagged: Vec<&LineResult> = recorded
        .iter()
        .filter(|l| l.status == LineStatus::Flagged)
        .collect();
    if flagged.is_empty() {
        // a rule or tolerance was corrected: an open finding that no longer holds is closed, with the reason written down
        let closed: Option<String> = sqlx::query_scalar(
            "UPDATE recon_cases SET status = 'resolved', resolution = 'Re-checked: no fee variance above the tolerance under the rules now in force.', resolved_at = now()
             WHERE kind = $1 AND subject_id = $2 AND status <> 'resolved' RETURNING id",
        )
        .bind(CASE_KIND)
        .bind(settlement_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some(id) = closed {
            tenant::audit(
                conn,
                tenant_id,
                actor,
                "fee_check.cleared",
                Some(&id),
                json!({ "settlement_id": settlement_id }),
            )
            .await?;
        }
        return Ok(check);
    }
    let expected: i64 = flagged.iter().filter_map(|l| l.expected_minor).sum();
    let actual: i64 = flagged.iter().map(|l| l.input.actual_minor).sum();
    let (over, under, n_over, n_under) = flagged_totals(&check.lines);
    let explanation = format!(
        "{} deducted {} in fees on {} payment(s) against {} from your configured rates: {}. Check it against your contract.",
        crate::timeline::provider_title(&check.provider),
        money(actual),
        flagged.len(),
        money(expected),
        match (n_over, n_under) {
            (_, 0) => format!("{} more than expected", money(over)),
            (0, _) => format!("{} less than expected", money(under)),
            _ => format!(
                "{} more and {} less than expected on different payments",
                money(over),
                money(under)
            ),
        }
    );
    let detail = json!({
        "settlement_id": settlement_id,
        "flagged_attempts": flagged.iter().map(|l| &l.input.attempt_id).collect::<Vec<_>>(),
        "over_deducted_minor": over, "under_deducted_minor": under,
    });
    let inserted = open_case(
        conn,
        tenant_id,
        CASE_KIND,
        "settlement",
        settlement_id,
        Some(expected),
        Some(actual),
        &explanation,
        detail.clone(),
    )
    .await?;
    if inserted {
        if let Some(o) = owner.as_deref().filter(|o| !o.is_empty()) {
            sqlx::query("UPDATE recon_cases SET assignee = $3, status = 'assigned' WHERE kind = $1 AND subject_id = $2")
                .bind(CASE_KIND)
                .bind(settlement_id)
                .bind(o)
                .execute(&mut *conn)
                .await?;
        }
    } else {
        // still open: keep it current. A resolved case is left alone: a person already reviewed this settlement.
        sqlx::query(
            "UPDATE recon_cases SET expected_minor = $3, actual_minor = $4, diff_minor = $4 - $3, explanation = $5, detail = $6
             WHERE kind = $1 AND subject_id = $2 AND status <> 'resolved'",
        )
        .bind(CASE_KIND)
        .bind(settlement_id)
        .bind(expected)
        .bind(actual)
        .bind(&explanation)
        .bind(&detail)
        .execute(&mut *conn)
        .await?;
    }
    let case_id: String =
        sqlx::query_scalar("SELECT id FROM recon_cases WHERE kind = $1 AND subject_id = $2")
            .bind(CASE_KIND)
            .bind(settlement_id)
            .fetch_one(&mut *conn)
            .await?;
    sqlx::query("UPDATE fee_check_lines SET case_id = $3 WHERE settlement_id = $1 AND status = 'flagged' AND tenant_id = $2")
        .bind(settlement_id)
        .bind(tenant_id)
        .bind(&case_id)
        .execute(&mut *conn)
        .await?;
    if inserted {
        tenant::audit(conn, tenant_id, actor, "fee_check.flagged", Some(&case_id), json!({ "settlement_id": settlement_id, "over_deducted_minor": over, "under_deducted_minor": under })).await?;
    }
    Ok(check)
}

// ---------- fee rules ----------

#[derive(Deserialize)]
pub struct NewRule {
    provider: String,
    method: String,
    /// Percentage in basis points (1.5% is 150). Take it from your own contract.
    bps: i64,
    flat_minor: Option<i64>,
    cap_minor: Option<i64>,
    /// GST charged on the fee, in basis points. Omit when the contract has none.
    gst_bps: Option<i64>,
    refund_returns_fee: Option<bool>,
    effective_from: NaiveDate,
    note: Option<String>,
}

fn rule_json(
    r: &(
        String,
        String,
        String,
        i32,
        i64,
        Option<i64>,
        Option<i32>,
        bool,
        NaiveDate,
        String,
        String,
    ),
) -> Value {
    json!({
        "id": r.0, "provider": r.1, "method": r.2, "bps": r.3, "flat_minor": r.4, "cap_minor": r.5,
        "gst_bps": r.6, "refund_returns_fee": r.7, "effective_from": r.8, "note": r.9, "created_by": r.10,
    })
}

const RULE_COLS: &str = "id, provider, method, bps, flat_minor, cap_minor, gst_bps, refund_returns_fee, effective_from, note, created_by";

pub async fn create_rule(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewRule>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("recon:write")?;
    let provider = b.provider.trim().to_lowercase();
    if provider.is_empty() {
        return Err(ApiError::invalid(
            "invalid_provider",
            "provider is required",
        ));
    }
    let method: Method = b
        .method
        .trim()
        .to_lowercase()
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_method", e))?;
    if !(0..=10_000).contains(&b.bps) {
        return Err(ApiError::invalid(
            "invalid_bps",
            "bps must be between 0 and 10000",
        ));
    }
    if b.flat_minor.is_some_and(|v| v < 0) || b.cap_minor.is_some_and(|v| v < 0) {
        return Err(ApiError::invalid(
            "invalid_amount",
            "flat_minor and cap_minor cannot be negative",
        ));
    }
    if b.gst_bps.is_some_and(|g| !(0..=10_000).contains(&g)) {
        return Err(ApiError::invalid(
            "invalid_gst_bps",
            "gst_bps must be between 0 and 10000",
        ));
    }
    let id = new_id("fr");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query(
        "INSERT INTO fee_rules (id, tenant_id, provider, method, bps, flat_minor, cap_minor, gst_bps, refund_returns_fee, effective_from, note, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(&provider)
    .bind(method.as_str())
    .bind(b.bps as i32)
    .bind(b.flat_minor.unwrap_or(0))
    .bind(b.cap_minor)
    .bind(b.gst_bps.map(|g| g as i32))
    .bind(b.refund_returns_fee.unwrap_or(false))
    .bind(b.effective_from)
    .bind(b.note.as_deref().unwrap_or(""))
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict(
            "duplicate_rule",
            "a rule for that provider and method already starts on that date; give the new rate a later effective_from",
        ),
        e => e.into(),
    })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "fee_rule.created",
        Some(&id),
        json!({ "provider": provider, "method": method.as_str(), "bps": b.bps, "flat_minor": b.flat_minor.unwrap_or(0), "cap_minor": b.cap_minor, "gst_bps": b.gst_bps, "effective_from": b.effective_from }),
    )
    .await?;
    let row: (
        String,
        String,
        String,
        i32,
        i64,
        Option<i64>,
        Option<i32>,
        bool,
        NaiveDate,
        String,
        String,
    ) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLS} FROM fee_rules WHERE id = $1"
    )))
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(rule_json(&row))))
}

pub async fn list_rules(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        String,
        String,
        String,
        i32,
        i64,
        Option<i64>,
        Option<i32>,
        bool,
        NaiveDate,
        String,
        String,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLS} FROM fee_rules ORDER BY provider, method, effective_from DESC"
    )))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({
        "data": rows.iter().map(rule_json).collect::<Vec<_>>(),
        "note": "Rates here are only what you entered from your own contract. Zyvor supplies none.",
    })))
}

/// Removing a rule does not rewrite findings already recorded: each keeps a copy of the rule it was checked against.
pub async fn delete_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i32, NaiveDate)> = sqlx::query_as(
        "DELETE FROM fee_rules WHERE id = $1 RETURNING provider, method, bps, effective_from",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (provider, method, bps, from) = row.ok_or_else(|| ApiError::not_found("fee rule"))?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "fee_rule.deleted",
        Some(&id),
        json!({ "provider": provider, "method": method, "bps": bps, "effective_from": from }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "deleted": true })))
}

// ---------- settings ----------

pub async fn get_settings(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (tolerance, owner) = load_tolerance(&mut tx).await?;
    Ok(Json(
        json!({ "tolerance_minor": tolerance, "owner": owner }),
    ))
}

#[derive(Deserialize)]
pub struct SettingsBody {
    /// Paise a payment's fee may differ from the rule before it is flagged (rounding).
    tolerance_minor: Option<i64>,
    /// Who new findings are assigned to. Empty string clears it.
    owner: Option<String>,
}

pub async fn set_settings(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    if b.tolerance_minor.is_some_and(|t| t < 0) {
        return Err(ApiError::invalid(
            "invalid_tolerance",
            "tolerance_minor cannot be negative",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (cur_tol, cur_owner) = load_tolerance(&mut tx).await?;
    let tol = b.tolerance_minor.unwrap_or(cur_tol);
    let owner = match b.owner {
        Some(o) if o.trim().is_empty() => None,
        Some(o) => Some(o.trim().to_string()),
        None => cur_owner,
    };
    sqlx::query(
        "INSERT INTO fee_check_settings (tenant_id, tolerance_minor, owner, updated_by) VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant_id) DO UPDATE SET tolerance_minor = $2, owner = $3, updated_by = $4, updated_at = now()",
    )
    .bind(&auth.tenant_id)
    .bind(tol)
    .bind(&owner)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "fee_check.settings",
        None,
        json!({ "tolerance_minor": tol, "owner": owner }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "tolerance_minor": tol, "owner": owner })))
}

// ---------- checks, findings, summary ----------

/// What the check says about one settlement, from the rows as they are now. Writes nothing.
pub async fn get_check(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(compute(&mut tx, &id).await?.to_json()))
}

/// Re-check one settlement and record the result (opens or updates the finding case).
pub async fn recheck(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let out = run(&mut tx, &auth.tenant_id, &id, &auth.actor)
        .await?
        .to_json();
    tx.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
pub struct RunBody {
    /// Only settlements settled on or after this date.
    since: Option<NaiveDate>,
    /// At most this many settlements, newest first (default 200, up to 500).
    limit: Option<i64>,
}

/// Check settlements that were imported before a rule existed (or after a rule changed).
pub async fn run_all(
    State(s): State<AppState>,
    auth: Auth,
    body: Option<Json<RunBody>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM settlements WHERE currency = 'INR' AND ($1::date IS NULL OR settled_on >= $1) ORDER BY settled_on DESC, id DESC LIMIT $2",
    )
    .bind(b.since)
    .bind(b.limit.unwrap_or(200).clamp(1, 500))
    .fetch_all(&mut *tx)
    .await?;
    let (mut flagged, mut ok, mut no_rule) = (0, 0, 0);
    for id in &ids {
        let c = run(&mut tx, &auth.tenant_id, id, &auth.actor).await?;
        let j = c.to_json();
        match j["status"].as_str() {
            Some("flagged") => flagged += 1,
            Some("ok") => ok += 1,
            _ => no_rule += 1,
        }
    }
    tx.commit().await?;
    Ok(Json(
        json!({ "settlements_checked": ids.len(), "flagged": flagged, "ok": ok, "no_rule": no_rule }),
    ))
}

#[derive(Deserialize)]
pub struct FindingsQuery {
    /// `open` (default): the case is not resolved. `reviewed`: resolved. `all`.
    status: Option<String>,
    provider: Option<String>,
    method: Option<String>,
    limit: Option<i64>,
}

const OPEN_FILTER: &str = "l.status = 'flagged' AND CASE $1::text WHEN 'all' THEN true WHEN 'reviewed' THEN c.status = 'resolved' ELSE (c.status IS NULL OR c.status <> 'resolved') END";

fn check_status(q: &Option<String>) -> Result<String, ApiError> {
    match q.as_deref().unwrap_or("open") {
        s @ ("open" | "reviewed" | "all") => Ok(s.to_string()),
        _ => Err(ApiError::invalid(
            "invalid_status",
            "status must be open, reviewed or all",
        )),
    }
}

pub async fn list_findings(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<FindingsQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let status = check_status(&q.status)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, i64, i64, i64, i64, i64, Option<Value>, Option<String>, Option<String>, Option<String>, NaiveDate)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT l.settlement_id, l.attempt_id, l.provider, l.method, l.payment_minor, l.actual_minor, l.expected_minor, l.variance_minor, l.tolerance_minor,
                    l.rule, l.case_id, c.status, c.assignee, st.settled_on
             FROM fee_check_lines l LEFT JOIN recon_cases c ON c.id = l.case_id JOIN settlements st ON st.id = l.settlement_id
             WHERE {OPEN_FILTER} AND ($2::text IS NULL OR l.provider = $2) AND ($3::text IS NULL OR l.method = $3)
             ORDER BY st.settled_on DESC, l.id DESC LIMIT $4"
        )))
        .bind(&status)
        .bind(q.provider.as_deref().map(str::to_lowercase))
        .bind(q.method.as_deref().map(str::to_lowercase))
        .bind(q.limit.unwrap_or(50).clamp(1, 200))
        .fetch_all(&mut *tx)
        .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(sid, att, prov, method, pay, actual, expected, variance, tol, rule, case, cstatus, owner, on)| {
            json!({
                "settlement_id": sid, "attempt_id": att, "provider": prov, "method": method, "settled_on": on,
                "payment_minor": pay, "actual_minor": actual, "expected_minor": expected, "variance_minor": variance,
                "direction": if variance > 0 { "over_deducted" } else { "under_deducted" },
                "tolerance_minor": tol, "rule": rule, "case_id": case, "case_status": cstatus, "owner": owner,
            })
        })
        .collect();
    Ok(Json(json!({
        "data": data,
        "note": "Findings ask a person to review. They never change a payment, a refund or the ledger.",
    })))
}

pub async fn summary(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rules: i64 = sqlx::query_scalar("SELECT count(*) FROM fee_rules")
        .fetch_one(&mut *tx)
        .await?;
    let rows: Vec<(String, String, i64, i64, i64, i64)> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT l.provider, l.method, count(*)::BIGINT,
                COALESCE(SUM(l.variance_minor) FILTER (WHERE l.variance_minor > 0), 0)::BIGINT,
                COALESCE(-SUM(l.variance_minor) FILTER (WHERE l.variance_minor < 0), 0)::BIGINT,
                count(DISTINCT l.settlement_id)::BIGINT
         FROM fee_check_lines l LEFT JOIN recon_cases c ON c.id = l.case_id
         WHERE {OPEN_FILTER} GROUP BY 1, 2 ORDER BY 1, 2"
        )))
        .bind("open")
        .fetch_all(&mut *tx)
        .await?;
    let no_rule_lines: i64 =
        sqlx::query_scalar("SELECT count(*) FROM fee_check_lines WHERE status = 'no_rule'")
            .fetch_one(&mut *tx)
            .await?;
    let open_cases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recon_cases WHERE kind = $1 AND status <> 'resolved'",
    )
    .bind(CASE_KIND)
    .fetch_one(&mut *tx)
    .await?;
    let (mut n, mut over, mut under) = (0, 0, 0);
    for r in &rows {
        n += r.2;
        over += r.3;
        under += r.4;
    }
    Ok(Json(json!({
        "rules_configured": rules,
        "open_findings": n,
        "open_cases": open_cases,
        "over_deducted_minor": over,
        "under_deducted_minor": under,
        "total_variance_minor": over - under,
        "total_absolute_variance_minor": over + under,
        "by_provider_method": rows.iter().map(|(p, m, n, o, u, settlements)| json!({
            "provider": p, "method": m, "open_findings": n, "settlements": settlements,
            "over_deducted_minor": o, "under_deducted_minor": u, "total_variance_minor": o - u,
        })).collect::<Vec<_>>(),
        "lines_without_a_rule": no_rule_lines,
        "message": if rules == 0 { "no expected fee configured" } else { "" },
        "note": "Over-deducted means the provider took more than your configured rate; under-deducted, less. Rates come from rules you entered; nothing is checked against a provider's own statement.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // Test rates are arbitrary round numbers made up for the tests; they are not any provider's or network's rate.
    fn rule(bps: i64, flat: i64) -> Rule {
        Rule {
            id: "fr_t".into(),
            provider: "p".into(),
            method: "m".into(),
            bps,
            flat_minor: flat,
            cap_minor: None,
            gst_bps: None,
            refund_returns_fee: false,
            effective_from: d(2026, 1, 1),
        }
    }

    fn input(payment: Option<i64>, refunded: i64, actual: i64) -> LineInput {
        LineInput {
            attempt_id: "a".into(),
            provider: "p".into(),
            method: "m".into(),
            paid_on: d(2026, 6, 1),
            attempt_amount_minor: payment.unwrap_or(10_000),
            payment_in_settlement_minor: payment,
            refunded_minor: refunded,
            actual_minor: actual,
        }
    }

    #[test]
    fn percentage_rounds_half_up() {
        // 150 bps of 333 paise = 4.995 -> 5
        assert_eq!(expected_fee(&rule(150, 0), 333).total_minor, 5);
        // 150 bps of 330 = 4.95 -> 5 ; of 300 = 4.5 -> 5 (half up) ; of 299 = 4.485 -> 4
        assert_eq!(expected_fee(&rule(150, 0), 330).total_minor, 5);
        assert_eq!(expected_fee(&rule(150, 0), 300).total_minor, 5);
        assert_eq!(expected_fee(&rule(150, 0), 299).total_minor, 4);
    }

    #[test]
    fn flat_is_added_and_zero_amount_costs_nothing() {
        let r = rule(100, 300);
        assert_eq!(expected_fee(&r, 10_000).total_minor, 100 + 300);
        assert_eq!(expected_fee(&r, 0).total_minor, 0);
    }

    #[test]
    fn cap_limits_the_fee_before_gst() {
        let mut r = rule(200, 0);
        r.cap_minor = Some(1_000);
        r.gst_bps = Some(1_000);
        let small = expected_fee(&r, 20_000); // 400, under the cap
        assert_eq!(
            (small.before_tax_minor, small.capped, small.gst_minor),
            (400, false, 40)
        );
        let big = expected_fee(&r, 1_000_000); // 20000 uncapped -> 1000
        assert_eq!(
            (
                big.before_tax_minor,
                big.capped,
                big.gst_minor,
                big.total_minor
            ),
            (1_000, true, 100, 1_100)
        );
    }

    #[test]
    fn gst_is_on_the_fee_and_rounds_half_up() {
        let mut r = rule(100, 0);
        r.gst_bps = Some(1_850); // made-up rate for the test
        // fee 1% of 10_050 = 100.5 -> 101 ; gst 18.5% of 101 = 18.685 -> 19
        let b = expected_fee(&r, 10_050);
        assert_eq!(
            (b.before_tax_minor, b.gst_minor, b.total_minor),
            (101, 19, 120)
        );
    }

    #[test]
    fn the_rule_in_force_is_the_latest_one_not_after_the_date() {
        let mut old = rule(100, 0);
        old.id = "old".into();
        old.effective_from = d(2026, 1, 1);
        let mut new = rule(200, 0);
        new.id = "new".into();
        new.effective_from = d(2026, 7, 1);
        let rules = vec![new, old];
        assert!(
            pick_rule(&rules, "p", "m", d(2025, 12, 31)).is_none(),
            "before any rule: none"
        );
        assert_eq!(
            pick_rule(&rules, "p", "m", d(2026, 6, 30)).unwrap().id,
            "old"
        );
        assert_eq!(
            pick_rule(&rules, "p", "m", d(2026, 7, 1)).unwrap().id,
            "new",
            "the start date is included"
        );
        assert!(pick_rule(&rules, "p", "other", d(2026, 8, 1)).is_none());
        assert!(pick_rule(&rules, "q", "m", d(2026, 8, 1)).is_none());
    }

    #[test]
    fn no_rule_is_reported_and_never_flagged() {
        let r = check_line(&input(Some(10_000), 0, 5_000), &[], 0);
        assert_eq!(r.status, LineStatus::NoRule);
        assert!(r.expected_minor.is_none() && r.variance_minor.is_none());
    }

    #[test]
    fn tolerance_decides_what_is_flagged_in_both_directions() {
        let rules = vec![rule(100, 0)]; // expected 100 on 10_000
        assert_eq!(
            check_line(&input(Some(10_000), 0, 100), &rules, 0).status,
            LineStatus::Ok
        );
        assert_eq!(
            check_line(&input(Some(10_000), 0, 101), &rules, 0).status,
            LineStatus::Flagged
        );
        assert_eq!(
            check_line(&input(Some(10_000), 0, 101), &rules, 1).status,
            LineStatus::Ok
        );
        let over = check_line(&input(Some(10_000), 0, 150), &rules, 1);
        assert_eq!(
            (over.status, over.variance_minor),
            (LineStatus::Flagged, Some(50))
        );
        let under = check_line(&input(Some(10_000), 0, 50), &rules, 1);
        assert_eq!(
            (under.status, under.variance_minor),
            (LineStatus::Flagged, Some(-50))
        );
    }

    #[test]
    fn a_zero_fee_rule_flags_any_deduction_and_no_rule_does_not() {
        let rules = vec![rule(0, 0)];
        assert_eq!(
            check_line(&input(Some(10_000), 0, 0), &rules, 0).status,
            LineStatus::Ok
        );
        assert_eq!(
            check_line(&input(Some(10_000), 0, 25), &rules, 0).status,
            LineStatus::Flagged
        );
        assert_eq!(
            check_line(&input(Some(10_000), 0, 25), &[], 0).status,
            LineStatus::NoRule
        );
    }

    #[test]
    fn a_refund_that_returns_the_fee_changes_the_expected_net() {
        let mut r = rule(100, 50);
        r.refund_returns_fee = true;
        let rules = vec![r];
        // payment 10_000: fee 150. Refund 4_000 leaves 6_000: fee 110, so 40 comes back, expected net 110.
        let partial = check_line(&input(Some(10_000), 4_000, 110), &rules, 0);
        assert_eq!(
            (partial.expected_minor, partial.status),
            (Some(110), LineStatus::Ok)
        );
        // keeping the whole fee is then a variance of +40
        assert_eq!(
            check_line(&input(Some(10_000), 4_000, 150), &rules, 0).variance_minor,
            Some(40)
        );
        // a full refund returns everything, flat included
        let full = check_line(&input(Some(10_000), 10_000, 0), &rules, 0);
        assert_eq!(
            (full.expected_minor, full.status),
            (Some(0), LineStatus::Ok)
        );
        // refund settled later, on its own: the fee comes back as a negative net deduction
        let later = check_line(&input(None, 4_000, -40), &rules, 0);
        assert_eq!(
            (later.expected_minor, later.status),
            (Some(-40), LineStatus::Ok)
        );
    }

    #[test]
    fn a_refund_that_does_not_return_the_fee_keeps_the_original_fee_expected() {
        let rules = vec![rule(100, 50)];
        // fee stays 150; a provider that gives 40 back is deducting less than the rule says
        assert_eq!(
            check_line(&input(Some(10_000), 4_000, 150), &rules, 0).status,
            LineStatus::Ok
        );
        let r = check_line(&input(Some(10_000), 4_000, 110), &rules, 0);
        assert_eq!(
            (r.status, r.variance_minor),
            (LineStatus::Flagged, Some(-40))
        );
        // a refund-only line with no fee on it has nothing to compare
        let none = check_line(&input(None, 4_000, 0), &rules, 0);
        assert_eq!(
            (none.expected_minor, none.status),
            (Some(0), LineStatus::Ok)
        );
    }

    #[test]
    fn the_sentence_names_the_amount_and_both_directions() {
        let rules = vec![rule(100, 0)];
        let a = check_line(&input(Some(10_000), 0, 350), &rules, 0);
        let s = variance_sentence(std::slice::from_ref(&a)).unwrap();
        assert!(
            s.starts_with("Fee differs from your configured rate by ₹2.50"),
            "{s}"
        );
        assert!(s.contains("more than expected"));
        let b = check_line(&input(Some(10_000), 0, 0), &rules, 0);
        let mixed = variance_sentence(&[a, b]).unwrap();
        assert!(mixed.contains("₹3.50") && mixed.contains("less"), "{mixed}");
        let ok = check_line(&input(Some(10_000), 0, 100), &rules, 0);
        assert!(variance_sentence(&[ok]).is_none());
    }
}
