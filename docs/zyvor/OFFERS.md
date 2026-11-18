# Offers: merchant-funded discounts, judged by method, instrument, amount and customer

Status: **implemented and tested against the in-process simulator and the local provider mock only.** No real provider,
bank or money is involved. An offer here is a discount the **merchant** gives. Nothing in this module models, claims or
reports an offer that a bank or a payment provider funds; if a provider ever supplies one, it needs its own field and its
own evidence. Every offer and every redemption carries `funded_by: merchant`, and the database refuses any other value.

Code: `crates/zyvor-api/src/offers.rs` (engine and API), `crates/zyvor-api/src/instrument_hint.rs` (what the payer used; not `instrument_hint.rs`, which holds the payer-instrument rules),
hooks in `payments.rs` (`start_attempt`, `apply_provider_state`), `refunds.rs` (`apply_refund_result`) and
`checkout.rs`. Migration `0086_offers_rules_alerts.sql`. Tests: `crates/zyvor-api/tests/offers.rs` and unit tests in
`offers.rs` and `instrument_hint.rs`.

## What already existed

`coupons.rs`: a percentage or flat coupon on a fixed-amount **payment link**, spent in the transaction that creates the
payer's payment. That is unchanged and still the way to give a coupon on a link. The offers engine reuses its rounding
(`half_up`) and its rule that a discount can never take the whole price (one minor unit always stays), and extends the
idea to any payment, with conditions the coupon does not have.

## When an offer is judged

When a payment **attempt** is created (`POST /v1/payment_intents/{id}/attempts`, or the public checkout), because only
then are the method and what the payer used known. The attempt is created for the discounted amount: that is what the
provider is asked to charge, what the payer sees, and what is captured, settled and refunded. The intent keeps its
list price. `POST /v1/offers/preview` runs the same evaluation without reserving anything, so what is previewed is what is
applied (the caps are counted again under a lock when the attempt is created).

The decision is stored: `route.offers` on the attempt, an `offer.applied` audit entry per offer, and one
`offer_redemptions` row per applied offer (`GET /v1/payment_intents/{id}/offers`).

## What an offer can depend on

| Condition | Field | Notes |
|---|---|---|
| Payment method | `methods` | `upi`, `card`, `netbanking`, `wallet` |
| Card network class | `card_networks` | `visa`, `mastercard`, `rupay`, `amex`, `diners`, `other` |
| BIN range class | `bin_ranges` | `[{ "from": "411100", "to": "411199" }]`, 6 digits |
| UPI handle | `upi_handles` | the part after `@` of the UPI ID the payer typed (`okaxis` or `@okaxis`) |
| UPI app | `upi_apps` | for example `gpay`; only when the caller says which |
| Bank | `banks` | for netbanking, for example `hdfc`; only when the caller says which |
| Amount band | `min_amount_minor`, `max_amount_minor` | on the amount before discount |
| Customer segment | `segment` | `new` (no earlier succeeded payment) or `returning`; needs a customer on the intent |
| Validity window | `valid_from`, `valid_until` | |
| A code | `code` | an offer with a code applies only when the payer types it; without one it applies by itself |

An offer can name card conditions only if its `methods` allows `card` (and likewise UPI and netbanking), so a condition
that can never hold is refused at creation. An offer with a per-customer cap or a segment needs a customer: with none,
the preview says `customer_required`.

**No card number, ever.** A caller may send `instrument: { card_network | card_bin, upi_app, bank }` with an attempt.
`card_bin` is the first 6 to 8 digits only; anything longer is refused with `invalid_instrument`. The BIN is used to class
the network and match a BIN range in memory and is **not stored**: the attempt keeps `{ card_network, upi_handle, upi_app,
bank, source }` in `payment_attempts.instrument`. A saved card token's brand (from our own `card_tokens`) overrides a hint.

**How far the instrument is trusted.** `source` says where it came from: `card_token`, `vpa` (the handle of the UPI ID
the payer typed) or `caller_hint`. A hint is whatever the caller or the checkout page sent; nothing checks it against the
provider after the payment. An offer keyed on a network, app or bank is therefore a **pricing nudge, not enforcement**:
a payer could claim a network they do not use. Use a budget and per-customer cap if that matters. Verifying it against the
instrument the provider reports needs real provider data this repository does not have.

## Caps and races

`max_redemptions` (all payers), `max_per_customer`, `budget_minor` (total discount). The offers an attempt could use are
locked (`SELECT ... FOR UPDATE`, in id order) inside the transaction that creates the attempt and counted again under the
lock, so concurrent attempts cannot over-use a cap. Tested with eight simultaneous attempts against a cap of one (exactly
one gets the discount), five against a per-customer cap, four against a budget.

