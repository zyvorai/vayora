# Payment links

A payment link is one address (`/pay/link/{tenant}/{token}`) that mints a fresh payment intent for each payer who opens it. This page covers the whole feature; the endpoints are in `openapi/zyvor.yaml` under `payment_links`. Everything here runs against the local simulators only: no real money, no real SMS, no live provider.

Scopes: creating, cancelling, bulk creation, emailing and texting need `payments:write`; listing, sharing, QR, export and analytics need `payments:read`. No new scopes were added. A branch-limited key can only create single links for its own branch (as before); bulk, SMS, share and analytics are closed to such keys.

## What existed already, and what this adds

Already there: fixed, open-amount and invoice links; caps (`max_uses`), expiry, cancel, coupons and a convenience fee, vendor splits, branch/counter binding, the QR (`GET /v1/payment_links/{id}/qr.svg`), emailing one link to its customer (`POST /v1/payment_links/{id}/send`) and the one reminder before expiry.

Added (migration 0083, additive): bulk creation from CSV, a reference per link, per-link wording / brand / return address / methods, paying in parts with balance reminders, privacy-safe analytics, a share text, an SMS hook, clearer states on the checkout page.

## Bulk creation from a CSV

`POST /v1/payment_links/bulk` with `{ "csv": "<file contents>", "dry_run"?, "filename"?, "message"?, "thank_you"?, "return_url"?, "use_branding"?, "allowed_methods"? }`. The last five apply to every link in the file.

Columns (any order; names ignore case, spaces and punctuation): `customer_name`, `customer_email`, `customer_phone`, `amount` (rupees, such as `1,250.50`; required), `description`, `reference`, `expires` (`YYYY-MM-DD`, valid to the end of that day in India, or a full RFC 3339 time), `currency` (INR default). Up to 500 rows and 512 KB per file.

