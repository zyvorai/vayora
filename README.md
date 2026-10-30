<div align="center">

<img src="docs/logo.svg" width="72" height="72" alt="Zyvor logo">

# Zyvor Payments

**Payment orchestration, ledger and reconciliation for Indian businesses.**

Connect your own Razorpay and Cashfree accounts. Zyvor routes, records and reconciles. Your provider settles.<br>
Rust core · PostgreSQL · Next.js checkout · TypeScript, Python and Go SDKs

[![CI](https://github.com/zyvorai/vayora/actions/workflows/ci.yml/badge.svg)](https://github.com/zyvorai/vayora/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-0088ff)](LICENSE)
![Status](https://img.shields.io/badge/status-release%202%20%C2%B7%20sandbox-ff9f0a)

[Status vs the brief](docs/zyvor/STATUS.md) · [Architecture](docs/zyvor/ARCHITECTURE.md) · [Regulatory position](docs/zyvor/REGULATORY.md) · [OpenAPI](openapi/zyvor.yaml)

</div>

> **Honest status.** Payments, ledger, reconciliation, billing, collections, disputes, payouts bookkeeping, GST invoicing and a merchant console are built and tested (every acceptance test in the brief passes). Providers are verified **only against local simulators**, never against live Razorpay or Cashfree. Zyvor does not move money to vendors, does not file GST returns, and is not a licensed payment service. The campus, hospital, factory and pharmacy product integrations are not built. [STATUS.md](docs/zyvor/STATUS.md) lists every gap; [MARKET.md](docs/zyvor/MARKET.md) compares it with Indian gateways.

## What it does
- **Never double-charges.** Idempotent writes; an uncertain provider outcome becomes `unknown` and blocks any new attempt, through any provider, until it is resolved. The system asks the provider; it never re-sends.
- **Explains every rupee.** `GET /v1/settlements/{id}/explain` lists payments, refunds, fees, adjustments, the bank match and why the net is what it is. Anything unexplained becomes an assignable case.
- **Keeps books you can audit.** Append-only double-entry ledger, balanced per currency by code and by a database constraint; corrections are reversals.
- **Routes with receipts.** Capability matrix, rules and configured fees choose a provider, and the decision records every provider considered and why.
- **Measures real cost.** Effective cost by provider and method from observed settlement deductions, with suggestions that cite their evidence and never change anything on their own.
- **Isolates tenants.** Row-level security enforced in tests as a non-superuser role; test and live are separate tenants.
- **Bills and collects.** Plans, subscriptions, usage, proration, milestones, instalment plans, credit notes, GST tax invoices (CGST/SGST/IGST, GSTIN-checked, GSTR-1 style summary), ageing, promises to pay, reminders you approve, optional AI-worded drafts a person reviews, disputes that hold the money.
- **Takes payments your way.** Payment links with QR codes, coupons and a payer-covered fee, marketplace splits to vendors with balances, bulk and maker-checker payouts (bookkeeping: you pay from your bank), risk limits, checkout in six languages (translations awaiting native review).
- **Is hard to abuse.** Per-key and per-address rate limits, API keys with expiry, address allow-lists and rotation, SSRF-guarded webhooks, rotatable master key, tamper-evident audit chain, named console users with TOTP and roles, default-deny network policy, supply-chain scanning in CI. See the [threat model](docs/zyvor/THREAT_MODEL.md).

## Try the sandbox (simulated providers, no real money)
```sh
# needs a local Postgres and an EMPTY database
OWNER_URL=postgres://you@localhost:5432/zyvor_sandbox ./scripts/zyvor-sandbox.sh
ZYVOR_API_URL=http://localhost:8080 pnpm --filter @zyvor/checkout dev    # then open the printed checkout URL
# the merchant console: see apps/zyvor-console/.env.example, then `pnpm --filter @zyvor/console dev`
```
```sh
cargo test --workspace      # 35+ API tests run against real Postgres (set DATABASE_URL to a superuser connection)
pnpm -r build && pnpm -r test
```
Deploy: `deploy/zyvor/k3s/` (verified on a single-node k3s sandbox, with HTTPS in front; see its README), `deploy/zyvor/docker-compose.yml` and `deploy/zyvor/helm/zyvor` (both validate; neither has been run here).

Runnable examples for a school's fee schedule, a clinic's GST billing and deposits, and a marketplace's splits and payouts are in [`examples/`](examples/README.md).

```ts
import { Zyvor } from "@zyvor/node";
const z = new Zyvor({ apiKey: process.env.ZYVOR_API_KEY! });
const intent = await z.intents.create({ amountMinor: 249900, externalRef: "FEE-STU-2231-T2" });
// give intent.client_secret to the browser; it can pay this one intent and nothing else
```

---

# Legacy: Vayora prototype
The original single-merchant simulator (`vayora-*` crates, `apps/dashboard`, `@vayora/node`, the GitHub Pages site) is kept while the dashboard and site are ported to the Zyvor API. It is unrelated to the multi-tenant platform above.

## How it works

```mermaid
flowchart LR
  A[POST /v1/orders] --> B[POST /v1/payments]
  B --> C{UpiGateway}
  C -- success --> D[Capture]
  C -- declined --> E[Failed payment]
  D --> F[Double-entry ledger]
  D --> G[Signed webhook]
  F --> H[POST …/refunds]
  H --> F
```

Everything for one request (idempotency key, payment, ledger entries, webhook event) commits in a single Postgres transaction, so the records can never disagree.

## What you get

| | |
|---|---|
| **Exactly once** | `Idempotency-Key` on every write; concurrent duplicates charge once, key reuse with different params is rejected |
| **Ledger** | Double-entry, append-only (enforced by DB triggers), balanced by construction |
| **Webhooks** | Transactional outbox, retries with backoff, HMAC-SHA256 signatures with replay protection |
| **SDK** | `@vayora/node`: typed client, typed errors, `verifyWebhook` |
| **Rails** | UPI behind a `UpiGateway` trait; deterministic simulator today |

## Status

- [x] Orders, payments, refunds on Postgres
- [x] Ledger, webhooks, Node SDK
- [x] Merchant dashboard (overview, payments, refund action, webhook log with retry)
- [ ] Hosted checkout wired to the API
- [ ] Cards, routing, settlement, risk
- [ ] Real rails, which require licences (see below)

## Regulatory notice

Publishing code is free; moving money in India is not. Operating Vayora in production requires, at minimum, RBI Payment Aggregator authorisation (or a licensed partner), a sponsor PSP bank and NPCI onboarding for UPI, PCI-DSS scoping for cards, and compliance with data-localisation, DPDP Act and CERT-In rules. See [docs/compliance](docs/compliance/README.md). The maintainers give no legal advice.

## Layout

| Path | Purpose |
|---|---|
| `crates/vayora-core` | Domain types (`Money` in paise, ids) |
| `crates/vayora-ledger` | Double-entry, append-only ledger (Postgres) |
| `crates/vayora-upi` | `UpiGateway` trait + deterministic simulator |
| `crates/vayora-webhooks` | Signed outbox delivery with retries |
| `crates/vayora-api` | axum API, idempotency, migrations |
| `packages/node` | `@vayora/node` server SDK |
| `apps/checkout` | Next.js hosted checkout |
| `apps/zyvor-console` | Merchant console for the Zyvor API: payments, links, refunds, payouts, reconciliation, collections, billing, disputes, webhooks, people |
| `apps/dashboard` | Legacy single-merchant dashboard (targets the old Vayora API; superseded by the console) |
| `deploy/` | docker-compose for Postgres and Redis |

Planned: cards, router, settlement, risk, dashboard.

## Quickstart

```sh
docker compose -f deploy/docker-compose.yml up -d postgres
export DATABASE_URL=postgres://vayora:vayora@localhost:5432/vayora
cargo test --workspace             # integration tests use real Postgres
VAYORA_API_KEY=sk_test_123 cargo run -p vayora-api   # migrates, then serves :8080
```

```sh
A="Authorization: Bearer sk_test_123"; J="content-type: application/json"
curl -s localhost:8080/v1/orders -H "$A" -H "$J" -H "idempotency-key: o-1" -d '{"amount_paise":49900}'
curl -s localhost:8080/v1/payments -H "$A" -H "$J" -H "idempotency-key: p-1" \
  -d '{"order_id":"<order id>","vpa":"asha@okvayora"}'
curl -s localhost:8080/v1/payments/<payment id>/refunds -H "$A" -H "$J" -H "idempotency-key: r-1" -d '{"amount_paise":10000}'
curl -s localhost:8080/v1/balance -H "$A"
```

### Dashboard

```sh
cp apps/dashboard/.env.example apps/dashboard/.env.local   # set the four values
pnpm install && pnpm --filter @vayora/node build
node scripts/seed.mjs 60                                   # optional demo data (dev databases only)
pnpm --filter @vayora/dashboard dev                        # http://localhost:3001
```

Sign-in is a single shared password (`DASHBOARD_PASSWORD`) with a signed, HttpOnly session cookie. The API key stays on the server and is shown masked. Per-user accounts, roles and key rotation are not built yet.

### Simulated UPI
The VPA handle picks the outcome: `fail…@x` declines (insufficient funds), `timeout…@x` times out, anything else succeeds.

### API
`POST /v1/orders` · `GET /v1/orders[/{id}]` · `POST /v1/payments` · `GET /v1/payments[/{id}]` · `POST /v1/payments/{id}/refunds` · `GET /v1/refunds` · `GET /v1/events` · `POST /v1/events/{id}/retry` · `GET /v1/stats` · `GET /v1/balance`. Lists are newest-first with `limit`, `starting_after` and `status`.
Writes require `Idempotency-Key`. Webhooks (`payment.captured`, `payment.failed`, `refund.processed`) are signed; set `VAYORA_WEBHOOK_URL` and `VAYORA_WEBHOOK_SECRET`, then verify with `verifyWebhook` from `@vayora/node`. Design notes: [ADR 0002](docs/adr/0002-vertical-slice.md).

## Principles

- Money is integer paise, never floats.
- Every mutating endpoint takes an `Idempotency-Key`.
- Ledger entries are immutable; corrections are new entries.

Licensed under [Apache-2.0](LICENSE).