A redemption moves `reserved` (the attempt exists, counted against every cap) to `redeemed` (captured). A failed or
cancelled attempt is `released` and the cap is given back. A capture that arrives after the attempt was marked failed (a
late webhook) takes the discount back even if the cap has been reached since: the payer already paid the lower price and
money wins. A fully refunded payment is `reversed` and frees its place under the caps.

## Stacking

An offer is `stackable` or not. A **non-stackable** offer applies alone, and among those only the best counts (most
discount, then `priority`, then the smaller id, so the result never depends on row order). **Stackable** offers combine
with each other: at most one per `stack_group` (the best), highest `priority` first, each taking its share of what is
left after the one before. The payer gets whichever way gives the larger discount; a tie goes to the single offer. A
discount never takes the whole price. Offers that were left out say why in the preview (`outranked`, `budget_exhausted`,
`redemption_limit_reached`, ...).

## Soft retry on another provider

Offers are provider-agnostic, so they survive a soft retry (see ROUTING.md section 6, `softretry.rs`). When the first
attempt fails definitively its redemption is **released** (the cap is given back), and the retry on another provider
creates a new attempt that judges the offers again under the same locks, so a capped offer is never counted twice (tested:
a cap of one, a provider that refuses the request, a retry on the other provider: one use in the end). The stored request
the retry repeats includes the instrument hint (a BIN of at most 8 digits, never a card number) and the offer codes, so
the retry is priced on the same facts, and they are cleared with it. **A retry never charges a different amount than the
attempt it repeats**: if the offers now price it differently (for example the offer was used up in between), no
automatic retry is made, the skip is recorded, and the payer simply sees the failure and can pay again at the price shown.

## Refunds

A refund hands back the same share of each discount as it hands back of the cash the payer paid, rounded half up on the
running total (so refunding everything returns the whole discount, whatever the pieces). Example: the payer paid 90,000 for
a 100,000 price (discount 10,000). A refund of 45,000 returns 5,000 of the discount: the offer's spent budget goes down by
5,000 and the redemption shows `discount_returned_minor: 5,000`. The rest of the refund returns the other 5,000, the
redemption becomes `reversed` and the offer's redemption count goes down by one. Each refund's share is recorded once in
`offer_refund_adjustments` (unique per redemption and refund) and audited as `offer.refund_adjusted`.
`POST /v1/offers/refund_preview` shows it before the refund is made.

## What is not covered

- **Not applied** to a payment on an invoice, a split (marketplace) payment, or a payment-link payment whose price already
  includes a coupon. Their amounts feed invoice allocations and vendor shares that expect the full price; the preview says
  `invoice_payment`, `split_payment` or `coupon_already_applied`.
- **No ledger entry for the discount.** It moves no money: the payer is simply charged less, and the ledger, the
  settlement and the refund all work on the amount actually charged. The cost of the offers is in the offer totals
  (`GET /v1/offers/{id}`: `discount_given_minor`, `discount_returned_by_refunds_minor`, `net_cost_minor`), not in the books.
- A payment-link payment with no coupon is offered offers like any other, on the whole price including a convenience fee the link adds.
- The customer statement's deposit lines use the intent's list price, so for a discounted deposit it shows the price and
  not the cash.
- No bank-funded or provider-funded offers, no EMI, no cashback after payment, no loyalty points, no per-day or
  time-of-day windows (use `valid_from` / `valid_until`; time-of-day is a routing-rule condition, not an offer one).
- An offer's conditions and discount cannot be changed after creation (create another); a PATCH changes only status,
  name, description, end date, caps (raise or set, not remove) and priority.
- Not checked against a real checkout page or a real provider. The console has no offers page yet.

## API

| Route | Scope | Purpose |
|---|---|---|
| `POST /v1/offers` | `offers:write` | create an offer (validated) |
| `GET /v1/offers[?status=]` | `offers:read` | list |
| `GET /v1/offers/{id}` | `offers:read` | one offer with `totals` (given, returned by refunds, net cost) |
| `PATCH /v1/offers/{id}` | `offers:write` | pause, resume, archive; name, end date, caps, priority |
| `POST /v1/offers/preview` | `offers:read` | what a payment would be offered, and why each other offer did not apply |
| `POST /v1/offers/refund_preview` | `offers:read` | how much discount a refund would hand back |
| `GET /v1/offers/{id}/redemptions` | `offers:read` | the payments an offer priced |
| `GET /v1/payment_intents/{id}/offers` | `offers:read` | which offers priced one payment, with `funded_by` |
| `POST /v1/checkout/{tenant}/{intent}/offers` | client secret | the price a payer would be asked for with a method; only the offers that apply, never a hidden code |

Attempts accept `instrument` and `offer_codes`. `finance` console roles get `offers:read`, `offers:write`; viewers get
`offers:read`; `admin` has everything. All five new tables are tenant-scoped by row-level security (tested as the app role).
