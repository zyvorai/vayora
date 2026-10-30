# Zyvor Payments: status against the brief

Legend: ✅ built and tested · 🟡 built, verified only against local simulators · ◐ partial · ❌ not built.
"Simulated" means the local mock provider (`zyvor-provider-mock`) or the in-process simulator. **Nothing here has run against a live Razorpay or Cashfree sandbox**; that needs your test credentials.

## Release 1: payments, ledger, reconciliation

| Area | State | Notes |
|---|---|---|
| Payment intents, attempts, refunds, customers | ✅ | Amounts are integer minor units. Raw provider responses kept per attempt (`raw`). |
| Authorisation / capture | 🟡 | Manual capture endpoint; relies on provider support (`manual_capture` in the capability matrix). |
| Payment links | ◐ | A checkout URL with a per-intent client secret (stored hashed). No expiry, reminders or link management UI. |
| Signed webhooks (outbound) | ✅ | HMAC-SHA256 `Zyvor-Signature` with timestamp; retries with backoff; dead-lettering; replay endpoint. |
| Razorpay adapter | 🟡 | Orders + hosted checkout, capture, refunds, lookup by receipt, settlements recon, webhook verification. Built from public docs. |
| Cashfree adapter | 🟡 | Orders, server-initiated UPI collect, refunds, lookup by `order_id`, settlement recon, webhook verification. Built from public docs. |
| Capability matrix | ✅ | Router cannot select a provider for an unsupported method or currency (tested). |
| Mobile-first checkout | ✅ | `apps/zyvor-checkout`. Resumable from the URL, clear pending/"confirming" states, accessible errors. Exercised in a browser against the mock. |
| Provider-hosted card fields | ◐ | Script wiring for Razorpay Checkout / Cashfree JS exists but is untested against live providers. No card data ever reaches Zyvor; no CVV is stored. |
| UPI intent / QR | ❌ | Only server-initiated UPI collect. |
| Safe routing | ✅ | Rules (method, amount band, prefer/exclude), configured fees, health (recent failures, unresolved payments). Every decision stores the providers considered and why. |
| Uncertain outcomes | ✅ | A timeout after the provider may have accepted produces `unknown`; no other attempt can start (through any provider) until it resolves; resolver asks the provider, never re-sends. |
| Financial ledger | ✅ | Append-only double-entry; balanced per currency in code **and** by a deferred database constraint; corrections are reversals; accounting only, it says nothing about custody. |
| Reconciliation | ✅ | Orders → payments → refunds → settlements → bank lines. Explains fees, missing settlements, duplicates, partial payments, mismatches, unmatched bank credits. Cases are assignable and resolvable. |
| Bank statement import | ◐ | JSON lines only (matched by UTR, else amount ± 3 days). No CSV/MT940/CAMT parsing. |
| Refund operations | ✅ | Full/partial, reservation-based over-refund protection, approval threshold with separation of duties, duplicate protection, unknown/failed queues, retry. |
| Organisations, test/live | ✅ | An organisation has separate test and live tenants; keys are tenant-bound. |
| Scoped API keys, audit | ✅ | Keys shown once, stored hashed, revocable; audit events are append-only. |
| Branches, users, roles | ◐ | Records exist; no login/SSO; roles are not yet mapped to scopes automatically. |
| Approval policies | ◐ | One policy: refund amount threshold. |
| OpenAPI | ✅ | `openapi/zyvor.yaml`; a test fails if a route is undocumented. Schemas are summary-level. |
| SDKs and CLI | ✅ TS · ◐ Python/Go | `@zyvor/node` (+ `zyvor` CLI) is complete; Python and Go are minimal (core calls + webhook verification). |
| Sandbox simulator, webhook inspector, replay | ✅ | `zyvor-provider-mock`, `scripts/zyvor-sandbox.sh`, `/v1/inbox`, `/v1/outbox`, replay. API only, no UI. |
| Kubernetes, Compose, Envoy TLS | ◐ | `helm lint`/`template` and `docker compose config` pass. **Not built or run** (no Docker daemon or cluster was available). |
| Prometheus | ◐ | `/metrics` exposes queue depths and open cases. |
| OpenTelemetry | ❌ | Not wired. |
| Redacted logs | ◐ | Secrets are never logged by construction; there is no generic redaction layer. |

## Acceptance tests (all passing)

| Requirement | Test |
|---|---|
| Idempotent repeat cannot create a second operation | `repeating_an_idempotent_request_cannot_create_a_second_operation` |
| Duplicate / out-of-order webhooks | `duplicate_and_out_of_order_webhooks_cannot_duplicate_postings_or_regress_state` + `state_machine` unit tests |
| Timeout after provider acceptance | `a_timeout_after_provider_acceptance_is_unresolved_never_a_second_charge` |
| Concurrent partial refunds | `concurrent_partial_refunds_cannot_exceed_the_refundable_amount` |
| Ledger balance and reversals | `ledger_balances_per_currency_corrections_are_reversals_and_history_is_immutable` |
| Cross-tenant access (APIs, exports, jobs, webhooks) | `cross_tenant_access_fails_across_apis_exports_jobs_webhooks_and_the_database` (runs as a non-superuser so row-level security is real) |
| Worker crash loses nothing | `worker_crashes_and_restarts_do_not_lose_acknowledged_events` |
| Full sandbox flow | `complete_sandbox_flow_checkout_to_invoice_to_settlement_to_partial_refund` |

## Release 2 and 3

Also built: OpenTelemetry tracing (set `OTEL_EXPORTER_OTLP_ENDPOINT`; spans carry the route template and an echoed `x-request-id`; verified against a stub OTLP collector, not a real backend), bank statement CSV import (`POST /v1/bank_statements/import_csv`), prorated upgrade plan changes, milestone billing, and `apps/zyvor-console`, a merchant console for the new API (overview, payments, refunds with approve/reject/retry, reconciliation cases, collections, billing, webhook replay); verified in a browser against the sandbox. Built in Release 2 so far: plans, subscriptions (pause/resume/cancel), metered usage, idempotent recurring invoicing (worker + `POST /v1/billing/run`), credit notes, voiding, customer statements, and a collections workspace (ageing, promises to pay, logged reminders that emit `invoice.reminder`). Not started: milestone billing, proration, reminder approvals, disputes, GST credit-note filing, and the Zyvor vertical integrations (campus fee balance, hospital deposits, factory milestones, pharmacy returns). Payment events already carry `external_ref` and `invoice_id` so those products can consume them. Also not started: AI assistance, a merchant UI for the new API (the existing dashboard still targets the legacy Vayora API), command palette, advanced routing optimisation beyond the cost suggestions.

## Deviations from the brief
- **Rust instead of Go** for the backend (decision made with the owner: it keeps the tested ledger/idempotency/outbox code).
- JSON bodies for bank-statement and settlement import instead of file formats.

## The claim to prove in pilots
Fewer unresolved payments, less reconciliation work, faster integration. None of that is demonstrated yet; the tests prove correctness of the mechanisms, not customer outcomes.
