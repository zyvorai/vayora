// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Whose fault was a declined payment?
//!
//! Routing wants to know whether a provider is *failing*, not whether payers are short of money. A decline
//! caused by the payer (insufficient funds, the payer left or turned the request down) says nothing about the
//! provider and must not count against it. A decline caused by the provider or the bank side counts against it.
//! Anything not recognised is [`DeclineClass::Unknown`], which routing treats as a provider-side failure: the
//! conservative choice, because wrongly excusing a failing provider hides an outage while wrongly counting a
//! payer decline only makes the breaker a little more cautious.
//!
//! Only strings and codes that appear in this repository's adapters, simulators, mocks, fixtures or documents are
//! recognised. They come from the simulator and mock, from Cashfree's documented payment statuses and from the
//! failure text its subscription tests use. **Nothing here was checked against a live provider**, and real
//! providers have many more codes. Every code that is not listed here is `Unknown` until a real sandbox run
//! shows what it means (see `docs/zyvor/REAL_SANDBOX.md`).

use serde_json::Value;

/// Who caused a decline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclineClass {
    /// The payer's doing: not enough money, the payer abandoned or refused the payment. Not the provider's fault.
    Payer,
    /// The provider or the bank side failed (provider error, technical failure).
    Provider,
    /// Not recognised. Counted as a provider-side failure.
    Unknown,
}

/// Lower-case, with underscores, hyphens and runs of spaces collapsed to single spaces.
fn normalise(s: &str) -> String {
    s.split(|c: char| c == '_' || c == '-' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reason texts the simulator and the Cashfree subscription tests use, shared by every adapter (including a
/// custom or sponsor-bank one that sends the same words).
fn generic(reason: &str) -> DeclineClass {
    match normalise(reason).as_str() {
        // the simulator's declines by UPI handle, by amount and for a mandate debit; Cashfree's subscription
        // failure text in the repository's tests
        "insufficient funds" => DeclineClass::Payer,
        // the simulator's payer approving nothing: a declined QR / intent or a refused mandate
        "declined by payer" => DeclineClass::Payer,
        // the simulator's provider-side failure of a pending payment
        "failed at provider" => DeclineClass::Provider,
        _ => DeclineClass::Unknown,
    }
}

/// Classify a failed payment. `provider` is the adapter name (`razorpay`, `cashfree`, `phonepe`, `sponsor_upi`,
/// `sim`, or a custom one), `reason` the provider's own failure text and `raw` the payload it came from (the
/// payment, or a webhook around it).
///
/// What is recognised, and what is deliberately not:
///
/// - Cashfree: a payment whose status is `USER_DROPPED` (or whose webhook is `PAYMENT_USER_DROPPED_WEBHOOK`) is the
///   payer leaving, so `Payer`. `FAILED`, `CANCELLED` and `VOID` say nothing about whose fault it was, and the mock's
///   "Payment declined by payer bank" names the payer's bank, not the payer: `Unknown`.
/// - Razorpay and PhonePe: the repository only shows generic statuses (`failed`, `PAYMENT_ERROR`, `PAYMENT_DECLINED`)
///   and the mock text "Payment failed at bank". None says whose fault it was: `Unknown`.
/// - The simulator and any adapter that sends the same plain words: see [`generic`].
pub fn classify_decline(provider: &str, reason: Option<&str>, raw: &Value) -> DeclineClass {
    if provider == "cashfree" {
        let status = [
            &raw["payment_status"],
            &raw["data"]["payment"]["payment_status"],
        ]
        .into_iter()
        .find_map(|v| v.as_str());
        if status == Some("USER_DROPPED")
            || raw["type"].as_str() == Some("PAYMENT_USER_DROPPED_WEBHOOK")
        {
            return DeclineClass::Payer;
        }
    }
    reason.map_or(DeclineClass::Unknown, generic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn simulator_and_subscription_reasons_are_recognised_whatever_the_spelling() {
        for (r, want) in [
            ("insufficient_funds", DeclineClass::Payer),
            ("insufficient funds", DeclineClass::Payer),
            ("  Insufficient-Funds ", DeclineClass::Payer),
            ("declined_by_payer", DeclineClass::Payer),
            ("failed_at_provider", DeclineClass::Provider),
        ] {
            for p in ["sim", "cashfree", "sponsor_upi", "custom"] {
                assert_eq!(
                    classify_decline(p, Some(r), &Value::Null),
                    want,
                    "{p} / {r}"
                );
            }
        }
    }

    #[test]
    fn anything_else_is_unknown() {
        for r in [
            "simulated_failure",
            "simulated_token_decline",
            "Payment failed at bank",
            "Payment declined by payer bank",
            "PAYMENT_DECLINED",
            "insufficient_balance",
            "",
        ] {
            assert_eq!(
                classify_decline("razorpay", Some(r), &json!({})),
                DeclineClass::Unknown,
                "{r}"
            );
        }
        assert_eq!(
            classify_decline("phonepe", None, &Value::Null),
            DeclineClass::Unknown
        );
    }

    #[test]
    fn cashfree_user_dropped_is_the_payer_but_failed_cancelled_void_are_not() {
        let pay = |s: &str| json!({ "payment_status": s });
        assert_eq!(
            classify_decline("cashfree", None, &pay("USER_DROPPED")),
            DeclineClass::Payer
        );
        // inside a webhook body, and by the webhook type
        assert_eq!(
            classify_decline(
                "cashfree",
                None,
                &json!({ "data": { "payment": { "payment_status": "USER_DROPPED" } } })
            ),
            DeclineClass::Payer
        );
        assert_eq!(
            classify_decline(
                "cashfree",
                None,
                &json!({ "type": "PAYMENT_USER_DROPPED_WEBHOOK" })
            ),
            DeclineClass::Payer
        );
        for s in ["FAILED", "CANCELLED", "VOID"] {
            assert_eq!(
                classify_decline("cashfree", None, &pay(s)),
                DeclineClass::Unknown,
                "{s}"
            );
        }
        // the status is read for Cashfree only
        assert_eq!(
            classify_decline("razorpay", None, &pay("USER_DROPPED")),
            DeclineClass::Unknown
        );
    }
}
