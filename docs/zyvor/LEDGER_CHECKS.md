# Ledger checks, ledger-versus-file report, and webhook verification helper

Everything here is read-only and simulator-only. Nothing posts to the ledger, stores a row, opens a case or calls a bank
or provider. Level 1: unit and property tests run; the HTTP endpoints were compile-checked and their database
integration test was written but needs PostgreSQL (see the changelog).

## `GET /v1/ledger/verify` (admin scope)
Re-reads the organisation's whole ledger in one `REPEATABLE READ READ ONLY` snapshot and applies the rules in
`zyvor_ledger::audit::check`:

| `kind` | severity | meaning |
|---|---|---|
| `unbalanced` | error | a transaction's debits differ from its credits in some currency (per currency, never in aggregate) |
| `trial_balance_mismatch` | error | all debits differ from all credits in a currency |
| `too_few_entries`, `no_entries` | error | fewer than two entries, or none |
| `non_positive_amount`, `invalid_side`, `unsupported_currency` | error | values the write path never produces |
| `orphan_entry` | error | entries whose transaction does not exist |
| `duplicate_reference` | error | one business reference used twice |
| `reversal_target_missing`, `reversal_of_self`, `reversal_not_mirror`, `duplicate_reversal` | error | a transaction that says it reverses another must mirror it exactly, once |
| `unexpected_balance_direction` | warning | `merchant_payable:*` / `vendor_payable:*` with a net debit, or `escrow:*` with a net credit |

Response: `ok` (false only for errors), counts, `violations_by_kind`, up to 500 `violations`, and `trial_balance` per
currency with amounts as strings. More than 1,000,000 entries is refused with `ledger_too_large` rather than loaded.
A warning is a business-level signal (for example a vendor paid more than accrued), not a broken ledger.

## `POST /v1/recon/ledger_check` (`recon:read`)
Body: `{ "from", "to", "lines": [{ "kind": "payment"|"refund", "provider_ref", "amount_minor", "currency"? }] }`.
`from`/`to` bound the ledger postings expected in the file (at most 31 days; at most 5,000 lines). Postings for any
reference named in the file are also considered even outside the window. Payments are matched to `capture:<attempt>`
postings through the attempt's `provider_payment_ref`, refunds to `refund:<refund>` through `provider_refund_ref`.
The posted amount is the sum of the transaction's debit entries.

Every line and posting falls into one category: `matched`, `amount_mismatch` (with `difference_minor`),
`currency_mismatch`, `missing_in_ledger`, `missing_in_file`, `duplicate_in_file`, `duplicate_in_ledger`,
`invalid_line`. Comparison is exact integer equality: no tolerance and no rounding. A duplicated reference is never
matched, so a double payment cannot hide behind one posting. Amounts that are not whole positive integers (1.5, "12",
-3) make that line `invalid_line`; they do not fail the request.

## Verifying webhooks (Node SDK)
`verifyWebhook` is unchanged. `verifyWebhookDetailed({ secret, header, rawBody, toleranceSeconds?, nowSeconds? })` returns
`{ ok: true, timestamp }` or `{ ok: false, reason }` with `malformed_header`, `signature_mismatch` or
`timestamp_outside_tolerance`. It tries every `v1` entry (secret rotation), ignores other schemes, accepts only digit
timestamps, refuses headers over 1 KiB or with more than 8 `v1` entries, and throws on a tolerance that is not positive.
`WebhookReplayGuard` remembers event ids (default 24 hours, 100,000 entries) so a retried or replayed delivery is
handled once; it is per process, so use a unique database constraint when you run several instances. Server signing is
`t=<unix>,v1=hex(HMAC-SHA256(secret, "<t>.<body>"))`; a vector is pinned in both the Rust and the Node tests.
