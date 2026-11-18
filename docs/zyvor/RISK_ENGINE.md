# Verification primitives and the risk rules engine

Status: **implemented and tested against the simulator and local mocks only.** The verification provider in every test
is the simulator, whose answers verify nothing real. Nothing here is fraud intelligence from a card network, and
there is no model training anywhere.

Code: `verify.rs`, `kyc.rs` (`Kyc::verify_vpa`, `Kyc::is_simulated`), `keyed.rs`, `risk.rs` (hard limits, payment score,
order score), `risk_engine.rs` (rules, lists, review queue). Tests: unit tests in those files and
`crates/zyvor-api/tests/link_rules.rs`. Migration `0087`. Scopes added: `verify:write`, `risk:read`, `risk:write`
(admin and finance roles have them).

## Verification: UPI ids and bank accounts

- `POST /v1/verify/vpa` `{ "vpa", "name"? }` and `POST /v1/verify/bank_account` `{ "account_number", "ifsc", "name"? }`.
  Results: `outcome` (`valid`, `invalid`, `inconclusive`), `name_match` (`match`, `review`, `mismatch`, `not_checked`),
  `safe_to_pay` (valid **and** `match`; nothing less), `provider`, `simulated`. `GET /v1/verifications` and
  `GET /v1/verifications/{id}` read them back.
- Routed through the KYC abstraction (`kyc::Kyc`), the same one onboarding uses, so a real provider is a new arm there.
  The simulator answers from patterns of the input (documented on `Kyc::verify_vpa` and `Kyc::verify_bank`).
  Cashfree Secure ID does the bank check (as in onboarding; run only against its documented sandbox inputs) and is **not
  wired for UPI ids**: it answers `inconclusive`, never a pass. With no provider configured a test-mode business gets the
  simulator, every response saying `simulated: true`; a live-mode business gets `409 verification_unavailable`, and so
  does a live business whose configured provider is the simulator.
- **Conservative name comparison** (`verify::compare_names`). `match` only when the words are the same (case, order,
  titles like Mr/Shri and company suffixes like Pvt/Ltd aside; a single initial may stand for a full word if at least one
  full word agrees). One extra word ("Ravi Kumar" against "RAVI KUMAR SHARMA") or one misspelt longer word is `review`: a
  person decides. Everything else, including a single shared word ("Kumar" against "Amit Kumar") or two bare initials, is
  `mismatch`. Nothing automatic acts on `review` or `mismatch`.
- **Stored masked:** the last four digits of an account (`******1191`), the first two characters and the handle of a
  UPI id (`ra***@okhdfcbank`), the IFSC, the outcome, the match result, the provider. Never the full number or id, and never
  the name the bank returned (responses do not contain it either, so the endpoints cannot be used to look up who holds an
  account). A keyed hash of the request lets a repeat within ten minutes reuse the stored answer.
- **Rate limited:** a per-business token bucket (burst 10, one more every four seconds) and an hourly cap counted from the
  stored results (100; 30 for a paid provider), both `429`. Limits are per business, in memory per replica for the
  bucket.

## Risk rules engine

Runs inside `start_attempt` after the existing hard limits (ceiling, attempts, failed-try velocity, blocked VPAs, score
floor) and never instead of them.

**Signals** (per attempt): customer id; a device token the checkout sends in `X-Zyvor-Device` (opaque, at most 128
characters; Zyvor does not generate one); the customer's phone and e-mail; the network address when
`ZYVOR_TRUST_PROXY_HOPS` or the console's signed header says what it is; the card (a hash of the network-token reference,
never a card number); the UPI id; the amount; the method. Each value becomes a keyed hash (`keyed.rs`: HMAC-SHA256 with a
per-business secret encrypted under the master key, 128 bits kept) before it is stored or compared. **No personal data is
stored by the engine or written to a log**; the signal rows are deleted after seven days by the worker. The per-business
secret does not change on master-key rotation, so list entries keep matching.

**Rules** (`POST /v1/risk/rules`, up to 50; actions `review` or `decline`):

- `velocity`: `{ dimension, metric, window_secs (60..604800), max }` with dimension one of customer, device, phone, email,
  ip, card, vpa and metric `attempts`, `amount_minor` (sum) or `distinct_customers`. It fires when the total **including
  the attempt being made** is above `max`.
- `amount`: `{ above_minor?, below_minor?, method? }`.

**Lists** (`POST /v1/risk/lists`): `block` entries decline; `allow` entries stop velocity and amount rules from firing for
that attempt. Block beats allow. A value is hashed on the way in and shown afterwards only as a masked hint plus your note.

**Decision:** the strongest action among what fired. `decline` refuses with `403 risk_blocked` (audited, `payment.risk_blocked`).
`review` lets the payment go ahead **and** opens a `risk_review` case (`GET /v1/risk/review_queue`; resolve with
`POST /v1/recon/cases/{id}/resolve`) and raises `risk.review_required`; this is a review after the fact, not a hold, because
a payer at checkout cannot be held for a person (manual capture is the way to hold). A rule needing a signal the attempt did
not carry is skipped and listed under `skipped`. Every decision where something fired is kept with the rule names and the
numbers they saw (`GET /v1/risk/decisions`); `POST /v1/risk/evaluate` shows what would happen for given values and records
nothing.

Limits: only as good as the signals; someone who changes device token, phone and card on every attempt is not caught by
velocity on those. The allow-list does not switch off the hard limits.

## Order risk score (cash on delivery)

`POST /v1/risk/order_score` `{ pin_code, order_value_minor, repeat_customer?, prior_refusals?, order_ref?, review_at?, decline_at? }`.
All inputs come from the merchant; nothing about the buyer is looked up. The score is a sum, clamped to 0-100 (higher is
riskier), formula `order-risk-v1`:

| factor | points |
|---|---|
| order value | under ₹500: 0; ₹500 to ₹1,999: +10; ₹2,000 to ₹4,999: +20; ₹5,000 or more: +30 |
| earlier refused or undelivered orders by this buyer | 0: 0; 1: +25; 2: +40; 3 or more: +55 |
| repeat customer (has received orders before) with no refusals | -15 (no credit if they refused any) |
| area, from your own tier for the longest matching pin-code prefix (`PUT /v1/risk/pin_tiers`) | high +20, medium +10, low 0, no tier on file +5 |

`review_at` (default 40) and `decline_at` (default 75) turn the score into `allow`, `review` or `decline`; bands are 0-29
low, 30-59 medium, 60 and over high. The response lists every factor with its points and a sentence. With an `order_ref`, a
`review` or `decline` opens one `order_risk_review` case per reference (same review queue) and raises `risk.review_required`;
the pin code is not stored. The weights are judgement, not measured on any real return data: tune the thresholds to your own
returns, and treat the score as a prompt to confirm or ask for prepayment, not as a finding about a person.
