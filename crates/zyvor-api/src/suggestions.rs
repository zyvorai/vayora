//! Suggestions for an open reconciliation case: the likeliest explanation and the evidence behind it,
//! worked out from rows the system already holds. Nothing is changed; a person decides and resolves
//! the case with a note. No model is involved.

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

fn inr(minor: i64) -> String {
    format!("₹{}.{:02}", minor / 100, minor.abs() % 100)
}

/// What a difference between expected and received might be, by size. A hint, never a verdict.
pub fn mismatch_hint(expected: i64, actual: i64) -> String {
    let diff = expected - actual;
    if expected <= 0 || diff == 0 {
        return "The amounts agree; the case may be stale.".into();
    }
    let pct = diff as f64 * 100.0 / expected as f64;
    if diff < 0 {
        format!(
            "{} more arrived than expected. Check for a duplicate payment or a tip/convenience amount added at the provider.",
            inr(-diff)
        )
    } else if (1.5..=3.5).contains(&pct) {
        format!(
            "{} short ({pct:.1}%), about the size of a provider fee. Check whether the provider took its fee out of this payment.",
            inr(diff)
        )
    } else if actual > 0 && (expected as f64 / actual as f64 - 2.0).abs() < 0.01 {
        format!(
            "{} short, exactly half of what was expected. Check for a partial capture or a split.",
            inr(diff)
        )
    } else {
        format!(
            "{} short ({pct:.1}%). Look for a partial capture, a refund taken from this settlement, or an adjustment.",
            inr(diff)
        )
    }
}

/// Median of a non-empty list (the lower middle for even lengths).
pub fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[(v.len() - 1) / 2])
}

