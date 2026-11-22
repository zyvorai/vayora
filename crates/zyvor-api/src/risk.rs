// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! A small, explicit risk screen plus a deterministic score.
//! Hard limits: ceiling on one payment, cap on tries, failed-try velocity, blocked VPAs.
//! Score (0–100): velocity / amount / VPA-handle heuristics — not card-network fraud intelligence.
//! Every refusal is audited and raises `payment.risk_blocked`.
//!
//! The rules engine (velocity, amount, allow/block lists, review queue) is in `risk_engine.rs` and runs after
//! these hard limits; this file also holds the order-risk score for cash-on-delivery orders (`order_score`).

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;

use crate::{ApiError, AppState, tenant};

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
pub struct Rules {
    /// Refuse a single payment above this amount.
    pub max_intent_amount_minor: Option<i64>,
    /// Refuse another attempt once a payment has had this many.
    pub max_attempts_per_intent: Option<i64>,
    /// Refuse attempts for a customer who has had this many failed attempts in the last hour.
    pub max_failed_per_customer_per_hour: Option<i64>,
    /// Exact VPAs (`name@bank`) or whole handles (`@bank`) to refuse.
    #[serde(default)]
    pub blocked_vpas: Vec<String>,
    /// Optional floor: scores strictly below this are blocked (0–100). None = scoring advisory only.
    #[serde(default)]
    pub score_block_below: Option<i64>,
}

pub fn validate(r: &Rules) -> Result<(), ApiError> {
    let pos = |v: Option<i64>| v.is_none_or(|n| n > 0 && n < 1_000_000_000_000);
    if !pos(r.max_intent_amount_minor)
        || !pos(r.max_attempts_per_intent)
        || !pos(r.max_failed_per_customer_per_hour)
    {
        return Err(ApiError::invalid(
            "invalid_risk_rules",
            "limits must be positive whole numbers",
        ));
    }
    if r.blocked_vpas.len() > 200
        || r.blocked_vpas.iter().any(|v| {
            v.len() > 80
                || !v.contains('@')
                || v.matches('@').count() != 1
                || v.contains(char::is_whitespace)
        })
    {
        return Err(ApiError::invalid(
            "invalid_risk_rules",
            "blocked_vpas: up to 200 entries like name@bank or @bank",
        ));
    }
    if r.score_block_below.is_some_and(|n| !(0..=100).contains(&n)) {
        return Err(ApiError::invalid(
            "invalid_risk_rules",
            "score_block_below must be 0-100",
        ));
    }
    Ok(())
}

/// Inputs for the deterministic score (higher = safer).
#[derive(Debug, Clone, Default)]
pub struct ScoreInput {
    pub amount_minor: i64,
    pub failed_last_hour: i64,
    pub attempts_on_intent: i64,
    pub vpa: Option<String>,
}

/// Score 0–100. Deducts for high amount, failure velocity, attempt thrash, and risky VPA handles.
pub fn score(input: &ScoreInput) -> i64 {
    let mut s: i64 = 100;
    if input.amount_minor >= 100_000_00 {
        s -= 25;
    } else if input.amount_minor >= 25_000_00 {
        s -= 10;
    }
    s -= (input.failed_last_hour * 15).min(45);
    s -= (input.attempts_on_intent.saturating_sub(1) * 8).min(24);
    if let Some(v) = &input.vpa {
        let v = v.to_lowercase();
        if v.ends_with("@okaxis") || v.ends_with("@ybl") {
            // common handles — no penalty
        } else if v.contains("test") || v.contains("fail") {
            s -= 40;
        }
    }
    s.clamp(0, 100)
}

pub fn score_blocked(rules: &Rules, input: &ScoreInput) -> Option<(i64, i64)> {
    let sc = score(input);
    rules
        .score_block_below
        .filter(|&floor| sc < floor)
        .map(|floor| (sc, floor))
}

pub async fn load(conn: &mut PgConnection) -> Result<Rules, sqlx::Error> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT risk_rules FROM tenant_settings LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn vpa_blocked(rules: &Rules, vpa: &str) -> bool {
    let v = vpa.trim().to_lowercase();
    rules.blocked_vpas.iter().any(|b| {
        let b = b.to_lowercase();
        b == v || (b.starts_with('@') && v.ends_with(&b))
    })
}

/// Record a refusal (in its own transaction, since the caller's is being abandoned) and build the error.
pub async fn blocked(
    s: &AppState,
    tenant_id: &str,
    actor: &str,
    rule: &str,
    detail: serde_json::Value,
) -> ApiError {
    if let Ok(mut tx) = tenant::begin(&s.pool, tenant_id).await {
        let _ = tenant::audit(
            &mut tx,
            tenant_id,
            actor,
            "risk.blocked",
            None,
            json!({ "rule": rule, "detail": detail }),
        )
        .await;
        let _ = tenant::emit(
            &mut tx,
            tenant_id,
            "payment.risk_blocked",
            json!({ "rule": rule, "detail": detail }),
        )
        .await;
        let _ = tx.commit().await;
    }
    ApiError::new(
        StatusCode::FORBIDDEN,
        "risk_blocked",
        format!("this payment was stopped by your risk rules ({rule})"),
    )
}

