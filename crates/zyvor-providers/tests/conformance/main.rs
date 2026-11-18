//! Adapter conformance suite: every adapter must pass every check in `harness.rs`.
//!
//! Everything here runs against SIMULATED providers (`zyvor-provider-mock`, two small fake gateways, and the
//! in-process simulator). Passing proves the adapters keep the provider contract; it says nothing about how any
//! real provider behaves. How to add an adapter: `docs/zyvor/ADD_A_PROVIDER.md`.

mod backends;
mod gateway;
mod harness;

/// One `#[tokio::test]` per check, per adapter, so a failure names the adapter and the rule.
macro_rules! suite {
    ($adapter:ident) => {
        mod $adapter {
            use crate::{backends, harness};

            macro_rules! check {
                ($name:ident) => {
                    #[tokio::test]
                    async fn $name() {
                        harness::$name(&backends::$adapter().await).await;
                    }
                };
            }
            check!(capabilities_are_honest);
            check!(captured_payment_round_trip);
            check!(declined_payment_is_failed);
            check!(definite_failure_vs_uncertain_outcome);
            check!(lost_response_is_resolved_by_lookup);
            check!(initiate_twice_is_safe);
            check!(amounts_round_trip_in_paise);
            check!(refunds_full_and_partial);
            check!(lost_refund_response_is_resolved_by_lookup);
            check!(unknown_payment_reference);
            check!(settlement_report_parsing);
            check!(webhook_verification);
            check!(refund_webhooks);
            check!(secrets_never_leave);
        }
    };
}

suite!(razorpay);
suite!(cashfree);
suite!(payu);
suite!(custom);
suite!(sponsor_upi);
suite!(phonepe);
suite!(sim);
