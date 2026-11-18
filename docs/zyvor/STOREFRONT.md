# Seller page ("link in bio" storefront)

A small hosted page for a seller who has no website: a title, a short description, an optional logo and a list of
items. A visitor chooses items, gives a name and a mobile number, and pays through the ordinary checkout. The seller
gets an order list with fulfil, cancel and refund actions. Public page: `/s/{slug}` in `apps/zyvor-checkout`.

## Honest status

- **Level 1: built and tested against the local simulators.** Payments in the tests go through the Razorpay simulator.
  Nothing here has run against a live provider, and no real money moved.
- The seller API, the public API, the stock rules and the isolation rules have integration tests (including a
  concurrent-order race). The visitor page (`/s/{slug}`) type-checks, builds and has unit tests for its cart logic; it
  was **not run in a browser** in this slice, and its translations (Hindi, Marathi, Tamil, Telugu, Bengali) were written
  without review by native speakers, like the rest of the checkout's. Have each checked before relying on it.
- No console page yet: everything is the API. No shipping or delivery charges, no discounts or coupons, no variants
  (size, colour), no carts that outlive the page, no order status page for the buyer, no email or SMS to the buyer.

## How an order works

1. The seller creates the page (`PUT /v1/storefront`), adds items and publishes it (`published: true`). A page that is
   not published does not exist for visitors.
2. A visitor opens `/s/{slug}`, picks quantities and posts the order (`POST /v1/store/{slug}/orders`). **Prices come
   from the page, never from the request.** The server takes the stock and opens a payment intent in one database
   transaction, and answers with the intent id and its client secret; the page then sends the visitor to
   `/pay/{tenant}/{intent}`, the same checkout that payment links use.
3. When the payment is captured, the order becomes `paid` in the same transaction as the capture. The seller marks it
   `fulfilled`, or refunds it.

Statuses: `pending` (waiting for payment), `paid`, `fulfilled`, `cancelled`, `expired`. Refund progress is shown on the
order (`refunded_minor`, `refund_in_progress_minor`), not as a status.

## Stock

- An item has an optional count. No count means "not counted". A count never goes below zero (a database check as well
  as the code).
- **No oversell.** The order takes stock with one conditional `UPDATE ... WHERE stock >= quantity` per item, inside the
  transaction that creates the order. Postgres serialises concurrent updates of a row, so of 12 visitors racing for 5
  items exactly 5 succeed and 7 are told `out_of_stock`; a rejected order takes nothing. Lines are taken in a fixed
  order, so two orders for the same items cannot wait on each other. A race test covers both.
- **Holds.** The stock of an unpaid order is held for 30 minutes (`HOLD_MINUTES`). A hold that lapses is given back by
  the next visit to the page, the next order on it, or the worker (once per hold). Before stock goes back the order's
  intent is closed (`canceled`), so nobody can pay for stock that was given back. If a payment is already in flight
  (created, pending, authorised or **unknown**) the hold is **not** released and the intent is not closed: the unknown-
  outcome rule is never weakened.
- **Money wins.** If a payment is captured for an order whose stock was already given back (it should not happen, since
  the intent is closed first; it is handled anyway), the order is marked `paid`, the stock is taken again if there is
  any, and if there is none the order is flagged `needs_attention` so the seller can ship late or refund. Stock never
  goes below zero.
- Adjust stock safely with `stock_delta` (adds or subtracts in one step and is refused if it would go below zero).
  `stock` sets an absolute value and can overwrite a concurrent change; use it for a stock take, not while orders arrive.
- **Cancel** is for an order that is waiting for payment: the stock goes back and the intent is closed. A paid order is
  refunded instead; a full refund with `restock: true` puts the stock back (once) and marks the order `cancelled`.

## Refunds

`POST /v1/storefront/orders/{id}/refund` calls the ordinary refund flow for the order's payment: the approval threshold,
the over-refund reservation, the idempotency rules and the unknown-outcome handling all apply, and it needs an
`Idempotency-Key` and the `refunds:write` scope as well as `storefront:write`. Repeating the same request neither refunds
nor restocks twice.

## Scopes

