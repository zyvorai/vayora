# ADR 0004: Dual money modes — orchestrate and acquire

**Status:** accepted

## Context

Zyvor was built so it never holds customer funds: merchants connect their own providers, and those providers settle to the merchant bank. That kept the open-source product out of payment-aggregator licensing. Competing with Razorpay / PhonePe requires an **acquire** path where Zyvor is the merchant-facing gateway and funds move through escrow under a licensed party (sponsor first, own PA later).

## Decision

1. Every organisation has a `money_mode`: `orchestrate` (default) or `acquire`.
2. **Orchestrate** keeps today's ledger semantics (`provider_receivable:*` on capture).
3. **Acquire** posts custody accounts (`escrow:*`, `merchant_payable:*`, …) and uses settlement / payout rails in `zyvor-settle`. Collection still goes through a `Provider` adapter — the sponsor/acquirer, not the merchant's Razorpay keys.
4. Live acquire is refused unless the operator has completed the fund-flow freeze in `docs/zyvor/REGULATORY.md` (enforced operationally; sandbox may set acquire for tests).
5. Orchestrate mode remains supported indefinitely for merchants who keep their own PG accounts.

## Consequences

- Capture, refund, dispute and payout posting branch on `money_mode`.
- New crates: `zyvor-settle` (escrow batches, payout rails), `zyvor-cards` (network tokens only).
- Regulatory docs and compliance pack must stay aligned with the live fund flow.
- Consumer P2P wallet features stay out of scope for the first acquire release.
