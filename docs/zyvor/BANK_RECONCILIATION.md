# Bank statement import and matching

What finance can do with a bank statement, how a bank line is matched to a settlement (or a payout, a refund, a
direct payment), and what has and has not been tested. Everything here runs against **simulated providers and
synthetic statements**. No real account number, real statement or live provider was used, and no real money moves.

## Status, honestly

| Part | State |
|---|---|
| CSV parsing with layouts, strict line-level errors, dry run, de-duplication | Built, tested on **synthetic** files written from the column headings these banks' downloads are commonly shown with. **Not tested on a real export from any bank.** Always preview a real file first. |
| Matching a credit to a settlement by UTR | Built (this was already there as an exact `utr =` test; it is now a token match on the reference column and the narration, with the evidence kept). |
| Amount and date matching | Built, but only ever a **suggestion**. It was an automatic match before; it no longer is. |
| Matching debits to payouts and refunds, credits to payments by RRN | Built, simulator and synthetic data only. |
| UTR / RRN stored on payments | Built for the simulator and for the field names Razorpay and Cashfree document (see below); the real field names have **not** been checked against a live account. |
| MT940 and CAMT.053 | Unchanged (the older strict parsers). They feed the same matching, but do not have layouts, dry run or fingerprints. |
| Console screens for any of this | Not built. API only. |

## The flow

1. `GET /v1/bank_statements/layouts` lists the supported layouts.
2. `POST /v1/bank_statements/preview` with `{ "csv": "...", "layout": "hdfc" }` reads the file and stores nothing. It
   returns `ok`, the row count, credit and debit totals, the period, how many rows are new and how many are already
   held, and every problem with its line number.
3. `POST /v1/bank_statements/import_csv` with the same body imports it. **All or nothing**: if any line has a problem
   the answer is `422 invalid_statement` listing every problem (line, column, message) and nothing is stored.
4. Matching runs in the same transaction. Credits that explain a settlement by reference are confirmed. Possible
   matches on amount and date are stored as suggestions. Each credit nothing explains gets an
   `unmatched_bank_credit` case.
5. `GET /v1/bank_statements/lines?status=unmatched|suggested|matched` lists lines;
   `POST /v1/bank_statements/matches/{id}/confirm` or `/reject` is how a person decides a suggestion.

Scopes: reading and previewing need `recon:read`; importing, re-matching, confirming and rejecting need
`recon:write`. All rows are per organisation (row-level security on `bank_statement_imports`,
`bank_statement_lines` and `bank_line_matches`).

## Layouts

Send `"layout": "<name>"` (default `auto`). Banks, headings and date patterns are as follows; a named layout requires
its columns to exist and says which are missing if they do not.

| Layout | Columns it expects | Dates |
|---|---|---|
| `auto` | finds the header row (the first row with a date column and a credit or amount column) and the columns from the usual names | any below |
| `hdfc` | Date, Narration, Chq./Ref.No., Value Dt, Withdrawal Amt., Deposit Amt., Closing Balance | `01/10/26`, `01/10/2026` |
| `icici` | Transaction Date, Value Date, Cheque Number, Transaction Remarks, Withdrawal Amount (INR ), Deposit Amount (INR ), Balance (INR ) | `01/10/2026`, `01-10-2026` |
| `sbi` | Txn Date, Value Date, Description, Ref No./Cheque No., Debit, Credit, Balance | `01 Oct 2026`, `01-Oct-2026`, `01/10/2026` |
| `axis` | Tran Date, CHQNO, PARTICULARS, DR, CR, BAL | `01-10-2026`, `01/10/2026` |
| `kotak` | Transaction Date, Value Date, Description, Chq / Ref No., Debit, Credit, Balance | `01-10-2026`, `01/10/2026`, `01 Oct 2026` |
| `generic_split` | Date, Narration, Reference, Debit, Credit | ISO or day first |
| `generic_signed` | Date, Narration, Reference, Amount (minus is a debit) | ISO or day first |
| `generic_drcr` | Date, Description, Amount, Dr/Cr | ISO or day first |

