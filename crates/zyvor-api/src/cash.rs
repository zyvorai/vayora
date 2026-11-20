// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Cash visibility: where the money is between "the customer paid" and "it is in your bank".
//!
//!   * a dashboard by provider: captured but not settled, settled but not matched to a bank credit, matched,
//!     refunds still pending, payouts still pending;
//!   * observed settlement lag per provider and method (median and spread of past settlements), shown only with
//!     enough samples;
//!   * a forecast of expected credits per day with an honest range, from that observed lag;
//!   * delay alerts (a reconciliation case plus a `settlement.delayed` event, once per subject) when a settlement
//!     is later than the provider's own observed range or later than the lag the merchant said to expect.
//!
//! Nothing here invents a provider SLA. A provider and method with too few settled payments gets "not enough
//! data", not a guess. Lag is counted in calendar days from the payment's IST date to the settlement's date;
//! weekends and bank holidays are not modelled. Nothing here moves money or posts to the ledger.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{Duration, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, exports::decimal, tenant};

/// Case kinds this module opens.
pub const CASE_PENDING: &str = "settlement_delayed";
pub const CASE_ARRIVED_LATE: &str = "settlement_late";
/// How far back observed lag looks.
pub const LAG_LOOKBACK_DAYS: i64 = 180;

// ---------- pure: lag statistics ----------

/// Past settlement lag for one provider and method, in calendar days.
#[derive(Debug, Clone, PartialEq)]
pub struct LagStats {
    pub samples: usize,
    pub min: i64,
    pub p10: i64,
    pub median: f64,
    pub p90: i64,
    pub max: i64,
}

impl LagStats {
    /// The median rounded up to a whole day: the day a payment is expected.
    pub fn median_day(&self) -> i64 {
        self.median.ceil() as i64
    }
    pub fn to_json(&self) -> Value {
        json!({
            "status": "observed", "samples": self.samples, "min_days": self.min, "p10_days": self.p10,
            "median_days": self.median, "p90_days": self.p90, "max_days": self.max,
        })
    }
}

/// Nearest-rank percentile of a sorted, non-empty slice.
pub fn percentile(sorted: &[i64], pct: usize) -> i64 {
    let n = sorted.len();
    let rank = (pct * n).div_ceil(100).clamp(1, n);
    sorted[rank - 1]
}

