# Partner setup and end-to-end test plan: Cashfree

The spec requires current partner documentation before any concrete endpoint is written. This page records what Cashfree's documentation says (read on 31 October 2026), what is still unconfirmed, and the order of the end-to-end test. Re-read the linked pages before relying on this: documentation can lag or conflict.

**Completion level today: native UPI intent and QR reached level 2 (partner sandbox) on 31 October 2026; everything else is level 1.** Nothing has run in production or on a real UPI app. See [IMPLEMENTATION_STATUS.md](IMPLEMENTATION_STATUS.md).

## Why Cashfree
The merchant already has a live Cashfree account. An adapter for it exists (`crates/zyvor-providers/src/cashfree.rs`), built from public docs and tested only against the local mock. The path is to run that adapter against Cashfree's sandbox, then add native UPI intent and QR.

## What the documentation says
Source pages: [Order Pay](https://www.cashfree.com/docs/api-reference/payments/latest/payments/pay), [UPI Collect deprecation](https://www.cashfree.com/docs/payments/manage/payment-methods/upi-collect), [UPI payment setup](https://www.cashfree.com/docs/payments/manage/payment-methods/upi).

| Topic | What the docs say | Effect here |
|---|---|---|
| UPI Collect (typed UPI ID) | Blocked on **Android and desktop since 28 February 2026**, in sandbox and production. iOS still allows it "until further notice". Mandates (OTM, AutoPay), IPO payments and eRupi are exempt | The typed-UPI-ID checkout cannot work on Android or desktop with Cashfree. Intent (mobile) and QR (desktop) are required, which is what [ADR 0003](../adr/0003-extend-rust-and-upi-qr-intent.md) moved to |
| Native intent / QR | `POST /pg/orders/sessions` with `payment_method.upi.channel` of `link` (intent) or `qrcode` (QR) | This is the call a native `upi_intent` capability would use |
| S2S flag | The docs say Order Pay needs the **S2S flag enabled on the account** by Cashfree support | The **sandbox did not require it**. Production behaviour is unconfirmed, so native intent / QR is opt-in per account (`"upi_intent": true` in the credentials). Without it, the hosted checkout still shows intent and QR |
| Client headers | `x-client-device`, `x-client-os`, `x-client-browser`, and `x-client-rendering-type` for mobile, on Order Pay. If missing, Cashfree assumes full migration to intent / QR | The adapter must pass them, from the payer's real device. The checkout would have to send the payer's device details to the API |
| API version | The Order Pay reference shows `x-api-version: 2026-01-01` | The order is created with `2023-08-01` and the UPI call uses `2026-01-01`; **this mix was verified in the sandbox** |
| Intent response shape | The reference excerpt did not show it | **Confirmed against the sandbox:** `channel: link` returns `data.payload` with `default`, `bhim`, `gpay`, `paytm`, `phonepe`, `web`; `channel: qrcode` returns `data.payload.qrcode` as a `data:image/png;base64,...` PNG. In the sandbox the links are Cashfree simulator **https** URLs, not `upi://`; the production link format is unknown |
| Expiry | `transaction_expiry_time` (ISO 8601): past it the transaction is failed and cannot be retried. Collect expiry is 5 to 15 minutes | Surface a real expiry in checkout only if the partner enforces it |
| WebView | Intent in a WebView needs code that intercepts `upi://`, `gpay://`, `phonepe://` URLs; otherwise only a QR shows | Matters for any native app wrapper |
| Rate limits | Reported in `x-ratelimit-*` headers; errors carry `code` and `type` (400, 401, 404, 409, 422 `idempotency_error`, 429, 500, 502) | Map to the adapter's `Rejected` / `Uncertain` split |

## What is needed from the merchant
1. **Sandbox API keys.** Done on 31 October 2026 (kept outside the repository, in environment variables). A production account needs its own keys and, for native intent / QR, the S2S flag confirmed.
2. **The S2S flag**, if native intent / QR is wanted (a Cashfree support request).
3. **A public HTTPS URL for sandbox webhooks** (a tunnel is enough for testing).
4. **Permission for small-value live tests** on real devices (see below).

## End-to-end test order
1. **Sandbox, native intent / QR: done** (level 2). The adapter issues an intent (phones) or a QR (desktop) and the payment resolves by asking Cashfree. Not yet done: the signed **webhook** path, which needs a public HTTPS URL for sandbox webhooks, and the hosted-checkout (card) flow.
2. **Sandbox, webhooks and hosted flow.** Point a sandbox webhook at a public test URL, pay in the simulator, and confirm the signed event arrives, verifies and posts once.
3. **Physical devices.** The sandbox does not move money and does not complete payments inside a real UPI app, so it cannot prove BHIM, PhonePe or Google Pay interoperability. That needs **small-value live payments**, each followed by a refund, run by a person on real Android and iOS devices with each common UPI app. These must be separately authorised; they are never automated and never charge real customers by default.
4. **Partner UAT and live approval** are separate states (levels 3 and 4), agreed with Cashfree.

## Gaps that block a real-partner claim
- The sandbox webhook path and the hosted card flow have not been run against Cashfree.
- The production intent link format and the S2S flag status are unknown.
- No real UPI app, no physical device, no money moved.