These headings are the ones these banks' downloads are commonly shown with, from memory of public examples; the files
in `crates/zyvor-api/tests/fixtures/bank/` were **written from them with made-up data**. A bank that changes its
headings, or a corporate/netbanking variant, will not match: use `auto`, or name your own columns:

```json
{ "csv": "...", "layout": "auto",
  "mapping": { "date": "Posted", "narration": "Memo", "credit": "Money in", "debit": "Money out", "reference": "#5" },
  "date_format": "%d.%m.%Y", "delimiter": ";" }
```

A mapping value is a heading (case, spaces and punctuation are ignored) or `#3` for the third column. `delimiter` is
`,` (detected by default), `;`, `|` or `tab`.

### What the reader accepts and refuses

* **Dates**: day first (`01/10/2026`, `01-10-26`, `01-Oct-2026`, `01 Oct 2026`, `01.10.2026`), and ISO
  (`2026-10-01`, with or without a time). Never month first. `31/31/2026` is an error, not a guess.
* **Amounts** become whole paise with no floating point. Indian grouping (`1,23,456.50`) and western grouping
  (`123,456.50`) are read; a currency sign (`₹`, `INR`, `Rs.`), a minus sign, brackets and a trailing `Dr`/`Cr` are
  read. Refused: more than two decimals, commas in unusual places (so `1,50` is not read as 150), anything that is not a number.
* **Debit/credit**: separate columns (both filled in one row is an error), or one amount with a Dr/Cr column or
  suffix. A Dr/Cr that contradicts the sign is an error.
* **References**: the apostrophe Excel adds is removed. A reference a spreadsheet has turned into `4.04E+11` has lost
  its digits and is refused.
* **Not transactions**: rows with no date (a footer such as "Closing balance") are skipped and listed as warnings.
  Rows with no amount are skipped and listed.
* **Running balance**: when every line has a balance, balances that do not follow from the amounts are a *warning*
  (a filtered export legitimately breaks), not an error.
* **Size**: at most 20,000 transactions per file; split by month. The JSON body limit of the API also applies.

## Not importing the same transaction twice

Every line gets a fingerprint: SHA-256 of the date, amount, narration (upper-cased, spaces collapsed), reference and
how many identical lines came before it *in that file*. Re-uploading a file, or uploading a file that overlaps an
earlier one, therefore adds only the lines that are new; two genuinely identical lines in one file (same day, same
amount, same narration) are two transactions. Each upload is still recorded in `GET /v1/bank_statements/imports`
(rows, added, duplicates, totals, period, who).

Limits: the fingerprint depends on the narration text, so a bank that words the same transaction differently in two
downloads would import it twice (the references would then match the same settlement, and the second line would
become an unmatched credit case). A transaction imported before this change (by the JSON, MT940 or CAMT routes,
without a fingerprint) is recognised by its reference, date and amount; one without a reference is not.

## How a line is matched

Two kinds of match, never confused. Both are rows in `bank_line_matches` with the rule and the evidence.

### Reference matches: `confirmed`

