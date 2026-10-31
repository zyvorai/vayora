//! Exchange rates for reporting. You enter the rate you use (Zyvor does not fetch market rates), one
//! per currency per day, and it is never edited afterwards. Rates convert foreign-currency amounts
//! to INR in reports such as ageing; the ledger itself stays per currency and is not revalued.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};

use crate::{ApiError, AppState, auth::Auth, tenant};

/// "83.2150" -> 83_215_000 (millionths). Up to 6 decimals, positive.
pub fn parse_rate(s: &str) -> Option<i64> {
    let (whole, frac) = s.trim().split_once('.').unwrap_or((s.trim(), ""));
    if whole.is_empty()
        || frac.len() > 6
        || !whole
            .chars()
            .chain(frac.chars())
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let micro = whole.parse::<i64>().ok()?.checked_mul(1_000_000)?
        + format!("{frac:0<6}").parse::<i64>().ok()?;
    (micro > 0).then_some(micro)
}

/// USD minor units -> INR minor units, rounded half up.
pub fn usd_to_inr(minor: i64, rate_micro: i64) -> i64 {
    ((minor as i128 * rate_micro as i128 + 500_000) / 1_000_000) as i64
}

pub fn inr_to_usd(minor: i64, rate_micro: i64) -> i64 {
    ((minor as i128 * 1_000_000 + rate_micro as i128 / 2) / rate_micro as i128) as i64
}

/// The latest rate on or before `as_of`, with the date it was set.
pub async fn rate_on(
    conn: &mut PgConnection,
    currency: &str,
    as_of: NaiveDate,
) -> Result<Option<(i64, NaiveDate)>, sqlx::Error> {
    sqlx::query_as("SELECT rate_micro, as_of FROM fx_rates WHERE currency = $1 AND as_of <= $2 ORDER BY as_of DESC LIMIT 1")
        .bind(currency)
        .bind(as_of)
        .fetch_optional(conn)
        .await
}

#[derive(Deserialize)]
pub struct NewRate {
    currency: String,
    /// INR per 1 unit of the currency, as a decimal string such as "83.2150".
    rate: String,
    /// Defaults to today (Indian time).
    as_of: Option<NaiveDate>,
    note: Option<String>,
}

pub async fn set_rate(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewRate>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let ccy: Currency = b
        .currency
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if ccy == Currency::Inr {
        return Err(ApiError::invalid(
            "invalid_currency",
            "rates are quoted against INR; give a foreign currency",
        ));
    }
    let micro = parse_rate(&b.rate)
        .filter(|m| (1_000..=100_000_000_000).contains(m))
        .ok_or_else(|| {
            ApiError::invalid(
                "invalid_rate",
                "rate must be a positive decimal with at most 6 decimals, such as 83.2150",
            )
        })?;
    let today = crate::ist_today();
    let as_of = b.as_of.unwrap_or(today);
    if as_of > today {
        return Err(ApiError::invalid(
            "invalid_date",
            "a rate cannot be dated in the future",
        ));
    }
    let id = new_id("fxr");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO fx_rates (id, tenant_id, currency, rate_micro, as_of, note, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(&id).bind(&auth.tenant_id).bind(ccy.code()).bind(micro).bind(as_of).bind(&b.note).bind(&auth.actor)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("rate_exists", "a rate for that currency and date is already recorded; rates are never edited, enter the next day's"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "fx_rate.set",
        Some(&id),
        json!({ "currency": ccy.code(), "rate_micro": micro, "as_of": as_of }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "currency": ccy.code(), "rate": b.rate.trim(), "as_of": as_of })),
    ))
}

pub async fn list_rates(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, NaiveDate, Option<String>, String)> = sqlx::query_as("SELECT id, currency, rate_micro, as_of, note, created_by FROM fx_rates ORDER BY as_of DESC, currency LIMIT 200").fetch_all(&mut *tx).await?;
    let data: Vec<Value> = rows.into_iter().map(|(id, c, m, d, note, by)| json!({ "id": id, "currency": c, "rate": format!("{}.{:06}", m / 1_000_000, m % 1_000_000), "as_of": d, "note": note, "set_by": by })).collect();
    Ok(Json(json!({ "data": data })))
}

#[derive(Deserialize)]
pub struct ConvertQuery {
    amount_minor: i64,
    from: String,
    to: String,
    as_of: Option<NaiveDate>,
}

