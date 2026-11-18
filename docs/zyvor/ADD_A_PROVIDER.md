# Add a provider adapter

A new gateway (Paytm PG, Easebuzz, ...) is a day's work if you follow this page, because everything above the adapter
(routing, the state machine, the ledger, reconciliation, the API) only sees the `Provider` trait. If the gateway can
already speak [PROVIDER_CONTRACT.md](PROVIDER_CONTRACT.md) you do not need an adapter at all: use the `custom` provider. Write a
native adapter when the gateway has its own API, as PayU does ([PAYU.md](PAYU.md) is a worked example).

## Steps

1. **Read the provider's public documentation and write down what is ambiguous.** Code the conservative reading and keep
   a "Needs live-sandbox verification" table in `docs/zyvor/<PROVIDER>.md` (see PAYU.md). Never put a real or live credential in
   code, tests or docs; use obviously fake values such as `payu_test_key`.
2. **Write the adapter** `crates/zyvor-providers/src/<name>.rs` implementing `Provider`, and add `pub mod <name>;` in `lib.rs`.
   The rules the conformance suite enforces:
   * Money is whole paise everywhere. Convert at the wire boundary without floats (`zyvor_core::minor_to_decimal`, `decimal_to_minor`).
   * A failure that left no trace is `Rejected` (4xx, refused connection, validation). Anything that may have happened (timeout, 5xx,
     unreadable answer) is `Uncertain` and is never retried elsewhere. Use `classify` and `read_json`; do not write your own.
   * Start `initiate` with `check_request(&caps, req)?`: it refuses an undeclared method or currency and a non-positive amount.
   * Our reference must find the payment again: send our attempt id as the provider's reference, or derive one deterministically
     (PayU does) so a lost response is resolved by `fetch_payment`. A reference the provider never saw is `Failed` with reason
     `not_found`, unless "not seen yet" is normal for that provider; then say so in `Expect` (PayU: `Pending`).
   * Declare only what you can do in `capabilities()`. A flag that is off must really be off.
   * Verify a webhook (signature, then age if the provider timestamps) before reading anything in it. The `event_id` must be stable
     across a redelivery and different for different events of one payment, and built from signed fields only.
   * Nothing secret in what is handed to a payer's device or stored as `raw`.
3. **Wire it in.** `crates/zyvor-api/src/registry.rs` (build it from the stored credentials), `admin.rs` `check_provider`
   (required credential names, the error text), `timeline.rs` `provider_name`, a migration that adds the name to the
   `provider_accounts_provider_check` constraint (take the next free number; see `0080_payu_provider.sql`), the `provider` enum
   and the credentials description in `openapi/zyvor.yaml`, `bank_reference` in `lib.rs` if the provider reports a bank reference,
   and `packages/zyvor-node` and the console form if people should connect it from there.
4. **Simulate it.** Add endpoints to `crates/zyvor-provider-mock` (as for PayU: its own routes plus `/_sim/<name>/*` controls that return the
   signed webhook for the test to deliver), or, for a small shape, a fake in `tests/conformance/gateway.rs`.
