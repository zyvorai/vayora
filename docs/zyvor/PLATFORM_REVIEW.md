# Zyvor review before live mode

Live mode does not open because a merchant says so, or because a second person inside the merchant's own
organisation approves them. It opens when a **Zyvor reviewer** approves the business. This page is for the people
who do that, and for whoever runs the deployment.

## How it works

1. A merchant signs up, gets their business details approved in **Onboarding** (automatically if every check
   passes, or by a second person in their own organisation), and presses **Request live access**.
2. The request waits as `pending`. The gauge `zyvor_live_access_pending` on `/metrics` counts them; alert on it.
3. A reviewer looks at what the business submitted and the checks that ran, and **approves**, or **rejects with a
   note**. The note is shown to the merchant and emailed to whoever asked.
4. On approval the merchant's console can switch to Live. The approved profile is copied into their live tenant with
   the reviewer's name (`reviewed_by = zyvor:<name>`), so live payouts and the Onboarding page agree. A rejected
   merchant fixes what the note says and asks again (the request returns to `pending`, round 2, 3, ...).
5. Every step is in the merchant's audit trail (`live_access.requested`, `.approved`, `.rejected`), and approvals and
   rejections also raise the `live_access.approved` / `live_access.rejected` webhook events.

A merchant's API key, console session key or second admin can never act as a reviewer: reviewers use their own keys
(`zp_...`), which only exist in the `platform_reviewers` table.

## Setting up reviewers

Reviewers are named people with their own key, so every decision says who made it.

```
# on the machine (or pod) that has DATABASE_URL for the restricted role
zyvor-api platform-reviewer add "Asha Rao"      # prints the key once; only its hash is stored
zyvor-api platform-reviewer list
zyvor-api platform-reviewer disable rev_...     # their key stops working at once
```

k3s: `kubectl -n zyvor exec deploy/zyvor-api -- zyvor-api platform-reviewer add "Asha Rao"`.
Hand the key to that person over a channel you trust; do not put it in chat or a ticket. Make one per person and
disable it when they leave. No key, no live mode: with no reviewer set up, every request stays pending.

## Taking live mode away (revoke) and giving it back

For a business that is already live and should not be: a chargeback spike, a failed re-check, a complaint, a request from
a provider. In the reviewer page open the business and use **Take live mode away** (a reason is required, and you tick
that you understand it stops things at once), or `scripts/platform-review.sh revoke <organisation_id> "why"`.

What stops at once, for the **live** side only (test mode is untouched):

- every write through a live key or console session except refunds: no new payments, payment links, checkout
  sessions, payouts, captures, mandates or settings changes (the API answers 403 with the reason);
- new payments from the public checkout and payment links (they get no provider, `live_access_revoked`);
- UPI AutoPay notices and debits, and automatic payment reminders, in the worker.

What keeps working: reading everything, refunds (so customers can still be repaid), refreshing and cancelling
payments already in flight, in-flight debits settling, webhooks and the audit trail.

The merchant sees a red bar on every live page and the reason on the Onboarding page, and is emailed. Their Test /
Live switch is refused with `live_access_revoked`, and "Request live access" does not reopen it: only a reviewer can.
**Give live mode back** (or `restore`) when what you asked for is fixed; it needs the business details to still be
approved. Every step is audited (`live_access.revoked`, `.restored`) and raises the same webhook events.

**Not covered:** money already with the provider is not clawed back or held here (do that at Razorpay or Cashfree);
revoking does not sign anyone out; and a business is frozen as a whole, not per provider or per branch.

## Doing a review in the console

Set `CONSOLE_REVIEWER=1` on the console that Zyvor's own people use (leave it off for any console merchants reach: it
then answers 404 for `/reviewer`). Open `/reviewer`, paste your reviewer key (`zp_...`; it is checked against the API,
kept encrypted in an httpOnly cookie for 8 hours, and never shown again), and you get:

- the queue, oldest first, filtered by pending / approved / rejected, with how many checks did not pass;
- a review page with the business, every check and its reason, bank accounts (last four digits only) and document
  references, and a warning when no automatic check ran at all;
- **Approve** (optional note) and **Reject** (a note is required: the merchant is shown it and emailed).

A merchant sign-in does not work on this page, and a reviewer key does not work anywhere else in the console. Sign out
ends it at once. Disabling the reviewer (`platform-reviewer disable`) makes the next click return to sign-in.

## Doing a review from a script

```
export ZYVOR_API_URL=https://api.example.com ZYVOR_PLATFORM_KEY=zp_...
scripts/platform-review.sh list pending
scripts/platform-review.sh show org_...
scripts/platform-review.sh approve org_... "documents checked"
scripts/platform-review.sh reject org_... "GSTIN certificate is missing"
```

`show` prints the legal name, business type, masked PAN, GSTIN, address, the document **references**, the bank
accounts (holder, IFSC, last four digits, verified or not), and every identity and bank check with its outcome.

**What the review is worth depends on what you check.** Zyvor keeps references to documents, not the files, and the
checks are only as real as the KYC provider behind them (with the simulator they verify nothing). Before approving:

- the legal name, the PAN holder and the bank account holder are the same party (a proprietor's PAN, a company's
  account, or the reverse, is a reason to ask);
- the documents behind the references exist and match (open them where they are kept);
- the GSTIN, if given, belongs to that PAN and that name;
- the business is one you are willing to put on live rails (look at the website, the line of business, and anything
  on your own do-not-serve list);
- every check says `pass`, or you understand why one does not. `show` prints the reasons a submission went to a
  person.

When unsure, reject with a note that says exactly what would change your mind.

## Not built yet

- A second reviewer for large or unusual merchants; assignment and an SLA clock.
- Re-review when the business changes after approval (profiles are locked once approved, but a new bank account is
  only verified by a person, not re-reviewed by Zyvor).
