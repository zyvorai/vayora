# Implementation status against the 31 October 2026 specification

The specification asks for four explicit completion levels. They are different states and none implies the next:

1. **Implemented with simulator**: works end to end against the in-process simulator or the local provider mock.
2. **Integrated with a partner sandbox**: runs against a real partner's sandbox.
3. **Tested in partner UAT**: passed the partner's acceptance tests, including physical-device checks.
4. **Approved and enabled live.**

**Nothing in this repository is above level 1.** Providers are verified only against simulators and local stubs, never a live gateway. No NPCI approval, bank agreement or partner credentials are claimed. See [STATUS.md](STATUS.md) for the product-level view and [ADR 0003](../adr/0003-extend-rust-and-upi-qr-intent.md) for why this extends the Rust codebase instead of rewriting it in Go.

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

