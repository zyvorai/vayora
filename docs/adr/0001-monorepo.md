# ADR 0001: Monorepo with Cargo and pnpm workspaces

**Status:** accepted

Rust services and TypeScript SDKs/UI evolve together around one OpenAPI contract. A single repo keeps changes atomic and CI simple. Revisit if release cadences diverge.

Related: [ADR 0004](0004-money-modes-orchestrate-acquire.md) adds acquire-mode custody without splitting the monorepo.
