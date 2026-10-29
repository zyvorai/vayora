# ADR 0001: Monorepo with Cargo and pnpm workspaces

**Status:** accepted

Rust services and TypeScript SDKs/UI evolve together around one OpenAPI contract. A single repo keeps changes atomic and CI simple. Revisit if release cadences diverge.
