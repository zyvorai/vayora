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

## Doing a review

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

- A reviewer **page** in the console; reviewers use the script or the API (`/v1/platform/reviews`).
- A second reviewer for large or unusual merchants; assignment and an SLA clock.
- Re-review when the business changes after approval (profiles are locked once approved, but a new bank account is
  only verified by a person, not re-reviewed by Zyvor).
- Revoking live access for a merchant already live.
