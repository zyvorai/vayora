# Changelog

What shipped, newest first. Everything here is verified against local simulators and a k3s sandbox, never against
live Razorpay or Cashfree. See [docs/zyvor/STATUS.md](docs/zyvor/STATUS.md) for what is not built.

## Release 2 (current)
**Merchant operations (Release 3, in progress)**
- Merchant onboarding: profile (PAN, GSTIN), document references, bank accounts, second-person verification and review; payouts wait for approval once onboarding has started. Console page and Node SDK included.
- Counters within branches: a counter's payment-link QR ties payments to the counter and branch; branch filter on payments; per-branch daily summary net of refunds. Open-amount QR (payer types the amount) is not built.
- Payment timeline: one ordered story per payment (attempts, provider messages, ledger, refunds, disputes, settlement) with a plain sentence on where it stands; each payment in the console opens it.
- Insights (no model involved): what is owed by due week with overdue, promised and payouts waiting, and refund pattern flags (a spike, or one person behind most refunds), shown on the console overview. A schedule, not a prediction.
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
