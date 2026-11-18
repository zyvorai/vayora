# Fee checks

Compares what a provider actually deducted from a settlement with what **your own contract** says it should deduct,
and asks a person to look at any difference. It explains and asks; it never moves money.

## Honest status

- **Level 1 only.** Built and tested against local simulators and hand-written settlement imports. It has **not** been
  checked against any provider's real settlement statements, invoices or contract wording.
- **Zyvor ships no rates.** There is no built-in percentage, flat fee, cap, tax rate or network charge (UPI MDR
  included) anywhere in the code or the database. Every rate comes from a fee rule you enter. The rates in the tests
  are arbitrary round numbers invented for the tests.
- **Take the rates from your own contract** (and the provider's invoice or fee schedule you were given). If you are
  unsure how your provider rounds, taxes or caps a fee, set a tolerance, ask the provider, and do not treat a finding as
  proof of an error. A finding means "this differs from the rule you entered", nothing more.
- If you have entered no rule for a provider and method, the answer is **"no expected fee configured"** and nothing is
  flagged. Silence is not approval: it means there was nothing to compare against.
- INR settlements only.

## What you configure

**Fee rules** (`POST /v1/fee_rules`): one per provider, method and start date.

| Field | Meaning |
|---|---|
| `provider`, `method` | e.g. `razorpay` + `upi`. Method is one of `upi`, `card`, `netbanking`, `wallet`. |
| `bps` | Percentage in basis points (1.5% is 150), 0 to 10000. 0 is allowed: it declares a zero-fee method. |
| `flat_minor` | Flat paise per payment (default 0). |
| `cap_minor` | Most the fee (percentage plus flat, **before GST**) can be for one payment. Optional. |
| `gst_bps` | GST charged **on the fee**, in basis points. Omit if your contract has none. You supply the rate. |
| `refund_returns_fee` | The provider gives the fee back on refunded value (default false). |
| `effective_from` | The rule applies to payments made on or after this date (IST). |

The rule in force for a payment is the one with the latest `effective_from` on or before the day the payment was made,
so older payments keep being checked against older rules. The same provider, method and start date cannot be entered
twice; a new rate needs a later date. Rules can be listed (`GET /v1/fee_rules`) and removed
(`DELETE /v1/fee_rules/{id}`, audited). Removing a rule does not rewrite findings already recorded: each one keeps a copy
of the rule it was checked against.

**Settings** (`GET/POST /v1/fee_checks/settings`):
- `tolerance_minor`: paise a payment's fee may differ from the rule before it is flagged (for rounding). Default 0.
- `owner`: who new findings are assigned to (a name or email, as with other cases). Optional.

## How the expected fee is worked out

Per payment, in whole paise, integer arithmetic only:

1. percentage = payment x `bps` / 10000, rounded half up
2. plus `flat_minor`
3. limited to `cap_minor` if set
4. plus GST = fee x `gst_bps` / 10000, rounded half up, if set

Expected net fee for a payment in a settlement = the fee on the payment line, less, when `refund_returns_fee` is set and
the settlement carries a refund for that payment, the fee that comes back: the fee on the payment amount minus the fee on
what is left after the refund (a full refund returns everything, flat and GST included). A refund that settles on its own
on a later day, with no payment line in that settlement, is checked the same way against the recorded payment amount.
Without `refund_returns_fee` a refund does not change the expected fee, so a provider that gives fee back anyway shows as
an under-deduction.

**Variance = actual - expected.** Positive: the provider deducted more than your rule says (over-deducted). Negative: less
(under-deducted). Both are reported. A payment is flagged when the variance is larger than the tolerance.

Limits you should know about:
- The check is per payment line, using the amount in the settlement. Several refunds spread over several settlements are
  each judged on their own; earlier partial refunds are not netted off the recorded amount.
- Fee lines the provider did not tie to one of your payments (a lump-sum fee, say) are reported as
  `unattributed_fee_minor` and not checked. Payment lines that are not in your records have no known method, so are not
  checked either.
- A currency other than INR is not checked.
- This is separate from the older `fee_variance` case, which compares a settlement's total fees with the rough routing
  estimate in your settings (`fees`, used for routing and cost suggestions). Both can appear; fee checks use the fee rules
  above, with per-payment detail and a tolerance you set.

## What a finding is

- A row per payment in `fee_check_lines`: status (`ok`, `flagged`, `no_rule`), actual, expected, variance, the tolerance
  and the rule exactly as applied (with its breakdown).
- For a settlement with any flagged payment, **one reconciliation case** of kind `fee_check` (so it appears in the existing
  cases list, can be assigned and resolved with a note, and puts the settlement in `exception` like any open case). If an
  owner is configured, the case is created already assigned to them.
- Re-checking keeps an open case current. If the rules or tolerance are corrected so nothing is flagged any more, the open
  case is closed with the reason written on it and an audit entry. A case a person already resolved is left alone.
- Findings **never change money**: no ledger entry, payment, refund or settlement amount is touched. They only explain and
  ask a person to review.

When a check runs: automatically when a settlement is ingested (provider sync, import, simulated), and on demand. Adding or
changing a rule does not re-check old settlements by itself; call `POST /v1/fee_checks/run` (optionally with `since` and
`limit`) or re-check one with `POST /v1/settlements/{id}/fee_check`.

## "Settlement explained"

`GET /v1/settlements/{id}/explain` gains a `fee_check` object and, when something is flagged, a line in the narrative such
as: "Fee differs from your configured rate by ₹7.30 (the provider deducted ₹7.30 more than expected on 1 payment(s)). A
person should check it against the contract; nothing has been changed." With no rule it says "No expected fee configured
for the payments in this settlement" inside `fee_check` and adds nothing to the narrative.

## Endpoints

| Method and path | Scope | Purpose |
|---|---|---|
| `POST /v1/fee_rules`, `GET /v1/fee_rules`, `DELETE /v1/fee_rules/{id}` | `recon:write` / `recon:read` / `recon:write` | Your rates |
| `GET/POST /v1/fee_checks/settings` | `recon:read` / `recon:write` | Tolerance and owner |
| `GET /v1/settlements/{id}/fee_check` | `recon:read` | The check for one settlement, computed now, writes nothing |
| `POST /v1/settlements/{id}/fee_check` | `recon:write` | Re-check one settlement and record it |
| `POST /v1/fee_checks/run` | `recon:write` | Re-check many settlements (newest first, up to 500) |
| `GET /v1/fee_checks/findings?status=open\|reviewed\|all&provider=&method=` | `recon:read` | Flagged payments; "reviewed" = the case is resolved |
| `GET /v1/fee_checks/summary` | `recon:read` | Open findings, total variance, over and under, by provider and method |

Branch-limited keys are refused, as for the other reconciliation endpoints. All tables have row-level security by tenant;
tests cover isolation over the API and directly in the database. Test-data reset clears the findings (they point at
settlements) and keeps your rules and settings.

## Tested

`crates/zyvor-api/src/fee_check.rs` unit tests (rounding half up, flat, cap before GST, GST rounding, effective dates,
no rule, tolerance both ways, zero-fee rule, refunds that return and do not return the fee) and
`crates/zyvor-api/tests/fee_checks.rs` (rule validation and scopes, over and under deduction to one owned case, no money
moved, idempotent re-runs, no rule, zero-fee, cap and effective dates through the API, refunds returning fees, a corrected
rule clearing a stale finding, tenant isolation). All against simulators and hand-built settlements.

## Not done

- No console page for rules or findings yet (API only), and no SDK convenience methods (types are generated).
- No import of a provider's fee schedule or invoice: rules are typed in by a person.
- No auto-applied credit or dispute with the provider; a person raises that outside Zyvor.
- Not verified against any real provider statement.