/// Median, 10th and 90th percentile and the extremes of observed lags. `Err(n)` when `n` is below
/// `min_samples`: the caller says "not enough data". Negative lags (a settlement dated before its payment) are
/// data errors and are left out before counting.
pub fn lag_stats(lags: &[i64], min_samples: usize) -> Result<LagStats, usize> {
    let mut v: Vec<i64> = lags.iter().copied().filter(|l| *l >= 0).collect();
    if v.len() < min_samples.max(1) {
        return Err(v.len());
    }
    v.sort_unstable();
    let n = v.len();
    let median = if n % 2 == 1 {
        v[n / 2] as f64
    } else {
        (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0
    };
    Ok(LagStats {
        samples: n,
        min: v[0],
        p10: percentile(&v, 10),
        median,
        p90: percentile(&v, 90),
        max: v[n - 1],
    })
}

/// What a forecast or alert rests on for one provider and method.
#[derive(Debug, Clone, PartialEq)]
pub enum Basis {
    /// Enough past settlements: median and a p10-p90 range.
    Observed(LagStats),
    /// Not enough past settlements, but the merchant stated an expected lag (days). No range.
    Tenant(i64),
    /// Neither. Shown as "not enough data".
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LagRule {
    pub provider: String,
    /// A method or `*`.
    pub method: String,
    pub days: i64,
}

/// The merchant's own expected lag: an exact method beats `*`.
pub fn tenant_lag(rules: &[LagRule], provider: &str, method: &str) -> Option<i64> {
    rules
        .iter()
        .find(|r| r.provider == provider && r.method == method)
        .or_else(|| {
            rules
                .iter()
                .find(|r| r.provider == provider && r.method == "*")
        })
        .map(|r| r.days)
}

// ---------- pure: forecast ----------

/// Captured, unsettled payments of one provider and method made on one IST day.
#[derive(Debug, Clone)]
pub struct Cohort {
    pub provider: String,
    pub method: String,
    pub captured_on: NaiveDate,
    pub payments: i64,
    /// What is still to come in: amount less refunds already made.
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DayOut {
    pub expected_minor: i64,
    pub cumulative_low_minor: i64,
    pub cumulative_expected_minor: i64,
    pub cumulative_high_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Forecast {
    /// One entry per day from today.
    pub days: Vec<DayOut>,
    /// Past the provider's own observed range (or the merchant's expected date) and still not settled.
    pub overdue_minor: i64,
    pub overdue_payments: i64,
    /// Expected after the horizon.
    pub later_minor: i64,
    /// Counted in `expected` only: forecast from the merchant's stated lag, which has no range.
    pub no_range_minor: i64,
    /// `(provider, method) -> (payments, amount)` with nothing to forecast from.
    pub no_basis: BTreeMap<(String, String), (i64, i64)>,
}

/// Expected credits per day for the next `horizon` days.
///
/// A payment with an observed basis is expected `ceil(median)` days after it was made, and could arrive from
/// `p10` to `p90` days after. The day-by-day figures are cumulative: `low` is what has arrived by that day if
/// every payment took as long as its p90 (nearly all of history arrived by then), `high` is what could have
/// arrived if every payment were as quick as its p10, `expected` sits between. A payment already past its p90
/// is overdue, not forecast. A payment whose expected day has passed but is still inside its range is expected
/// today. The range describes past behaviour, not a guarantee.
pub fn forecast(
    cohorts: &[Cohort],
    basis: &dyn Fn(&str, &str) -> Basis,
    today: NaiveDate,
    horizon: i64,
) -> Forecast {
    let h = horizon.max(1) as usize;
    let mut f = Forecast {
        days: vec![DayOut::default(); h],
        ..Default::default()
    };
    // arrivals per day offset
    let (mut low, mut exp, mut high) = (vec![0i64; h], vec![0i64; h], vec![0i64; h]);
    let off = |d: NaiveDate| (d - today).num_days().max(0);
    for c in cohorts {
        if c.amount_minor <= 0 {
            continue;
        }
        match basis(&c.provider, &c.method) {
            Basis::None => {
                let e = f
                    .no_basis
                    .entry((c.provider.clone(), c.method.clone()))
                    .or_default();
                e.0 += c.payments;
                e.1 += c.amount_minor;
            }
            Basis::Tenant(days) => {
                let due = c.captured_on + Duration::days(days);
                if due < today {
                    f.overdue_minor += c.amount_minor;
                    f.overdue_payments += c.payments;
                    continue;
                }
                f.no_range_minor += c.amount_minor;
                let o = off(due);
                if (o as usize) < h {
                    exp[o as usize] += c.amount_minor;
                } else {
                    f.later_minor += c.amount_minor;
                }
            }
            Basis::Observed(st) => {
                let hi = c.captured_on + Duration::days(st.p90);
                if hi < today {
                    f.overdue_minor += c.amount_minor;
                    f.overdue_payments += c.payments;
                    continue;
                }
                let exp_o = off(c.captured_on + Duration::days(st.median_day()));
                let lo_o = off(c.captured_on + Duration::days(st.p10));
                let hi_o = off(hi);
                if (exp_o as usize) < h {
                    exp[exp_o as usize] += c.amount_minor;
                } else {
                    f.later_minor += c.amount_minor;
                }
                if (lo_o as usize) < h {
                    high[lo_o as usize] += c.amount_minor;
                }
                if (hi_o as usize) < h {
                    low[hi_o as usize] += c.amount_minor;
                }
            }
        }
    }
    let (mut cl, mut ce, mut ch) = (0, 0, 0);
    for i in 0..h {
        cl += low[i];
        ce += exp[i];
        ch += high[i];
        f.days[i] = DayOut {
            expected_minor: exp[i],
            cumulative_low_minor: cl,
            cumulative_expected_minor: ce,
            cumulative_high_minor: ch,
        };
    }
    f
}

// ---------- pure: alert thresholds ----------

/// The lag, in days, beyond which a provider and method's settlement counts as late, and why. The merchant's
/// own stated lag wins; otherwise the provider's observed p90; otherwise no alert (not enough data). `grace`
/// days are added to either.
pub fn alert_threshold(
    rules: &[LagRule],
    provider: &str,
    method: &str,
    observed: Option<&LagStats>,
    grace: i64,
) -> Option<(i64, &'static str)> {
    if let Some(d) = tenant_lag(rules, provider, method) {
        return Some((d + grace, "tenant_expected_lag"));
    }
    observed.map(|s| (s.p90 + grace, "observed_range"))
}

fn rupees(minor: i64) -> String {
    format!("Rs {}", decimal(minor))
}

// ---------- loading ----------

#[derive(Debug, Clone)]
pub struct Settings {
    pub min_samples: i64,
    pub alert_grace_days: i64,
    pub refund_sla_hours: i64,
}

pub async fn load_settings(conn: &mut PgConnection) -> Result<Settings, sqlx::Error> {
    let r: Option<(i32, i32, i32)> = sqlx::query_as(
        "SELECT min_samples, alert_grace_days, refund_sla_hours FROM cash_settings LIMIT 1",
    )
    .fetch_optional(conn)
    .await?;
    let (m, g, h) = r.unwrap_or((10, 0, 48));
    Ok(Settings {
        min_samples: i64::from(m),
        alert_grace_days: i64::from(g),
        refund_sla_hours: i64::from(h),
    })
}

async fn load_rules(conn: &mut PgConnection) -> Result<Vec<LagRule>, sqlx::Error> {
    let rows: Vec<(String, String, i32)> = sqlx::query_as(
        "SELECT provider, method, expected_lag_days FROM settlement_lag_rules ORDER BY provider, method",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(provider, method, d)| LagRule {
            provider,
            method,
            days: i64::from(d),
        })
        .collect())
}

/// One settled payment: which settlement, which provider and method, how many days it took.
#[derive(Debug, Clone)]
pub struct Sample {
    pub settlement_id: String,
    pub settled_on: NaiveDate,
    pub provider: String,
    pub method: String,
    pub lag_days: i64,
    pub amount_minor: i64,
}

pub async fn load_samples(
    conn: &mut PgConnection,
    today: NaiveDate,
) -> Result<Vec<Sample>, sqlx::Error> {
    let rows: Vec<(String, NaiveDate, String, String, i64, i64)> = sqlx::query_as(
        "SELECT DISTINCT ON (a.id) s.id, s.settled_on, a.provider, a.method,
                (s.settled_on - (a.created_at AT TIME ZONE 'Asia/Kolkata')::date)::BIGINT, i.amount_minor
         FROM settlement_items i
         JOIN settlements s ON s.id = i.settlement_id
         JOIN payment_attempts a ON a.id = i.attempt_id
         WHERE i.kind = 'payment' AND s.currency = 'INR' AND a.currency = 'INR' AND s.settled_on >= $1
         ORDER BY a.id, s.settled_on, s.id",
    )
    .bind(today - Duration::days(LAG_LOOKBACK_DAYS))
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(settlement_id, settled_on, provider, method, lag_days, amount_minor)| Sample {
                settlement_id,
                settled_on,
                provider,
                method,
                lag_days,
                amount_minor,
            },
        )
        .collect())
}

pub async fn load_cohorts(conn: &mut PgConnection) -> Result<Vec<Cohort>, sqlx::Error> {
    let rows: Vec<(String, String, NaiveDate, i64, i64)> = sqlx::query_as(
        "SELECT a.provider, a.method, (a.created_at AT TIME ZONE 'Asia/Kolkata')::date, count(*)::BIGINT,
                COALESCE(SUM(a.amount_minor - a.refunded_minor), 0)::BIGINT
         FROM payment_attempts a
         WHERE a.status = 'captured' AND a.currency = 'INR' AND a.amount_minor > a.refunded_minor
           AND a.created_at >= now() - interval '365 days'
           AND NOT EXISTS (SELECT 1 FROM settlement_items i WHERE i.attempt_id = a.id AND i.kind = 'payment')
         GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(provider, method, captured_on, payments, amount_minor)| Cohort {
                provider,
                method,
                captured_on,
                payments,
                amount_minor,
            },
        )
        .collect())
}

