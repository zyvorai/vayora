# Events and ticketing

Sell tickets for an event on top of the payment features that already exist: payment intents and routing, coupons,
refunds (with the over-refund check and the approval threshold) and vendor splits. An organiser creates an event with
ticket tiers and a registration form, buyers pay on the public pages in `apps/zyvor-checkout`, tickets carry a signed QR
code, door staff check them in, and cancelling an event refunds every paid order as one tracked batch.

## Honest status

- **Level 1: simulators only.** Everything was run against the local provider mock and the in-process simulators.
  No real payment was made and no live or sandbox provider account was involved.
- **QR check-in was not tested on a device.** The QR payload is generated and verified by the server and the tests
  scan by sending the payload to the check-in endpoint. No camera, phone or scanner app was used. The QR image is a
  plain SVG made by the `qrcode` crate. There is no door app in this repository.
- **The public pages were checked with a type check, unit tests and a production build, not in a browser or against a
  running API.** Strings are English; Hindi covers about half of them and is machine-assisted and unreviewed by a native
  speaker. The other four checkout languages fall back to English on these pages.
- **Partial payments with reminders are not built.** The existing instalment mechanism splits an agreed total into dated
  invoices for a *customer*. A ticket order is a one-off purchase with no customer-owned invoice, and seats would have to
  stay held across the instalments, so reusing it cleanly is not possible. Left out rather than faked.
- **No email or SMS beyond one "your tickets" email.** That email is queued once per paid order when
  `ZYVOR_CHECKOUT_PUBLIC_URL` is set, and sent by the existing mail worker. No WhatsApp, no reminders before the event.
- INR only. A tier price of 0 is a free ticket: the order needs no payment and is issued at once.

## The model

| Thing | Notes |
|---|---|
| Event (`ticket_events`) | name, description, `venue` or `online` (with an https join link shown only on paid tickets), `starts_at`, `ends_at`, `capacity`, `hold_minutes` (1-180, default 10), status `draft` / `on_sale` / `closed` / `cancelled`, optional vendor shares. |
| Tier (`ticket_tiers`) | name, price in paise, `quantity` cap, `per_order_max`, sale window (`sale_starts_at`, `sale_ends_at`), optional early bird (a lower price inside its own window), optional `allowed_methods` (`upi`, `card`, `netbanking`, `wallet`). |
| Form field (`event_form_fields`) | `text`, `email`, `phone`, `select`, `checkbox`; `required`; `min_len` / `max_len`; select options. Answers are checked on the server and stored per order. Unknown keys are refused. |
| Order (`event_orders` + lines) | buyer name, email, phone; the tier quantities with the unit price frozen at order time; answers; status `pending` / `paid` / `expired` / `cancelled` / `refunded` / `needs_refund`; the payment intent. |
| Ticket (`tickets`) | one per seat, unique code, status `issued` / `checked_in` / `refunded` / `void`. `(order, seq)` is unique, so a seat can never be issued twice. |
| Scan (`ticket_scans`) | every scan, good or bad, with who and when. |

All nine tables have row-level security (`tenant_isolation`, `USING` and `WITH CHECK`). `tests/events.rs` reads them as
another tenant, tries to write across tenants and calls every organiser route with the other tenant's key.

## Seats are never oversold

`seats_held` (unpaid orders inside their hold) and `seats_sold` (paid tickets) are counters on the event and on each
tier. They change only while the **event row is locked** (`SELECT ... FOR UPDATE`), so purchases for one event are
serialised, and the database has `CHECK (seats_held + seats_sold <= capacity)` (and the same for each tier), so a bug
would fail the transaction instead of selling a seat twice. The cost of this simple design is that two buyers of the same
event wait for each other for the few milliseconds the order takes; different events never wait.

Lock order is always event first, then order, so the capture, expiry, cancellation and refund paths cannot deadlock.

Test: `concurrent_buyers_cannot_oversell_the_event_or_a_tier` sends 24 simultaneous orders for a 5-seat event and
checks that exactly 5 seats are granted and that the counters equal what the orders and tickets say. Every test that
touches seats calls the same invariant check.

## Hold and release

An order holds its seats for the event's `hold_minutes`. The worker (`ticketing::run_worker`, every cycle) releases a
pending order whose hold ran out: seats and the coupon use go back, the payment intent is cancelled so it can no longer
be paid, and `ticket_order.expired` is queued for your webhook.

- An order is **not** released while its payment is authorised or has an unknown outcome, or while an attempt was
  touched in the last five minutes: money may be on its way.
