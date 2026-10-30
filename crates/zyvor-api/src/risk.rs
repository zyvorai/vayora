//! A small, explicit risk screen. It does not score anything and it does not know about card
//! networks: it enforces the limits a merchant chooses (a ceiling on one payment, a cap on tries per
//! payment, a cap on recent failures per customer, VPAs and VPA handles to refuse). Every refusal is
//! audited and raises `payment.risk_blocked`, so a merchant sees exactly what the screen did.

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
    Ok(())
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
}