/// Observed lag stats per (provider, method), or the number of samples when there are too few.
pub fn stats_by_key(
    samples: &[Sample],
    min_samples: usize,
) -> BTreeMap<(String, String), Result<LagStats, usize>> {
    let mut by: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    for s in samples {
        by.entry((s.provider.clone(), s.method.clone()))
            .or_default()
            .push(s.lag_days);
    }
    by.into_iter()
        .map(|(k, v)| (k, lag_stats(&v, min_samples)))
        .collect()
}

fn basis_for(
    stats: &BTreeMap<(String, String), Result<LagStats, usize>>,
    rules: &[LagRule],
    provider: &str,
    method: &str,
) -> Basis {
    match stats.get(&(provider.to_string(), method.to_string())) {
        Some(Ok(st)) => Basis::Observed(st.clone()),
        _ => match tenant_lag(rules, provider, method) {
            Some(d) => Basis::Tenant(d),
            None => Basis::None,
        },
    }
}

// ---------- endpoints: lag, forecast ----------

/// `GET /v1/cash/lag`: observed settlement lag per provider and method, the merchant's stated lags, and the
/// provider and methods with too few settled payments.
pub async fn lag(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = crate::clock::today(&mut tx).await?;
    let settings = load_settings(&mut tx).await?;
    let rules = load_rules(&mut tx).await?;
    let samples = load_samples(&mut tx, today).await?;
    let stats = stats_by_key(&samples, settings.min_samples as usize);
    let data: Vec<Value> = stats
        .iter()
        .map(|((p, m), r)| {
            let mut v = match r {
                Ok(st) => st.to_json(),
                Err(n) => json!({
                    "status": "not_enough_data", "samples": n, "needed": settings.min_samples,
                    "message": format!("{n} settled payments so far; at least {} are needed before a lag is shown.", settings.min_samples),
                }),
            };
            v["provider"] = json!(p);
            v["method"] = json!(m);
            v["tenant_expected_lag_days"] = json!(tenant_lag(&rules, p, m));
            v
        })
        .collect();
    Ok(Json(json!({
        "as_of": today,
        "lookback_days": LAG_LOOKBACK_DAYS,
        "min_samples": settings.min_samples,
        "data": data,
        "tenant_rules": rules.iter().map(|r| json!({ "provider": r.provider, "method": r.method, "expected_lag_days": r.days })).collect::<Vec<_>>(),
        "note": "Calendar days from the payment's IST date to the settlement's date, from this business's own imported settlements. INR only. Weekends and bank holidays are not modelled. No provider SLA is assumed.",
    })))
}

#[derive(Deserialize)]
pub struct ForecastQuery {
    /// Days ahead, 1-30, default 7.
    days: Option<i64>,
}

