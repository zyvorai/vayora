# Changelog

What shipped, newest first. Everything here is verified against local simulators and a k3s sandbox, never against
live Razorpay or Cashfree. See [docs/zyvor/STATUS.md](docs/zyvor/STATUS.md) for what is not built.

## Release 2 (current)
**Merchant operations (Release 3, in progress)**
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
