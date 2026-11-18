# Vendor onboarding, commission rules, payout batches and schedules

Everything here is tested against simulators only: a scripted payout rail that counts what it is asked, the simulated
KYC provider, synthetic bank statements, and a test clock. Nothing has paid a real vendor or touched a real bank.
Money is whole paise; the ledger is the existing append-only double-entry ledger; no rate or limit is built in.

## What already existed, and what this adds

Already there: vendors with encrypted bank details, single and JSON-bulk payouts with a second person's approval, split
payments that accrue to a vendor's payable balance and reverse in proportion on refunds, balance-funded payouts that
cannot overspend, `mark_paid` and `send_rail` (acquire mode), the unique bank reference, onboarding's bank check and
name comparison, uploads (encrypted files), the bank statement import and its matching of payouts to bank debits, the
test clock, the worker.

Added (migration 0082): the vendor onboarding workflow and per-vendor payout settings; commission rules; payout
batches (CSV, list, or vendors with a balance) with batch approval, sending, retries and a report; payout schedules;
payout limits. Every batch item is an ordinary row in `payouts`, so every older rule still applies to it.

## Vendors: invited, details, bank account, verified, active

| Step | Call | Who |
|---|---|---|
| Invite (a record; nothing is sent) | `POST /v1/vendors/invite` | `payouts:write` |
| Details the vendor gave | `PUT /v1/vendors/{id}/profile` | `payouts:write` |
| Bank account or UPI id | `POST /v1/vendors/{id}/bank_account` | `payouts:write` |
| Verify the account | `POST /v1/vendors/{id}/verify` | `payouts:approve`, **not** the person who added it |
| Activate | `POST /v1/vendors/{id}/activate` | `payouts:approve` |
| Suspend / reactivate | `POST /v1/vendors/{id}/suspend`, `/reactivate` | `payouts:approve` |
| Hold days, minimum payout, default commission rule | `PUT /v1/vendors/{id}/payout_settings` | `payouts:approve` |
| Optional documents | `POST/GET /v1/vendors/{id}/documents`, `GET .../{doc}/file` | write / read / approve |

- The bank account is encrypted like any vendor bank detail; only the last four digits are shown. When a KYC provider is
  configured (`ZYVOR_KYC_PROVIDER`), the same penny-drop check onboarding uses runs and its outcome is kept; the simulator
  verifies nothing real. The account holder's name is compared with the vendor's legal name with the loose comparison
  onboarding uses; a name that does not match needs a written `override_reason` from the verifier. A failed bank check
  cannot be verified.
- Only an `active` vendor can be given a share of a payment or be paid. Vendors made the older way
  (`POST /v1/vendors` with bank details) are `active` already. Changing the bank account of an active vendor puts it
  back to `bank_added` until a person verifies it again. A suspended vendor's already-proposed payouts are refused at
  approval and at sending.
- Documents are PDF, PNG or JPEG, up to 5 MB, kept encrypted, at most 12 per vendor; Aadhaar documents are refused (same
  rule as onboarding uploads). Each download is audited. Nothing reads or approves on the strength of a document.
- Not built: sending the vendor an email or a link to fill in the details themselves (the invitation is a record), and
  reading vendor documents for facts.

## Commission rules

A rule is a percentage in basis points and/or a flat amount in paise, with an optional floor and cap on the commission,
for a vendor, a payment link and/or an event reference, effective from a date to a date. **No rate is built in**: with no
rule in force, a share given as `gross_minor` is refused (`no_commission_rule`) rather than guessed.

- Use: on a payment (or a payment link) give a vendor's share as `{ "vendor_id": ..., "gross_minor": 10000 }` instead of a
  fixed `amount_minor`. The commission rule in force today turns it into the fixed share `gross - commission` before the
  split is stored; the split row keeps the gross, the commission and the rule. From then on the share accrues on capture
  and reverses in proportion on refunds exactly like any other split. Commission is `floor(gross * bp / 10000) + flat`,
  raised to the floor and cut to the cap; the share must stay positive. Fixed `amount_minor` splits work as before.
- Which rule: of the active rules in force today, in the payment's currency, whose scope fits, the most specific (a
  payment link beats an event reference beats a vendor; combinations add up). Two active rules of one scope may not
  overlap in time (`rule_overlaps`). With none, the vendor's default rule is used if in force. A rule with no scope is a
  shared rule that applies only where a vendor names it as its default. The event reference is `metadata.event_ref` on
  the payment.