/// `GET /v1/cash/forecast?days=7`: expected credits per day with a range, for captured payments not yet settled.
pub async fn forecast_endpoint(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ForecastQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let horizon = q.days.unwrap_or(7).clamp(1, 30);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = crate::clock::today(&mut tx).await?;
    let settings = load_settings(&mut tx).await?;
    let rules = load_rules(&mut tx).await?;
    let samples = load_samples(&mut tx, today).await?;
    let stats = stats_by_key(&samples, settings.min_samples as usize);
    let cohorts = load_cohorts(&mut tx).await?;
    let f = forecast(
        &cohorts,
        &|p, m| basis_for(&stats, &rules, p, m),
        today,
        horizon,
    );
    let mut pending: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();
    for c in &cohorts {
        let e = pending
            .entry((c.provider.clone(), c.method.clone()))
            .or_default();
        e.0 += c.payments;
        e.1 += c.amount_minor;
    }
    let basis: Vec<Value> = pending
        .iter()
        .map(|((p, m), (n, amt))| {
            let mut v = match basis_for(&stats, &rules, p, m) {
                Basis::Observed(st) => st.to_json(),
                Basis::Tenant(d) => json!({
                    "status": "tenant_expected_lag", "expected_lag_days": d,
                    "message": "Too few settled payments to show an observed lag; forecast from the lag you stated, with no range.",
                }),
                Basis::None => {
                    let have = stats.get(&(p.clone(), m.clone())).and_then(|r| r.as_ref().err().copied()).unwrap_or(0);
                    json!({
                        "status": "not_enough_data", "samples": have, "needed": settings.min_samples,
                        "message": "Not enough settled payments to forecast, and no expected lag set. These payments are left out of the forecast.",
                    })
                }
            };
            v["provider"] = json!(p);
            v["method"] = json!(m);
            v["pending_payments"] = json!(n);
            v["pending_minor"] = json!(amt);
            v
        })
        .collect();
    Ok(Json(json!({
        "as_of": today,
        "currency": "INR",
        "horizon_days": horizon,
        "days": f.days.iter().enumerate().map(|(i, d)| json!({
            "date": today + Duration::days(i as i64),
            "expected_minor": d.expected_minor,
            "cumulative_low_minor": d.cumulative_low_minor,
            "cumulative_expected_minor": d.cumulative_expected_minor,
            "cumulative_high_minor": d.cumulative_high_minor,
        })).collect::<Vec<_>>(),
        "overdue": { "payments": f.overdue_payments, "amount_minor": f.overdue_minor, "note": "Past the provider's own observed range (or the lag you stated) and still not settled. Not in the forecast; see the delay alerts." },
        "expected_after_horizon_minor": f.later_minor,
        "expected_without_range_minor": f.no_range_minor,
        "not_forecast": f.no_basis.iter().map(|((p, m), (n, amt))| json!({ "provider": p, "method": m, "payments": n, "amount_minor": amt, "status": "not_enough_data" })).collect::<Vec<_>>(),
        "basis": basis,
        "note": "Cumulative low assumes every pending payment takes as long as the slowest 10% of past settlements; high, as quick as the fastest 10%; expected, the median. The range describes this business's past settlements, not a guarantee. Refunds not yet settled are netted off. INR only.",
    })))
}

// ---------- settings and tenant lag rules ----------

pub async fn get_settings(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st = load_settings(&mut tx).await?;
    Ok(Json(json!({
        "min_samples": st.min_samples, "alert_grace_days": st.alert_grace_days, "refund_sla_hours": st.refund_sla_hours,
    })))
}

#[derive(Deserialize)]
pub struct SettingsBody {
    /// Settled payments needed per provider and method before an observed lag is used (3-500).
    min_samples: Option<i64>,
    /// Days allowed beyond the expected or observed lag before an alert opens (0-30).
    alert_grace_days: Option<i64>,
    /// Hours a refund may stay unknown, failed, processing or awaiting approval before it breaches the SLA.
    refund_sla_hours: Option<i64>,
}

pub async fn set_settings(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:write")?;
    let in_range = |v: Option<i64>, lo: i64, hi: i64| v.is_none_or(|x| (lo..=hi).contains(&x));
    if !in_range(b.min_samples, 3, 500)
        || !in_range(b.alert_grace_days, 0, 30)
        || !in_range(b.refund_sla_hours, 1, 24 * 90)
    {
        return Err(ApiError::invalid(
            "invalid_settings",
            "min_samples is 3-500, alert_grace_days 0-30, refund_sla_hours 1-2160",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur = load_settings(&mut tx).await?;
    let next = Settings {
        min_samples: b.min_samples.unwrap_or(cur.min_samples),
        alert_grace_days: b.alert_grace_days.unwrap_or(cur.alert_grace_days),
        refund_sla_hours: b.refund_sla_hours.unwrap_or(cur.refund_sla_hours),
    };
    sqlx::query(
        "INSERT INTO cash_settings (tenant_id, min_samples, alert_grace_days, refund_sla_hours, updated_by) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id) DO UPDATE SET min_samples = EXCLUDED.min_samples, alert_grace_days = EXCLUDED.alert_grace_days,
             refund_sla_hours = EXCLUDED.refund_sla_hours, updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(&auth.tenant_id)
    .bind(next.min_samples as i32)
    .bind(next.alert_grace_days as i32)
    .bind(next.refund_sla_hours as i32)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "cash.settings_changed",
        None,
        json!({ "min_samples": next.min_samples, "alert_grace_days": next.alert_grace_days, "refund_sla_hours": next.refund_sla_hours }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "min_samples": next.min_samples, "alert_grace_days": next.alert_grace_days, "refund_sla_hours": next.refund_sla_hours,
    })))
}

#[derive(Deserialize)]
pub struct NewLagRule {
    provider: String,
    /// A payment method (`upi`, `card`, ...) or `*` for all of the provider's methods.
    method: Option<String>,
    expected_lag_days: i64,
    note: Option<String>,
}

fn plain(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.len() <= max
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '*'))
}

