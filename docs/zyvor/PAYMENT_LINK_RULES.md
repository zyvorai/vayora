# Payment link rules: amount modes, lifecycle, payer-instrument rules

Status: **implemented and tested against the in-process simulator and the local provider mock only.** No real provider
was used. The instrument rules in particular depend on what a provider reports about the payer (see below).

Code: `crates/zyvor-api/src/link_amount.rs` (amount modes), `link_lifecycle.rs` (status model, transitions, events),
`instrument.rs` (payer-instrument rules and auto-refund), `crates/zyvor-providers/src/lib.rs` (`payer_instrument`).
Tests: unit tests in those files and `crates/zyvor-api/tests/link_rules.rs`. Migration `0087`.

These are additive. A link created without the new fields behaves exactly as before, and the original handlers in
`payment_links.rs` are unchanged: `link_amount` and `link_lifecycle` wrap them (`POST /v1/payment_links`,
`GET /v1/payment_links`, `POST /v1/payment_links/{id}/cancel`, `GET /v1/checkout/links/{tenant_id}/{token}`).

## 1. Amount modes

`POST /v1/payment_links` takes `amount_mode`:

| mode | the payer may pay | you give |
|---|---|---|
| `exact` (default) | the amount on the link, or what an invoice still owes | `amount_minor` or `invoice_id` |
| `minimum` | that amount or more (no cap but the platform ceiling) | `min_amount_minor` |
| `up_to` | any amount from 1 minor unit up to a cap | `max_amount_minor` |
| `pay_what_you_want` | any amount, optionally bounded | `min_amount_minor` and/or `max_amount_minor`, and a `suggested_amount_minor` shown first |

- Bounds are inclusive, in whole minor units, positive, and never above the platform ceiling. A mode that is given
  fields it does not take (a `minimum` link with a maximum, an `exact` link with a minimum, any open mode with an
  amount or invoice) is refused with `invalid_amount_mode`; nothing is created.
- Enforced on the server when the payer starts a checkout (`POST /v1/checkout/links/{tenant_id}/{token}/start`): an
  amount outside the rule is `422 amount_out_of_range`, a missing amount is `amount_required`, and a payer-chosen
  amount on an `exact` link is `amount_not_allowed`. The payment intent carries the amount the payer chose.
- The public link details (`GET /v1/checkout/links/{tenant_id}/{token}`) add `amount_mode`, `suggested_amount_minor`
  and `amount_rule`, one sentence the page can show ("Pay ₹500.00 or more. You choose the amount.").
- Coupons and the convenience fee still apply to `exact` links only. Links made before this change have no stored mode
  and read as `exact` (or `pay_what_you_want` if they were open-amount links).

## 2. Lifecycle

One status vocabulary for every link, beside the older `status` field (which keeps its spelling, `canceled` and all).

| status | meaning |
|---|---|
| `created` | The link exists and can be paid; no checkout has been started. |
| `active` | A checkout was started and the link can still take payments. |
| `partially_paid` | Money has come in but the link expects more (an invoice with a balance, a part-payment link with a balance, a link capped at several uses that has not used them all). |
| `paid` | The link has everything it expects: its invoice is settled, its balance is zero, or every use it was capped at has been paid. An open, reusable link has no total to reach and never becomes `paid`. |
| `expired` | The expiry passed before the link was paid in full. |
| `cancelled` | The merchant cancelled it. |
| `failed` | Capped at N uses, all N checkouts were started, none paid, none in progress. |

Computed (never set by a client) in this order, first match wins: `paid`, `cancelled`, `failed`, `expired`,
`partially_paid`, `active`, `created`. A payment that finishes after a cancel or an expiry (a checkout opened earlier)
can still complete the link and move it to `paid`.

Allowed transitions; anything else is refused and audited (`payment_link.lifecycle_refused`), never applied:

```
created        -> active, partially_paid, paid, failed, expired, cancelled
active         -> partially_paid, paid, failed, expired, cancelled
partially_paid -> paid, expired, cancelled
failed         -> partially_paid, paid, cancelled
expired        -> paid, cancelled
cancelled      -> paid
paid           (final)
```

Each change is written once to `payment_link_status_events` (compare-and-set, so concurrent workers record it once) and
raised as `payment_link.created`, `.activated`, `.partially_paid`, `.paid`, `.expired`, `.cancelled` or `.failed`
(in `GET /v1/events/catalogue`). Creating and cancelling record theirs at once; the worker records the rest each cycle.

API: `GET /v1/payment_links/lifecycle` (the model above, machine-readable), `GET /v1/payment_links/{id}/lifecycle`
(status computed now, the recorded status, `payable`, `paid_minor`, history), and `lifecycle_status`, `payable`,
`paid_minor` on each row of `GET /v1/payment_links`.

Relation to the payment-link-upgrades work (part payments, branch `feat/payment-link-upgrades`): this model reads
that branch's `allow_partial` column when it exists (`to_jsonb(link)->>'allow_partial'`, which is simply false
where the column is absent) and counts a part-payment link as `partially_paid` until its balance is paid. Nothing here
edits those handlers; whoever merges second should check `partially_paid` for part-payment links.

## 3. Payer-instrument rules

`instrument_rules` on a link (in the create body, or `PUT /v1/payment_links/{id}/instrument_rules`) or on one payment
(`PUT /v1/payment_intents/{id}/instrument_rules`, before it succeeds; it wins over the link's):

```json
{ "allowed_banks": ["HDFC", "SBI"], "allowed_vpa_handles": ["okhdfcbank", "oksbi"] }
```

**The limitation, stated first.** The rule is applied after capture, not before. A payer's bank or UPI id is only
known when the provider reports it on the payment record or webhook, and none of the adapters lets us refuse a UPI id
before the payer pays. So a payment from a disallowed instrument is captured and then refunded through the normal
refund flow (the refund approval threshold applies; the requester is `system:instrument_rules`), a case
`disallowed_instrument` is opened, and `payment.instrument_refused` is raised.

**When the provider does not say who paid, nothing is checked** and the check is recorded as `unverifiable`
(`GET /v1/instrument_checks`); the payment is kept. Today Razorpay (`vpa`, `bank`) and Cashfree
(`payment_method.upi.upi_id`, `payment_method.netbanking.netbanking_bank_code`) are read from their documented
fields, and the simulator reports a VPA and a guessed bank; PayU, PhonePe and custom providers report nothing here.
The Razorpay and Cashfree readings were checked against this repository's mocks only.

Each list applies only to the attribute the provider reported: a reported VPA must have a handle on
`allowed_vpa_handles` (if set), a reported bank must be on `allowed_banks` (if set). Any applicable list rejecting means
`disallowed`; at least one applicable list accepting everything means `allowed`; otherwise `unverifiable`. A payer paying
by UPI from a bank not on `allowed_banks` is therefore only caught if the provider reports the bank. Only the handle of
a UPI id is stored, never the name part. The worker checks each captured payment once and retries a refund that did not
get made.
