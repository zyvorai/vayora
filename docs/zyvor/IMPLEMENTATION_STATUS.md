# Implementation status against the 31 October 2026 specification

The specification asks for four explicit completion levels. They are different states and none implies the next:

1. **Implemented with simulator**: works end to end against the in-process simulator or the local provider mock.
2. **Integrated with a partner sandbox**: runs against a real partner's sandbox.
3. **Tested in partner UAT**: passed the partner's acceptance tests, including physical-device checks.
4. **Approved and enabled live.**

**One capability has reached level 2** (Cashfree UPI intent and QR, below). Everything else is level 1. Nothing is at level 3 or 4, and no live gateway has been used. No NPCI approval, bank agreement or partner credentials are claimed. See [STATUS.md](STATUS.md) for the product-level view and [ADR 0003](../adr/0003-extend-rust-and-upi-qr-intent.md) for why this extends the Rust codebase instead of rewriting it in Go.

## Repository state and stack
Rust workspace (axum, sqlx, PostgreSQL with row-level security), Next.js checkout and console, Compose, Helm, k3s and Envoy assets, OpenAPI, Node/Python/Go SDKs. The spec's proposed Go/Vite stack is **not** used; the reason is recorded in ADR 0003.

## Delivered in this slice: UPI QR / intent checkout (level 1)
| Behaviour | Evidence |
|---|---|
| UPI no longer requires a typed UPI ID when a provider can issue a QR / intent; the checkout reports `upi_flow` (`intent` or `collect`) | `upi_checkout_issues_a_qr_or_intent_instead_of_asking_for_a_upi_id` |
| A UPI payment with no UPI ID is routed only to a provider with the `upi_intent` capability, and refused (`no_route`) otherwise rather than turned into a collect | same test, router unit tests |
| The provider payload is carried and shown exactly as issued (`upi://pay?...`), as a QR (`GET /v1/checkout/{tenant}/{intent}/upi-qr.svg`, client-secret authorised) and an "open my UPI app" link | same test; checked in a browser |
| A QR, an app launch or returning to the page never marks anything paid; the page moves to paid only after a verified provider result | same test; checked in a browser |
| A finished payment has no QR to show | same test |
| Return URLs must be https (http in a sandbox) with no embedded credentials | same test |
| Checkout text in six languages (translations not reviewed by native speakers) | `lib/i18n.test.ts` |

**Limits.** Razorpay and Cashfree do not issue a QR / intent payload yet (`upi_intent: false`); they need a flow verified with each partner. Desktop, Android and iOS behaviour is not tested on physical devices, and the iOS flow in particular must follow the partner's documented behaviour. BHIM, PhonePe and Google Pay interoperability is not verified: that requires partner UAT on real devices. Wording says "any UPI app" and does not use BHIM branding until the current NPCI brand guidance is checked.

## Gaps against the specification
This list comes from a scan of the repository, not a line-by-line audit; items marked "to audit" are not yet confirmed either way.

| Area | State |
|---|---|
| Per-merchant return-URL allowlist | Not built (basic validation only) |
| OIDC sign-in with PKCE | Not built. The console uses its own TOTP sign-in with roles |
| UPI AutoPay / mandates, vendor payouts through a partner | Not built (payouts are bookkeeping only) |
| Object storage with malware scanning for documents | Not built |
| Authenticated SSE status stream | Not built (the checkout polls) |
| Amounts as decimal strings in the API | Not done: integer minor units are used |
| "Orders" naming | The repository calls them payment intents |
| Real partner adapter, physical-device UAT | Needs partner credentials, current partner documentation and devices |
| The 20 mandatory acceptance tests (spec section 17) | Existing tests cover idempotency, unknown outcomes, refund reservation, tenant isolation, ledger balance and webhook SSRF; mapping each of the 20 to a test is **to audit** |
| Restore drill, load and fault test evidence | Not run |

## Checkout support reference (level 1)
Every checkout state shows a short **support reference** derived from the payment id (it grants no access), and, if the operator sets `NEXT_PUBLIC_ZYVOR_SUPPORT_URL` at build time (an `https` URL or a `mailto:` address, nothing else is accepted), a **Report a problem** link that carries the reference. Strings are in six languages (not natively reviewed). Tests: `lib/checkout.test.ts`.