- **Strict, line-level errors.** An unknown or repeated column, a row with the wrong number of cells, an unreadable amount, a bad email or phone number, an expiry in the past, a name missing when an email or phone is given, a reference used twice in the file: each is reported with its line and column. Nothing is stored while any remain (`422 invalid_csv`, `stored: false`).
- **Dry run.** `dry_run: true` runs the same checks, including the database ones (a reference that already names a different link), then rolls everything back. Always `200`, with `ok`, `would_create`, `existing`, a preview of the first 20 rows and the errors.
- **All or nothing.** The whole file is one transaction (each row in a savepoint so every problem is found, not only the first). A bad row near the end leaves no links and no customers behind.
- **Safe to send twice.** A row's `reference` is the link's `external_ref`, unique per business; a row without one gets a reference derived from its contents. A second upload finds the links it made and creates nothing (`repeat: true` when it is the same file, the same import is returned). A reference that already names a link with a different amount, description or customer is an error, never a second link. Two identical rows in one file are an error too, not a silent drop.
- **Customers** are matched by email, then phone (a row with neither: by name alone) and created when new; phone numbers are normalised (a bare Indian mobile number gets +91).
- **Export.** `GET /v1/payment_links/imports/{id}/export.csv` lists what an upload made or found, with each link's address (`url` is empty unless `ZYVOR_CHECKOUT_PUBLIC_URL` is set; `path` is always there). Cells that start with `=`, `+`, `-` or `@` are prefixed with `'` so a name typed by someone else cannot become a spreadsheet formula.

Single links accept the same `external_ref`: creating again with the same reference and details returns the first link (`existing: true`, 200); with different details it is `409 external_ref_conflict`.

## What the payer sees

On `POST /v1/payment_links` (and as defaults in a bulk upload):

| Field | Meaning |
|---|---|
| `message` | A note shown on the link page and in the emailed link. Plain text, up to 500 characters, no web addresses (a payment page is a poor place to send people elsewhere). Control characters are dropped and runs of blank lines squeezed; it is shown only as text, never as markup. |
| `thank_you` | Shown on the paid page. Same rules, up to 300 characters. |
| `return_url` | A "Back to the business" button on the paid page. https only, no credentials; checked against the business's `return_url_origins` (migration 0073) when it has set any, exactly as for payment attempts. It is a button, not an automatic redirect. |
| `use_branding` | Show the business's logo and accent colour from its email look in settings (`email_logo_url`, `email_accent`). The logo is shown only if it is an https address. The colour is used only if it can carry text and stand out from the white card (button text at least 4.5:1, fill against the card at least 3:1, WCAG); otherwise the page keeps its own colour. The colour is applied in light mode only. |
| `allowed_methods` | Subset of `upi`, `card`, `netbanking`, `wallet`. The checkout offers only these, and the attempt endpoint refuses any other (`422 method_not_allowed`) before a provider is called. If none of the allowed methods is offered by your enabled providers, the payer sees no method to choose; check this when you set it. |

The checkout session (`GET /v1/checkout/{tenant}/{intent}`) carries a `link` block with these for payments made through a link.

## Paying in parts

`allow_partial: true` with `min_part_minor` (and optionally `balance_reminder_days`, 1 to 60, which needs `customer_id`). Works on fixed-amount links and invoice links. Not with open amounts, `max_uses` or splits; coupons do not apply to parts. A convenience fee, if any, is added to each part.

- The payer chooses an amount from the smallest part (or whatever is left, if that is less) up to the balance; by default the whole balance. Each part is a normal payment intent with its own checkout.
- **Balance.** For a fixed-amount link: the amount minus the sum of intents through this link that the capture path has marked succeeded (a provider's verified result, never something a browser reports). For an invoice link: the invoice's own settled amount, so payments made by other means count too and the invoice moves to `partially_paid` and `paid` by the normal rules. The link's status becomes `paid` only when the balance reaches zero. A failed, abandoned or unknown payment moves nothing. Refunds do not reopen a link.
- **Concurrent payers.** Starting a checkout holds its amount for 30 minutes against other payers (the link row is locked while this is worked out, so two payers cannot both claim the same balance). While the whole balance is held, new starts get `409 link_busy`. After 30 minutes an unfinished checkout stops holding. If two checkouts both capture (one started more than 30 minutes ago and finished late), the link reports `overpaid_minor` in its analytics and the business refunds the excess by the normal refund route.
- **Balance reminders.** `balance_reminder_days` days after the customer's last part (or the last reminder), while a balance is left, the worker emails the customer what is paid and what is left, and texts it when SMS is on. At most three reminders per link, none once paid, cancelled or expired, none by email to an address that unsubscribed. Mail goes through the normal mail queue (`mail::enqueue_once_as`, marked automatic).
- Uses of the existing "instalments" mechanism: an invoice that belongs to an instalment plan can have a partial link like any invoice; plans themselves (fixed dated invoices) are unchanged. Parts of one link are not instalment plans.

## Analytics

`GET /v1/payment_links/{id}/analytics?days=30` and `GET /v1/payment_links/analytics?days=30` (all links of the business): totals, a per-day series (UTC days, 1 to 90 days), a funnel (`started_of_opened`, `paid_of_started`, `paid_of_opened`, each capped at 1 and `null` when there is nothing to divide by) and, per link paid in parts, the balance.

| Number | Where it comes from |
|---|---|
| `opened` | A counter: the link page was loaded by a coarse visitor token, once per token per day. |
| `started` | A counter: a checkout was started by a coarse visitor token, once per token per day. |
| `paid` | Read from the payments: intents through the link that were captured (and `paid_minor`, their base amounts). |
| `failed` | Read from the payments: intents through the link that had a failed attempt. |
| `expired` | Read from the links: links whose expiry passed unpaid (a part-paid link: with a balance still owing), on the day they expired. |

**Privacy.** No personal data in any counter. The visitor token is a keyed hash (key derived from the master key) of the UTC day, the link, the address rounded to /24 (IPv4) or /48 (IPv6), and the device class, system and browser read from the user-agent. Only the hash is stored, only for the day (the worker deletes it after two days); the counters are plain numbers. No raw address, user-agent text, cookie or customer id is stored.

**Bots.** Requests from clients that identify as machines (crawlers, link-preview fetchers such as WhatsApp or Slack, command-line tools, scripting libraries, headless browsers, an absent or implausibly short user-agent) are not counted. A repeated visit by the same coarse token the same day is not counted again, so reloading or hammering the page does not move the numbers. At most 5,000 distinct tokens per link per event per day are counted. `paid`, `failed` and `expired` cannot be raised by a bot: they are read from payments and links.

**Limits, plainly.** A token is a coarse guess at one visitor. People sharing a network and the same kind of phone and browser (a school, an office) count once; one person on a changing mobile address counts more than once; a bot that looks like a browser and comes from many networks is counted. Treat `opened` and `started` as estimates of reach and `paid` as exact. Counting needs the checkout app to pass the visitor's address on (`ZYVOR_TRUST_PROXY_HOPS` set to the number of proxies in front of the API); if the API only ever sees the checkout server's address, all visitors with the same kind of device collapse into one, which undercounts. Nothing is counted for a visit that never reaches the API (a cached page). Not benchmarked.

## Sharing

- **Copy link / QR / email** (existing): the link's address is in the list and the share response; `GET /v1/payment_links/{id}/qr.svg`; `POST /v1/payment_links/{id}/send` (email, once per link per day).
- **Share text**: `GET /v1/payment_links/{id}/share` returns `url`, a plain `text` to paste anywhere, a short `sms_text`, an `email_subject`, and whether the customer can be emailed or texted. The wording states who is asking, how much, for what, how to pay in parts if that is allowed, until when, and ends with "Pay only if you were expecting this from {business}". No urgency, no promises. For a part-paid link it states what is left, not the original amount. Needs `ZYVOR_CHECKOUT_PUBLIC_URL`.
- **SMS**: there was no SMS mechanism, so this adds only a provider-agnostic hook, off by default (`src/sms.rs`). `ZYVOR_SMS_MODE`: `off` (default; `POST /v1/payment_links/{id}/send_sms` is `409 sms_not_configured`), `sandbox` (the text is recorded as `logged` in `sms_queue`; nothing leaves the server; the log line shows only the last four digits), `hook` (the text is queued and the worker posts `{id, to, body}` signed with `Zyvor-Signature` like a webhook to `ZYVOR_SMS_HOOK_URL`, secret `ZYVOR_SMS_HOOK_SECRET`, at least 16 characters; the business's own relay sends it through its own provider). Retries with backoff, dead after 6 tries; the number and text are blanked once sent. Once per link per day. Zyvor holds no SMS provider credentials and registers no sender or template; in India that registration is the business's and its provider's job. The hook has only been exercised against a local stand-in receiver. No WhatsApp.

## Checkout states and languages

The link page now says plainly, with a heading and what to do next, when a link is expired, used up, cancelled, already paid, or has its balance held by another payer. Errors are announced (`role="alert"`), the amount field has `aria-invalid` and `aria-describedby`, and the server's reasons (`link_busy`, `amount_out_of_range`, `link_unavailable`) have their own wording. The new strings are in all six languages through the existing mechanism (`apps/zyvor-checkout/lib/i18n.ts`); the Hindi, Marathi, Tamil, Telugu and Bengali versions of the strings added here are machine-assisted drafts, like the rest, and need a native reader before relying on them.

## Not done

- No console page for any of this (API and docs only). The Node, Python and Go SDKs have the generated types but no methods for the new endpoints yet.
- Balance reminders and the SMS hook run in the worker and were exercised with a test clock and a stand-in receiver, not over a long period.
- Parts are not refunded back into a link's balance; a part-paid link has no minimum interval between parts.
- Analytics days are UTC, not IST.
