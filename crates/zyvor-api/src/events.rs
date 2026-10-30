//! The webhook event catalogue and subscription matching. Merchants pick the types they want
//! (`payment.captured`, a family like `payment.*`, or `*`); events they did not ask for are never queued.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth};

/// Every event type Zyvor emits, with what it means. A debug assertion in `tenant::emit` keeps this complete.
pub const CATALOGUE: &[(&str, &str)] = &[
    (
        "payment.authorized",
        "A payment was authorised and is waiting to be captured.",
    ),
    (
        "payment.captured",
        "A payment succeeded and the money is on its way to you.",
    ),
    ("payment.failed", "A payment attempt failed."),
    (
        "payment.risk_blocked",
        "Your risk rules stopped a payment or an attempt; the audit trail says which rule.",
    ),
    (
        "payment.unresolved",
        "The provider's answer is unknown; no new attempt can start until it resolves.",
    ),
    ("refund.processed", "A refund was completed."),
    ("refund.failed", "A refund failed and needs attention."),
    (
        "refund.approval_required",
        "A refund is above your threshold and needs a second person.",
    ),
    (
        "invoice.created",
        "An invoice was created (by you, a subscription run or a milestone).",
    ),
    ("invoice.voided", "An invoice was voided."),
    (
        "invoice.reminder",
        "A payment reminder should be sent to a customer through your own channel.",
    ),
    (
        "credit_note.created",
        "A credit note reduced what a customer owes.",
    ),
    ("subscription.created", "A subscription started."),
    (
        "subscription.paused",
        "A subscription was paused; no invoices until it resumes.",
    ),
    ("subscription.active", "A paused subscription resumed."),
    ("subscription.canceled", "A subscription was canceled."),
    (
        "subscription.plan_changed",
        "A subscription moved to another plan.",
    ),
    (
        "dispute.opened",
        "A customer disputed a payment; the amount is held.",
    ),
    (
        "dispute.won",
        "A dispute was decided in your favour; the held amount is released.",
    ),
    (
        "dispute.lost",
        "A dispute was lost; the amount was written off and the invoice re-opened.",
    ),
    (
        "payout.approval_required",
        "A payout to a vendor needs a second person's approval.",
    ),
    (
        "payout.approved",
        "A payout was approved and can be paid from your bank.",
    ),
    ("payout.rejected", "A payout was rejected."),
    (
        "onboarding.submitted",
        "Onboarding was submitted for review.",
    ),
    ("onboarding.approved", "Onboarding was approved."),
    (
        "onboarding.rejected",
        "Onboarding was rejected, with a note.",
    ),
    (
        "payout.paid",
        "A payout was recorded as paid, with its bank reference.",
    ),
];

pub fn known(ty: &str) -> bool {
    CATALOGUE.iter().any(|(t, _)| *t == ty)
}

/// A subscription entry is `*`, a family such as `payment.*`, or an exact type from the catalogue.
pub fn valid_pattern(p: &str) -> bool {
    p == "*"
        || known(p)
        || p.strip_suffix(".*").is_some_and(|fam| {
            CATALOGUE
                .iter()
                .any(|(t, _)| t.split('.').next() == Some(fam))
        })
}

/// Does a tenant's subscription list (`None` = everything) want this event type?
pub fn subscribed(list: Option<&[String]>, ty: &str) -> bool {
    let Some(list) = list else { return true };
    list.iter().any(|p| {
        p == "*"
            || p == ty
            || p.strip_suffix(".*")
                .is_some_and(|fam| ty.split('.').next() == Some(fam))
    })
}

/// `GET /v1/events/catalogue`
pub async fn catalogue(State(_): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    Ok(Json(
        json!({ "data": CATALOGUE.iter().map(|(t, d)| json!({ "type": t, "description": d })).collect::<Vec<_>>() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_and_matching() {
        assert!(valid_pattern("*") && valid_pattern("payment.*") && valid_pattern("payout.paid"));
        assert!(
            !valid_pattern("payments.*") && !valid_pattern("payment.nope") && !valid_pattern("")
        );
        let l = vec!["payment.*".to_string(), "invoice.created".to_string()];
        assert!(
            subscribed(Some(&l), "payment.captured") && subscribed(Some(&l), "invoice.created")
        );
        assert!(!subscribed(Some(&l), "invoice.voided") && !subscribed(Some(&l), "refund.failed"));
        assert!(
            subscribed(None, "anything") && subscribed(Some(&["*".to_string()]), "refund.failed")
        );
    }
}
