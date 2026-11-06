# Zyvor Payments architecture

```mermaid
flowchart LR
  BR([Customer's browser]) --> CO[Checkout<br/>Next.js]
  CO -- "/v1/checkout/*" --> API
  MS([Merchant systems and console]) -- "/v1/* with API key" --> API
  PR[Providers] -- "signed webhooks<br/>/v1/webhooks/*" --> API

  subgraph SVC[Zyvor services]
    API[zyvor-api<br/>axum, one transaction per request]
    WK[zyvor-worker<br/>resolve unknowns, deliver outbox,<br/>scan for missing settlements]
  end

  API <--> DB[(PostgreSQL<br/>source of truth, row-level security)]
  WK <--> DB
  API -- "create payment" --> PR
  WK -- "ask, never re-send" --> PR
  WK -- "signed events" --> MS
  API -. acquire: settle / payout .-> Rail[zyvor-settle rails]
```

Providers today: Razorpay, Cashfree, PhonePe, `sponsor_upi`, `custom`, `sim`. New to the system? Start with [HOW_IT_WORKS.md](HOW_IT_WORKS.md) and [ACQUIRE_MODE.md](ACQUIRE_MODE.md).

## Money-flow model (read this first)

Organisations choose `money_mode` ([ADR 0004](../adr/0004-money-modes-orchestrate-acquire.md)):

| Mode | Who holds funds | Ledger on capture |
|---|---|---|
| **`orchestrate`** (default) | Merchant's own provider | `provider_receivable:<provider>` / `collections` |
| **`acquire`** | Licensed sponsor escrow | `escrow:in_flight` / `merchant_payable:<org>` |

Acquire settlement batches, escrow proof and payout rails are in `zyvor-settle`. Live acquire is gated by counsel + sponsor LOI + acquire KYC + `ZYVOR_ALLOW_LIVE_ACQUIRE` — see [REGULATORY.md](REGULATORY.md). Network card tokens (never PAN/CVV) live in `zyvor-cards`.

## Invariants and where they are enforced
| Invariant | Enforced by |
|---|---|
| A repeated request never repeats the operation | `idempotency_keys` row written in the request's transaction; replay returns the resource's current state |
| An uncertain provider outcome is never retried elsewhere | attempt is persisted *before* the provider call; `unknown` blocks further attempts on the intent; the worker only *asks* the provider |
| State never regresses | `state_machine::attempt_transition` (pure, unit-tested); the single apply function locks the attempt row |
| A capture posts to the ledger once | unique `(tenant, reference)` on ledger transactions + the transition applies once |
| Debits equal credits per currency | `Transaction::new` and a deferred constraint trigger on `ledger_entries` |
| History is immutable | triggers reject UPDATE/DELETE on ledger, audit and allocations; corrections are reversals |
| Refunds never exceed the payment | reservation counter locked per attempt + a CHECK constraint |
| Tenants cannot see each other | every table has `tenant_id` + row-level security keyed on `zyvor.tenant_id`; the service must run as a non-superuser role |
| Provider secrets are never readable | AES-256-GCM, key from `ZYVOR_MASTER_KEY`, never returned by any endpoint |

## Provider calls happen outside database transactions
Persist → call provider → record result in a second transaction. A crash in between leaves an attempt in `created`; the worker asks the provider (by our attempt id) and adopts the truth, or marks it failed if the provider never saw it.

## Operating notes
- Providers are adapters behind the `Provider` trait (`crates/zyvor-providers`: Razorpay, Cashfree, PhonePe, sponsor_upi, custom, sim). See [BRING_YOUR_OWN_PROVIDER.md](BRING_YOUR_OWN_PROVIDER.md) and the [HTTP contract](PROVIDER_CONTRACT.md).
- Crates `zyvor-settle` (escrow batches, payout rails) and `zyvor-cards` (network tokens) support acquire mode.
- Run migrations with the schema owner (`ZYVOR_MIGRATE_DATABASE_URL`) and serve as `zyvor_app`, a non-superuser role (`deploy/zyvor/sql/roles.sql`). A superuser bypasses row-level security.
- Webhook endpoints are per tenant and provider: `/v1/webhooks/{provider}/{tenant_id}`.
- Workers can be scaled out: delivery uses `FOR UPDATE SKIP LOCKED`.
