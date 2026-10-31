# Partner setup and end-to-end test plan: Cashfree

The spec requires current partner documentation before any concrete endpoint is written. This page records what Cashfree's documentation says (read on 31 October 2026), what is still unconfirmed, and the order of the end-to-end test. Re-read the linked pages before relying on this: documentation can lag or conflict.

**Completion level today: 1, implemented with simulator.** Nothing here has run against Cashfree's sandbox yet. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

## Why Cashfree
The merchant already has a live Cashfree account. An adapter for it exists (`crates/zyvor-providers/src/cashfree.rs`), built from public docs and tested only against the local mock. The path is to run that adapter against Cashfree's sandbox, then add native UPI intent and QR.

## What the documentation says
Source pages: [Order Pay](https://www.cashfree.com/docs/api-reference/payments/latest/payments/pay), [UPI Collect deprecation](https://www.cashfree.com/docs/payments/manage/payment-methods/upi-collect), [UPI payment setup](https://www.cashfree.com/docs/payments/manage/payment-methods/upi).

| Topic | What the docs say | Effect here |
|---|---|---|
| UPI Collect (typed UPI ID) | Blocked on **Android and desktop since 28 February 2026**, in sandbox and production. iOS still allows it "until further notice". Mandates (OTM, AutoPay), IPO payments and eRupi are exempt | The typed-UPI-ID checkout cannot work on Android or desktop with Cashfree. Intent (mobile) and QR (desktop) are required, which is what [ADR 0003](../adr/0003-extend-rust-and-upi-qr-intent.md) moved to |
| Native intent / QR | `POST /pg/orders/sessions` with `payment_method.upi.channel` of `link` (intent) or `qrcode` (QR) | This is the call a native `upi_intent` capability would use |
| S2S flag | Order Pay is a server-to-server call and needs the **S2S flag enabled on the account** by Cashfree support | Not confirmed as enabled. Without it, use Cashfree's hosted checkout, which already shows intent and QR |
| Client headers | `x-client-device`, `x-client-os`, `x-client-browser`, and `x-client-rendering-type` for mobile, on Order Pay. If missing, Cashfree assumes full migration to intent / QR | The adapter must pass them, from the payer's real device. The checkout would have to send the payer's device details to the API |
| API version | The Order Pay reference shows `x-api-version: 2026-01-01`; the adapter uses `2023-08-01`; an older example of the UPI call used `2023-08-01` | Pick one deliberately and test it. Do not change it blind |
| Intent response shape | The excerpt does not document the `link` / `qrcode` response payload keys | **Unconfirmed.** Read it from a real sandbox response; do not guess |
| Expiry | `transaction_expiry_time` (ISO 8601): past it the transaction is failed and cannot be retried. Collect expiry is 5 to 15 minutes | Surface a real expiry in checkout only if the partner enforces it |
| WebView | Intent in a WebView needs code that intercepts `upi://`, `gpay://`, `phonepe://` URLs; otherwise only a QR shows | Matters for any native app wrapper |
| Rate limits | Reported in `x-ratelimit-*` headers; errors carry `code` and `type` (400, 401, 404, 409, 422 `idempotency_error`, 429, 500, 502) | Map to the adapter's `Rejected` / `Uncertain` split |

## What is needed from the merchant
1. **Sandbox API keys.** In the dashboard, switch to the Test environment and generate keys. Keep them in environment variables, never in the repository.
2. **The S2S flag**, if native intent / QR is wanted (a Cashfree support request).
3. **A public HTTPS URL for sandbox webhooks** (a tunnel is enough for testing).
4. **Permission for small-value live tests** on real devices (see below).

## End-to-end test order
1. **Sandbox, hosted checkout.** Run the existing adapter against the sandbox: create an order, pay it with Cashfree's sandbox test methods, receive the signed webhook, see the payment, ledger entry and merchant event. This is the first real-partner evidence (level 2).
2. **Sandbox, native intent / QR.** Needs the S2S flag. Capture the real response, then implement `upi_intent` for the adapter from it, with the client headers.
3. **Physical devices.** The sandbox does not move money and does not complete payments inside a real UPI app, so it cannot prove BHIM, PhonePe or Google Pay interoperability. That needs **small-value live payments**, each followed by a refund, run by a person on real Android and iOS devices with each common UPI app. These must be separately authorised; they are never automated and never charge real customers by default.
4. **Partner UAT and live approval** are separate states (levels 3 and 4), agreed with Cashfree.

## Gaps that block a real-partner claim
- No sandbox run yet; the adapter has never seen a real Cashfree response.
- The adapter does not send client headers, and the checkout does not pass the payer's device details.
- The intent / QR response is undocumented in what was read.
- The S2S flag status is unknown.