## Payer apps to test on physical devices
UPI payers use many apps and the checkout must work with all of them, not one. Before any live claim, test the QR scan and the intent launch on real devices with each of the common UPI apps (BHIM, PhonePe, Google Pay, Paytm and other bank and fintech UPI apps), on Android and on iOS, including launch failure and backgrounding. None of this has been done.

## Cashfree UPI intent and QR (level 2: integrated with a partner sandbox)
Run on 31 October 2026 against Cashfree's real **sandbox**, with the whole local stack (API, worker, checkout) and a real sandbox account.

| What was shown | Evidence |
|---|---|
| A phone request gets a real intent response with per-app links (`bhim`, `gpay`, `paytm`, `phonepe`) | `tests/cashfree_sandbox.rs` (run with `--ignored` and sandbox keys); also through the API with an Android user agent |
| A desktop request gets Cashfree's own PNG QR, shown in the checkout | same test; checked in a browser |
| The payer's device is derived from their own request and sent to Cashfree (`x-client-*`) | `clientenv.rs` unit tests |
| Without the account opt-in the adapter keeps the hosted flow | `without_the_opt_in_the_hosted_flow_is_still_used` (this test caught a bug: the adapter had relied only on the router) |
| Paying in Cashfree's sandbox simulator does **not** mark anything paid by itself; the page stayed `awaiting_payment` | observed |
| The server asks Cashfree (`refresh`), sees the capture, and the page becomes `paid`, one balanced ledger pair is posted once (`capture:<attempt>`), and a `payment.captured` event is queued | observed; ledger and outbox read from the database |

**Limits, plainly.**
- **No money moved.** The sandbox simulator is not a UPI app. In the sandbox the intent links are Cashfree simulator HTTPS URLs, not `upi://`; the real production link format is **not known** and was not tested.
- **Real apps are untested.** BHIM, PhonePe, Google Pay and others, on Android and iOS, need small-value live payments on physical devices (see [PARTNER_SETUP.md](PARTNER_SETUP.md)).
- **Webhooks were not tested.** Sandbox webhooks need a public HTTPS URL; this run resolved the payment by asking Cashfree instead.
- **Production is opt-in per account.** Set `"upi_intent": true` in the account's credentials only after Cashfree has enabled native intent / QR (its S2S flag). The sandbox did not require the flag; production behaviour is unconfirmed.
- The adapter uses Order Pay API version `2026-01-01` for this call (verified in the sandbox) and `2023-08-01` for the rest.

## iOS test apps (level 1: implemented with the simulator)
`apps/ios` holds **Vayora Pay** (payer) and **Vayora Merchant** (counter). They exercise the platform end to end on a phone against the in-process simulator; they do not move real money and are not bank-linked UPI apps (that needs NPCI TPAP approval and a sponsor bank).

| What was shown | Evidence |
|---|---|
| A payment created in Merchant's Collect screen, paid from Pay (`POST /v1/sim/pay`), polled to captured by Merchant, on two physical iPhones | observed on a local API with a temporary Postgres; one balanced ledger pair (`capture:<attempt>`) and one `payment.captured` event per payment |
| Full refunds from Merchant are `processed`, the reversal is balanced, and a repeat refund is refused (`422 refund_exceeds_refundable`) | observed; ledger totals equal |
| `POST /v1/sim/pay` is off unless `ZYVOR_SIM_PAYER=1` (404), accepts only the simulator's payee (`422 not_a_sim_payment`), and a second pay is `409 nothing_to_pay` | `crates/zyvor-api/src/sim_pay.rs` unit tests; observed |
| Setup QRs carry a server address and a **test** key; live keys are refused | `SetupPayloadTests` |
| Money is integer paise end to end; the keypad builds amounts from text, never floats | `AmountEntryTests`, `MoneyTests` |

**Person-to-person features (Pay only, phone-local).** Payees, send, split a bill (`vayora://request` QR), small payments without a PIN (up to ₹200 a payment and ₹1,000 a day, off by default) and bill reminders are recorded on the phone. The simulator knows only merchant payments, so none of this reaches Zyvor's ledger. The screens say so.

**Not verified.** No simulator run (CoreSimulator was out of date on the build Macs), no XCUITest, no TestFlight, no CI job for iOS. Dynamic Type, VoiceOver, the iOS 26 glass cards, haptics, the no-PIN path and reminder notifications have not been checked on a device. Row-level security was not exercised in the phone runs (a superuser database role was used).

