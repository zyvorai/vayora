# Changelog

What shipped, newest first. Everything here is verified against local simulators and a k3s sandbox, except Cashfree UPI
intent and QR, which was also run against Cashfree's own sandbox. Nothing has run against a live provider. See
[docs/zyvor/STATUS.md](docs/zyvor/STATUS.md) for what is not built.


## 18 November 2026 — Events and ticketing (simulators only)
- **Events** with ticket tiers (price in paise, quantity cap, per-order maximum, sale window, early-bird price and window, optional per-tier payment methods), a registration form (text, email, phone, select, checkbox; required, length limits) and optional vendor shares. Draft, on sale, closed, cancelled.
- **Seats never oversold:** held and sold counters on the event and each tier change only under the event row lock, with a database check that held + sold never passes capacity. An unpaid order holds its seats for a configurable time; the worker releases it (not while a payment is authorised or unknown), gives its coupon use back and cancels its intent. A payment that arrives after the release takes free seats back or is queued for a refund with a reconciliation case.
- **Payment:** an order creates an ordinary payment intent through the existing routing; tickets are issued in the same transaction as the verified capture, once (one row per order and seat). Coupons, vendor splits, refunds and idempotency keys are the existing ones. Free tiers need no payment.
- **Check-in:** each ticket has a signed QR payload (HMAC bound to its event) and a code; a scoped `events:checkin` key can scan, sync offline scans, export valid tickets and read counts, and nothing else. A second scan is reported with who and when; two devices at once admit a ticket once.
- **Attendees:** search and a CSV export (formula-defused). **Cancellation:** cancelling an event or an order refunds through the ordinary refund flow in a tracked batch (the over-refund guard and the approval threshold apply; the requester cannot approve), with progress and a per-event refund report.
- **Public pages** in `apps/zyvor-checkout`: event page, registration form, order and ticket page with QR codes (English; Hindi partly, machine-assisted and unreviewed).
- Migration 0081 (nine tables, all with row-level security), new scopes `events:read`, `events:write`, `events:checkin`, four webhook event types, OpenAPI and generated SDK types updated, `refunds::reserve_refund` extracted from the refund endpoint so bulk refunds share it. See [docs/zyvor/EVENTS.md](docs/zyvor/EVENTS.md).
- **Not done:** partial payments with reminders (the instalment mechanism does not fit a one-off ticket order), a door app, console pages. QR check-in was never tried on a device and the public pages were not run in a browser.

