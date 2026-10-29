# Vayora

Open-source payments infrastructure for India: a Rust core, TypeScript SDKs, and an Apple-grade checkout and merchant dashboard.

> **Status: early scaffold.** Everything runs against a built-in bank simulator. No real money moves.

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

### Simulated UPI
The VPA handle picks the outcome: `fail…@x` declines (insufficient funds), `timeout…@x` times out, anything else succeeds.

### API
`POST /v1/orders` · `GET /v1/orders/{id}` · `POST /v1/payments` · `GET /v1/payments/{id}` · `POST /v1/payments/{id}/refunds` · `GET /v1/balance`.
Writes require `Idempotency-Key`. Webhooks (`payment.captured`, `payment.failed`, `refund.processed`) are signed; set `VAYORA_WEBHOOK_URL` and `VAYORA_WEBHOOK_SECRET`, then verify with `verifyWebhook` from `@vayora/node`. Design notes: [ADR 0002](docs/adr/0002-vertical-slice.md).

## Principles

- Money is integer paise, never floats.
- Every mutating endpoint takes an `Idempotency-Key`.
- Ledger entries are immutable; corrections are new entries.

Licensed under [Apache-2.0](LICENSE).
