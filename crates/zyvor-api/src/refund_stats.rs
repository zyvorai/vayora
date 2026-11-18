//! Refund analytics: how much is refunded against what was captured over time, how long refunds take, which are
//! stuck past the merchant's SLA, and where money leaks (fees not given back, duplicate refunds stopped).
//!
//! Everything is aggregated or an id-only list: no customer, requester, email or phone appears. Read-only.
//! Dates are IST days (weeks start on Monday). INR only. "Captured" and "refunded" in one bucket are different
//! payments, so the rate is a flow ratio per period, not the share of those payments that came back. Refund
//! processing time is the refund row's creation to its last update once it is `processed`, which includes any wait
//! for approval; a later touch to the row would lengthen it, so read it as an upper estimate.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{Datelike, Duration, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    ApiError, AppState,
    auth::Auth,
    cash::{lag_stats, load_settings},
    tenant,
};

/// Refund rate in basis points (processed refunds over captured), `None` when nothing was captured.
pub fn rate_bps(refunded_minor: i64, captured_minor: i64) -> Option<i64> {
    (captured_minor > 0).then(|| {
        ((i128::from(refunded_minor) * 10_000 + i128::from(captured_minor) / 2)
            / i128::from(captured_minor)) as i64
    })
}

/// First day of the bucket a day belongs to.
pub fn bucket_start(day: NaiveDate, weekly: bool) -> NaiveDate {
    if weekly {
        day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
    } else {
        day
    }
}

/// Every bucket start from `from` to `to`.
pub fn bucket_starts(from: NaiveDate, to: NaiveDate, weekly: bool) -> Vec<NaiveDate> {
    let step = if weekly { 7 } else { 1 };
    let mut v = vec![];
    let mut d = bucket_start(from, weekly);
    while d <= to {
        v.push(d);
        d += Duration::days(step);
    }
    v
}

#[derive(Deserialize)]
pub struct RefundQuery {
    /// First IST day, default 30 days before `to`.
    from: Option<String>,
    /// Last IST day, default today.
    to: Option<String>,
    /// `day` (default) or `week`.
    bucket: Option<String>,
    /// How many SLA breaches to list (1-200, default 50).
    limit: Option<i64>,
}

fn day(name: &str, v: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|_| {
        ApiError::invalid(
            "invalid_range",
            format!("{name} must be a date, YYYY-MM-DD"),
        )
    })
}

fn hours(seconds: i64) -> f64 {
    (seconds as f64 / 36.0).round() / 100.0
}

