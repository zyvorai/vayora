//! A small, explicit risk screen plus a deterministic score.
//! Hard limits: ceiling on one payment, cap on tries, failed-try velocity, blocked VPAs.
//! Score (0–100): velocity / amount / VPA-handle heuristics — not card-network fraud intelligence.
//! Every refusal is audited and raises `payment.risk_blocked`.

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
    if r.score_block_below
        .is_some_and(|n| !(0..=100).contains(&n))
    {
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
        assert!(score_blocked(
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
        .is_some());
    }
}