- A failed attempt does **not** release the seats at once, because the buyer may try another method inside the hold.
  They are released when the hold ends, or at once if the buyer cancels (`POST .../orders/{id}/cancel`).
- A payment that **still arrives after the release** takes the seats back if they are free. If they are not, the order
  becomes `needs_refund`, a `ticket_order_unfulfilled` case is opened, `ticket_order.needs_refund` is queued, and the
  refund goes out through the normal refund flow.
- One buyer (by email) can hold at most three unpaid orders per event, so a script cannot park the whole event.

## Payment and issuing tickets

Placing an order (`POST /v1/public/events/{tenant}/{event}/orders`) creates a normal payment intent in the same
transaction as the hold; the buyer then pays on the existing checkout page (`/pay/{tenant}/{intent}`), which already
handles methods, UPI QR, retries and the "unknown outcome" state. Splits, coupons and refunds are the existing ones:

- **Tickets are issued only after a verified capture.** `ticketing::on_payment_captured` is called from
  `payments::on_captured`, the single place a provider result is applied, in the same transaction, once per real
  transition. A repeated webhook, a refresh and a direct second call all leave the ticket count unchanged
  (`a_ticket_is_issued_only_after_the_capture_and_only_once`).
- **Discount codes** are the existing coupons: spent in the order's transaction and given back if the order is released
  unpaid. Percent or flat, never taking the total below one paisa. A coupon on a free order is refused.
- **Vendor shares** (`splits` on the event, in basis points of the order total) are attached to each order's intent and
  accrue on capture, reverse on refund, exactly as for any split payment.
- **Per-tier payment methods.** The methods of every tier in an order are intersected (an empty intersection is refused),
  stored on the intent, enforced when an attempt starts (`method_not_allowed`) and used to filter the methods the
  checkout page offers.
- Send an `Idempotency-Key` header with the order and a repeated request returns the same order.

## Check-in

Each ticket's QR holds `ZT1.<code>.<signature>`. The code is 12 characters from a 31-letter alphabet (about 59 random
bits); the signature is the first 12 bytes of an HMAC-SHA256 over the event id and code, keyed with a per-event secret
that is sealed with the master key (and included in `rekey`). A payload with the wrong signature, or one made for another
event, is `invalid`. Door staff can also type the code by hand (it is looked up, not signature-checked).

| Endpoint | Scope | What |
|---|---|---|
| `POST /v1/events/{id}/checkin` | `events:checkin` | One scan. Always 200 with `result`: `ok`, `duplicate` (with who got in first and when), `refunded`, `void`, `wrong_event`, `invalid`. The state change is one conditional `UPDATE`, so two devices scanning at once admit the ticket once. |
| `POST /v1/events/{id}/checkin/sync` | `events:checkin` | Up to 1000 scans a device collected offline, applied oldest first with their own scan times (never in the future); a ticket already in is reported as `duplicate`. |
| `GET /v1/events/{id}/checkin/export` | `events:checkin` | Valid tickets (issued and checked in, not refunded or cancelled) with their QR payloads, JSON or `?format=csv`, for a device to keep offline. A snapshot: a ticket refunded afterwards stays in it until the next export. |
| `GET /v1/events/{id}/checkin/stats` | `events:checkin` | Counts and the latest double scans. |

A key with only `events:checkin` can do exactly these four things and read nothing else (not the attendee list).

## Attendees

`GET /v1/events/{id}/attendees` (one row per ticket; `q` matches name, email, phone or exact ticket code; `status`,
`tier_id`, `limit`, `starting_after`) and `GET /v1/events/{id}/attendees.csv` (up to 20,000 rows, one column per form
field). A CSV cell that starts with `=`, `+`, `-`, `@`, tab or CR is prefixed with `'` so a name typed into the form
cannot run as a formula when the file is opened in a spreadsheet.

## Cancellation and refunds

- **Cancel an event** (`POST /v1/events/{id}/cancel`, needs `events:write` **and** `refunds:write`): sales stop, unpaid
  orders are released, every paid ticket becomes `void` and its seat is freed, and one refund per paid order is queued in a
  **batch**. The first 50 are sent in the request and the worker sends the rest each cycle. Calling it again returns the
  same batch. Free orders have nothing to refund.
- **Cancel one order** (`POST /v1/events/{id}/orders/{order}/cancel`): an unpaid order is released; a paid one has its
  tickets voided and a one-order batch queued. Refused if a ticket was already used at the door unless `force` is set.