pub async fn create_lag_rule(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewLagRule>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("cash:write")?;
    let method = b.method.clone().unwrap_or_else(|| "*".into());
    if !plain(&b.provider, 40)
        || !plain(&method, 40)
        || (method.contains('*') && method != "*")
        || b.provider.contains('*')
        || !(0..=60).contains(&b.expected_lag_days)
    {
        return Err(ApiError::invalid(
            "invalid_lag_rule",
            "provider and method are plain names (method `*` for all), expected_lag_days is 0-60",
        ));
    }
    let note = b.note.clone().unwrap_or_default();
    if note.len() > 500 {
        return Err(ApiError::invalid(
            "invalid_lag_rule",
            "note is at most 500 characters",
        ));
    }
    let id = new_id("slr");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id: String = sqlx::query_scalar(
        "INSERT INTO settlement_lag_rules (id, tenant_id, provider, method, expected_lag_days, note, created_by) VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (tenant_id, provider, method) DO UPDATE SET expected_lag_days = EXCLUDED.expected_lag_days, note = EXCLUDED.note, created_by = EXCLUDED.created_by
         RETURNING id",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(b.provider.trim())
    .bind(method.trim())
    .bind(b.expected_lag_days as i32)
    .bind(&note)
    .bind(&auth.actor)
    .fetch_one(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "cash.lag_rule_set",
        Some(&id),
        json!({ "provider": b.provider, "method": method, "expected_lag_days": b.expected_lag_days }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "provider": b.provider.trim(), "method": method.trim(), "expected_lag_days": b.expected_lag_days, "note": note }),
        ),
    ))
}

pub async fn list_lag_rules(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i32, String, String)> = sqlx::query_as(
        "SELECT id, provider, method, expected_lag_days, note, created_by FROM settlement_lag_rules ORDER BY provider, method",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({
        "data": rows.into_iter().map(|(id, p, m, d, n, by)| json!({ "id": id, "provider": p, "method": m, "expected_lag_days": d, "note": n, "created_by": by })).collect::<Vec<_>>(),
    })))
}

pub async fn delete_lag_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("DELETE FROM settlement_lag_rules WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("lag rule"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "cash.lag_rule_deleted",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "deleted": true })))
}

// ---------- delay alerts ----------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AlertSummary {
    /// New cases for captured payments that are still unsettled past their threshold.
    pub pending_opened: usize,
    /// New cases for settlements that arrived later than their threshold.
    pub late_opened: usize,
}

/// Open a case and queue a `settlement.delayed` event for every late subject that has none yet. Safe to run
/// repeatedly (the worker does): a subject that already has a case, resolved or not, gets nothing new, so the
/// event goes out once. Never touches money or the ledger.
pub async fn scan_alerts(
    conn: &mut PgConnection,
    tenant_id: &str,
    lookback_days: i64,
) -> Result<AlertSummary, ApiError> {
    let today = crate::clock::today(conn).await?;
    let settings = load_settings(conn).await?;
    let rules = load_rules(conn).await?;
    let samples = load_samples(conn, today).await?;
    let min = settings.min_samples as usize;
    let mut sum = AlertSummary::default();

    // 1. captured payments still unsettled past the threshold
    let stats = stats_by_key(&samples, min);
    for c in load_cohorts(conn).await? {
        let observed = stats
            .get(&(c.provider.clone(), c.method.clone()))
            .and_then(|r| r.as_ref().ok());
        let Some((threshold, basis)) = alert_threshold(
            &rules,
            &c.provider,
            &c.method,
            observed,
            settings.alert_grace_days,
        ) else {
            continue;
        };
        let age = (today - c.captured_on).num_days();
        if age <= threshold {
            continue;
        }
        let subject = format!("{}:{}:{}", c.provider, c.method, c.captured_on);
        let explanation = format!(
            "{} {} payment(s) made on {} ({}) are not in any settlement after {age} days; {}.",
            c.payments,
            crate::timeline::provider_title(&c.provider),
            c.captured_on,
            rupees(c.amount_minor),
            if basis == "tenant_expected_lag" {
                format!(
                    "you expected {threshold} days at most (including {} grace)",
                    settings.alert_grace_days
                )
            } else {
                format!(
                    "this provider's own settlements have taken at most {threshold} days for the slowest 10%"
                )
            }
        );
        let detail = json!({
            "state": "pending", "provider": c.provider, "method": c.method, "captured_on": c.captured_on,
            "payments": c.payments, "age_days": age, "threshold_days": threshold, "basis": basis,
        });
        if open_case(
            conn,
            tenant_id,
            CASE_PENDING,
            "payment_cohort",
            &subject,
            Some(c.amount_minor),
            Some(0),
            &explanation,
            detail.clone(),
        )
        .await?
        {
            let mut d = detail;
            d["amount_minor"] = json!(c.amount_minor);
            d["subject"] = json!(subject);
            tenant::emit(conn, tenant_id, "settlement.delayed", d).await?;
            sum.pending_opened += 1;
        }
    }

    // 2. settlements that arrived later than the threshold, judged against history without that settlement
    let recent_from = today - Duration::days(lookback_days.max(1));
    let mut by_settlement: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for s in samples.iter().filter(|s| s.settled_on >= recent_from) {
        by_settlement.entry(&s.settlement_id).or_default().push(s);
    }
    for (sid, items) in by_settlement {
        let (mut late_n, mut late_amount, mut worst) =
            (0i64, 0i64, (0i64, 0i64, "", String::new(), String::new()));
        for it in &items {
            let others: Vec<i64> = samples
                .iter()
                .filter(|o| {
                    o.provider == it.provider && o.method == it.method && o.settlement_id != sid
                })
                .map(|o| o.lag_days)
                .collect();
            let observed = lag_stats(&others, min).ok();
            let Some((threshold, basis)) = alert_threshold(
                &rules,
                &it.provider,
                &it.method,
                observed.as_ref(),
                settings.alert_grace_days,
            ) else {
                continue;
            };
            if it.lag_days > threshold {
                late_n += 1;
                late_amount += it.amount_minor;
                if it.lag_days - threshold > worst.0 - worst.1 {
                    worst = (
                        it.lag_days,
                        threshold,
                        basis,
                        it.provider.clone(),
                        it.method.clone(),
                    );
                }
            }
        }
        if late_n == 0 {
            continue;
        }
        let explanation = format!(
            "{late_n} payment(s) worth {} reached this {} settlement after up to {} days, later than the {} days {}.",
            rupees(late_amount),
            crate::timeline::provider_title(&worst.3),
            worst.0,
            worst.1,
            if worst.2 == "tenant_expected_lag" {
                "you expected"
            } else {
                "this provider's own history shows for all but the slowest 10%"
            },
        );
        let detail = json!({
            "state": "arrived_late", "settlement_id": sid, "provider": worst.3, "method": worst.4,
            "late_payments": late_n, "lag_days": worst.0, "threshold_days": worst.1, "basis": worst.2,
        });
        if open_case(
            conn,
            tenant_id,
            CASE_ARRIVED_LATE,
            "settlement",
            sid,
            None,
            None,
            &explanation,
            detail.clone(),
        )
        .await?
        {
            let mut d = detail;
            d["amount_minor"] = json!(late_amount);
            d["subject"] = json!(sid);
            tenant::emit(conn, tenant_id, "settlement.delayed", d).await?;
            sum.late_opened += 1;
        }
    }
    Ok(sum)
}

