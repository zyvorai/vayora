# Acceptance specification mapping (spec §17)

The product brief asks for twenty money-safety acceptance cases. This file maps each requirement to the test that
proves it. Providers in these tests are the local mock or in-process simulator — **not** live gateways.

| # | Requirement | Test |
|---|---|---|
| 1 | Idempotent repeat cannot create a second operation | `acceptance::repeating_an_idempotent_request_cannot_create_a_second_operation` |
| 2 | Concurrent identical attempts create one charge | same test (eight-way race on attempts) |
| 3 | Duplicate / out-of-order webhooks cannot duplicate postings or regress state | `acceptance::duplicate_and_out_of_order_webhooks_cannot_duplicate_postings_or_regress_state` + `state_machine` unit tests |
| 4 | Timeout after provider acceptance stays unresolved; never a second charge | `acceptance::a_timeout_after_provider_acceptance_is_unresolved_never_a_second_charge` |
| 5 | An attempt the provider never saw resolves to failed and unblocks the intent | `acceptance::an_attempt_the_provider_never_saw_resolves_to_failed_and_unblocks_the_intent` |
| 6 | Concurrent partial refunds cannot exceed the refundable amount | `acceptance::concurrent_partial_refunds_cannot_exceed_the_refundable_amount` |
| 7 | Ledger balances per currency; corrections are reversals; history is immutable | `acceptance::ledger_balances_per_currency_corrections_are_reversals_and_history_is_immutable` |
| 8 | Cross-tenant access fails across APIs, exports, jobs, webhooks and the database | `acceptance::cross_tenant_access_fails_across_apis_exports_jobs_webhooks_and_the_database` |
| 9 | Worker crashes and restarts do not lose acknowledged events | `acceptance::worker_crashes_and_restarts_do_not_lose_acknowledged_events` |
| 10 | A reused provider reference opens a case instead of wedging the payment | `acceptance::a_reused_provider_reference_opens_a_case_instead_of_wedging_the_payment` |
| 11 | Full sandbox flow: checkout → invoice → settlement → partial refund | `flow::complete_sandbox_flow_checkout_to_invoice_to_settlement_to_partial_refund` |
| 12 | Webhook URLs cannot target private networks when saved or delivered | `flow::webhook_urls_cannot_target_private_networks_when_saved_or_delivered` |
| 13 | Large refunds need a second person's approval | `flow::large_refunds_need_a_second_persons_approval` |
| 14 | Unknown refund outcomes stay reserved; failures go to a queue | `flow::refund_outcomes_that_are_unknown_stay_reserved_and_failures_go_to_a_queue` |
| 15 | Routing respects capabilities, picks by cost and records why | `flow::routing_respects_capabilities_picks_by_cost_and_records_why` |
| 16 | API keys are scoped, revocable; secrets encrypted at rest | `flow::api_keys_are_scoped_revocable_and_secrets_are_encrypted_at_rest` |
| 17 | Public checkout is secret-scoped, resumable and shows clear states | `flow::public_checkout_is_secret_scoped_resumable_and_shows_clear_states` |
| 18 | Disputes hold money; a lost dispute hits the ledger and reopens the invoice | `flow::disputes_hold_the_money_and_a_lost_one_hits_the_ledger_and_reopens_the_invoice` |
| 19 | Audit trail is sealed in a chain that shows any edit or deletion | `flow::the_audit_trail_is_sealed_in_a_chain_that_shows_any_edit_or_deletion` |
| 20 | Reconciliation turns unexplained differences into assignable cases | `flow::reconciliation_turns_every_unexplained_difference_into_an_assigned_task` |

Run them with `cargo test -p zyvor-api --test acceptance --test flow` (Postgres required; see `crates/zyvor-api/tests/common`).
