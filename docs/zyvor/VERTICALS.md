# Vertical templates: school fees, society dues, donations

Three small templates built on what already exists: invoices and instalment plans, payment links and the checkout,
payment intents, refunds and mandates. Each is a data model, an API, a CSV import or export and this document. There are
no console pages yet.

## Honest status

- **Level 1: built and tested against the local simulators.** Nothing has run against a live provider; no real money,
  student, member or donor was involved (the names in the tests are invented).
- **Zyvor does not give tax, accounting or legal advice.** The donation receipt can print words the merchant typed (a
  registration line, an exemption line, a note). Zyvor prints them as typed, does not check or interpret them and makes
  no statement about tax benefits, Section 80G or any other provision. Whether and how a receipt may be used for a tax
  purpose is for the organisation and its adviser.
- Not built: console pages, SMS or WhatsApp reminders (email only), automatic scheduled reminders (a person or a job
  you run calls the reminders endpoint), a gap-free receipt number series, late fees and concessions, student
  promotion between classes, a parent portal, GST on fees, debit scheduling for recurring donors (see below).

## School fees and society dues (one engine)

`kind` in the path is `school` or `society`. The same tables and code serve both; only the words differ.

| Concept | School | Society |
|---|---|---|
| group | class | block or wing |
| member | student | resident |
| member reference | roll or admission number | flat number |
| fee head | tuition, transport, exam fee | maintenance, sinking fund |
| payer | guardian | resident or owner |

### Data model

- **Group** (`dues_groups`): a named class or block. Unique per template.
- **Fee head** (`dues_heads`): a name and an amount in paise that **everyone in the group owes**. Different amounts for
  different students mean different groups (or assign to chosen members with `head_ids`).
- **Member** (`dues_members`): name, group and the business's own reference (unique per template), plus the **payer**,
  who is an ordinary `customers` row (name, email, mobile). Siblings may share a payer by giving the same
  `payer_customer_id`.
- **Assignment** (`dues_assignments`): the heads of a group turned into invoices for one member and one label ("2026-27
  Term 1"). One assignment per member per label, so repeating the call is safe. With `instalments: 1` it is one invoice;
  with 2 to 60 it is an ordinary instalment plan (`installment_plans`, exact split, the remainder on the last
  instalment) with due dates by month or day. The fee heads are copied onto the assignment, so changing a head later does
  not rewrite old bills.
- The invoices are ordinary invoices (`GET /v1/invoices/{id}`), numbered `{number_prefix}-{reference}-{n}`. They are paid
  through payment links and the checkout like any invoice; the existing invoice email (`POST /v1/invoices/{id}/send`),
  dunning, credit notes, refunds, ledger and reconciliation apply unchanged.

### API

All under `/v1/dues/{kind}`. Reads need `verticals:read`, changes need `verticals:write`.

| Call | What it does |
|---|---|
| `POST /groups`, `GET /groups` | Classes or blocks, with member counts |
| `POST /heads`, `GET /heads?group_id=` | Fee heads for a group |
| `POST /members`, `GET /members?group_id=` | Add or update a member by reference (the payer by name, email and mobile, or `payer_customer_id`) |
| `POST /members/import` | Import many members from a CSV (below) |
| `GET /members.csv` | Export members as a CSV that can be edited and imported again |
| `POST /assign` | Turn fee heads into invoices for a `group_id` or for `member_ids`: `label`, `instalments`, `first_due_date`, `every_unit` (`month` or `day`), `every`, `number_prefix`, optional `head_ids`. Answers with counts: created, already assigned, skipped (no payer), skipped (no fee heads). All or nothing: a number clash (`number_taken`) saves nothing. |
| `GET /report`, `GET /report.csv` | Per member and instalment: billed, paid, due, overdue (due date before today, unpaid). Filters `group_id`, `label`, `status=due`, `limit` |
| `POST /reminders` | Email the payer of every unpaid instalment that is due within `days_before` days (default 3, at most 30) or overdue. Once per invoice per day; includes a payment link when `ZYVOR_CHECKOUT_PUBLIC_URL` is set; skips payers without an email and addresses that unsubscribed. Answers with counts only. |
| `GET /invoices/{invoice_id}/receipt.pdf` | A PDF receipt (built with `pdf.rs`) for what has been received on that instalment: receipt number `RC-{invoice number}`, date of the latest payment, payer, student or member, group, label and instalment, amount due, received to date and balance, then the business's own receipt note. Refused (`nothing_paid`) when nothing has been received. Reflects what is received at the time of download. |
| `GET\|PUT /settings` | The receipt note: up to 300 characters printed as typed on every receipt of that template |

### CSV import

Header row first; the delimiter (comma, semicolon, tab or pipe) is detected. Columns are matched by name, ignoring case,
spaces and punctuation:

| Meaning | Accepted names |
|---|---|
| group (required) | `Class`, `Group`, `Block`, `Wing`, `Building` |
| name (required) | `Name`, `Student Name`, `Student`, `Member`, `Resident` |
| reference (required) | `Ref`, `Reference`, `Roll No`, `Admission No`, `Student Ref`, `Flat`, `Flat No`, `Unit` |
| payer name | `Payer`, `Guardian`, `Guardian Name`, `Parent`, `Parent Name` |
| payer email | `Email`, `Payer Email`, `Guardian Email`, `Parent Email` |
| payer mobile | `Phone`, `Mobile`, `Payer Phone`, `Guardian Phone`, `Parent Phone` |

At most 5000 rows and 1 MB. **All or nothing:** one bad row saves nothing, and every problem is listed with its line.
`dry_run: true` checks a file and saves nothing. Rows with an existing reference update that member. Classes and blocks
named in the file are created. The export (`members.csv`) uses the same columns; every text cell that a spreadsheet
could run as a formula (starting with `=`, `+`, `-` or `@`) is written with a leading apostrophe.

### Children's data and the DPDP Act

A student is kept as a **name, a class and the school's own reference**: no date of birth, address, photograph, health
or caste data, and no government ID. A reference of exactly 12 digits (the shape of an Aadhaar number) is refused, and
the docs say plainly: use a roll or admission number. The guardian is stored as a customer (name, email, mobile), which is
the data needed to bill and remind them.

Under India's Digital Personal Data Protection Act, 2023 a child's data is handled with extra care (verifiable consent
of a parent or guardian, no tracking or behavioural monitoring, no targeted advertising). Zyvor does not do any of the
last three with this data. Collecting consent, deciding the purpose and the retention period, and deleting records
that are no longer needed are the school's duties as the data fiduciary; Zyvor has no retention job yet. Names and
contacts are never written to the audit trail or logs (audit entries hold ids and counts), and the reminder and receipt
code puts them only in the email or PDF addressed to the payer. This is a description of what the code does, not legal
advice.

### Tenant isolation

All tables have row-level security; a business cannot read, assign or receipt another's members or invoices (tests use
the same role and a second business, including the unscoped database role).