/// What a rule decides. Severity runs allow < review < decline; when several rules fire the highest wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Allow,
    Review,
    Decline,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Review => "review",
            Action::Decline => "decline",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "allow" => Action::Allow,
            "review" => Action::Review,
            "decline" => Action::Decline,
            _ => return None,
        })
    }
}

// ---- order risk score (cash on delivery) ----------------------------------------------------------------------

/// What the merchant tells us about an order. Nothing here is looked up or inferred about the buyer.
#[derive(Debug, Clone, Default)]
pub struct OrderInput {
    /// Six digits, first not zero.
    pub pin_code: String,
    pub order_value_minor: i64,
    /// The buyer has ordered from this merchant before and received it.
    pub repeat_customer: bool,
    /// How many earlier orders this buyer refused at the door or returned undelivered.
    pub prior_refusals: u32,
    /// The merchant's own tier for this area, from `risk_pin_tiers`, when they have one.
    pub pin_tier: Option<PinTier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinTier {
    Low,
    Medium,
    High,
}

impl PinTier {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "low" => PinTier::Low,
            "medium" => PinTier::Medium,
            "high" => PinTier::High,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Contribution {
    pub factor: &'static str,
    pub points: i64,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderScore {
    /// 0 (least risky) to 100 (most risky). Note the direction: it is the opposite of the payment score above.
    pub score: i64,
    pub band: &'static str,
    pub action: Action,
    pub contributions: Vec<Contribution>,
    pub review_at: i64,
    pub decline_at: i64,
    pub formula: &'static str,
}

pub const ORDER_FORMULA: &str = "order-risk-v1";
pub const DEFAULT_REVIEW_AT: i64 = 40;
pub const DEFAULT_DECLINE_AT: i64 = 75;

pub fn valid_pin_code(p: &str) -> bool {
    p.len() == 6 && p.chars().all(|c| c.is_ascii_digit()) && !p.starts_with('0')
}

/// The score, as a sum of listed contributions, clamped to 0-100:
///
/// - order value (₹ 500 = 50,000 minor units): under ₹500 +0, under ₹2,000 +10, under ₹5,000 +20, otherwise +30;
/// - prior refusals by this buyer: none +0, one +25, two +40, three or more +55;
/// - repeat customer with no refusals: -15 (a buyer who has received orders before; no credit when they have refused any);
/// - area, from the merchant's own tier for the longest matching pin-code prefix: high +20, medium +10, low +0,
///   and +5 when the merchant has no tier for it ("unknown area", not "risky area").
///
/// `review_at` (default 40) and `decline_at` (default 75) turn the score into an action: below `review_at` allow,
/// from `review_at` review, from `decline_at` decline. The bands are 0-29 low, 30-59 medium, 60 and over high.
/// Fixed arithmetic on four inputs, no learning, no hidden features; the weights are judgement, not measured, and
/// the merchant should tune the thresholds to their own returns.
pub fn order_score(i: &OrderInput, review_at: i64, decline_at: i64) -> OrderScore {
    let mut c: Vec<Contribution> = Vec::new();
    let mut add = |factor: &'static str, points: i64, why: String| {
        c.push(Contribution {
            factor,
            points,
            why,
        })
    };
    let v = i.order_value_minor;
    let (p, why) = if v < 50_000 {
        (0, "order under ₹500")
    } else if v < 200_000 {
        (10, "order of ₹500 to ₹1,999")
    } else if v < 500_000 {
        (20, "order of ₹2,000 to ₹4,999")
    } else {
        (30, "order of ₹5,000 or more")
    };
    add("order_value", p, why.to_string());
    let (p, why) = match i.prior_refusals {
        0 => (0, "no earlier refusals".to_string()),
        1 => (25, "one earlier refused or undelivered order".to_string()),
        2 => (40, "two earlier refused or undelivered orders".to_string()),
        n => (55, format!("{n} earlier refused or undelivered orders")),
    };
    add("prior_refusals", p, why);
    if i.repeat_customer && i.prior_refusals == 0 {
        add(
            "repeat_customer",
            -15,
            "has received orders before and never refused one".to_string(),
        );
    } else if i.repeat_customer {
        add(
            "repeat_customer",
            0,
            "repeat customer, but no credit because of earlier refusals".to_string(),
        );
    } else {
        add(
            "repeat_customer",
            0,
            "no earlier orders on record".to_string(),
        );
    }
    let (p, why) = match i.pin_tier {
        Some(PinTier::High) => (20, "area marked high risk by the merchant"),
        Some(PinTier::Medium) => (10, "area marked medium risk by the merchant"),
        Some(PinTier::Low) => (0, "area marked low risk by the merchant"),
        None => (
            5,
            "no tier on file for this area (unknown, not necessarily risky)",
        ),
    };
    add("area", p, why.to_string());
    let score = c.iter().map(|x| x.points).sum::<i64>().clamp(0, 100);
    let band = if score < 30 {
        "low"
    } else if score < 60 {
        "medium"
    } else {
        "high"
    };
    let action = if score >= decline_at {
        Action::Decline
    } else if score >= review_at {
        Action::Review
    } else {
        Action::Allow
    };
    OrderScore {
        score,
        band,
        action,
        contributions: c,
        review_at,
        decline_at,
        formula: ORDER_FORMULA,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpa_matching_handles_exact_and_whole_handles() {
        let r = Rules {
            blocked_vpas: vec!["bad@oksbi".into(), "@scamnet".into()],
            ..Default::default()
        };
        assert!(vpa_blocked(&r, "BAD@oksbi") && vpa_blocked(&r, "anyone@scamnet"));
        assert!(!vpa_blocked(&r, "good@oksbi") && !vpa_blocked(&r, "x@notscamnet2"));
    }

    #[test]
    fn rules_are_validated() {
        assert!(validate(&Rules::default()).is_ok());
        assert!(
            validate(&Rules {
                max_intent_amount_minor: Some(0),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&Rules {
                blocked_vpas: vec!["nohandle".into()],
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            validate(&Rules {
                blocked_vpas: vec!["@ok".into()],
                max_attempts_per_intent: Some(3),
                ..Default::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn score_drops_on_failures_and_test_vpa() {
        let high = score(&ScoreInput {
            amount_minor: 100,
            failed_last_hour: 0,
            attempts_on_intent: 1,
            vpa: Some("a@oksbi".into()),
        });
        let low = score(&ScoreInput {
            amount_minor: 50_000_00,
            failed_last_hour: 3,
            attempts_on_intent: 4,
            vpa: Some("fail@test".into()),
        });
        assert!(high > low);
        assert!(
            score_blocked(
                &Rules {
                    score_block_below: Some(50),
                    ..Default::default()
                },
                &ScoreInput {
                    amount_minor: 50_000_00,
                    failed_last_hour: 3,
                    attempts_on_intent: 4,
                    vpa: Some("fail@test".into()),
                }
            )
            .is_some()
        );
    }

    fn order(value: i64, refusals: u32, repeat: bool, tier: Option<PinTier>) -> OrderInput {
        OrderInput {
            pin_code: "560001".into(),
            order_value_minor: value,
            repeat_customer: repeat,
            prior_refusals: refusals,
            pin_tier: tier,
        }
    }

    #[test]
    fn order_score_is_the_sum_of_its_listed_parts() {
        // small order, new buyer, unknown area: 0 + 0 + 0 + 5
        let s = order_score(&order(30_000, 0, false, None), 40, 75);
        assert_eq!(s.score, 5);
        assert_eq!(s.action, Action::Allow);
        assert_eq!(s.contributions.iter().map(|c| c.points).sum::<i64>(), 5);
        // large order, one refusal, high-risk area: 30 + 25 + 0 + 20
        let s = order_score(&order(600_000, 1, false, Some(PinTier::High)), 40, 75);
        assert_eq!(s.score, 75);
        assert_eq!(s.action, Action::Decline);
        assert_eq!(s.band, "high");
        // a repeat buyer with a clean record is credited, never below zero
        let s = order_score(&order(10_000, 0, true, Some(PinTier::Low)), 40, 75);
        assert_eq!(s.score, 0);
        let s = order_score(&order(300_000, 0, true, Some(PinTier::Medium)), 40, 75);
        assert_eq!(s.score, 20 - 15 + 10);
    }

    #[test]
    fn a_repeat_buyer_who_refused_gets_no_credit() {
        let with = order_score(&order(100_000, 2, true, None), 40, 75);
        let without = order_score(&order(100_000, 2, false, None), 40, 75);
        assert_eq!(with.score, without.score);
        assert_eq!(with.score, 10 + 40 + 5);
        assert_eq!(with.action, Action::Review);
    }

    #[test]
    fn thresholds_decide_the_action_and_the_score_is_capped() {
        let i = order(600_000, 5, false, Some(PinTier::High));
        let s = order_score(&i, 40, 75);
        assert_eq!(s.score, 100);
        assert_eq!(order_score(&i, 40, 101).action, Action::Review);
        assert_eq!(order_score(&i, 101, 101).action, Action::Allow);
        assert_eq!(
            order_score(&order(100_000, 1, false, None), 40, 75).action,
            Action::Review
        );
    }

    #[test]
    fn pin_codes_are_six_digits_not_starting_with_zero() {
        assert!(valid_pin_code("560001") && valid_pin_code("110001"));
        assert!(!valid_pin_code("060001") && !valid_pin_code("56001") && !valid_pin_code("56000a"));
    }
}
