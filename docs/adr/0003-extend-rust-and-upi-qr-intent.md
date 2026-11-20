# ADR 0003: Extend the Rust codebase; UPI QR / intent as the default UPI flow

**Status:** accepted

## Context
A 31 October 2026 implementation specification proposes Go (pgx/sqlc), Vite apps and a rebuilt repository. This repository is already a tested Rust platform (axum, sqlx, PostgreSQL with row-level security, an append-only ledger, a payment state machine, refunds, reconciliation, a hosted checkout, signed webhooks, Helm/Envoy deployment). The spec allows a different stack when the repository gives a strong reason, recorded in an ADR.

## Decision
1. **Keep Rust, axum and sqlx.** Close the spec's gaps in milestone order instead of rewriting. A rewrite would discard a working, tested system and delay every milestone, for no gain in correctness.
2. **UPI defaults to a provider-issued QR / intent, not a typed UPI ID.** The spec (section 3) and current provider documentation treat manual UPI ID entry (Collect) as deprecated outside specific exceptions. The checkout asks the API which flow applies (`upi_flow`: `intent` or `collect`) and only falls back to typing a UPI ID when no eligible provider can issue a QR / intent.
3. **A new capability, `upi_intent`, defaults to off.** An adapter turns it on only for a flow verified with that partner. Razorpay and Cashfree stay off; the simulator turns it on. The router will not send a UPI payment without a UPI ID to a provider that lacks it.
4. **The payload is opaque.** `ClientAction::UpiIntent { uri }` carries the partner's `upi://pay` string exactly as issued, because partners may sign fields. A QR or an app opening is never treated as proof of payment; only a verified provider result moves a payment.
5. **Return URLs are validated.** https only (http in a sandbox), no embedded credentials. A per-merchant allowlist of origins is still to do.

## Consequences
- Until a partner flow is verified, real merchants still use the hosted checkout or collect; the QR / intent flow works with the simulator only. That is stated in `STATUS.md` and `IMPLEMENTATION_STATUS.md`.
- Not decided here: OIDC sign-in (the console currently uses its own TOTP sign-in), mandates / UPI AutoPay, object storage with malware scanning, decimal-string amounts, and the "orders" naming (the repository calls them payment intents).