- **Each refund is made by `refunds::reserve_refund`**, the same function `POST /v1/payment_intents/{id}/refunds` uses,
  so the over-refund protection (it refunds only what is still refundable, including a part-refund already made by hand)
  and the **approval threshold** apply: a refund above the threshold waits as `pending_approval`, and the person who
  cancelled the event cannot approve it. An item the refund flow refuses is recorded as an `error` with the reason, and
  `POST .../refund_batches/{id}/process` with `{"retry_errors": true}` queues it again.
- **Progress:** `GET /v1/events/{id}/refund_batches/{batch}` returns the number of orders, how many are queued, requested
  or errored, the refunds by status and the amounts, `finished` and `needs_attention`.
- **Report:** `GET /v1/events/{id}/refund_report` totals the money by refund status (processed, in progress, awaiting
  approval, failed or rejected, not yet sent, errors) and lists one line per order with its refund.
- When a refund is processed (by a batch or by the ordinary refund endpoint), a fully refunded order becomes `refunded`
  and its tickets `refunded`; a partial refund changes nothing.

## Public pages (`apps/zyvor-checkout`)

| Page | What |
|---|---|
| `/events/{tenant}/{event}` | The event, its tiers with the current price, early-bird label and sale-window notes, quantity steppers, the buyer's details and the registration form, an optional coupon, and "continue to pay". A draft event does not exist for the public. |
| `/pay/{tenant}/{intent}?...&eo=...&et=...` | The existing payment page; once paid it links on to the tickets. |
| `/events/{tenant}/orders/{order}?t=...` | The buyer's order: waits (and refreshes itself) while unpaid, shows each ticket with its QR code once paid, and lets the buyer cancel an unpaid order. |

The order page is protected by a token derived from the order id and the event's secret (`eot_...`), so nothing is stored
and the link in the email keeps working; a wrong token looks the same as an unknown order. The page asks the API
through `/api/zyvor-events/...`, proxied like the existing `/api/zyvor/...`.

## API

Organiser routes need an API key. Scopes: `events:read`, `events:write`,
`events:checkin`; cancelling also needs `refunds:write`. The finance role gets `events:read` and `events:write`.

| Route | Scope |
|---|---|
| `POST /v1/events`, `PUT /v1/events/{id}`, `POST .../status`, `POST/PUT .../tiers...`, `POST .../tiers/{id}/archive`, `PUT .../form_fields` | `events:write` |
| `GET /v1/events`, `GET /v1/events/{id}`, `GET .../orders`, `GET .../orders/{id}`, `GET .../attendees`, `GET .../attendees.csv`, `GET .../refund_report`, `GET .../refund_batches/{id}` | `events:read` |
| `POST .../cancel`, `POST .../orders/{id}/cancel` | `events:write` + `refunds:write` |
| `POST .../refund_batches/{id}/process` | `refunds:write` |
| `POST .../checkin`, `POST .../checkin/sync`, `GET .../checkin/export`, `GET .../checkin/stats` | `events:checkin` |
| `GET /v1/public/events/{tenant}/{event}`, `POST .../orders`, `GET/POST .../orders/{order}[/cancel]`, `GET .../tickets/{id}/qr.svg` | none (order token) |

Webhook events added: `ticket_order.paid`, `ticket_order.expired`, `ticket_order.needs_refund`, `event.cancelled`.
OpenAPI is in `openapi/zyvor.yaml` (tags `events` and `public_events`).

## What is tested

`crates/zyvor-api/tests/events.rs` (19 tests, simulators only): issue-after-capture and its idempotency; the concurrent
oversell race and the tier cap; hold expiry through the worker (seats, coupon use, intent); a failed payment; a late
payment that is re-seated or refunded; check-in (duplicate, forged, wrong event, offline sync, two devices at once,
scope separation); attendee search and CSV defusing; event cancellation with a mix of paid, part-refunded and unpaid
orders, the approval threshold, separation of duties and the over-refund guard; a 55-order cancellation finished by the
worker; single-order cancellation; a full refund through the ordinary endpoint; tier payment methods and early-bird
prices; the registration form; free tickets and vendor shares; the ticket email; tenant isolation. Unit tests in
`src/ticketing.rs` cover sale windows, early-bird edges, QR signing, parsing, form validation, CSV and vendor shares.
`apps/zyvor-checkout/lib/events.test.ts` covers the page helpers and string fallbacks.

## Not done

Partial payments and reminders (see above); a door app; resale or transfer of a ticket; seat maps; per-ticket attendee
names (every ticket carries the buyer's name); a console page (everything here is API plus the public pages); SMS.
