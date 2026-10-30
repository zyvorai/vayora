//! Deterministic read-only insights: what is due to come in, and refund patterns worth a look.
//! No model is involved and nothing here changes anything. These are prompts for a person to check,
//! not findings of wrongdoing.

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, collections::OUTSTANDING, tenant};

#[derive(Deserialize)]
pub struct CashQuery {
    /// Weeks ahead to show (1-12, default 4).
    weeks: Option<i64>,
}

/// Open invoices by the week they fall due, what is already overdue, money promised, payouts waiting to
/// go out, and the recent run-rate of card/UPI receipts. INR only. This is a schedule of what is
/// owed, not a prediction that it will be paid on time.
pub async fn cashflow(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<CashQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let weeks = q.weeks.unwrap_or(4).clamp(1, 12);
    let today = Utc::now().date_naive();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(Option<NaiveDate>, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.due_date, {OUTSTANDING} FROM invoices i WHERE i.status IN ('open', 'partially_paid') AND i.currency = 'INR'"
    ))).fetch_all(&mut *tx).await?;
    let (mut overdue, mut undated, mut later) = (0i64, 0i64, 0i64);
    let mut buckets = vec![0i64; weeks as usize];
    for (due, out) in rows.into_iter().filter(|(_, o)| *o > 0) {
        match due {
            None => undated += out,
            Some(d) if d < today => overdue += out,
            Some(d) => {
                let w = (d - today).num_days() / 7;
                if (w as usize) < buckets.len() {
                    buckets[w as usize] += out;
                } else {
                    later += out;
                }
            }
        }
    }
    let promised: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(p.promised_minor), 0)::BIGINT FROM payment_promises p JOIN invoices i ON i.id = p.invoice_id
         WHERE p.canceled_at IS NULL AND p.promised_date >= $1 AND i.status IN ('open', 'partially_paid') AND i.currency = 'INR'",
    ).bind(today).fetch_one(&mut *tx).await?;
    let payouts_waiting: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM payouts WHERE status IN ('pending_approval', 'approved') AND currency = 'INR'",
    ).fetch_one(&mut *tx).await?;
    let (recent, days): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor - refunded_minor), 0)::BIGINT, 28::BIGINT FROM payment_attempts WHERE status = 'captured' AND currency = 'INR' AND created_at >= now() - interval '28 days'",
    ).fetch_one(&mut *tx).await?;
    Ok(Json(json!({
        "as_of": today,
        "currency": "INR",
        "weeks": buckets.iter().enumerate().map(|(i, v)| json!({ "starts": today + Duration::days(i as i64 * 7), "due_minor": v })).collect::<Vec<_>>(),
        "overdue_minor": overdue,
        "due_later_minor": later,
        "no_due_date_minor": undated,
        "promised_minor": promised,
        "payouts_waiting_minor": payouts_waiting,
        "daily_receipts_run_rate_minor": recent / days,
        "note": "A schedule of what is owed, by due date. It does not predict when people will actually pay.",
    })))
}

#[derive(Debug, PartialEq)]
pub struct Flag {
    pub code: &'static str,
    pub message: String,
}

/// Refund count this week against the average of the previous twelve weeks, and one requester
/// standing behind most of this week's refunds. Both need a handful of refunds to say anything.
pub fn refund_flags(
    this_week: i64,
    prior_weekly_avg: f64,
    by_requester: &[(String, i64)],
) -> Vec<Flag> {
    let mut out = vec![];
    if this_week >= 5 && (this_week as f64) > 2.0 * prior_weekly_avg.max(1.0) {
        out.push(Flag {
            code: "refund_spike",
            message: format!("{this_week} refunds in the last 7 days, against about {prior_weekly_avg:.1} a week before."),
        });
    }
    let total: i64 = by_requester.iter().map(|(_, n)| n).sum();
    if let Some((who, n)) = by_requester
        .iter()
        .max_by_key(|(_, n)| *n)
        .filter(|(_, n)| total >= 5 && *n * 10 >= total * 6)
    {
        out.push(Flag {
            code: "refunds_concentrated",
            message: format!("{who} requested {n} of the last 7 days' {total} refunds."),
        });
    }
    out
}

pub async fn refund_anomalies(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (this_week, prior): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at >= now() - interval '7 days')::BIGINT,
                count(*) FILTER (WHERE created_at < now() - interval '7 days' AND created_at >= now() - interval '91 days')::BIGINT
         FROM refunds WHERE status <> 'rejected'",
    ).fetch_one(&mut *tx).await?;
    let by: Vec<(String, i64)> = sqlx::query_as(
        "SELECT requested_by, count(*)::BIGINT FROM refunds WHERE created_at >= now() - interval '7 days' AND status <> 'rejected' GROUP BY 1 ORDER BY 2 DESC",
    ).fetch_all(&mut *tx).await?;
    let flags = refund_flags(this_week, prior as f64 / 12.0, &by);
    Ok(Json(json!({
        "refunds_last_7_days": this_week,
        "weekly_average_before": prior as f64 / 12.0,
        "flags": flags.iter().map(|f| json!({ "code": f.code, "message": f.message })).collect::<Vec<_>>(),
        "note": "Prompts for a person to check, not findings of wrongdoing.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spike_needs_volume_and_a_real_jump() {
        assert!(refund_flags(4, 0.0, &[]).is_empty(), "too few to say");
        assert_eq!(refund_flags(6, 1.0, &[])[0].code, "refund_spike");
        assert!(
            refund_flags(6, 4.0, &[]).is_empty(),
            "within the usual range"
        );
    }
    #[test]
    fn concentration_needs_most_of_at_least_five() {
        let by = vec![("meena".to_string(), 4), ("ravi".to_string(), 2)];
        assert_eq!(refund_flags(6, 6.0, &by)[0].code, "refunds_concentrated");
        let even = vec![("a".to_string(), 3), ("b".to_string(), 3)];
        assert!(refund_flags(6, 6.0, &even).is_empty());
        assert!(refund_flags(3, 3.0, &[("a".to_string(), 3)]).is_empty());
    }
}