/// `GET /v1/fx/convert?amount_minor=10000&from=USD&to=INR`
pub async fn convert(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ConvertQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let (from, to): (Currency, Currency) = (
        q.from
            .parse()
            .map_err(|e: String| ApiError::invalid("invalid_currency", e))?,
        q.to.parse()
            .map_err(|e: String| ApiError::invalid("invalid_currency", e))?,
    );
    if q.amount_minor < 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must not be negative",
        ));
    }
    if from == to {
        return Ok(Json(
            json!({ "amount_minor": q.amount_minor, "currency": to.code(), "rate": null }),
        ));
    }
    let foreign = if from == Currency::Inr { to } else { from };
    let today = crate::ist_today();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (micro, on) = rate_on(&mut tx, foreign.code(), q.as_of.unwrap_or(today))
        .await?
        .ok_or_else(|| {
            ApiError::conflict(
                "no_rate",
                format!("no {} rate has been recorded for that date", foreign.code()),
            )
        })?;
    let out = if from == Currency::Inr {
        inr_to_usd(q.amount_minor, micro)
    } else {
        usd_to_inr(q.amount_minor, micro)
    };
    Ok(Json(
        json!({ "amount_minor": out, "currency": to.code(), "rate": format!("{}.{:06}", micro / 1_000_000, micro % 1_000_000), "rate_date": on }),
    ))
}

#[derive(Deserialize)]
pub struct ExposureQuery {
    as_of: Option<NaiveDate>,
}

/// `GET /v1/fx/exposure?as_of=2026-03-31`: open USD invoices valued at the rate on the day each was raised and at
/// the rate on `as_of`, with the difference. Reporting only: nothing is posted and the ledger is not revalued.
/// A figure is null (never a guess) when no rate covers that day.
pub async fn exposure(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ExposureQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let as_of = q.as_of.unwrap_or_else(crate::ist_today);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, NaiveDate)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, i.number, {out} AS outstanding, (i.created_at AT TIME ZONE 'Asia/Kolkata')::date
         FROM invoices i
         WHERE i.currency = 'USD' AND i.status IN ('open', 'partially_paid')
           AND (i.created_at AT TIME ZONE 'Asia/Kolkata')::date <= $1
         ORDER BY i.number LIMIT 500",
        out = crate::collections::OUTSTANDING
    )))
    .bind(as_of)
    .fetch_all(&mut *tx)
    .await?;
    let now = rate_on(&mut tx, "USD", as_of).await?;
    let mut data = Vec::new();
    let (mut usd, mut booked, mut current, mut complete) = (0i64, 0i64, 0i64, now.is_some());
    for (id, number, outstanding, raised) in rows {
        if outstanding <= 0 {
            continue;
        }
        let then = rate_on(&mut tx, "USD", raised).await?;
        let booked_inr = then.map(|(m, _)| usd_to_inr(outstanding, m));
        let current_inr = now.map(|(m, _)| usd_to_inr(outstanding, m));
        let diff = booked_inr.zip(current_inr).map(|(b, c)| c - b);
        usd += outstanding;
        match (booked_inr, current_inr) {
            (Some(b), Some(c)) => {
                booked += b;
                current += c;
            }
            _ => complete = false,
        }
        data.push(json!({ "invoice_id": id, "number": number, "outstanding_usd_minor": outstanding, "raised_on": raised,
            "booked_inr_minor": booked_inr, "current_inr_minor": current_inr, "unrealised_inr_minor": diff }));
    }
    Ok(Json(json!({
        "as_of": as_of, "rate_date": now.map(|n| n.1), "outstanding_usd_minor": usd, "complete": complete,
        "booked_inr_minor": complete.then_some(booked), "current_inr_minor": complete.then_some(current),
        "unrealised_inr_minor": complete.then_some(current - booked),
        "note": "reporting only: nothing is posted to the ledger", "data": data
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_parse_and_convert_with_half_up_rounding() {
        assert_eq!(parse_rate("83.215"), Some(83_215_000));
        assert_eq!(parse_rate("83"), Some(83_000_000));
        assert_eq!(parse_rate("0.000001"), Some(1));
        for bad in ["", ".5", "83.1234567", "-1", "1e3", "abc", "0"] {
            assert_eq!(parse_rate(bad), None, "{bad}");
        }
        assert_eq!(usd_to_inr(10_000, 83_215_000), 832_150); // $100.00 -> ₹8321.50
        assert_eq!(usd_to_inr(1, 83_215_000), 83); // 83.215 paise rounds to 83
        assert_eq!(usd_to_inr(1, 83_500_000), 84);
        assert_eq!(inr_to_usd(832_150, 83_215_000), 10_000);
    }
}
