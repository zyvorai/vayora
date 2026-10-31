# Provider HTTP contract

The JSON API your own gateway implements so Zyvor's generic `custom` adapter can use it in place of Razorpay or Cashfree. The adapter is [`crates/zyvor-providers/src/custom.rs`](../../crates/zyvor-providers/src/custom.rs); the tests in [`tests/custom.rs`](../../crates/zyvor-providers/tests/custom.rs) run it against a stub that follows this page. Why this exists and when to use it: [BRING_YOUR_OWN_PROVIDER.md](BRING_YOUR_OWN_PROVIDER.md).

## Ground rules
- Every request carries `Authorization: Bearer <api_key>` (the `api_key` you registered).
- Bodies and responses are JSON. **Amounts are integer minor units** (₹499.00 is `49900`). Currencies are ISO codes (`INR`, `USD`).
- The base URL must be a public `https` address. Redirects are not followed.
- **Status codes carry meaning.** 2xx is an answer. 5xx, a timeout or a dropped connection after the request was sent means *the outcome is unknown*: Zyvor will not retry it elsewhere and will ask you what happened. 4xx means *definitely not done*. A refused connection also counts as definitely not done.
- `attempt_id` is Zyvor's own reference. Use it as your reference too, so a payment can be found again even if the response was lost.

## Payments

### `POST /payments` creates a payment
```json
{ "attempt_id": "att_01H…", "amount_minor": 49900, "currency": "INR", "method": "upi",
  "vpa": "asha@okzyvor", "customer_id": null, "customer_phone": null,
  "capture_automatic": true, "return_url": null }
```
`method` is one of `upi`, `card`, `netbanking`, `wallet`. Respond:
```json
{ "status": "pending", "provider_payment_ref": "gw_123",
  "action": { "type": "upi_collect", "vpa": "asha@okzyvor" } }
```
- `status`: `pending`, `authorized`, `captured` or `failed`. Anything else is treated as an unknown outcome, never as success or failure.
- `failure_reason` (optional string) explains a `failed`.
- `action` tells the payer's device what to do next: `{"type":"none"}`, `{"type":"upi_collect","vpa":"…"}`, or `{"type":"hosted_checkout","params":{…}}` to open your hosted page. Card data must stay on your side.

### `GET /payments/{attempt_id}` looks a payment up by our reference
Same response as above. Return **404 if you never saw it**: Zyvor reads that as a definite `failed` and unblocks the payment. This is how a lost response is resolved.

### `POST /payments/{provider_payment_ref}/capture` captures an authorised payment
Body `{ "attempt_id": "…", "amount_minor": 49900, "currency": "INR" }`. Respond with the payment shape.

## Refunds

### `POST /refunds`
```json
{ "refund_id": "rfd_01H…", "attempt_id": "att_01H…", "provider_payment_ref": "gw_123",
  "amount_minor": 10000, "currency": "INR" }
```
Respond `{ "status": "pending" | "processed" | "failed", "provider_refund_ref": "rf_9", "failure_reason": null }`. `refund_id` is Zyvor's, and is the key to look the refund up again.

### `GET /refunds/{refund_id}`
Same response shape.

## Settlements

### `GET /settlements?date=YYYY-MM-DD`
```json
{ "settlements": [ {
  "provider_ref": "st_1", "utr": "UTR123", "currency": "INR", "settled_on": "2026-10-30", "net_minor": 49000,
  "items": [
    { "kind": "payment", "provider_ref": "gw_123", "amount_minor": 49900, "description": "payment" },
    { "kind": "fee", "provider_ref": null, "amount_minor": -900, "description": "gateway fee" }
  ] } ] }
```
`kind` is `payment`, `refund`, `fee` or `adjustment`. Amounts are signed from the merchant's view: payments positive, refunds and fees negative. **The items must add up to `net_minor`**, which is what lets Zyvor explain every settlement.

## Webhooks (you call Zyvor)

POST events to `/v1/webhooks/custom/{tenant_id}` on the Zyvor API. Sign each one:

| Header | Value |
|---|---|
| `x-zyvor-timestamp` | Unix time in seconds |
| `x-zyvor-signature` | lower-case hex of `HMAC-SHA256(webhook_secret, "{timestamp}.{raw body}")` |

Events more than five minutes from Zyvor's clock are rejected as possible replays. Sign the exact bytes you send.

```json
{ "event_id": "evt_001", "type": "payment.captured", "attempt_id": "att_01H…",
  "provider_payment_ref": "gw_123", "amount_minor": 49900 }
```
- `type`: `payment.authorized`, `payment.captured`, `payment.failed`, `refund.processed`, `refund.failed`. Other types are accepted and ignored.
- **`event_id` is required and unique.** Delivery is at least once, so send the same `event_id` when you retry and Zyvor will apply it once. An event without one is refused.
- Refund events carry `refund_id` (ours) and `provider_refund_ref`. Failures carry `failure_reason`.

Signing example (Node):
```js
import { createHmac } from "node:crypto";
const ts = String(Math.floor(Date.now() / 1000));
const sig = createHmac("sha256", process.env.WEBHOOK_SECRET).update(`${ts}.${body}`).digest("hex");
// send `body` unchanged with x-zyvor-timestamp: ts and x-zyvor-signature: sig
```

## Capabilities
The router decides where to send a payment from what the adapter declares. The generic adapter defaults to UPI and card in INR with partial refunds; if your gateway differs, change it where the adapter is built (`Custom::with_capabilities`). Declaring something you cannot do sends real payments to a gateway that will fail them.

## Status
Verified only against the local stub, never a live gateway. Run your own gateway through your provider's sandbox before you rely on it.