pub async fn case_suggestions(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let c: Option<(String, String, String, Option<i64>, Option<i64>, String)> = sqlx::query_as(
        "SELECT kind, subject_type, subject_id, expected_minor, actual_minor, explanation FROM recon_cases WHERE id = $1",
    ).bind(&id).fetch_optional(&mut *tx).await?;
    let (kind, _stype, subject, expected, actual, explanation) =
        c.ok_or_else(|| ApiError::not_found("recon case"))?;

    let mut evidence: Vec<Value> = vec![];
    let mut next: Vec<String> = vec![];
    let summary: String = match kind.as_str() {
        "unmatched_bank_credit" => {
            let line_id: i64 = subject.parse().map_err(|_| ApiError::internal())?;
            let line: Option<(i64, NaiveDate, String)> = sqlx::query_as(
                "SELECT amount_minor, posted_on, narration FROM bank_statement_lines WHERE id = $1",
            )
            .bind(line_id)
            .fetch_optional(&mut *tx)
            .await?;
            let (amt, on, narr) = line.ok_or_else(|| ApiError::not_found("bank line"))?;
            let near: Vec<(String, String, i64, NaiveDate)> = sqlx::query_as(
                "SELECT id, provider, net_minor, settled_on FROM settlements WHERE bank_line_id IS NULL AND settled_on BETWEEN $2::date - 7 AND $2::date + 7
                 ORDER BY abs(net_minor - $1), abs(settled_on - $2::date) LIMIT 3",
            ).bind(amt).bind(on).fetch_all(&mut *tx).await?;
            for (sid, prov, net, d) in &near {
                evidence.push(json!({ "settlement_id": sid, "provider": prov, "net_minor": net, "settled_on": d, "difference_minor": amt - net }));
            }
            match near.first() {
                Some((sid, _, net, _)) if *net == amt => {
                    next.push(format!("Settlement {sid} has exactly this net amount. Check its UTR against the narration \"{narr}\" and record that in the resolution note."));
                    format!("Probably settlement {sid}: the net amount is identical.")
                }
                Some((sid, _, net, _)) => {
                    next.push(format!("Compare settlement {sid} ({}) with the credit ({}); explain the difference before resolving.", inr(*net), inr(amt)));
                    format!(
                        "Closest unmatched settlement is {sid}, {} away.",
                        inr((amt - net).abs())
                    )
                }
                None => {
                    next.push("No unmatched settlement is within a week. Import the provider's settlement report, or check whether this credit is not from the provider (a transfer or a refund reversal).".into());
                    "No provider settlement near this credit.".into()
                }
            }
        }
        "missing_settlement" => {
            let row: Option<(String, i64, i64)> = sqlx::query_as(
                "SELECT provider, amount_minor, (current_date - created_at::date)::BIGINT FROM payment_attempts WHERE id = $1",
            ).bind(&subject).fetch_optional(&mut *tx).await?;
            let (prov, amt, waiting) = row.ok_or_else(|| ApiError::not_found("payment attempt"))?;
            let lags: Vec<i64> = sqlx::query_scalar(
                "SELECT (s.settled_on - a.created_at::date)::BIGINT FROM settlement_items i JOIN settlements s ON s.id = i.settlement_id
                 JOIN payment_attempts a ON a.id = i.attempt_id WHERE i.kind = 'payment' AND a.provider = $1 ORDER BY s.created_at DESC LIMIT 200",
            ).bind(&prov).fetch_all(&mut *tx).await?;
            let typical = median(lags.clone());
            let (same_day_total, same_day_settled): (i64, i64) = sqlx::query_as(
                "SELECT count(*)::BIGINT, count(*) FILTER (WHERE EXISTS (SELECT 1 FROM settlement_items i WHERE i.attempt_id = a.id))::BIGINT
                 FROM payment_attempts a WHERE a.provider = $1 AND a.status = 'captured' AND a.created_at::date = (SELECT created_at::date FROM payment_attempts WHERE id = $2)",
            ).bind(&prov).bind(&subject).fetch_one(&mut *tx).await?;
            evidence.push(json!({ "provider": prov, "amount_minor": amt, "days_waiting": waiting, "typical_days_to_settle": typical, "settled_after_n_payments": lags.len(), "same_day_payments": same_day_total, "same_day_settled": same_day_settled }));
            if same_day_total > 1 && same_day_settled == 0 {
                next.push(format!("None of that day's {same_day_total} {prov} payments has settled either: this looks like a late or missing settlement report, not one lost payment. Import the latest report or ask {prov}."));
            } else if same_day_settled > 0 {
                next.push(format!("Other payments from the same day settled but this one did not. Ask {prov} for the status of this payment reference."));
            } else {
                next.push(
                    "Import the provider's latest settlement report, then run a scan again.".into(),
                );
            }
            match typical {
                Some(t) if waiting <= t => format!(
                    "Waiting {waiting} days; {prov} usually settles in about {t}. It may simply not be due yet."
                ),
                Some(t) => format!("Waiting {waiting} days, longer than the usual {t} for {prov}."),
                None => format!(
                    "Waiting {waiting} days; there is no settled history for {prov} to compare against yet."
                ),
            }
        }
        "amount_mismatch" => {
            let (e, a) = (expected.unwrap_or(0), actual.unwrap_or(0));
            next.push("Open the payment's timeline and compare the provider's amount with the order amount before deciding.".into());
            mismatch_hint(e, a)
        }
        "duplicate_payment" => {
            next.push("Refund the extra payment from the Payments page if the payer paid twice, then resolve this case with the refund reference.".into());
            "The provider reported a second payment for an order that was already paid.".into()
        }
        _ => {
            next.push("Read the case explanation, open the payment's timeline for the full story, and resolve with a note saying what you found.".into());
            explanation.clone()
        }
    };
    Ok(Json(json!({
        "case_id": id,
        "kind": kind,
        "summary": summary,
        "evidence": evidence,
        "next_steps": next,
        "note": "A suggestion from the data on file. A person decides and resolves the case.",
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hints_by_size() {
        assert!(mismatch_hint(100_000, 98_000).contains("provider fee"));
        assert!(mismatch_hint(100_000, 50_000).contains("half"));
        assert!(mismatch_hint(100_000, 120_000).contains("duplicate"));
        assert!(mismatch_hint(100_000, 70_000).contains("partial capture"));
        assert!(mismatch_hint(100_000, 100_000).contains("stale"));
    }
    #[test]
    fn median_is_the_lower_middle() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3, 1, 2]), Some(2));
        assert_eq!(median(vec![4, 1, 3, 2]), Some(2));
    }
}