| Scope | Allows |
|---|---|
| `storefront:read` | Read the page, items and orders |
| `storefront:write` | Change the page and items; upload images; fulfil, cancel, refund orders |
| `refunds:write` | Needed as well to refund an order |

The finance console role gets both storefront scopes. The public endpoints (`/v1/store/...`) need no key.

## Abuse controls

| Risk | Control |
|---|---|
| Slug squatting and look-alikes | 3 to 40 characters of `a-z`, `0-9` and single hyphens; not all digits; **reserved words** (`admin`, `api`, `pay`, `checkout`, `support`, ...) and any slug containing a payment brand or authority name (`razorpay`, `paytm`, `npci`, `zyvor`, ...) are refused. The slug is set once. This is a starter list, not a complete defence against impersonation: review pages before promoting them. |
| HTML or script injection | Text is stored as typed and **escaped where it is shown** (React escapes the page; the PDF and CSV writers escape theirs). Control characters, zero-width and bidirectional override characters are refused. There are no user-supplied URLs anywhere, so no external image or link can be planted. |
| Malicious images | PNG, JPEG, GIF or WebP only, recognised by their first bytes (never the claimed type), up to 200 KB, at most 60 per business. **SVG is refused** (it can carry script). Images are served with `X-Content-Type-Options: nosniff` and `Content-Security-Policy: default-src 'none'; sandbox`, and only for a published page. |
| Oversized content | Title 80, description 500, item name 80, item description 300, delivery note 300 characters; at most 100 items; at most 20 lines of 1 to 20 each per order. |
| Order floods | Per visitor address: 10 orders, then one more every 20 seconds. Per page: 120, then 2 a second. At most 200 unpaid orders per page. Past the limits the answer is `429` (`rate_limited`) or `409` (`store_busy`). The address is the one the API trusts (`ZYVOR_TRUST_PROXY_HOPS`, or the signed address a console vouches for); limits are in memory per replica, as the existing limiter is. Many people behind one carrier address share a visitor budget. |
| Holding stock hostage | The limits above, plus the 30-minute hold. A determined attacker with many addresses can still hold some stock for a short time; this is a guard rail, not a guarantee. |
| Search engines | `noindex` is **on by default**; the page is sent with `robots: noindex, nofollow` until the seller sets `noindex: false`. |
| Buyer data | Name, mobile number, optional email and a delivery note are kept on the order and shown only to the business's own keys. They are never written to the audit trail or logs. See the data note below. |
| Another business's data | Every table has row-level security; foreign keys cannot be used to attach another business's image or item (checked as the tenant first); tests cover reads, writes and the unscoped database role. |

## Data note

The page collects what a seller needs to deliver an order: the buyer's name and mobile number, an optional email and a
free-text delivery note. Treat these as personal data under India's DPDP Act, 2023: tell buyers what you use them for,
delete orders you no longer need, and do not ask for more than you need. Zyvor stores the fields on the order, does not
send them anywhere, does not log them, and has no retention job yet (a seller deletes by contacting the platform
operator, or the business removes its test data with `POST /v1/test_data/reset` in test mode). This is a description of
what the code does, not legal advice.

## API summary

Seller (key needed): `GET|PUT /v1/storefront`, `POST /v1/storefront/images` (the body is the image),
`GET|POST /v1/storefront/items`, `PATCH /v1/storefront/items/{id}`, `GET /v1/storefront/orders`,
`GET /v1/storefront/orders/{id}`, `POST .../fulfill`, `POST .../cancel`, `POST .../refund`.
Public: `GET /v1/store/{slug}`, `GET /v1/store/{slug}/images/{id}`, `POST /v1/store/{slug}/orders`. In the checkout
app these are reached as `/api/store/...` (a rewrite to the API, like `/api/zyvor/...`).

See [openapi/zyvor.yaml](../../openapi/zyvor.yaml) for the shapes.

## What is not done

- A console page for the seller (items, orders, fulfil, refund), a QR for the page's address, and buyer notifications.
- Shipping, taxes and GST invoices on orders, coupons, variants, delivery-area rules.
- Moderation by the platform (suspending a page): the page is controlled by its owner only. A business in live mode is
  subject to the same live-access review as any other use of live payments; the page adds no new gate.
- Real-browser and device testing of the visitor page; native-speaker review of the translations.
