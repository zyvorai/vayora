<div align="center">

<img src="docs/logo.svg" width="72" height="72" alt="Vayora logo">

# Vayora

**Payments for India. Open to everyone.**

An open-source payments platform with an exactly-once API, a double-entry ledger and signed webhooks.<br>
Rust core · TypeScript SDK · simulated UPI

[![CI](https://github.com/zyvorai/vayora/actions/workflows/ci.yml/badge.svg)](https://github.com/zyvorai/vayora/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-0088ff)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2024-orange)
![TypeScript](https://img.shields.io/badge/TypeScript-5-3178c6)
![Status](https://img.shields.io/badge/status-early%20%C2%B7%20simulated%20rails-ff9f0a)

[**Live demo**](https://zyvorai.github.io/vayora/) · [Quickstart](#quickstart) · [API](#api) · [Compliance](docs/compliance/README.md)

<a href="https://zyvorai.github.io/vayora/"><img src="site/assets/og.png" width="720" alt="Vayora: payments for India, open to everyone"></a>

</div>

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
| `apps/dashboard` | Merchant dashboard: overview, payments, refunds, webhooks |
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
