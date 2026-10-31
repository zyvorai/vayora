# Changelog

What shipped, newest first. Everything here is verified against local simulators and a k3s sandbox, except Cashfree UPI
intent and QR, which was also run against Cashfree's own sandbox. Nothing has run against a live provider. See
[docs/zyvor/STATUS.md](docs/zyvor/STATUS.md) for what is not built.

## 31 October 2026
**People page and FX exposure**
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
