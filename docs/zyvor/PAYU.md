# PayU adapter

**Status: simulator only. Built from PayU's public documentation, not verified against a live sandbox or account.**
The adapter ([`crates/zyvor-providers/src/payu.rs`](../../crates/zyvor-providers/src/payu.rs)) was written from the pages
listed below and tested against `zyvor-provider-mock`, a PayU simulation written from the same pages. The two can agree
with each other and both be wrong about PayU. Treat every row under [Needs live-sandbox verification](#needs-live-sandbox-verification)
as unconfirmed until it has been run with PayU test credentials.

## What it does

| Capability | How | PayU's documentation |
|---|---|---|
| Take a payment | **Hosted checkout.** `initiate` makes no network call: it returns the form the payer's browser POSTs to PayU's `/_payment`, with a SHA-512 hash computed on the server. The payment stays `Pending`. UPI, cards, net banking and wallets are whatever the merchant has enabled on PayU's page. | Web Integration - PayU Hosted, Generate Hash |
| Look a payment up | `verify_payment` (`sha512(key\|command\|var1\|salt)`, `var1` = our transaction id) | Verify Payment API |
| Refund (full or partial) | `cancel_refund_transaction` (`var1` PayU id, `var2` our token, `var3` amount). Asynchronous: starts `Pending`. | Refund Transaction API |
| Follow a refund | `check_action_status` with `var2=payuid`, matching our token | Check Refund Status API |
| Payment webhook | Form-encoded, verified with PayU's reverse hash | Webhook Events and Sample Payloads, Generate Hash |
| Settlement report | Settlement Detail Range API (HMAC `Authorization` from the merchant secret) | Settlement Detail Range API |

Declared capabilities: UPI, card, net banking and wallet; INR only; partial refunds; **no** manual capture, **no** server-sent
UPI collect, **no** UPI intent or QR, **no** mandates. Those exist in PayU's documentation as separate server-to-server
products and are not built here, so they are not claimed.

### Ids
PayU limits our transaction id to 25 characters and the refund token (`var2`) to 23. Our attempt and refund ids are
longer, so both are derived deterministically from them (`txnid_for`, `refund_token_for`, hex of a SHA-512, alphanumeric).
The derivation is what lets a lost response be looked up again, and what makes a repeated refund id safe: PayU answers a
reused token with "token already used or request pending", and the adapter then reads the refund that already exists.

### Payment states
Mapped as PayU's "Payment State Explanations" table says, on `unmappedstatus`: `captured` is paid; `auth` is `Authorized`;
`userCancelled`, `bounced`, `dropped`, `failed`, `autoRefund` are failed; `initiated`, `in progress`, `pending` are pending.
Anything else is an **unknown outcome**, not a failure. A bare `status: success` without `unmappedstatus` is also unknown
(it covers `auth` too), so money is only called captured when PayU says `captured`.

### Webhooks
* The payment callback is verified with the reverse hash `sha512(SALT|status||||||udf5|…|udf1|email|firstname|productinfo|amount|txnid|key)`
  (with `additionalCharges|` in front when present). The hash is keyed by the salt held in the credentials, so the stored
  `webhook_secret` is not used for PayU (any non-empty value is accepted when the account is saved).
* PayU sends no timestamp and no event id. The event id is built from fields the hash covers, `payu:{txnid}:{status}`, so a
  redelivery de-duplicates and a later event for the same transaction does not.
* `unmappedstatus` and `mihpayid` are **not** covered by the hash. The adapter decides the outcome from the signed `status`
  only; `unmappedstatus` can only make it do less (an `auth` is ignored and left to a lookup).
* PayU's **refund and dispute callbacks are JSON and the documentation describes no signature for them.** The adapter refuses
  them (`MissingSignature`) rather than trust an unauthenticated body. Refund state comes from `check_action_status`
  (`POST /v1/refunds/{id}/refresh`, and the worker for refunds left unknown or processing).

### Settlements
`fetch_settlements` needs `merchant_secret` in the credentials; without it the answer is `Unsupported`. PayU's `settlementAmount`
is kept as the net. Items are rebuilt from the transaction rows: payment, a fee equal to PayU's own row net minus the gross,
refunds negative, chargebacks and debits negative, reversals and credits positive. If PayU's net includes something the rows
do not explain, one labelled adjustment carries the difference, so the items always add up to the net.

## Credentials and setup
`PUT /v1/provider_accounts/payu` with `credentials: { "key": "...", "salt": "...", "merchant_secret": "..." }` (`merchant_secret`
optional) and any non-empty `webhook_secret`. With no `base_url` the adapter uses PayU's documented hosts for the
environment (test: `test.payu.in`, `apitest.payu.in`; live: `secure.payu.in`, `info.payu.in`); a `base_url` points every call at one
address, which is how the simulation is used. A PayU production address is refused in a test environment. Register
`/v1/webhooks/payu/{tenant_id}` as the webhook URL in PayU's dashboard, per environment. A `return_url` is required on every
attempt, because PayU makes `surl` and `furl` mandatory; without one the attempt is refused before anything is sent.
All values in tests and docs are obviously fake (`payu_test_key`, `payu_test_salt`).

## Sources (docs.payu.in, read November 2026)
Web Integration - PayU Hosted; Generate Hash; Verify Payment API; Refund Transaction API; Check Refund Status API (with PayU ID,
with Request ID); Webhook Events and Sample Payloads; Settlement Detail Range API; Settlement Transaction Details API;
APIs used for Integration; General FAQs.

## Needs live-sandbox verification
Where the documentation is ambiguous or silent the conservative reading is coded. Each of these should be checked with PayU test credentials.

| # | Question | What is coded |
|---|---|---|
| 1 | Refund `var3` amount: the page's own split example is decimal rupees (`0.10`) but its API schema types it as an integer | Decimal rupees, two places (`500.00`) |
| 2 | `check_action_status`: the page uses both `check_action_status` and `check_action_status_txnid`; the response is keyed by payu id then request id in the sample but flat in the schema; status words appear upper case (`QUEUED`) and lower case (`success`) | Command `check_action_status` with `var2=payuid`; both response shapes read; status compared case-insensitively |
| 3 | Does PayU sign refund callbacks (header or hash)? The page documents none | Refused as unsigned; refund state is polled |
| 4 | Reverse hash with additional charges: the formula says `additional_charges`, the payload list says `additionalCharges` | Reads `additionalCharges` |
| 5 | Hash case: samples lower-case it, the prose does not say | Sent lower case; a received hash is compared case-insensitively |
| 6 | Transaction id limits: 25 characters on one page, none on another; allowed characters not stated | 25 characters, alphanumeric only |
| 7 | `verify_payment` answers "Not Found" until the payer reaches PayU's page | Treated as `Pending`, never as failed. Abandoned attempts rely on the platform's own expiry |
| 8 | `dropped` is a "failure" in PayU's table; can a reconciliation later turn it into a success? | Failed; a later `captured` webhook is applied by the state machine like any late event |
| 9 | Server host and path: `postservice.php?form=2` in the environment table, `postservice?form=2` in samples; one page lists only `info.payu.in` | `/merchant/postservice.php?form=2` on `test.payu.in` (test) and `info.payu.in` (live) |
| 10 | Does PayU accept the placeholder `payer@zyvor.invalid` and `9999999999` when we have no payer details (`email`, `phone`, `firstname` are mandatory)? | Sent as placeholders, the same way the other hosted adapters do |
| 11 | Settlement Range API: which secret signs (`merchant_secret` from the dashboard vs the salt), the `username` (merchant key in the sample code, `smsplus` in the table), the sign of refund and `ADJ_*` rows, and how a requested date maps to `settlementCompletedDate` | `merchant_secret`; key as username; signs by action name; the date requested is used for the day |
| 12 | Webhook delivery: retries and the response PayU expects are not documented; source IPs are, and cannot be checked inside an adapter | Idempotent by event id; return 200 with a JSON body |
| 13 | Refund on the day of capture returns status 1 with "check the status the next day" | Reported `Pending` like any queued refund |
| 14 | Dispute (chargeback) webhooks | Not handled |
| 15 | `mihpayid` has stray spaces in the refund sample (`" 23907365951 "`) | Trimmed |
