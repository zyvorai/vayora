# Research notes: payments-platform practice and where Vayora stands

Date of research: 23 November 2026. Scope: this is a simulated-UPI product; nothing here proposes a real bank, PSP or
network integration or handling real card or personal data.

Evidence labels: **[F]** = read on a fetched page during this research. **[U]** = from memory or general knowledge, not
read from a fetched page; treat as unverified and re-check before relying on it.

## 1. What others do

### Webhooks and signatures
- Stripe signs every delivery in a `Stripe-Signature` header of `t=<unix>`, one or more `v1=<hmac-sha256 of "t.body">`,
  and ignorable extras. It tells receivers to check *every* `v1` (several appear while a signing secret is being rolled,
  with the old secret kept up to 7 days), ignore other schemes (downgrade protection), compare in constant time, use the
  raw body, and reject timestamps outside a tolerance (default 5 minutes; a tolerance of 0 disables the check). A retry
  gets a new timestamp and signature. Duplicates are expected: de-duplicate on the event id, not on `created`. Events are
  unordered; retries run for up to three days with exponential backoff in live mode. [F]
  https://docs.stripe.com/webhooks
- Razorpay: `X-Razorpay-Signature` is an HMAC-SHA256 of the raw body; each event has a unique `x-razorpay-event-id` to
  record and skip repeats; after a secret rotation, older retried events must be validated with the old secret. Its
  retry policy was not on the page read. [F] https://razorpay.com/docs/webhooks/validate-test/
- Adyen: HMAC-SHA256 over a defined subset of notification fields, base64, in the payload; notifications must be
  acknowledged with `[accepted]`; retries for several days. [U]

### Idempotency
- Stripe: the first request's status code and body are saved after execution begins and replayed for the same key,
  including errors; a key reused with different parameters is an error; keys up to 255 characters; keys may be pruned
  after 24 hours; validation failures and concurrent-conflict responses are not saved so they can be retried. [F]
  https://docs.stripe.com/api/idempotent_requests
- TigerBeetle: the transfer `id` is the idempotency key: a retry after a lost reply must not move the money twice. [F]
  https://docs.tigerbeetle.com/coding/data-modeling/

### Ledgers
- TigerBeetle: accounts hold cumulative posted debits and credits; the balance is derived; per-account flags
  (`debits_must_not_exceed_credits`, `credits_must_not_exceed_debits`) enforce one-sided balances; accounts and transfers
  are immutable; amounts are unsigned integers with a fixed asset scale; only accounts on the same ledger (currency) can
  transact directly. [F] https://docs.tigerbeetle.com/coding/data-modeling/
- Modern Treasury Ledgers describes an "immutable, scalable, double-entry ledger"; its guarantees and "account
  normality" guides were not fetched. [F for the quote, [U] for details] https://docs.moderntreasury.com/ledgers/docs/ledgers-overview
- Formance and similar: ledgers with pending and posted balances, reconciliation of the ledger against external
  statements as a first-class operation. [U]

### Reconciliation and settlement (India)
- NPCI publishes UPI settlement and raw transaction files to participants and uses statuses such as success, failure and
  "deemed" with later reversal (TCC/RET style adjustments). The NPCI page fetched carried no technical content. [U]
- RBI payment-data and PCI-DSS scoping guidance (card data out of scope when only tokens are held) is already reflected
  in `docs/compliance/` for this repository. [U]

## 2. What Vayora already has (read from the repository)
- Exactly-once creation through `Idempotency-Key` with request fingerprinting and replay of the stored response
  (`idempotency.rs`); `idempotency_key_reused` on a different body.
- Append-only double-entry journal with a deferred database trigger that refuses an unbalanced transaction per currency,
  plus immutable tables (`0001_core.sql`, `zyvor-ledger`).
- Signed outbound webhooks `t=...,v1=...` with at-least-once delivery, exponential backoff capped at one hour, dead
  letters after 10 attempts, a delivery inspector, replay, and verification helpers in the Node, Python and Go SDKs with a
  300 second tolerance (`outbox.rs`, SDKs).
- Inbound provider webhooks de-duplicated on (provider, event id); settlement import, bank statement matching, fee checks,
  reconciliation cases and ageing (`recon.rs`, `bankmatch.rs`); a tamper-evident sealed audit chain with a verify endpoint.

## 3. Gaps and ranked ideas

| # | Idea | Why | Fit and risk | Decision |
|---|------|-----|--------------|----------|
| 1 | Read-only ledger invariant checker and trial balance (`GET /v1/ledger/verify`) | The only enforcement is at write time (trigger plus `Transaction::new`); nothing lets an operator or auditor prove the stored rows still satisfy the rules, e.g. after a restore, migration or manual repair. TigerBeetle and Modern Treasury both treat the invariants as something to check, not assume. | Pure function, no write path touched. Low risk. | **Built** |
| 2 | Ledger-versus-settlement-file report with explicit mismatch categories | The existing import explains one settlement; there is no stateless "does this file agree with what we posted" report that names duplicates, missing lines on either side and currency mismatches. | Read-only; the classification is pure. Medium (SQL joins untested here without a database). | **Built** (`POST /v1/recon/ledger_check`) |
| 3 | Webhook verification helper that says why, accepts rotation, and a replay guard | Stripe documents multi-`v1` rotation and de-duplication on event id; the SDK helpers return a bare boolean, take the last `v1` only, and accept `1e3` as a timestamp. | Additive SDK code; the server's signing is untouched and pinned by a shared vector. | **Built** (Node SDK) |
| 4 | Server sends two `v1` signatures during a secret roll | Needed to rotate a merchant's webhook secret without downtime. | Changes what is signed and sent: a signature-semantics change. | **Rejected for now**; recorded in the changelog as future work |
| 5 | Idempotency-key expiry (24 h pruning) and not caching 5xx | Matches Stripe, bounds table growth. | Changes exactly-once behaviour; needs a design review and database tests. | **Rejected** (out of scope for a conservative change) |
| 6 | Two-phase (pending/posted) ledger entries | TigerBeetle pending transfers fit authorisation then capture. | Schema and semantics change in the ledger. | **Rejected** |
| 7 | Per-account non-negative balance flags | TigerBeetle flags; stops overspend at write time. | Write-path change; instead the checker reports a wrong-direction balance as a *warning*. | **Partly** (warning only) |
| 8 | Settlement/payout simulation report | Useful for demos. | Overlaps cash-visibility and payout batch reports already present. | **Rejected** (duplicate) |

## 4. Boundaries kept
No ledger, idempotency, signature or rounding behaviour was changed. New code only reads. Every number crossing the API as
a sum is a string, because sums of `i64` amounts can exceed what a JSON number carries.