5. **Run the conformance suite against it.** In `crates/zyvor-providers/tests/conformance/backends.rs` add a `Backend` (the
   simulated provider's side: the payer paying, a refund finishing, a settlement) and a `Subject` constructor, then add
   `suite!(<name>);` in `main.rs`. Declare every deliberate difference in `Expect` with a comment saying why. Run
   `cargo test -p zyvor-providers --test conformance`.
6. **Add one API test** in the style of `crates/zyvor-api/tests/payu.rs`: connect the account, attempt, webhook delivery (and a tampered
   one), de-duplication, refund. It proves the migration, the registry and the webhook route.
7. **Docs.** A row in `STATUS.md` ("simulator only, built from public docs, not verified against a live sandbox"), the provider page, and a `CHANGELOG.md` entry.
   Label simulator versus real sandbox honestly; no comparisons with other providers.

## Conformance checklist

Each check runs once per adapter (`cargo test -p zyvor-providers --test conformance -- <adapter>::`).

| Check | Rule |
|---|---|
| `capabilities_are_honest` | Refuses an undeclared method, currency or non-positive amount; undeclared mandates are `Unsupported`; the UPI flow returned matches `server_initiated_upi` and `upi_intent`; without `manual_capture`, `capture` moves no money |
| `captured_payment_round_trip` | Create, follow, look up by our reference; order and payment references present; webhook kind, amount and order match the lookup |
| `declined_payment_is_failed` | A declined payment reads as `Failed` by lookup and by webhook |
| `definite_failure_vs_uncertain_outcome` | Refused connection and 4xx are `Rejected`; 5xx, no answer and an unreadable 200 are `Uncertain`, on `initiate` and on lookup |
| `lost_response_is_resolved_by_lookup` | After a response lost once the provider acted, `Uncertain`, then a lookup finds the payment (never "not found") |
| `initiate_twice_is_safe` | A repeat is answered with the same payment or `Rejected`, and money paid on either is found by the attempt id |
| `amounts_round_trip_in_paise` | 100, 101, 12 345, 123 456 789 and 999 999 999 999 paise survive the wire format; every declared currency is accepted |
| `refunds_full_and_partial` | Partial, partial, full; over-refund is `Rejected`; a repeated refund id refunds once; lookup by our refund id reaches `Processed` with the same reference |
| `lost_refund_response_is_resolved_by_lookup` | 5xx is `Uncertain`, 4xx `Rejected`; a lost refund is found once by its id and exactly one refund exists |
| `unknown_payment_reference` | A reference the provider never saw is reported as the adapter declared |
| `settlement_report_parsing` | Items add up to the net; payments positive, fees, refunds and adjustments negative; gross, fees and net exact in paise; items point at our references; UTR present |
| `webhook_verification` | Valid, tampered, wrong secret, missing signature, replay (same event id; `Stale` where timestamped), out of order (distinct event ids) |
| `refund_webhooks` | A refund callback is verified and names our refund, or is refused on purpose where the provider documents no signature |
| `secrets_never_leave` | No credential appears in the client action or the stored answer |

## Deliberate exceptions on record

Declared in `Expect` in `backends.rs`; each is the adapter doing something different on purpose.

| Adapter | Exception | Why |
|---|---|---|
| razorpay | No replay window; initiating twice makes a second order; a repeated refund id is not de-duplicated | The webhook has no timestamp (the event id stops a replay); `receipt` is not unique at Razorpay (the lookup reads every order of the attempt); the refund call has no idempotency key we rely on (a lost response is resolved by lookup, never resent) |
| cashfree | none | |
| payu | No replay window; `initiate` makes no call; unknown payment is `Pending`; unknown refund is `Uncertain`; refund callback refused | See [PAYU.md](PAYU.md) |
| custom, sponsor_upi | none | |
| phonepe | Settlements `Unsupported` | Settlement sync is not wired; import settlements by statement |
| sim | No webhooks, no settlements, unknown references `Rejected`, refund bounds not enforced, a repeated refund id is not de-duplicated | In-process simulator; the API checks refund bounds (`refunds.rs`) before calling any provider |

## What the suite found when it was first run (fixed)
* `read_json` treated a 200 with an unreadable body as an empty answer. Razorpay's lookup therefore reported a payment (and a refund) that may exist as `Failed`/`not_found`, which unblocks a retry. It is now `Uncertain`.
* Razorpay's lookup read only the first order of a receipt; a payment made on a second order was invisible. It now reads every order.
* Cashfree, PhonePe and the simulator accepted a USD payment although they declare INR only, and sent it on. Every adapter now refuses an undeclared method or currency, and an amount that is not positive, before sending (`check_request`).
* PhonePe: a refused connection was `Uncertain` (the contract says definitely not done); a refund lookup returned no reference where the create call did; a webhook without a timestamp skipped the replay window.
* `custom` and PhonePe: a lookup for a refund the gateway never saw was `Rejected` rather than a definite not-found, unlike payments.
* `sponsor_upi` declares UPI intent, but the `custom` adapter under it never sent the request flag, so a gateway could not know to answer with one.
