# Vayora

Open-source payments infrastructure for India: a Rust core, TypeScript SDKs, and an Apple-grade checkout and merchant dashboard.

> **Status: early scaffold.** Everything runs against a built-in bank simulator. No real money moves.

## Regulatory notice

Publishing code is free; moving money in India is not. Operating Vayora in production requires, at minimum, RBI Payment Aggregator authorisation (or a licensed partner), a sponsor PSP bank and NPCI onboarding for UPI, PCI-DSS scoping for cards, and compliance with data-localisation, DPDP Act and CERT-In rules. See [docs/compliance](docs/compliance/README.md). The maintainers give no legal advice.

## Layout

| Path | Purpose |
|---|---|
| `crates/vayora-core` | Domain types (`Money` in paise, ids) |
| `crates/vayora-ledger` | Double-entry, append-only ledger |
| `crates/vayora-api` | axum HTTP API with idempotency |
| `packages/node` | `@vayora/node` server SDK |
| `apps/checkout` | Next.js hosted checkout |
| `deploy/` | docker-compose for Postgres and Redis |

Planned: UPI/cards adapters, router, webhooks, settlement, risk, dashboard.

## Quickstart

```sh
cargo test --workspace
cargo run -p vayora-api            # http://localhost:8080/healthz
pnpm install && pnpm -r build
```

## Principles

- Money is integer paise, never floats.
- Every mutating endpoint takes an `Idempotency-Key`.
- Ledger entries are immutable; corrections are new entries.

Licensed under [Apache-2.0](LICENSE).