The reference we hold for the thing (a settlement's UTR, a payout's bank reference, a refund's provider reference, a
payment's RRN) must appear on the bank line **as a whole token**, upper-cased with punctuation ignored, in the
reference column (`reference_column`) or in the narration (`reference_in_narration`). A reference needs at least six
characters and at least one digit, so words never match. `UTR998877` and `RRN404223241811` (label glued on) match the number;
a reference written with a separator (`UTR_0001`, `UTR-0001`) still matches. A prefix or a substring of a longer
token does not.

| Bank line | Looked up against | Rule when it fits |
|---|---|---|
| credit | settlements by UTR | confirmed, even if the amount differs (the difference becomes a `bank_amount_mismatch` case, as before) |
| credit | captured payments by bank reference (RRN) | confirmed when the amount is equal |
| debit | paid payouts by `bank_ref` | confirmed when the amount is equal |
| debit | processed refunds by provider refund reference | confirmed when the amount is equal |

Where the reference fits but the amount does not (payments, payouts, refunds), or the same reference is on several
lines, nothing is confirmed: a suggestion with rule `reference_amount_differs` or `reference_ambiguous` is stored for
a person.

### Amount and date: only ever `suggested`

No reference matched, but the same amount arrived within 3 days of the settlement (or left within 3 days of a payout
or refund being recorded). Stored as `amount_date_window` with the evidence: amounts, days apart, how many
competing candidates there were. It is **never confirmed automatically**, whatever the evidence looks like. A person
confirms or rejects it; a rejected suggestion is not suggested again.

Behaviour change: before this, a settlement with no UTR match was linked automatically to a credit of the same amount
within 3 days. That is now a suggestion.

Not built: a person cannot un-confirm a match; one UTR covering several settlements (a provider paying two
settlement records in one transfer) matches only one of them; a settlement matched to several credits (part payment).

## What becomes a case (assignable, with ageing)

| Case kind | Opened when | Closes when |
|---|---|---|
| `unmatched_bank_credit` | a credit has no confirmed match (text says if suggestions are waiting) | the line is matched |
| `bank_amount_mismatch` | the reference matched but the credit differs from the settlement's net | resolved by a person |
| `settlement_no_bank_credit` | `POST /v1/recon/scan?older_than_days=N`: a settlement has no bank credit and the imported statements run N days past its date | the credit is matched |
| `payout_no_bank_debit` | the same scan, for a payout recorded as paid with no debit matched | the debit is matched |

The scan deliberately says nothing until a statement covers the period: a statement nobody has imported yet is not a
missing credit. Unmatched **debits** do not become cases (a statement is full of charges and transfers that are not
Zyvor's); they are listed with `GET /v1/bank_statements/lines?direction=debit&status=unmatched`, and a debit that
looks like a payout or refund gets a suggestion.

`GET /v1/recon/cases` now carries `age_days`; `GET /v1/recon/ageing` counts open cases per kind in the buckets
0-1, 2-3, 4-7, 8-14 and 15+ days, with how many are unassigned, the oldest age and the amount involved.
Assigning and resolving work as for every other case.

## UTR and RRN on payments and settlements

* A settlement's `utr` comes from the provider's settlement report (Razorpay and Cashfree adapters read
  `settlement_utr`; the simulator and the local mock generate one). This was already stored.
* A payment now has `bank_reference`, filled when the provider reports one for a captured or authorized payment and
  shown on the settlement explanation. The adapters read the fields the providers' public API references describe:
  Razorpay `acquirer_data.rrn` (else `acquirer_data.upi_transaction_id`) on the payment entity or webhook, and
  Cashfree `bank_reference` on the payment. **These names are taken from public documentation, not checked against a
  live account**, and nothing is inferred when they are absent. The simulator gives each payment a made-up twelve
  digit reference; the local mock adds the same fields to its Razorpay and Cashfree payments. PhonePe, the custom
  gateway and the sponsor adapter report none.

## The settlement explanation

`GET /v1/settlements/{id}/explain` keeps its existing fields and adds:

* `bank_match`: now also `bank_line_id`, `reference`, `rule`, `key_matched`, `evidence`, `confirmed_by` (`system` for a
  reference match, the person for a confirmed suggestion), `match_id`.
* `suggested_bank_matches`: amount-and-date suggestions waiting for a person, each with its evidence.
* `bank_difference` (when a bank line is matched): `bank_minor`, `provider_net_minor`, `expected_minor`,
  `difference_from_expected_minor`, `explained_by` (the existing fee/adjustment breakdown), `components` (payments,
  refunds netted off, fees and taxes, adjustments) and `unexplained_minor` (bank minus provider net: not explained by
  the provider's own figures, and a case).
* the narrative says how the line was matched and any gap.

## Tests

Unit tests in `bankcsv.rs` and `bankmatch.rs`; integration tests in `crates/zyvor-api/tests/bank_recon.rs` against
the simulated providers: all-or-nothing import with line errors, the dry run, re-upload and overlap, each layout's
fixture file, custom mapping, reference match with evidence and the explanation, the late settlement report, the short
credit, suggestion then confirm and reject, payouts, aged cases, scopes, and tenant isolation (including
row-level security on the new tables). All of them use made-up references (`SYNTH…`) and zero-filled account numbers.
