# Contributing

1. Open an issue for anything non-trivial before a PR.
2. Rust: `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --workspace` must pass.
3. TypeScript: `pnpm -r lint && pnpm -r test && pnpm -r build` must pass.
4. Sign off commits (`git commit -s`). Never include real card, UPI or personal data in tests or fixtures.