## 18 November 2026 — MCP server for AI tools (sandbox; tested with an SDK client only)
- **`packages/zyvor-mcp`** (`@zyvor/mcp`, private, not published): a Model Context Protocol server over `@zyvor/node`, built on the official `@modelcontextprotocol/server` 2.x, local stdio only. **Read-only by default** with 18 tools: `whoami`, payments and attempts, payment timeline, `explain_routing_decision`, refunds, settlements and `explain_settlement`, bank lines, fee findings and fee check, routing health, reconciliation cases and their suggestions, payment links.
- **Opt-in write tools** (`ZYVOR_MCP_ENABLE_WRITES=1`; not registered at all otherwise): `create_payment_link` (capped), `request_refund` (still goes through the approval policy: the server reads the approval threshold first and refuses a refund the API would start without a second person, unless the operator raises `ZYVOR_MCP_MAX_UNAPPROVED_REFUND_MINOR`), `confirm_bank_match`, `reject_bank_match`. Destructive tools need a `confirm` that repeats the id. Write tools are refused when the key holds `admin` or any approve scope, and on a live key unless `ZYVOR_MCP_ALLOW_LIVE_WRITES=1`. Never exposed: key management, approvals, payouts, settings, provider credentials, webhooks.
- The API key comes only from `ZYVOR_API_KEY`; no tool takes a credential, nothing logs it, API error text is redacted. Responses are redacted (the API's key/card/UPI rules plus emails, phones, PAN, Aadhaar-shaped numbers, raw provider payloads, bank narrations withheld by default) and free text is stripped of control and bidi characters, collapsed to one line and cut, under a notice that it is data, not instructions.
- Docs: [docs/zyvor/AI_INTEGRATION.md](docs/zyvor/AI_INTEGRATION.md) (client setup, tool list with scopes, threat model for prompt injection, honest limits). A scripted sandbox demo (`pnpm --filter @zyvor/mcp demo`) creates a payment link and explains a made-up settlement.
- 48 vitest tests: schema validity and examples for every tool, write tools absent without the flag, confirm echo, approval-policy fences, redaction, config, and the built server spawned over stdio. Run once against a locally built sandbox API with synthetic data. **Not tried in any real MCP host, with any language model, or against a live provider.** No API, migration or Rust change.

## 18 November 2026 — Console pages for fee checks, bank statements and routing health
- **Fee checks** (`/fees`): fee rules (percentage, flat, cap, GST on the fee, refund fee return, effective date), tolerance and owner, findings and summary (took more / took less, by provider and method), run the check. Rates are typed by a person; none are built in.
- **Bank statements** (`/bank-statements`): upload a CSV with a layout or your own column names, check it first (nothing saved, line-level problems listed), import, past uploads, lines with the evidence for each match, suggestions labelled as needing a person with confirm and reject, ageing and open cases. The old paste box on Reconciliation now points here.
- **Routing health** (`/routing`): recent success rate (or "not enough data" under the minimum), saved versus current state in plain words, recent set-aside and recovery events, and the limits (admin only to change).
- Writes need a finance role (limits: admin); viewers can read. Console only, no API change. Checked in a browser against a stand-in API with made-up data; not against a running Zyvor API or a real bank file.

## 18 November 2026 — Decline classes, health sweeper, one health rule, routing explanations (simulators only)
**Routing health tells the truth**
- Declines are classified when recorded (`payment_attempts.failure_class`: `payer_declined`, `provider_fault`, `declined` = unknown, `provider_rejected`, `cancelled`). Payer declines (insufficient funds, the payer left) are left out of the success rate and the circuit breaker, shown as `payer_declines` in `GET /v1/routing/health`, and are not a reason to fail over. Unknown causes still count against the provider. Only codes found in the repository's simulator, mock and tests are recognised; nothing was checked against a live provider.
- Worker health sweeper: a quiet business degrades or recovers without a new payment, once per transition across worker replicas (advisory lock plus a conditional update), writing the same `provider.degraded` / `provider.recovered` audit entry and event. It never starts a payment or takes a probe slot.
- **Behaviour change:** the router's older heuristic (any unresolved payment, or most of the last 50 attempts failing, only reordered providers) is removed. The circuit breaker is the one health rule, and `GET /v1/routing/health` now shows `routing_effect` for each provider and method, so it cannot say healthy while routing skips. One unresolved payment alone no longer deprioritises a provider; with the breaker switched off routing ignores health entirely.
- Every `route.considered` entry and `route.explanation` carry a plain-language sentence ("Chosen because it has the lowest estimated fee ..."). The reason `only eligible provider` is no longer shown when another provider was set aside; it names what was skipped.
- Migration 0079 (widens the allowed `failure_class` values; additive). OpenAPI and generated SDK types updated. See [docs/zyvor/ROUTING.md](docs/zyvor/ROUTING.md).

## 18 November 2026 — SDK methods for fee checks, bank statements and routing health
- Node, Python and Go SDKs gain typed methods for fee rules, fee checks (settings, summary, findings, run, per-settlement check), bank statements (preview, layouts, imports, lines, rematch, match confirm and reject), reconciliation ageing and routing health with its settings. CSV import and preview accept the layout options. The Node CLI gains matching read commands. Tested with mocked HTTP only; no server change.

## 18 November 2026 — Adapter conformance suite and a PayU adapter (simulators only)
**One suite every adapter must pass, and a PayU adapter built from PayU's public documentation**
- `crates/zyvor-providers/tests/conformance`: a reusable harness run against Razorpay, Cashfree, PayU, `custom`, `sponsor_upi`, PhonePe and the simulator (98 checks). It covers Rejected versus Uncertain mapping, a response lost after the provider acted, idempotent create, refunds (partial, full, over, repeated), webhook signatures (valid, tampered, wrong secret, missing, replayed, out of order), settlement parsing, paise round trips, currency handling and capability honesty. Faults are injected by a proxy in front of each simulated provider.
- Bugs it found and fixed: an unreadable 200 was read as an empty answer, so Razorpay's lookup could report an existing payment or refund as "not found"; Razorpay's lookup ignored a second order for the same attempt; Cashfree, PhonePe and the simulator accepted a currency they do not declare (every adapter now refuses an undeclared method or currency and a non-positive amount before sending); PhonePe treated a refused connection as unknown, dropped the refund reference on lookup and skipped the replay window when a timestamp was missing; `custom` and PhonePe now treat a refund the gateway never saw as a definite not-found; the `custom` adapter now sends `upi_intent` so a declared intent capability can be used. See [docs/zyvor/ADD_A_PROVIDER.md](docs/zyvor/ADD_A_PROVIDER.md).
- PayU (`provider: payu`): hosted checkout with the SHA-512 request hash, Verify Payment, refunds with a derived 23-character token, `check_action_status`, reverse-hash webhooks, the Settlement Detail Range report. Claims only what the documentation supports. PayU's refund callback has no documented signature and is refused (refund state is polled). Migration 0080 adds `payu` to the provider list. Mock provider: PayU endpoints and `/_sim/payu/*` controls.
- **Mock only. Built from PayU's public documentation; not verified against a live sandbox or account.** What to check first: [docs/zyvor/PAYU.md](docs/zyvor/PAYU.md). No console form for PayU yet.
- Docs: [ADD_A_PROVIDER.md](docs/zyvor/ADD_A_PROVIDER.md) (steps and the conformance checklist, so Paytm PG or Easebuzz is a day's work), [PAYU.md](docs/zyvor/PAYU.md), PROVIDER_CONTRACT (`upi_intent` field; a 404 on a refund lookup).

## 18 November 2026 — Success-rate routing, circuit breaker and failover (simulators only)
**Routing**
- Per-tenant rolling success rate per provider and method, from stored attempts, ignored below a minimum sample; rules can set `prefer_success_rate`.
- Circuit breaker per provider and method: degraded providers are skipped for new payments, one probe payment after a cool-down, automatic recovery; `provider.degraded` / `provider.recovered` audit entries and webhook events.
- Failover: after a definitive failure (provider refused or declined) the next attempt may use another provider; never after an unknown outcome, which still blocks any new attempt until the provider is asked.
- Every routing decision now records each provider's signal and why it was skipped or chosen.
- API: `GET /v1/routing/health`, `GET|PUT /v1/routing/health/settings`. Migration 0076. Mock provider: sticky `POST /_sim/outage`.
- Tested against simulators only; defaults not tuned on real traffic. See [docs/zyvor/ROUTING.md](docs/zyvor/ROUTING.md).

## 18 November 2026 — Fee checks on settlements
**What a provider deducted, against your own contract (simulators only)**
- Fee rules you enter (percentage in basis points, flat paise, cap, GST on the fee, refund fee return, effective-from date) and a tolerance in paise. No rate for any provider, method or network is built in; with no rule the answer is "no expected fee configured" and nothing is flagged.
- Each settlement is checked per payment as it is ingested (and on demand): expected vs actual deduction, the rule applied, over- and under-deduction. A variance above tolerance becomes one `fee_check` reconciliation case per settlement (assigned to the configured owner), and the settlement explanation says "Fee differs from your configured rate by X". Findings never change money or the ledger.
- API: `/v1/fee_rules`, `/v1/fee_checks/{settings,summary,findings,run}`, `/v1/settlements/{id}/fee_check`; OpenAPI and generated SDK types updated; migration 0078 with row-level security. See `docs/zyvor/FEE_CHECKS.md`.
- Not checked against any real provider statement. No console page yet.


## 18 November 2026 — Bank statement import and UTR/RRN matching
**Simulators and synthetic statements only; nothing has run against a real bank export**
- CSV import: named layouts (hdfc, icici, sbi, axis, kotak, generic) or your own column mapping, delimiter and date format; Indian and western amount grouping, Dr/Cr columns and suffixes; amounts as whole paise. All or nothing per file, every problem reported with its line, plus `POST /v1/bank_statements/preview` (dry run) and `GET /v1/bank_statements/layouts`.
- Re-uploading a file, or an overlapping one, adds no duplicate lines (per-line fingerprint); every upload is recorded (`GET /v1/bank_statements/imports`).
- Matching records its evidence (`bank_line_matches`): a UTR/RRN found as a whole token in the reference column or narration is confirmed; **amount and date alone is now only a suggestion** (it used to link automatically) that a person confirms or rejects (`/v1/bank_statements/matches/{id}/confirm|reject`). Debits match payouts and refunds, credits match payments by RRN.
- Settlements with no bank credit and paid payouts with no bank debit become cases once a statement covers the period; cases carry `age_days` and `GET /v1/recon/ageing`; the unmatched-credit case closes when the line is matched.
- Payments store the provider's bank reference (`payment_attempts.bank_reference`) when it is reported: simulator, local mock, and the fields Razorpay and Cashfree document.
- Settlement explanation shows the matched bank line, the rule and key, suggestions, and the bank-versus-expected difference with its reasons.
- Migration 0077 (additive). OpenAPI, SDK types and docs ([docs/zyvor/BANK_RECONCILIATION.md](docs/zyvor/BANK_RECONCILIATION.md)) updated.

## 7 November 2026 — Android parity follow-through
**More iOS test-mode extras on Android**
- Pay: Receive QR (`vayora://request` open amount), category memory on settle, open-amount typing on review.
- Merchant: tip chips + repeat last amount on Collect; hourly takings chart; shareable plain-text receipt.
- Kit: `RequestPayload`, `Tip`, `HourlyTotals`, `ReceiptText`, `CategoryMemory` + tests.

## 7 November 2026 — Android parity complete (post v0.3.0)
**In-repo iOS→Android test-mode gaps closed**
- Kit: connection/setup, mandates, jars, lock/scam hints, budgets, Circle payload, voice parse, payees, ops models + client (circle sim, branches/cash/closings) and unit tests.
- Merchant: Owed, Cashflow/GST/settlements, AutoPay, end of day (cash + close), setup QR, Test connection.
- Pay: mandate approve, jars, budgets (review warning), Circle create/join/pay-from, voice pay, biometric lock, scam hints, Test connection.
- Physical-device camera/voice CI and EXTERNAL (sponsor LOI, RBI PA, NPCI TPAP, live sandboxes, PCI vault) remain open.

## 7 November 2026 — v0.3.0
**Android apps (Compose, like iOS)**
- `apps/android`: Vayora Pay + Vayora Merchant, shared `vayora-kit` / `vayora-ui` (Money, hostile-safe UPI parse, API client, indigo/emerald dark tokens).
- Core flows: Merchant Collect → QR (demo auto-pay or sandbox); Pay scan or paste `upi://pay` → test PIN → on-phone wallet; Settings for server / demo.
- Camera QR scan: CameraX + ZXing viewfinder (permission, torch, dimmed brackets) into the same review/PIN path as paste.
- CI: kit unit tests + debug APK assemble; Android SDK packages without the removed `tools` package; promtool via image entrypoint; path-crate versions aligned; clippy/fmt clean.
- Most iOS extras (Owed, AutoPay, jars, Circle) not ported yet. Sponsor LOI, RBI PA, NPCI TPAP, live sandboxes and PCI vault remain operator / external work.

## 7 November 2026 — v0.2.0
**In-repo PG path closed further (simulators only)**
- Bank statements: SWIFT MT940 and ISO 20022 CAMT.053 import alongside CSV/JSON; console and SDKs.
- Webhooks: per-attempt outbox delivery history; Cashfree mandate status/authorization webhooks refresh the mandate; checkout SSE for payment progress; Developers inbox UI.
- Providers: Razorpay UPI intent/QR opt-in (docs + mock); generic log redaction on stderr; promtool CI for alert rules; WooCommerce and Shopify release zips.
- Ops / console: authenticated API request log (`GET /v1/api_requests`) on Developers; `GET /v1/me` with shared `scopes_for_role`; recon “scan for missing settlements”; open a dispute from a payment; force the next simulator refund to fail (`.13` paise / `POST /v1/sim/fail_next_refund`); organisation test clock drives billing, dunning, mandate end dates, payment-link expiry reminders and cashflow insights.
- Deploy: `scripts/compose-smoke.sh`; CI validates `docker compose config`; acceptance map (`ACCEPTANCE_SPEC.md`).
- **Docs and GitHub Pages:** STATUS and IMPLEMENTATION_STATUS refreshed; site FAQ/self-host list for plugins, bank formats, compose smoke and Developers inspector. Sponsor LOI, RBI PA, NPCI TPAP, live provider sandboxes and PCI vault remain operator / external work.

## 6 November 2026
**Own-gateway path: orchestrate vs acquire**
- Organisations have `money_mode` (`orchestrate` default, `acquire` for sponsor/PA custody). Capture, refund and dispute ledger postings branch on mode; live acquire needs `ZYVOR_ALLOW_LIVE_ACQUIRE=1` and approved acquire KYC.
- New crates: `zyvor-settle` (escrow batches, payout rails, escrow proof) and `zyvor-cards` (network tokens only; PAN/CVV refused).
- Providers: `sponsor_upi` (TPAP/sponsor PSP contract) and `phonepe` (merchant PhonePe PG). Settlement batch APIs, `POST /v1/payouts/{id}/send_rail`, card token APIs, `GET/PUT /v1/acquire_kyc`.
- Risk scoring (0–100) with optional `score_block_below`; Shopify/WooCommerce plugin scaffolds under `plugins/`.
- Compliance pack: dual-mode REGULATORY, CERT-In runbook, acquire KYC/AML, PCI scope, own-licences track, ADR 0004. Sponsor LOI / RBI PA / NPCI TPAP remain operator actions.
- **Docs and GitHub Pages:** [ACQUIRE_MODE.md](docs/zyvor/ACQUIRE_MODE.md) operator guide; HOW_IT_WORKS, ARCHITECTURE, MARKET, IMPLEMENTATION_STATUS, THREAT_MODEL, README and the site hero/FAQ/footer updated for dual money modes, PhonePe/sponsor adapters, settle rails and card tokens.
- **Acquire completion (in-repo):** captures/refunds auto-enrol into settlement batches; worker releases after `settlement_delay_days`; RLS on batches/items/card_tokens; charge with `card_token_id` (sim stub vault); Razorpay recurring/mandates adapter (docs + local mock); Magento 2 plugin scaffold.
- **Follow-through:** OpenAPI + Node/Python/Go `card_token_id` on attempts; console Settlement page (batches, escrow proof, card tokens); Magento/Woo/Shopify webhook mark-paid; factory + pharmacy examples.
- **Docs/Pages + allowlist:** return_url origin allowlist; bank CSV import in console/SDK; site FAQ/footer/examples updated for Settlement, plugins and verticals.

## 1 November 2026
**Merchant: items, bills and sales by item (on-phone, test mode)**
- A product list with optional GST lines, itemised totals on Collect, and top sellers. No server change.
**Pay and Merchant: Test connection (on-phone, test mode)**
- A Settings row that checks the server and the key and explains failures in plain words. No server change.
**Pay: your week (on-phone, test mode)**
- A weekly recap from the phone's own history. No server change.
**Pay and Merchant: demo mode (on-phone, test mode)**
- A built-in test bank so the apps work with no server. No server change.
**Pay: shareable payment card (on-phone, test mode)**
- A picture of a finished payment with the UPI ID masked. No server change.
**Merchant: refund reasons and notes (on-phone, test mode)**
- A reason and note when refunding, and a 7-day refund pattern line. No server change (the existing refund `reason` field is used).
**Pay and Merchant: Reduce Motion, larger customer view (on-phone)**
- UI only.

**Cashfree Subscriptions adapter for UPI AutoPay (mocked, unverified)**
- Cashfree can now hold a mandate: create (an on-demand subscription plus a payer approval link), refresh, revoke, and debit (a `CHARGE` keyed by our attempt id, with an idempotency key derived from it). `mandates` is on for Cashfree. Built from Cashfree's public docs and tested only against a local mock HTTP server; **not run against Cashfree's sandbox**. Unclear points are marked `UNCLEAR` in the code and listed in the pull request.
- A debit whose outcome is unknown is resolved by asking Cashfree for the charge (new `Provider::fetch_debit`, used by the refresh of an attempt that belongs to a mandate), never by sending it again; a charge Cashfree cannot find stays unknown rather than failed.
- `SUBSCRIPTION_PAYMENT_SUCCESS/FAILED/CANCELLED` webhooks for charges go through the existing Cashfree signature check and become payment events. Status-change and authorization webhooks are accepted and ignored.
- With both Cashfree and the simulator connected, a mandate with no `provider` now goes to Cashfree (first by name); pass `"provider": "sim"` to use the simulator.

## 31 October 2026
**People page and FX exposure**
- Fix: a settlement refund line that names our refund (as the simulated settlement does) no longer opens a false "matches none of our records" case.
- Test mode: billing and reminder runs accept `as_of`, to see what a later day would bring (a partial test clock).
- Test mode: `POST /v1/payment_attempts/{id}/simulate_event` settles a pending simulated payment as captured or failed, standing in for a provider's late update.
- `GET /v1/console/branches` lists the branches a person can be tied to, read from the home environment that `invite` checks. The People page uses it, so a cashier can be invited with a branch while looking at live.
- Payments can be searched: `q=` on `GET /v1/payment_intents` matches a payment ID exactly or part of its reference or description (case-insensitive, `%` and `_` are plain text), with a search box on the Payments page and in its CSV export. Python (`list_payments`) and Go (`ListPayments`) can list and search payments too; Node already could.
- The Sandbox page can pay by UPI, card, net banking or wallet. The simulator still decides the outcome from the handle (no card number is asked for); a flow test covers all three non-UPI methods.
- Settlement simulation: `POST /v1/settlements/sync` with provider `sim` builds a day's settlement from the organisation's own simulated payments (gross, an invented 2% fee, refunds), once per day, tied to those payments so it can explain itself. Reconciliation has a button for it in test mode.
- `POST /v1/settlements/{id}/simulate_bank` (test mode, simulated settlements only) adds the bank credit the settlement would produce, in full or short by an amount, and matches it the normal way, so the whole payment → settlement → bank → case path can be tried without a provider. Buttons on Reconciliation.
- UPI AutoPay mandates in all three SDKs: create, list, get, refresh/pause/resume/revoke, schedule a debit and list debits (Node `mandates.*`, Python `create_mandate` and friends, Go `CreateMandate`, `GetMandate`, `MandateAction`, `ScheduleDebit`). Each checks its arguments before any request. Simulator only until a live provider supports mandates.
- `/v1/fx/exposure` reports `complete: true` when no open dollar invoice lacks a rate (it said false with no dollar invoices).

**iOS test apps**
- `apps/ios`: **Vayora Pay** (a payer: scan, slide to pay, PIN, receipt, History, Settings) and **Vayora Merchant** (counter keypad, payment QR with Customer view, Today dashboard, refunds, setup by QR), on a shared `VayoraKit` and `VayoraUI` design system. Run on two physical iPhones against the simulator; no real money. A `VayoraGallery` tool renders the screens to PNG for review without a simulator.
- `POST /v1/sim/pay` lets a test payer approve or decline the simulator's UPI payload. Off unless `ZYVOR_SIM_PAYER=1` (404 otherwise); only touches the in-process simulator.
- Pay also has test-mode payees, send, split a bill (a request QR), small payments without a PIN (off by default) and bill reminders. These person-to-person items are recorded on the phone and never reach Zyvor's ledger.

**Merchant: share QR, accountant export, staff tags (on-phone, test mode)**
- Share the payment QR as an image, export payments and daily totals as CSV, and tag sales with who was on the till. No server change.
**Pay: working app lock, privacy, backup (on-phone, test mode)**
- Face ID lock that actually locks, an app-switcher cover, hide balance, scam warning on the review screen, and backup and restore of the phone's data. Merchant's Info.plist now has a local-network usage string. No server change.
**Pay: voice pay (on-phone, test mode)**
- A spoken payment in English or Hindi that opens the usual review for a saved payee. Never pays by itself. No server change.
**Circle (simulator, test mode)**
- `POST /v1/sim/circle`, `GET /v1/sim/circle/{id}`, `POST /v1/sim/circle/revoke` and `POST /v1/sim/circle/pay`: a primary payer lets a delegate pay from their account up to a per-payment and a total limit, played by the simulated bank. The amount is read from the payment, never from the request. Off unless `ZYVOR_SIM_PAYER=1`; only the in-process simulator. A Circle tab in Pay. The real rules of UPI Circle are not verified.

**Pay: receipts vault and split by item (on-phone, test mode)**
- Notes, tags and photos on payments, a month PDF, and itemised splits with exact shares. No server change.

**Pay: learns categories and jars (on-phone, test mode)**
- Recategorise a payee once and it sticks; savings jars with goals that conserve the test wallet. No server change.

**Pay: Siri and Shortcuts (on-phone, test mode)**
- Two App Intents: spend this month and test balance. No server change.

**Merchant: cashier mode, closing reminder, regulars (on-phone, test mode)**
- An owner-PIN cashier lock, a daily closing reminder, and per-name running totals. No server change. Counter tagging of payments is not built: the payment-intent API has no counter field.

**Pay: planned payments, repeating payments, split tracker (on-phone, test mode)**
- A Planned tab in Bills: payments you confirm on the day, repeating payments found in your history, and a hand-ticked split tracker. No server change.

**Merchant: counter speaker and end of day (test mode)**
- A spoken "received" in English or Hindi, and a guided end-of-day close over the existing cash entry and closing APIs. No server change.

**Pay: safety checks, Pay again, Ask your money (on-phone, test mode)**
- Warnings before the slide (first-time payee, lookalike UPI ID, unusually large), a 10-second pause for a large first payment, a Pay again row on Home, and plain-word questions in History with a total. No server change.

**Pay and Merchant extras (on-phone, test mode)**
- Pay: monthly budgets with a warning on the review screen, History filters and CSV export, and a "Receive" QR with no amount. Merchant: tips on the keypad, repeat last amount, takings by the hour, and a shareable receipt. No server change.

**AutoPay screens in the iOS apps**
- Pay: scan a mandate QR, review, slide and PIN to approve; an AutoPay list inside Bills with debits taken, and stop. Merchant: an AutoPay tab to create (with QR), schedule debits, pause, resume, revoke, and a sandbox Run now.
- `GET /v1/sim/mandate/{id}` (simulated payer's view of a mandate and the debits it took; off unless `ZYVOR_SIM_PAYER=1`). The simulated mandate link now carries the start and end dates.

**UPI AutoPay mandates (simulator)**
- Create a mandate for the payer to approve, then schedule debits (`POST /v1/mandates`, `/v1/mandates/{id}/debits`, refresh, pause, resume, revoke); a debit is an ordinary payment attempt, so the ledger, events and unknown-outcome resolution are the existing ones. Rules enforced in code: a per-debit cap, the mandate's dates and currency, a pre-debit notice window, one debit per frequency period and never two on one day. A mandate with a subscription collects its invoices automatically. Migration `0037_mandates.sql` (row-level security forced), scopes `mandates:read` and `mandates:write`, ten `mandate.*` events, and the simulated payer `POST /v1/sim/mandate`. `ZYVOR_MANDATE_MAX_MINOR` and `ZYVOR_MANDATE_NOTICE_HOURS` are configurable. No real provider adapter yet, and the rules are not verified against NPCI's circular (see `docs/zyvor/REGULATORY.md`).

**Providers and checkout**
- Cashfree native UPI intent and QR through Order Pay, opt-in per account, **verified against Cashfree's sandbox** (the first capability at level 2). Partner setup and the end-to-end test plan are in `docs/zyvor/PARTNER_SETUP.md`.
- UPI QR and intent checkout: no UPI ID to type when a provider can issue a payload; a QR is only a way to present it, never proof of payment.
- Bring your own provider: a generic `custom` adapter over a small JSON contract (`docs/zyvor/PROVIDER_CONTRACT.md`), with a stub-gateway test suite.
- A support reference on every checkout state.

**Repository**
- Removed the legacy single-merchant prototype (`vayora-*` crates, `apps/dashboard`, `apps/checkout`, `@vayora/node`, `scripts/seed.mjs`, the old Compose file). Nothing current depended on them; ADR 0002 is marked superseded.
- Fixed the OpenAPI spec (`POST /v1/sim/pay` was undocumented, which failed `ci`) and the secrets scan (four false positives in test fixtures).
- Rewrote the README and refreshed the status docs; added `docs/README.md` as an index.

## Release 3: merchant operations
- Providers page (admin): connect or replace a Cashfree or Razorpay account from the console (`PUT /v1/provider_accounts/{provider}`, secrets typed once and never shown), turn accounts on or off, see the webhook address to paste into the provider.
- Correct a customer (`PATCH /v1/customers/{id}`, console Customers page, all three SDKs): left-out fields stay, an empty string clears, audited; issued invoices keep the GSTIN they were issued with.
- Printable invoice page (`/invoice/{id}` in the console; browser print gives a PDF), laid out as a GST tax invoice when it has tax lines; invoice and customer reads now return the buyer, place of supply and address.
- Draft e-invoice (`GET /v1/invoices/{id}/einvoice`, IRP schema 1.1 shape) for B2B tax invoices, with seller/buyer addresses; says what is missing; not submitted, no IRN, not validated against the portal.
- FX exposure report (`GET /v1/fx/exposure`, SDK `fx.exposure`): open dollar invoices at the rate when raised vs the report date, with the difference; reporting only, nothing is posted to the ledger.
- Merchant onboarding: profile (PAN, GSTIN), document references, bank accounts, second-person verification and review; payouts wait for approval once onboarding has started. Console page and Node SDK included.
- Counters within branches: a counter's payment-link QR ties payments to the counter and branch; branch filter on payments; per-branch daily summary net of refunds. Open-amount QR (payer types the amount) is not built.
- Payment timeline: one ordered story per payment (attempts, provider messages, ledger, refunds, disputes, settlement) with a plain sentence on where it stands; each payment in the console opens it.
- Insights (no model involved): what is owed by due week with overdue, promised and payouts waiting, and refund pattern flags (a spike, or one person behind most refunds), shown on the console overview. A schedule, not a prediction.
- Cashier console role: a person who can record cash and count the till on the closing page and nothing else (every other page redirects there; sign-off stays with an admin and a different person). A cashier can be tied to one branch when added (the branch is checked as belonging to the tenant): the console then shows and accepts only that branch's cash and closings, forcing the branch on the server side. The limit is in the console; the console's own API key is still tenant-wide.
- Branch-pinned API keys: a key tied to one branch (no `admin` scope) can only create and see that branch's payments, links, cash entries, closings and summary; rotation keeps the pin. Such a key is refused everywhere else by default (customers, invoices, payouts, link lists, insights, settlements and the rest); only handlers that enforce the branch accept it.
- Console page for API keys: create a key (read-only or take-payments, optionally limited to one branch; the secret is shown once), see last use, revoke.
- Reconciliation suggestions (no model): for an open case, the likeliest explanation with its evidence and next steps (nearest unmatched settlement for a bank credit, how long this provider usually takes to settle for a missing settlement, what the size of a difference suggests), shown under each case in the console. Nothing is changed by reading one.
- Node SDK `intents.create` accepts `branchId` and `counterId`; new runnable example `examples/shop-counter.mjs` (found that gap by running it).
- Signed actors: with `ZYVOR_ACTOR_SECRET` on the API and the console, the API believes a person's name (`Zyvor-Actor`) only when the console signs it, so an integration key can no longer approve its own requests by naming someone else. Off by default for local use; the k3s deploy now creates and wires the secret. Node SDK option `actorSecret`.
- Open-amount payment links: the payer types the amount within a minimum and maximum you set (a printed counter QR). Fixed fix: pricing a zero amount (an open link before the payer chooses, or a settled invoice) no longer panics on the public link-info request. The console link form offers it, with an optional counter, and keeps what was typed when the API refuses.
- Python and Go SDKs gained the Release 3 methods (timeline, onboarding, counters, branch summary, cash, closings, insights). The closing page no longer offers sign-off to the person who counted.
- Console pages for Branches and counters (30-day collected per counter, switch a counter off) and Daily closing (record cash, close the day, second-person sign-off); Node SDK `branches`, `cash` and `closings`.
- Daily closing: cash entries (never edited), a per-branch IST-day closing comparing expected cash and captured digital totals with the counted cash, second-person sign-off. Records only: cash is not posted to the ledger.
- Fix: a payment link, an invoice, a counter or a cash entry could name another tenant's customer or branch id, because foreign keys are checked without row-level security. They now check the id belongs to the caller; a regression test covers every create path that takes a customer, invoice, vendor or branch id.
## Release 2

**Console**
- CSV export of payments (with the status filter) and invoices, with spreadsheet-formula defusing for customer-supplied text. Built and unit-tested; not yet clicked through in a browser.
**Security**
- Rate limits per API key and per address; API keys with expiry, source-address allow-lists, last-used tracking and rotation with an overlap.
- SSRF guard on merchant webhook URLs (checked when saved and again at delivery, with the connection pinned to vetted addresses).
- Rotatable master key (`zyvor-api rekey`); tamper-evident audit chain with a verify endpoint.
- Named console users (admin, finance, viewer) with Argon2id passwords, TOTP, recovery codes and lockout.
- One ceiling for every money amount; default-deny network policy; cargo-deny, `pnpm audit`, gitleaks, CodeQL, Trivy and SBOM in CI.

**Payments and checkout**
- Payment links with QR codes, caps and expiry; coupons; a payer-covered convenience fee.
- Marketplace splits to vendors with per-vendor balances that follow captures and refunds.
- Rule-based risk limits (payment ceiling, tries per payment, failures per customer, blocked UPI ids).
- Checkout in English, Hindi, Marathi, Tamil, Telugu and Bengali (translations awaiting native review).

**Billing and collections**
- Instalment plans; GST tax invoices (CGST/SGST/IGST, GSTIN check, credit notes, GSTR-1 style summary).
- Opt-in automatic dunning; reminder drafts a person approves; optional AI-worded drafts that never touch money.
- Exchange rates you enter, used in reporting (INR and USD).

**Payouts and webhooks**
- Vendors with encrypted bank details; payouts that need a second person; bulk payouts; pay-from-balance; ledger posting. Zyvor records, you transfer.
- Webhook event catalogue and per-tenant subscriptions.

**Console**
- Pages for payment links and coupons, payouts, billing forms, collections review, settings, people and account.

**Docs and examples**
- Threat model, a comparison with Indian gateways, runnable examples for fees, clinic billing and marketplaces.

## Release 1
- Payments with idempotency and unresolved-outcome handling, a capability-matrix router, refunds with approval,
  a double-entry ledger, settlement and bank reconciliation with an assignable case queue.
- Billing (plans, subscriptions, usage, proration, milestones), disputes, bank CSV import, OpenTelemetry tracing.
- Merchant console, Node, Python and Go SDKs, OpenAPI, a k3s sandbox with HTTPS.
