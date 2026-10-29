# ADR 0002: Vertical slice design

**Status:** accepted

- **Postgres is the source of truth.** Orders, payments, refunds, ledger, idempotency keys and the webhook outbox commit in one DB transaction, so they cannot disagree.
- **Idempotency** is a row in `idempotency_keys` written in the request's own transaction. Concurrent duplicates block on the primary key, then replay the stored response. Reusing a key with different parameters returns 422. Failed requests roll back and leave the key reusable.
- **Ledger** is double-entry and append-only (DB triggers reject UPDATE/DELETE). Capture: debit `upi_clearing`, credit `merchant_payable`. Refunds reverse it. `Transaction::new` refuses unbalanced entries; `reference` is unique so an event can't post twice.
- **Webhooks** use a transactional outbox: an event exists iff the change committed. Delivery is at-least-once with exponential backoff (5s doubling, cap 1h, dead after 10 attempts). Consumers must dedupe on `event.id` and verify `Vayora-Signature`.
- **UPI** sits behind the `UpiGateway` trait. `SimulatedUpi` is deterministic from the VPA handle (`fail*` declines, `timeout*` times out, otherwise succeeds).

## Known limits (deliberate, for this slice)
- The simulator is called inside the DB transaction because it answers instantly. A real PSP is asynchronous: introduce a `pending` payment state, persist before calling out, and finish via callback/poll. A UPI refund that succeeds upstream but whose DB commit fails would then need reconciliation.
- Single merchant, one static API key (constant-time compare). Multi-tenant keys, scopes and rotation come next.
- No rate limiting, no per-merchant ledger accounts, no settlement yet.

## Dashboard (ADR addendum)
- `apps/dashboard` is a server-rendered Next.js app. It calls the API from the server with `@vayora/node`, so the API key never reaches the browser and no CORS is needed.
- Auth is one shared password and an HMAC-signed, HttpOnly, 12-hour session cookie checked in `proxy.ts`. Failed logins are delayed 500ms. There is no rate limiter, per-user identity or audit log yet.
- Refund forms mint their `Idempotency-Key` at render time, so a double submit replays instead of refunding twice. Amounts are parsed strictly to integer paise (`parseRupees`).
- Lists use keyset pagination on `(created_at, id)`. Daily volume is bucketed by IST calendar day.