/// `GET /v1/analytics/refunds`
pub async fn analytics(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<RefundQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("cash:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let today = crate::clock::today(&mut tx).await?;
    let to = match &q.to {
        Some(v) => day("to", v)?,
        None => today,
    };
    let from = match &q.from {
        Some(v) => day("from", v)?,
        None => to - Duration::days(29),
    };
    if to < from || to - from > Duration::days(366) {
        return Err(ApiError::invalid(
            "invalid_range",
            "from..to must be at most 366 days and not reversed",
        ));
    }
    let weekly = match q.bucket.as_deref().unwrap_or("day") {
        "day" => false,
        "week" => true,
        _ => return Err(ApiError::invalid("invalid_bucket", "bucket is day or week")),
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let settings = load_settings(&mut tx).await?;

    #[derive(Default, Clone)]
    struct B {
        captured_n: i64,
        captured: i64,
        requested_n: i64,
        requested: i64,
        processed_n: i64,
        processed: i64,
    }
    let mut series: BTreeMap<NaiveDate, B> = bucket_starts(from, to, weekly)
        .into_iter()
        .map(|d| (d, B::default()))
        .collect();

    let cap: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        "SELECT (created_at AT TIME ZONE 'Asia/Kolkata')::date, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT
         FROM payment_attempts WHERE status = 'captured' AND currency = 'INR'
           AND (created_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $1 AND $2 GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    for (d, n, a) in cap {
        let b = series.entry(bucket_start(d, weekly)).or_default();
        b.captured_n += n;
        b.captured += a;
    }
    let req: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        "SELECT (created_at AT TIME ZONE 'Asia/Kolkata')::date, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT
         FROM refunds WHERE currency = 'INR' AND status <> 'rejected'
           AND (created_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $1 AND $2 GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    for (d, n, a) in req {
        let b = series.entry(bucket_start(d, weekly)).or_default();
        b.requested_n += n;
        b.requested += a;
    }
    let done: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        "SELECT (updated_at AT TIME ZONE 'Asia/Kolkata')::date, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT
         FROM refunds WHERE currency = 'INR' AND status = 'processed'
           AND (updated_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $1 AND $2 GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    for (d, n, a) in done {
        let b = series.entry(bucket_start(d, weekly)).or_default();
        b.processed_n += n;
        b.processed += a;
    }
    let (tc, tp, tr): (i64, i64, i64) = series.values().fold((0, 0, 0), |a, b| {
        (a.0 + b.captured, a.1 + b.processed, a.2 + b.requested)
    });

    // time to refund, per provider and overall
    let times: Vec<(String, i64)> = sqlx::query_as(
        "SELECT a.provider, EXTRACT(EPOCH FROM (r.updated_at - r.created_at))::BIGINT
         FROM refunds r JOIN payment_attempts a ON a.id = r.attempt_id
         WHERE r.currency = 'INR' AND r.status = 'processed'
           AND (r.updated_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $1 AND $2",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    let min = settings.min_samples as usize;
    let time_json = |secs: &[i64]| match lag_stats(secs, min) {
        Ok(st) => json!({
            "status": "observed", "samples": st.samples, "median_hours": hours(st.median.round() as i64),
            "p90_hours": hours(st.p90), "max_hours": hours(st.max), "min_hours": hours(st.min),
        }),
        Err(n) => json!({
            "status": "not_enough_data", "samples": n, "needed": settings.min_samples,
            "message": format!("{n} processed refunds in this range; at least {} are needed before a time is shown.", settings.min_samples),
        }),
    };
    let all: Vec<i64> = times.iter().map(|t| t.1).collect();
    let mut by_provider: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (p, t) in &times {
        by_provider.entry(p).or_default().push(*t);
    }

    // refunds past the SLA (total counts, then a page of ids)
    let sla = settings.refund_sla_hours;
    let breach_counts: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT status, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT,
                COALESCE(MAX(EXTRACT(EPOCH FROM (now() - created_at))::BIGINT), 0)
         FROM refunds WHERE currency = 'INR' AND status IN ('unknown', 'failed', 'processing', 'pending_approval')
           AND created_at < now() - make_interval(hours => $1::int) GROUP BY 1 ORDER BY 1",
    )
    .bind(sla as i32)
    .fetch_all(&mut *tx)
    .await?;
    let breaches: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT r.id, r.status, a.provider, r.amount_minor, EXTRACT(EPOCH FROM (now() - r.created_at))::BIGINT
         FROM refunds r JOIN payment_attempts a ON a.id = r.attempt_id
         WHERE r.currency = 'INR' AND r.status IN ('unknown', 'failed', 'processing', 'pending_approval')
           AND r.created_at < now() - make_interval(hours => $1::int)
         ORDER BY r.created_at, r.id LIMIT $2",
    )
    .bind(sla as i32)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;

    // leakage
    let fee: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT l.provider, l.method, count(*)::BIGINT, count(DISTINCT l.settlement_id)::BIGINT, COALESCE(SUM(l.variance_minor), 0)::BIGINT
         FROM fee_check_lines l JOIN settlements st ON st.id = l.settlement_id
         WHERE l.refunded_minor > 0 AND l.status = 'flagged' AND l.variance_minor > l.tolerance_minor
           AND (l.rule->>'refund_returns_fee')::boolean IS TRUE AND st.settled_on BETWEEN $1 AND $2
         GROUP BY 1, 2 ORDER BY 1, 2",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;
    let unchecked: i64 = sqlx::query_scalar(
        "SELECT count(*)::BIGINT FROM fee_check_lines l JOIN settlements st ON st.id = l.settlement_id
         WHERE l.refunded_minor > 0 AND l.status = 'no_rule' AND st.settled_on BETWEEN $1 AND $2",
    )
    .bind(from)
    .bind(to)
    .fetch_one(&mut *tx)
    .await?;
    let blocks: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT reason, count(*)::BIGINT, COALESCE(SUM(amount_minor), 0)::BIGINT FROM refund_blocks
         WHERE (created_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $1 AND $2 GROUP BY 1 ORDER BY 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx)
    .await?;

    Ok(Json(json!({
        "from": from, "to": to, "bucket": if weekly { "week" } else { "day" }, "currency": "INR",
        "series": series.iter().map(|(d, b)| json!({
            "starts": d,
            "captured_count": b.captured_n, "captured_minor": b.captured,
            "refunds_requested_count": b.requested_n, "refunds_requested_minor": b.requested,
            "refunds_processed_count": b.processed_n, "refunds_processed_minor": b.processed,
            "refund_rate_bps": rate_bps(b.processed, b.captured),
        })).collect::<Vec<_>>(),
        "totals": {
            "captured_minor": tc, "refunds_requested_minor": tr, "refunds_processed_minor": tp,
            "refund_rate_bps": rate_bps(tp, tc),
        },
        "time_to_refund": {
            "all": time_json(&all),
            "by_provider": by_provider.iter().map(|(p, v)| { let mut j = time_json(v); j["provider"] = json!(p); j }).collect::<Vec<_>>(),
            "note": "From the refund request to its processed state; includes any wait for approval.",
        },
        "sla": {
            "threshold_hours": sla,
            "breaches": breach_counts.iter().map(|(st, n, a, oldest)| json!({ "status": st, "count": n, "amount_minor": a, "oldest_hours": hours(*oldest) })).collect::<Vec<_>>(),
            "breach_count": breach_counts.iter().map(|b| b.1).sum::<i64>(),
            "breach_amount_minor": breach_counts.iter().map(|b| b.2).sum::<i64>(),
            "list": breaches.iter().map(|(id, st, p, a, age)| json!({ "refund_id": id, "status": st, "provider": p, "amount_minor": a, "age_hours": hours(*age) })).collect::<Vec<_>>(),
            "list_limit": limit,
        },
        "leakage": {
            "fee_not_returned": {
                "lines": fee.iter().map(|f| f.2).sum::<i64>(),
                "settlements": fee.iter().map(|f| f.3).sum::<i64>(),
                "amount_minor": fee.iter().map(|f| f.4).sum::<i64>(),
                "by_provider_method": fee.iter().map(|(p, m, n, st, a)| json!({ "provider": p, "method": m, "lines": n, "settlements": st, "amount_minor": a })).collect::<Vec<_>>(),
                "refund_lines_without_a_fee_rule": unchecked,
                "note": "Refunded payments where your own fee rule says the provider returns the fee and the settlement deducted more than that rule expects (beyond your tolerance). Needs fee rules; a refund with no rule is counted as unchecked, not as fine.",
            },
            "duplicate_refunds_blocked": {
                "count": blocks.iter().map(|b| b.1).sum::<i64>(),
                "by_reason": blocks.iter().map(|(r, n, a)| json!({ "reason": r, "count": n, "amount_minor": a })).collect::<Vec<_>>(),
                "note": "idempotent_replay: the same request was sent again and answered with the first refund. exceeds_refundable: a request for more than was left to refund (including refunds in progress). Counted from when this feature was installed.",
            },
        },
        "note": "Aggregates and ids only; no names or contact details. Captured and refunded in one period are different payments. INR only.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn rate_is_integer_basis_points_with_no_rate_when_nothing_was_captured() {
        assert_eq!(rate_bps(0, 0), None);
        assert_eq!(rate_bps(5_000, 100_000), Some(500));
        assert_eq!(rate_bps(1, 3), Some(3_333));
        assert_eq!(rate_bps(2, 3), Some(6_667));
        assert_eq!(rate_bps(100_000, 100_000), Some(10_000));
    }

    #[test]
    fn weekly_buckets_start_on_monday_and_cover_the_range() {
        // 2026-11-18 is a Wednesday
        assert_eq!(bucket_start(d(2026, 11, 18), true), d(2026, 11, 16));
        assert_eq!(bucket_start(d(2026, 11, 16), true), d(2026, 11, 16));
        assert_eq!(bucket_start(d(2026, 11, 22), true), d(2026, 11, 16));
        assert_eq!(
            bucket_starts(d(2026, 11, 18), d(2026, 12, 1), true),
            vec![d(2026, 11, 16), d(2026, 11, 23), d(2026, 11, 30)]
        );
        assert_eq!(
            bucket_starts(d(2026, 11, 1), d(2026, 11, 3), false).len(),
            3
        );
    }
}