## Donations

### Model

- **Campaign** (`donation_campaigns`): name, description, up to 6 `suggested_minor` amounts, `min_minor` and
  `max_minor` (defaults ₹1 and ₹10,00,000), an optional `goal_minor`, a `recurring_enabled` switch, a status (`active` or
  `closed`) and **receipt fields**: `registration_text`, `exemption_text` and `receipt_note`, each up to 300 characters.
  Only the merchant sets them (`POST` or `PATCH /v1/donations/campaigns`); a donor cannot influence them.
- **Donation** (`donations`): campaign, the donor's own payment intent, an optional name, email and mobile (or
  `anonymous`, which stores none), the amount and status (`pending`, `paid`). A paid donation gets a receipt number
  `DN{date}-{id suffix}`, **not a gap-free series**.

### API

| Call | What it does |
|---|---|
| `POST\|GET /v1/donations/campaigns`, `PATCH /v1/donations/campaigns/{id}` | Manage campaigns (`verticals:write` / `verticals:read`) |
| `GET /v1/donations/campaigns/{id}/summary` | Paid count and amount, refunded, net, pending, goal, recurring pledges |
| `GET /v1/donations` | Donations (filter `campaign_id`, `status`) |
| `GET /v1/donations.csv` | Paid donations for the books: receipt number, date, campaign, amount, refunded, donor name, email, mobile. Not a tax statement. |
| `GET /v1/donations/{id}/receipt.pdf` | PDF receipt for a paid donation **that has a donor name** (`no_receipt` otherwise): receipt number, date, donor, campaign, amount, then the merchant's registration, exemption and note lines as typed, and a closing line saying the receipt records a payment, that the wording was written by the organisation and that Zyvor does not check it and gives no tax advice |
| `GET /v1/donate/{tenant_id}/{campaign_id}` (public) | What a donation page needs: name, description, suggested amounts, limits |
| `POST /v1/donate/{tenant_id}/{campaign_id}/start` (public) | Record the donation and open the donor's own payment intent; answers with `intent_id`, `client_secret` and `path` (`/pay/{tenant}/{intent}`, the ordinary checkout). Rate limited per visitor address and per campaign like the seller page; the amount must be inside the campaign's range. |

A refund goes through `POST /v1/payment_intents/{intent_id}/refunds` as for any payment and shows in the summary and the
CSV. There is no ready-made donation page in the checkout app: a website or app calls the two public endpoints and sends
the donor to the checkout path.

### Recurring donors

`POST /v1/donations/campaigns/{id}/pledges` (needs `verticals:write` and `mandates:write`, and an `Idempotency-Key`)
makes the donor a customer and creates a **UPI AutoPay mandate** through the ordinary mandate flow, with the donor's
amount as the most any one debit may take. The answer carries the mandate with the authorisation payload the donor
approves in their UPI app; the mandate rules (ceiling, pre-debit notice, revoke) apply as they do for any mandate. The
campaign must have `recurring_enabled`, and a provider that supports mandates must be connected (the simulator does).

**Not built:** scheduling the debits is the existing `POST /v1/mandates/{id}/debits`; the pledge does not schedule
them for you, and the debits are not listed in the campaign's donation list, summary or CSV. Mandates are Level 1
(simulator) as described in [MANDATES_LIVE.md](MANDATES_LIVE.md).

### Donor data

A donor may give nothing (anonymous), or a name, an email and a mobile number. These are kept on the donation, never
written to the audit trail or logs, and shown only to the business's own keys. Retention and deletion are the
organisation's duty; there is no retention job yet. This is a description of what the code does, not legal advice.

## Scopes

| Scope | Allows |
|---|---|
| `verticals:read` | Read groups, heads, members, reports, receipts, campaigns, donations and exports |
| `verticals:write` | Create and change them; import; assign; send reminders; add recurring donors |
| `mandates:write` | Needed as well for recurring donors |

The finance console role gets both `verticals` scopes.

## Test data

`POST /v1/test_data/reset` (test mode only) clears orders, donations, members, assignments and their invoices; it keeps
groups, fee heads, campaigns, the seller page and its items (setup).