#[derive(Deserialize)]
pub struct ScanQuery {
    /// How far back to look for settlements that arrived late (1-90, default 14).
    lookback_days: Option<i64>,
}

/// `POST /v1/cash/alerts/scan`: run the delay check now. The worker runs it too.
pub async fn scan(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ScanQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sum = scan_alerts(
        &mut tx,
        &auth.tenant_id,
        q.lookback_days.unwrap_or(14).clamp(1, 90),
    )
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "cash.alerts_scanned",
        None,
        json!({ "pending_opened": sum.pending_opened, "late_opened": sum.late_opened }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "pending_cases_opened": sum.pending_opened, "late_cases_opened": sum.late_opened,
        "case_kinds": [CASE_PENDING, CASE_ARRIVED_LATE],
    })))
}

// ---------- dashboard ----------

#[derive(Deserialize)]
pub struct DashQuery {
    /// Window for "matched" (1-365 days, default 30).
    matched_days: Option<i64>,
}

fn entry<'a>(m: &'a mut BTreeMap<String, Value>, provider: &str) -> &'a mut Value {
    m.entry(provider.to_string()).or_insert_with(|| {
        json!({
            "provider": provider,
            "in_transit": { "payments": 0, "gross_minor": 0, "refunded_minor": 0, "net_minor": 0 },
            "settled_not_matched_to_bank": { "settlements": 0, "net_minor": 0, "oldest_settled_on": null },
            "matched": { "settlements": 0, "net_minor": 0 },
            "refunds_pending": { "count": 0, "amount_minor": 0, "by_status": {} },
        })
    })
}

fn add_i(v: &mut Value, path: &[&str], by: i64) {
    let mut cur = v;
    for p in &path[..path.len() - 1] {
        cur = &mut cur[*p];
    }
    let last = path[path.len() - 1];
    cur[last] = json!(cur[last].as_i64().unwrap_or(0) + by);
}

