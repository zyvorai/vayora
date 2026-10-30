//! Coupons and convenience fees for payment links. A coupon takes a percentage or a flat amount off a
//! fixed-amount link; a link may also add a convenience fee the payer covers. The price is worked out
//! the same way for the preview and for the payment, and a coupon is spent in the same transaction
//! that creates the payer's payment, so a capped coupon can never be over-used.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Debug, PartialEq, Eq)]
pub struct Price {
    pub base: i64,
    pub discount: i64,
    pub fee: i64,
    pub total: i64,
}

fn half_up(n: i64, bp: i64) -> i64 {
    ((n as i128 * bp as i128 + 5_000) / 10_000) as i64
}

/// Discount first (never below one minor unit), then the fee on what is left.
pub fn price(base: i64, coupon: Option<(&str, i64)>, fee_bp: i64) -> Price {
    let raw = match coupon {
        Some(("percent", bp)) => half_up(base, bp),
        Some((_, flat)) => flat,
        None => 0,
    };
    let discount = raw.clamp(0, base - 1);
    let after = base - discount;
    let fee = half_up(after, fee_bp);
    Price {
        base,
        discount,
        fee,
        total: after + fee,
    }
}

pub fn normalise(code: &str) -> String {
    code.trim().to_uppercase()
}

pub struct Redeemable {
    pub id: String,
    pub kind: String,
    pub value: i64,
}

/// Look up a coupon a payer typed. `spend` also takes one use (the caller's transaction commits it).
pub async fn find(
    conn: &mut PgConnection,
    code: &str,
    spend: bool,
) -> Result<Redeemable, ApiError> {
    let row: Option<(String, String, i64, Option<i32>, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = sqlx::query_as(if spend {
        "SELECT id, kind, value, max_uses, uses, expires_at, deactivated_at FROM coupons WHERE code = $1 FOR UPDATE"
    } else {
        "SELECT id, kind, value, max_uses, uses, expires_at, deactivated_at FROM coupons WHERE code = $1"
    })
    .bind(normalise(code))
    .fetch_optional(&mut *conn)
    .await?;
    // unknown, expired, used up and deactivated all look the same to a payer
    let bad = || ApiError::conflict("coupon_invalid", "this coupon is not valid");
    let (id, kind, value, max, uses, exp, off) = row.ok_or_else(bad)?;
    if off.is_some() || exp.is_some_and(|e| e <= Utc::now()) || max.is_some_and(|m| uses >= m) {
        return Err(bad());
    }
    if spend {
        sqlx::query("UPDATE coupons SET uses = uses + 1 WHERE id = $1")
            .bind(&id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(Redeemable { id, kind, value })
}

#[derive(Deserialize)]
pub struct NewCoupon {
    code: String,
    /// `percent` or `flat`.
    kind: String,
    /// Percent: 1-100 (whole or to two decimals as basis points via `value_bp`). Flat: minor units.
    value: i64,
    max_uses: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
}

pub async fn create_coupon(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewCoupon>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payments:write")?;
    let code = normalise(&b.code);
    if !(3..=24).contains(&code.len())
        || !code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::invalid(
            "invalid_coupon",
            "code must be 3-24 letters, digits, - or _",
        ));
    }
    let stored = match b.kind.as_str() {
        // percent is given in basis points: 1000 = 10%
        "percent" if (1..=10_000).contains(&b.value) => b.value,
        "flat" if b.value > 0 && b.value < 1_000_000_000_000 => b.value,
        _ => {
            return Err(ApiError::invalid(
                "invalid_coupon",
                "kind is percent (value in basis points, 1-10000) or flat (value in minor units)",
            ));
        }
    };
    if b.max_uses.is_some_and(|m| m <= 0) || b.expires_at.is_some_and(|e| e <= Utc::now()) {
        return Err(ApiError::invalid(
            "invalid_coupon",
            "max_uses must be positive and expires_at in the future",
        ));
    }
    let id = new_id("cpn");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO coupons (id, tenant_id, code, kind, value, max_uses, expires_at, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&id).bind(&auth.tenant_id).bind(&code).bind(&b.kind).bind(stored).bind(b.max_uses).bind(b.expires_at).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("coupon_exists", "that code already exists"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "coupon.created",
        Some(&id),
        json!({ "code": code, "kind": b.kind }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "code": code }))))
}

pub async fn list_coupons(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, i64, Option<i32>, i32, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT id, code, kind, value, max_uses, uses, expires_at, deactivated_at FROM coupons ORDER BY created_at DESC LIMIT 200").fetch_all(&mut *tx).await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, code, kind, value, max, uses, exp, off)| {
        let status = if off.is_some() { "deactivated" } else if exp.is_some_and(|e| e <= Utc::now()) { "expired" } else if max.is_some_and(|m| uses >= m) { "used_up" } else { "active" };
        json!({ "id": id, "code": code, "kind": kind, "value": value, "max_uses": max, "uses": uses, "expires_at": exp, "status": status })
    }).collect();
    Ok(Json(json!({ "data": data })))
}

pub async fn deactivate_coupon(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE coupons SET deactivated_at = now() WHERE id = $1 AND deactivated_at IS NULL",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("active coupon"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "coupon.deactivated",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "deactivated" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discount_then_fee_never_below_one_unit() {
        assert_eq!(
            price(100_000, None, 0),
            Price {
                base: 100_000,
                discount: 0,
                fee: 0,
                total: 100_000
            }
        );
        assert_eq!(price(100_000, Some(("percent", 1_000)), 0).total, 90_000);
        assert_eq!(price(100_000, Some(("flat", 25_000)), 0).total, 75_000);
        // fee on the discounted amount: 90,000 at 2% = 1,800
        assert_eq!(
            price(100_000, Some(("percent", 1_000)), 200),
            Price {
                base: 100_000,
                discount: 10_000,
                fee: 1_800,
                total: 91_800
            }
        );
        // a flat coupon bigger than the price cannot make it free or negative
        assert_eq!(price(5_000, Some(("flat", 9_999)), 0).total, 1);
        assert_eq!(price(5_000, Some(("percent", 10_000)), 0).total, 1);
        assert_eq!(price(33_333, None, 175).fee, 583); // 583.33 -> 583
    }
}
