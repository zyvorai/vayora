# Accounting exports

CSV exports of the ledger-derived journal for a date range, a Tally-style layout, and a GST summary. Read-only:
nothing here posts to the ledger.

## Honest status

- **Synthetic and simulator data only.** Tested on journals produced by the simulators and hand-built records. Never
  imported into a real accounting package.
- **Tally: partly verified.** The XML follows the sample voucher import on Tally's public help pages
  (<https://help.tallysolutions.com/sample-xml/>): envelope header `VERSION 1`, `TALLYREQUEST Import`, `TYPE Data`,
  `ID Vouchers`; `DATE` as `YYYYMMDD`; `VOUCHERTYPENAME`, `VOUCHERNUMBER`, `NARRATION`; `LEDGERENTRIES.LIST` with
  `LEDGERNAME`, `ISDEEMEDPOSITIVE` and `AMOUNT`, a debit being `Yes` with a **negative** amount and a credit `No` with
  a positive one; `VCHTYPE` and `ACTION` attributes on `VOUCHER`. **Not verified:** the published samples show Sales
  vouchers, not Journal vouchers, so the Journal form is inferred; and **it has never been run against Tally**. Ledgers
  must already exist in the Tally company under the exported names (use `GET /v1/exports/account_mappings` for the
  list). The `tally_csv` format is a flat layout in Tally's vocabulary for spreadsheet-based or third-party
  converters; Tally itself imports XML.
- The account codes are placeholders for you to map onto your own chart of accounts; they are not statutory.

## Journal

`GET /v1/exports/journal?from=YYYY-MM-DD&to=YYYY-MM-DD` (scope `exports:read`; IST days, inclusive, at most 366 days).

| Parameter | |
|---|---|
| `format` | `csv` (generic, default), `tally_csv`, `tally_xml`, `json` |
| `currency` | `INR` (default) or `USD` |
| `kinds` | comma list of `payments, refunds, fees, settlements, payouts, other` (default all) |
| `limit`, `after` | page size 1-5000 (default 1000) and the cursor from the previous page's `x-next-cursor` header (`next_cursor` in JSON; empty on the last page) |

Where vouchers come from:

- **Ledger postings** as they are: captures (`payments`), refunds, payouts, acquire-mode settlement batches
  (`settlements`) and others (splits, lost disputes) as `other`.
- **Provider settlement records** (imported settlements, which the ledger does not post today): a `settlement` voucher
  (bank debited with the net the provider paid, provider receivable credited) and a `fees` voucher (processing fees
  debited, provider receivable credited). Whatever the provider's net does not explain from its own lines goes to
  the `settlement_adjustments` account instead of being hidden, so the voucher still balances. These two kinds are
  built from the record, labelled `settlement_record` / `fee_record` in the JSON.
- Together, for an orchestrate-mode provider whose settlement adds up, the `provider_receivable` account nets to zero.

Guarantees, each tested: every voucher balances (debits equal credits) or the export is refused; output is
deterministic (ordered by IST day then voucher number, bytewise; the same request returns the same bytes); paging
follows the cursor with no gaps or repeats and the CSV header only on the first page; tenant-scoped by row-level
security; every page writes an `export.journal` audit event with who, range, format, counts, totals and the SHA-256 of
the bytes returned (also in the `x-export-sha256` header). Response headers also carry `x-export-vouchers`,
`x-export-lines`, `x-export-debit-total-minor`, `x-export-credit-total-minor` and `x-export-unmapped-accounts`.
CSV text that starts with `=`, `+`, `-` or `@` is prefixed with an apostrophe so a spreadsheet never runs it.

Generic CSV columns: `voucher_no, date, kind, line, account_code, account_name, ledger_account, debit, credit,
currency, narration` (amounts are decimals with two places, from whole paise).

## Account codes

Built-in codes (`bank` 1000, `provider_receivable:*` 1200, `escrow:*` 1210/1220, `settlement_clearing` 1230,
`merchant_payable:*` 2100, `vendor_payable:*` 2200, `collections` 4000, `fee_income` 4100, `processing_fees` 5100,
`payouts` 5200, `settlement_adjustments` 5900) apply until you map your own: `POST /v1/exports/account_mappings`
(`exports:write`) with a ledger account or a family ending in `:*`; an exact mapping beats a longer family beats a
shorter one beats the built-in. A ledger account that nothing covers exports under code `9999` with its ledger name
and is listed in `x-export-unmapped-accounts`; it is never dropped. Mappings are per tenant and audited.

## GST summary

`GET /v1/exports/gst_summary?from&to&format=csv|json` (`exports:read`): invoices (not void) and credit notes in the
range, summed by supply type (`b2b` when the buyer has a GSTIN, else `b2c`) and rate, with documents, taxable value,
CGST, SGST, IGST, and a closing `net` row (sales less credit notes). Same population as the GSTR-1 preparation view; a
preparation aid, not a filing. Audited like the journal.
