//! One ordered story per payment: the intent, each attempt, the provider's webhooks that mention it,
//! ledger postings, settlement, refunds and disputes, plus a plain sentence on where it stands and what
//! happens next. Read-only; everything comes from rows the system already keeps.

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

type At = DateTime<Utc>;

fn ev(at: At, kind: &str, title: String, detail: Value) -> (At, Value) {
    (
        at,
        json!({ "at": at, "kind": kind, "title": title, "detail": detail }),
    )
}

/// Where a payment stands, in words a shop owner can act on.
pub fn standing(intent_status: &str, attempt: Option<(&str, &str)>, settled: bool) -> String {
    match (intent_status, attempt) {
        (_, None) => "Created, but nobody has tried to pay yet.".into(),
        ("succeeded", _) if settled => "Paid and settled to your bank.".into(),
        ("succeeded", _) => "Paid. The provider has not yet included it in a settlement; settlements usually arrive one to three working days later, and Reconciliation will match it when the bank line appears.".into(),
        (_, Some((_, "pending" | "created"))) => "Waiting for the provider to confirm. UPI and bank confirmations can take a few minutes. Zyvor marks it paid only from a verified provider message or a status check, never from the browser returning to your site.".into(),
        (_, Some((_, "authorized"))) => "Authorised by the bank and waiting for you to capture it.".into(),
        (_, Some((_, "failed"))) => "The last attempt failed. The payer can try again on the same payment.".into(),
        (_, Some((_, "unknown"))) => "The provider's answer is unclear. A case is open in Reconciliation; do not ask the payer to pay again until it is resolved.".into(),
        _ => format!("Payment is {intent_status}."),
    }
}