/// `GET /v1/cash/dashboard`: money in transit and where it is stuck, by provider. INR only.
pub async fn dashboard(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<DashQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let matched_days = q.matched_days.unwrap_or(30).clamp(1, 365);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = crate::clock::today(&mut tx).await?;
    let mut by: BTreeMap<String, Value> = BTreeMap::new();

    let transit: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT a.provider, count(*)::BIGINT, COALESCE(SUM(a.amount_minor), 0)::BIGINT, COALESCE(SUM(a.refunded_minor), 0)::BIGINT
         FROM payment_attempts a
         WHERE a.status = 'captured' AND a.currency = 'INR'
           AND NOT EXISTS (SELECT 1 FROM settlement_items i WHERE i.attempt_id = a.id AND i.kind = 'payment')
         GROUP BY 1",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (p, n, g, r) in transit {
        let e = entry(&mut by, &p);
        add_i(e, &["in_transit", "payments"], n);
        add_i(e, &["in_transit", "gross_minor"], g);
        add_i(e, &["in_transit", "refunded_minor"], r);
        add_i(e, &["in_transit", "net_minor"], g - r);
    }

    let unbanked: Vec<(String, i64, i64, Option<NaiveDate>)> = sqlx::query_as(
        "SELECT provider, count(*)::BIGINT, COALESCE(SUM(net_minor), 0)::BIGINT, min(settled_on) FROM settlements st
         WHERE currency = 'INR' AND bank_line_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'settlement' AND m.subject_id = st.id AND m.status = 'confirmed')
         GROUP BY 1",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (p, n, net, oldest) in unbanked {
        let e = entry(&mut by, &p);
        add_i(e, &["settled_not_matched_to_bank", "settlements"], n);
        add_i(e, &["settled_not_matched_to_bank", "net_minor"], net);
        e["settled_not_matched_to_bank"]["oldest_settled_on"] = json!(oldest);
    }

    let matched: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT provider, count(*)::BIGINT, COALESCE(SUM(net_minor), 0)::BIGINT FROM settlements st
         WHERE currency = 'INR' AND settled_on >= $1
           AND (bank_line_id IS NOT NULL OR EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'settlement' AND m.subject_id = st.id AND m.status = 'confirmed'))
         GROUP BY 1",
    )
    .bind(today - Duration::days(matched_days))
    .fetch_all(&mut *tx)
    .await?;
    for (p, n, net) in matched {
        let e = entry(&mut by, &p);
        add_i(e, &["matched", "settlements"], n);
        add_i(e, &["matched", "net_minor"], net);
    }

    let refunds: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.provider, r.status, count(*)::BIGINT, COALESCE(SUM(r.amount_minor), 0)::BIGINT
         FROM refunds r JOIN payment_attempts a ON a.id = r.attempt_id
         WHERE r.currency = 'INR' AND r.status IN ('pending_approval', 'processing', 'unknown', 'failed')
         GROUP BY 1, 2",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (p, status, n, amt) in refunds {
        let e = entry(&mut by, &p);
        add_i(e, &["refunds_pending", "count"], n);
        add_i(e, &["refunds_pending", "amount_minor"], amt);
        e["refunds_pending"]["by_status"][&status] = json!({ "count": n, "amount_minor": amt });
    }

    let payouts: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT status, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT FROM payouts
         WHERE currency = 'INR' AND status IN ('pending_approval', 'approved') GROUP BY 1 ORDER BY 1",
    )
    .fetch_all(&mut *tx)
    .await?;
    let (unmatched_n, unmatched_amt): (i64, i64) = sqlx::query_as(
        "SELECT count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT FROM payouts p
         WHERE p.currency = 'INR' AND p.status = 'paid'
           AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'payout' AND m.subject_id = p.id AND m.status = 'confirmed')",
    )
    .fetch_one(&mut *tx)
    .await?;

    let mut totals = json!({
        "in_transit_net_minor": 0, "settled_not_matched_to_bank_minor": 0, "matched_minor": 0, "refunds_pending_minor": 0,
    });
    for v in by.values() {
        add_i(
            &mut totals,
            &["in_transit_net_minor"],
            v["in_transit"]["net_minor"].as_i64().unwrap_or(0),
        );
        add_i(
            &mut totals,
            &["settled_not_matched_to_bank_minor"],
            v["settled_not_matched_to_bank"]["net_minor"]
                .as_i64()
                .unwrap_or(0),
        );
        add_i(
            &mut totals,
            &["matched_minor"],
            v["matched"]["net_minor"].as_i64().unwrap_or(0),
        );
        add_i(
            &mut totals,
            &["refunds_pending_minor"],
            v["refunds_pending"]["amount_minor"].as_i64().unwrap_or(0),
        );
    }
    Ok(Json(json!({
        "as_of": today,
        "currency": "INR",
        "matched_window_days": matched_days,
        "by_provider": by.into_values().collect::<Vec<_>>(),
        "totals": totals,
        "payouts_pending": {
            "by_status": payouts.iter().map(|(st, n, a)| json!({ "status": st, "count": n, "amount_minor": a })).collect::<Vec<_>>(),
            "amount_minor": payouts.iter().map(|p| p.2).sum::<i64>(),
            "note": "Payouts are not tied to a collecting provider, so they are shown for the business, not by provider.",
        },
        "paid_payouts_without_bank_debit_match": { "count": unmatched_n, "amount_minor": unmatched_amt, "note": "Counts every paid payout with no confirmed bank debit match, including ones whose statement has not been imported yet." },
        "note": "Captured and in transit: paid by the customer, in no imported settlement yet. Settled and not matched: in a provider settlement, no bank credit confirmed against it. Matched: a bank credit is confirmed (suggestions needing a person do not count). Refunds pending: waiting for approval, processing, unknown or failed. INR only; other currencies are not shown.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn stats(lags: &[i64]) -> LagStats {
        lag_stats(lags, 5).unwrap()
    }

    #[test]
    fn needs_the_minimum_number_of_samples() {
        assert_eq!(lag_stats(&[1, 2, 2, 3], 5), Err(4));
        assert!(lag_stats(&[1, 2, 2, 3, 3], 5).is_ok());
        // negative lags are bad data and do not count towards the minimum
        assert_eq!(lag_stats(&[1, 2, 2, 3, -1], 5), Err(4));
        assert_eq!(lag_stats(&[], 1), Err(0));
    }

    #[test]
    fn median_and_percentiles_by_nearest_rank() {
        // 1..=10: median 5.5, p10 = rank 1 = 1, p90 = rank 9 = 9
        let st = stats(&[10, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(
            (st.min, st.p10, st.p90, st.max, st.samples),
            (1, 1, 9, 10, 10)
        );
        assert_eq!(st.median, 5.5);
        assert_eq!(st.median_day(), 6);
        // odd count: the middle value
        let st = stats(&[1, 1, 2, 2, 7]);
        assert_eq!((st.median, st.p90, st.max), (2.0, 7, 7));
        // all the same: no spread
        let st = stats(&[2, 2, 2, 2, 2, 2]);
        assert_eq!((st.min, st.p10, st.p90, st.max), (2, 2, 2, 2));
    }

    #[test]
    fn tenant_lag_prefers_the_exact_method() {
        let rules = vec![
            LagRule {
                provider: "rz".into(),
                method: "*".into(),
                days: 3,
            },
            LagRule {
                provider: "rz".into(),
                method: "upi".into(),
                days: 1,
            },
        ];
        assert_eq!(tenant_lag(&rules, "rz", "upi"), Some(1));
        assert_eq!(tenant_lag(&rules, "rz", "card"), Some(3));
        assert_eq!(tenant_lag(&rules, "cf", "upi"), None);
    }

    #[test]
    fn threshold_prefers_what_the_merchant_said_then_the_observed_range_then_nothing() {
        let rules = vec![LagRule {
            provider: "rz".into(),
            method: "upi".into(),
            days: 2,
        }];
        let st = stats(&[1, 1, 1, 2, 2, 2, 2, 3, 3, 4]);
        assert_eq!(
            alert_threshold(&rules, "rz", "upi", Some(&st), 1),
            Some((3, "tenant_expected_lag"))
        );
        assert_eq!(
            alert_threshold(&rules, "rz", "card", Some(&st), 1),
            Some((4, "observed_range"))
        );
        assert_eq!(
            alert_threshold(&rules, "cf", "card", None, 0),
            None,
            "no data and nothing stated: no alert, no invented SLA"
        );
    }

    fn cohort(on: NaiveDate, amount: i64) -> Cohort {
        Cohort {
            provider: "rz".into(),
            method: "upi".into(),
            captured_on: on,
            payments: 1,
            amount_minor: amount,
        }
    }

    #[test]
    fn forecast_places_money_on_the_median_day_with_a_range() {
        let today = d(2026, 11, 10);
        // lags 1..=10: median 5.5 -> day 6, p10 1, p90 9
        let st = stats(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let basis = |_: &str, _: &str| Basis::Observed(st.clone());
        // paid today: expected today+6, could be as early as today+1, nearly always by today+9
        let f = forecast(&[cohort(today, 1_000)], &basis, today, 10);
        let exp_day = f
            .days
            .iter()
            .position(|x| x.expected_minor == 1_000)
            .unwrap();
        assert_eq!(exp_day, 6);
        assert_eq!(f.days[0].cumulative_high_minor, 0);
        assert_eq!(
            f.days[1].cumulative_high_minor, 1_000,
            "as quick as the fastest 10%"
        );
        assert_eq!(f.days[8].cumulative_low_minor, 0);
        assert_eq!(
            f.days[9].cumulative_low_minor, 1_000,
            "by the p90 day it has nearly always arrived"
        );
        assert_eq!(f.days[5].cumulative_expected_minor, 0);
        assert_eq!(f.days[6].cumulative_expected_minor, 1_000);
        // ordering: low <= expected <= high on every day
        for x in &f.days {
            assert!(x.cumulative_low_minor <= x.cumulative_expected_minor);
            assert!(x.cumulative_expected_minor <= x.cumulative_high_minor);
        }
        assert_eq!((f.overdue_minor, f.later_minor), (0, 0));
    }

    #[test]
    fn forecast_separates_overdue_late_and_beyond_the_horizon() {
        let today = d(2026, 11, 10);
        let st = stats(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]); // p90 = 9, median day 6
        let basis = |_: &str, _: &str| Basis::Observed(st.clone());
        let cohorts = vec![
            cohort(d(2026, 11, 1), 500), // paid 9 days ago: at p90 exactly, not yet overdue, expected today
            cohort(d(2026, 10, 30), 700), // 11 days ago: past p90, overdue
            cohort(d(2026, 11, 9), 300), // yesterday: expected in 5 days
        ];
        let f = forecast(&cohorts, &basis, today, 3);
        assert_eq!((f.overdue_minor, f.overdue_payments), (700, 1));
        assert_eq!(
            f.days[0].expected_minor, 500,
            "median day has passed but still inside the range: expected today"
        );
        assert_eq!(f.later_minor, 300, "expected after the 3 day horizon");
    }

    #[test]
    fn forecast_never_invents_a_basis() {
        let today = d(2026, 11, 10);
        let basis = |_: &str, _: &str| Basis::None;
        let f = forecast(&[cohort(today, 1_000)], &basis, today, 5);
        assert!(f.days.iter().all(|x| x.cumulative_expected_minor == 0));
        assert_eq!(
            f.no_basis[&("rz".to_string(), "upi".to_string())],
            (1, 1_000)
        );
        // a merchant-stated lag forecasts the expected day only, with no range, and flags it
        let basis = |_: &str, _: &str| Basis::Tenant(2);
        let f = forecast(
            &[cohort(today, 1_000), cohort(d(2026, 11, 1), 400)],
            &basis,
            today,
            5,
        );
        assert_eq!(f.days[2].expected_minor, 1_000);
        assert_eq!(f.no_range_minor, 1_000);
        assert_eq!(
            f.days[4].cumulative_low_minor + f.days[4].cumulative_high_minor,
            0
        );
        assert_eq!(f.overdue_minor, 400, "past the stated lag");
    }

    #[test]
    fn empty_and_zero_amount_cohorts_forecast_nothing() {
        let today = d(2026, 11, 10);
        let st = stats(&[1, 2, 3, 4, 5]);
        let basis = |_: &str, _: &str| Basis::Observed(st.clone());
        let f = forecast(&[cohort(today, 0)], &basis, today, 3);
        assert_eq!(
            f,
            Forecast {
                days: vec![DayOut::default(); 3],
                ..Default::default()
            }
        );
    }
}