- `POST /v1/commission_rules/preview` shows exactly what each party would get on a sample amount (the commission, the
  vendor's share, what the merchant keeps), and, with `refund_minor`, what is reversed and kept after a refund. A rule
  that is not saved can be tried inline. It stores nothing.
- Writing rules needs `payouts:approve`; retiring a rule leaves the commission on recorded shares alone. A payment link
  with `gross_minor` shares is checked when it is opened, not when it is created.
- The commission is not posted to its own ledger account: it is what remains of `collections` after the vendor's share,
  as with any split. It is recorded on each split row and shown by the preview.

## Payout batches

Build one from a CSV (`csv` in the request body: columns `vendor` (id or your reference), `amount` in rupees or
`amount_minor`, `purpose`, `reference`, optional `currency`), from a list (`items`), or from vendors' balances
(`POST /v1/payout_batches/from_balances`: each vendor's balance past hold days, at least the minimum payout; vendors with
nothing payable are listed as skipped).

- Strict and all or nothing, in the style of the bank statement import: every line must have as many cells as the header
  (so `12,500` cannot silently become 12), an amount must be plainly an amount (no commas, sign, symbol or exponent), a
  cell starting `=`, `+`, `-` or `@` is refused, an exactly repeated line is refused. Every problem is returned with its
  line number and nothing is stored while any remain. `POST /v1/payout_batches/preview` runs the same checks and the same
  rules and stores nothing.
- Same content, same fingerprint, whatever the order and the file name: a batch whose payouts are already in an open or
  approved batch is refused (`duplicate_batch`, a preview says `duplicate_of`). A deliberate second run needs a different
  `batch_ref`. Rejecting or canceling a batch frees its content.
- **Every item waits for approval**, whatever the tenant's threshold for single payouts says. A different person approves
  the whole batch (`POST .../approve`), and may leave items out (`exclude: [item ids]`, which cancels their payouts, or
  `POST .../items/{item}/exclude` beforehand). The proposer cannot approve or reject their own batch.
- Rules checked when a batch is proposed, again when it is approved (if any item fails, nothing is approved and the
  failing items are listed) and again when an item is sent: vendor active, the vendor's minimum payout, the tenant's
  limits (`GET/PUT /v1/payout_policy`, admin to change: a single payout, a batch total, items per batch, one vendor per
  24 hours), the payee not on the blocked list, and for payouts from a balance, the money is there and past its hold
  days. The same vendor and limit rules now apply to single payouts.
- Sending (`POST .../send`, acquire mode, through the payout rail): each item is claimed before the rail is called, so
  two sends at once never pay an item twice, and the payout's id is the rail's idempotency key. A payout is marked paid
  and posted to the ledger (Dr payouts, or the vendor's payable for a balance payout; Cr bank) in one transaction, and
  the ledger reference `payout:{id}` makes a second posting fail. Safe to call again.
- Per-item status: `pending_approval`, `approved`, `sending`, `failed`, `unknown`, `paid`, `rejected`, `canceled`,
  `excluded`, read from the payout itself (so a payout paid by hand shows as paid). `failed` means the rail said it did
  not pay; `POST .../retry` sends it again with the same payout id, up to five attempts in all. `unknown` means the rail
  could not say (a timeout, an `accepted` that is not yet settled): it is **never sent again**, only asked about
  (`POST .../resolve`, and the worker after `resolve_after`), the same rule payments follow. An answer that it was paid is
  recorded once; an answer that it failed makes it retryable.
- Orchestrate mode: sending is refused; the batch report lists the transfers to make from your bank, and each is recorded
  with `POST /v1/payouts/{id}/mark_paid`. A payout inside a batch cannot be approved, rejected or sent on its own, and
  cannot be recorded paid by hand while a rail call is in flight.
- The report (`GET .../report.csv`) has one line per item with status, attempts, bank reference, the last error, and the
  matched bank debit; cells that would run as a spreadsheet formula are neutralised.
- Bank matching: the bank statement import (see [BANK_RECONCILIATION.md](BANK_RECONCILIATION.md)) already matches paid
  payouts to debits by their bank reference. A batch shows, for each paid item, the confirmed or suggested statement
  line, and `bank_reconciliation` counts matched, suggested and unmatched paid items. A bulk transfer the bank shows as one
  consolidated debit is not matched to the batch as a whole.
- Events: `payout_batch.approval_required`, `payout_batch.approved`, `payout_batch.rejected`, `payout_batch.completed`
  (once, when no item is open any more), plus the existing `payout.approved` and `payout.paid` for each payout.

## Schedules

`POST /v1/payout_schedules` names a vendor and a rule: weekly (`day_of_week` 1 = Monday to 7), monthly (`day_of_month` 1 to
28, or 0 for the last day), or threshold (when the vendor's payable balance, past its hold days, reaches
`threshold_minor`; looked at once a day), optionally capped by `max_amount_minor`. The worker (or
`POST /v1/payout_schedules/run`) turns schedules due today into **one proposed batch for the run, waiting for approval**,
created by `system:schedule`, from the vendor's balance. A schedule is skipped, with the reason recorded
(`nothing_payable`, `held_back`, `below_threshold`, `previous_proposal_still_open`, a vendor or limit rule), without
stopping the others.

- Dates are the organisation's calendar: the test clock in a test environment. A run is identified by its schedule and
  its date, so running twice in a day, or two workers at once, proposes nothing twice; a schedule whose day was missed
  runs once when the worker returns, not once per missed day.
- **Nothing is paid by a schedule on its own** unless the tenant has set `auto_approve_scheduled_under_minor` in its
  payout policy (admin only, unset by default) and the whole run, and each payout in it, is within that amount. Only then
  is the batch approved as it is made (recorded as `system:auto-approve`, audited), and, in acquire mode, the worker sends
  it. Anything over the limit waits for a person.
- Not built: schedules for own-funds payouts (a schedule pays from a vendor's balance), per-vendor rail choice (a batch
  uses one rail: `imps`, `neft` or `upi`).

## Tested, and not

Tested (simulators): the vendor workflow with two people and the name check; the simulated bank check failing; commission
rules (cap, floor, flat, event and link scope, dates, overlap, retirement, refund reversal, ledger balance); CSV parsing
line by line; preview and duplicate detection; approval with exclusions and the recheck; limits; sending, retry,
unknown and resolve against a scripted rail that counts calls (each payout reaches the rail once, and is in the ledger
once); concurrent creates, approvals and sends; balances with hold days on the test clock; schedules on the test clock,
catch-up and the auto-approve switch through the worker; bank matching shown on a batch; tenant isolation (row-level
security on every new table, and the other tenant gets 404); the orchestrate-mode path.

Not tested or not built: a real payout rail or bank (the HTTP rail contract is in `zyvor-settle` but was not exercised
against a real sponsor); real KYC providers; vendor self-service; console pages (API, SDK types and docs only).