pub async fn payment_timeline(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require_branch("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let intent: Option<(At, String, i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT created_at, status, amount_minor, currency, invoice_id, external_ref FROM payment_intents WHERE id = $1",
    ).bind(&id).fetch_optional(&mut *tx).await?;
    let (created, status, amount, ccy, invoice_id, ext) =
        intent.ok_or_else(|| ApiError::not_found("payment"))?;
    if let Some(own) = &auth.branch_id {
        let branch: Option<String> =
            sqlx::query_scalar("SELECT branch_id FROM payment_intents WHERE id = $1")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        if branch.as_deref() != Some(own.as_str()) {
            return Err(ApiError::not_found("payment"));
        }
    }
    let mut out: Vec<(At, Value)> = vec![ev(
        created,
        "intent",
        format!(
            "Payment of {}.{:02} {ccy} created",
            amount / 100,
            amount % 100
        ),
        json!({ "external_ref": ext, "invoice_id": invoice_id }),
    )];

    #[allow(clippy::type_complexity)]
    let attempts: Vec<(String, String, String, String, At, At, Option<String>, Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT id, provider, method, status, created_at, updated_at, failure_reason, provider_order_ref, provider_payment_ref, refunded_minor FROM payment_attempts WHERE intent_id = $1 ORDER BY created_at",
    ).bind(&id).fetch_all(&mut *tx).await?;
    for (aid, provider, method, st, c, u, fail, order, pay, _) in &attempts {
        out.push(ev(
            *c,
            "attempt",
            format!("Attempt started on {provider} ({method})"),
            json!({ "attempt_id": aid }),
        ));
        if u > c {
            let title = match st.as_str() {
                "captured" => "Payment captured".to_string(),
                "failed" => "Attempt failed".to_string(),
                other => format!("Attempt is now {other}"),
            };
            out.push(ev(
                *u,
                "attempt",
                title,
                json!({ "attempt_id": aid, "status": st, "failure_reason": fail }),
            ));
        }
        for r in [order, pay].into_iter().flatten() {
            let hooks: Vec<(At, String, String, String)> = sqlx::query_as(
                "SELECT received_at, provider, kind, outcome FROM webhook_inbox WHERE provider = $1 AND position($2 in payload::text) > 0 ORDER BY received_at",
            ).bind(provider).bind(r).fetch_all(&mut *tx).await?;
            for (at, p, kind, outcome) in hooks {
                out.push(ev(
                    at,
                    "webhook",
                    format!("{p} sent {kind}"),
                    json!({ "outcome": outcome }),
                ));
            }
        }
        let ledger: Vec<(At, String)> = sqlx::query_as(
            "SELECT created_at, reference FROM ledger_transactions WHERE reference = $1",
        )
        .bind(format!("capture:{aid}"))
        .fetch_all(&mut *tx)
        .await?;
        for (at, r) in ledger {
            out.push(ev(
                at,
                "ledger",
                "Recorded in the ledger".into(),
                json!({ "reference": r }),
            ));
        }
    }

    let refunds: Vec<(String, At, At, i64, String, Option<String>)> = sqlx::query_as(
        "SELECT id, created_at, updated_at, amount_minor, status, reason FROM refunds WHERE intent_id = $1 ORDER BY created_at",
    ).bind(&id).fetch_all(&mut *tx).await?;
    for (rid, c, u, amt, st, reason) in refunds {
        out.push(ev(
            c,
            "refund",
            format!("Refund of {}.{:02} requested", amt / 100, amt % 100),
            json!({ "refund_id": rid, "reason": reason }),
        ));
        if u > c {
            out.push(ev(
                u,
                "refund",
                format!("Refund {}", st.replace('_', " ")),
                json!({ "refund_id": rid }),
            ));
        }
    }
    let disputes: Vec<(String, At, Option<At>, i64, String, String)> = sqlx::query_as(
        "SELECT id, created_at, resolved_at, amount_minor, status, reason FROM disputes WHERE intent_id = $1 ORDER BY created_at",
    ).bind(&id).fetch_all(&mut *tx).await?;
    for (did, c, res, amt, st, reason) in disputes {
        out.push(ev(
            c,
            "dispute",
            format!("Dispute opened for {}.{:02}", amt / 100, amt % 100),
            json!({ "dispute_id": did, "reason": reason }),
        ));
        if let Some(r) = res {
            out.push(ev(
                r,
                "dispute",
                format!("Dispute {st}"),
                json!({ "dispute_id": did }),
            ));
        }
    }
    let settled_rows: Vec<(At, String, Option<String>, String, chrono::NaiveDate)> = sqlx::query_as(
        "SELECT s.created_at, s.provider, s.utr, s.status, s.settled_on FROM settlement_items i JOIN settlements s ON s.id = i.settlement_id
         JOIN payment_attempts a ON a.id = i.attempt_id WHERE a.intent_id = $1 AND i.kind = 'payment' ORDER BY s.created_at",
    ).bind(&id).fetch_all(&mut *tx).await?;
    let settled = !settled_rows.is_empty();
    for (at, provider, utr, st, on) in settled_rows {
        out.push(ev(
            at,
            "settlement",
            format!("Included in a {provider} settlement for {on}"),
            json!({ "utr": utr, "status": st }),
        ));
    }

    out.sort_by_key(|(at, _)| *at);
    let last = attempts.last().map(|a| (a.1.as_str(), a.3.as_str()));
    Ok(Json(json!({
        "payment_id": id,
        "status": status,
        "standing": standing(&status, last, settled),
        "events": out.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::standing;
    #[test]
    fn plain_words_for_each_state() {
        assert!(standing("requires_payment_method", None, false).contains("nobody"));
        assert!(
            standing("processing", Some(("razorpay", "pending")), false)
                .contains("never from the browser")
        );
        assert!(standing("succeeded", Some(("razorpay", "captured")), false).contains("not yet"));
        assert!(standing("succeeded", Some(("razorpay", "captured")), true).contains("settled"));
        assert!(standing("failed", Some(("razorpay", "unknown")), false).contains("do not ask"));
    }
}
