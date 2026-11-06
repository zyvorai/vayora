# Acquire mode (own-gateway path)

Zyvor can run in two money modes. This page is the operator guide for **`acquire`**. For the legal posture see [REGULATORY.md](REGULATORY.md) and [ADR 0004](../adr/0004-money-modes-orchestrate-acquire.md). **Not legal advice.**

## Orchestrate vs acquire

| | `orchestrate` (default) | `acquire` |
|---|---|---|
| Who collects | Merchant's Razorpay / Cashfree / PhonePe / custom | Sponsor PA / bank PSP (`sponsor_upi` or `custom`) |
| Ledger on capture | `provider_receivable:<provider>` / `collections` | `escrow:in_flight` / `merchant_payable:<org>` |
| Settlement | Provider settles to merchant bank; Zyvor reconciles | Settlement batches + payout rail (`zyvor-settle`) |
| Payouts | `mark_paid` after you transfer | `POST /v1/payouts/{id}/send_rail` (or `mark_paid`) |
| Live gate | Platform live-access review | Also: acquire KYC approved + `ZYVOR_ALLOW_LIVE_ACQUIRE=1` |

## API surface

| Area | Endpoints |
|---|---|
| Mode | `GET/PUT /v1/money_mode` (`settlement_delay_days` 0–7) |
| KYC gate | `GET/PUT /v1/acquire_kyc` |
| Batches | `GET/POST /v1/settlement_batches`, `GET …/{id}`, `POST …/{id}/items`, `POST …/{id}/release` |
| Escrow proof | `GET /v1/escrow/proof` |
| Rail payout | `POST /v1/payouts/{id}/send_rail` |
| Cards | `GET/POST /v1/card_tokens`, `POST …/{id}/revoke` (network tokens only) |
| Charge with token | `POST /v1/payment_intents/{id}/attempts` with `method=card` and `card_token_id` (sim/stub vault; never PAN) |

Captures and refunds in acquire mode **auto-enrol** into the open settlement batch for that currency (idempotent). The worker releases batches when `settlement_delay_days` has elapsed.

SDKs: Node `moneyMode`, `acquireKyc`, `settlementBatches`, `escrow`, `cardTokens`, `payouts.sendRail`; Python/Go equivalents. Console: Settings → Money mode / Acquire KYC; Providers → PhonePe & Sponsor UPI; Payouts → Send via rail.

## Environment

| Variable | Role |
|---|---|
| `ZYVOR_ALLOW_LIVE_ACQUIRE=1` | Required to set `money_mode=acquire` on a **live** tenant |
| `ZYVOR_PAYOUT_RAIL_URL` / `ZYVOR_PAYOUT_RAIL_KEY` | Sponsor HTTP payout rail; without them the sim rail is used |

## Go-live checklist (acquire)

1. Counsel-signed fund-flow diagram ([REGULATORY.md](REGULATORY.md))
2. Sponsor LOI (escrow banks + PSP) — [OWN_LICENCES.md](../compliance/OWN_LICENCES.md)
3. Named PMLA programme — [KYC_AML_ACQUIRE.md](../compliance/KYC_AML_ACQUIRE.md)
4. CERT-In contacts tested — [CERT_IN_RUNBOOK.md](../compliance/CERT_IN_RUNBOOK.md)
5. PCI scope understood — [PCI_SCOPE.md](../compliance/PCI_SCOPE.md)
6. Connect `sponsor_upi` (or custom) + set acquire KYC to `approved`
7. Set `settlement_delay_days` (T+N); open batches; release after the delay
8. Only then enable `ZYVOR_ALLOW_LIVE_ACQUIRE` for production

Sandbox may exercise acquire ledger paths against simulators without a sponsor.

## Related

- [BRING_YOUR_OWN_PROVIDER.md](BRING_YOUR_OWN_PROVIDER.md) — adapters including PhonePe and sponsor_upi
- [MANDATES_LIVE.md](MANDATES_LIVE.md) — AutoPay before live use
- [STATUS.md](STATUS.md) — what is simulator-only
- [plugins/](../../plugins/README.md) — Shopify / WooCommerce scaffolds
