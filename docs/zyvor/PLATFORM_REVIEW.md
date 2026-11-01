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
zyvor-api platform-reviewer lead rev_...        # a lead may set other reviewers' weekly hours; `unlead` takes it back
```

k3s: `kubectl -n zyvor exec deploy/zyvor-api -- zyvor-api platform-reviewer add "Asha Rao"`.
Hand the key to that person over a channel you trust; do not put it in chat or a ticket. Make one per person and
disable it when they leave. No key, no live mode: with no reviewer set up, every request stays pending.

## Two reviewers, and how long a request may wait

**Second reviewer.** For the cases that warrant it, one reviewer's approval is only the *first*: the review stays
`pending` (stage `awaiting_second`) until a **different** reviewer approves too. The same person approving twice is
refused (`second_reviewer_required`). Either reviewer can **reject on their own**, and a merchant who asks again after a
rejection starts the whole review over (new clock, no earlier approval carried over). `ZYVOR_REVIEW_TWO_PERSON`:

- `flagged` (default): two reviewers when a check did not pass, **no automatic check ran at all**, the business has asked
  three or more times, or a reviewer escalated it;
- `always`: every approval needs two;
- `never`: only an escalation does.

A reviewer can **escalate** any pending review (`scripts/platform-review.sh escalate <org> "why"`, or "Not sure? Ask for
a second reviewer" on the review page) when no rule fired but something looks wrong; the note is shown to whoever gives
the second approval. Both approvals are in the merchant's audit trail (`live_access.first_approval`, then
`live_access.approved`, which names the first approver). **You need at least two active reviewers** for `flagged` or
`always` to ever open anything; with one, set `never` or add another (`zyvor-api platform-reviewer add`). Revoking, and
approving an unflagged change, still take one reviewer.

**The clock.** Every request and every held change has a *target*: `ZYVOR_REVIEW_SLA_HOURS` (default 24) for live access,
`ZYVOR_CHANGE_SLA_HOURS` (default 4) for a bank account that holds a live business's payouts. It is a target, not a
promise to the merchant, and the API says so: the merchant sees "we aim to answer by ..." (`expected_by`), reviewers see
"due in 5h" / "overdue by 3h" in the queue, and the lists and reviews carry `due_at`, `overdue` and `sla_hours`.

- gauges: `zyvor_live_access_overdue`, `zyvor_live_changes_overdue`, `zyvor_review_oldest_pending_seconds`; alerts
  `ZyvorLiveAccessOverdue` and `ZyvorLiveChangeOverdue` (deploy/zyvor/alerts.yml) fire when any is past its target;
- with `ZYVOR_REVIEW_ALERT_EMAIL` set (on the **worker**), the worker emails that address once for each item that passes
  its target. With no address nothing is sent and nothing is marked, so setting one later still reports what is already late.

## Who has a review (assignment)

A reviewer can take a pending review (`scripts/platform-review.sh claim <org>`, or "Take this review" on its page) so two
people do not read the same file. While it is held, **only the holder can approve or reject it** (anyone else gets
`assigned_to_other`); the queue can be filtered to the ones you hold (`list pending me`) or that nobody holds
(`list pending none`; the page has Mine / Nobody's). Any reviewer can hand a review to a named, active reviewer
(`assign <org> <reviewer id>`) or take one from its holder on purpose (`take-over`); that is written to the audit log
(`live_access.assigned`, with who it was taken from). A review goes back to the queue when its holder is disabled, when
the first of two approvals is given (so a different reviewer can give the second; the first approver cannot be handed
it), and when it is decided or the business asks again. Escalating a review is not blocked by who holds it.

Assignment is a courtesy that stops duplicated work, not an access control: any reviewer can take over. It covers
live-access requests and for changes alike (`change-claim|change-release|change-take-over <id>`, `change-assign <id>
<reviewer id>`, "Take this change" in the queue; `changes pending me|none` filters). Holding a change only stops others
deciding it: its live payout hold is unaffected. It returns to the queue on the same events (holder disabled, first of
two approvals, decided).

## The rota (who is on duty)

Each reviewer sets weekly hours in the review desk's local time: the **Rota** page in the reviewer console, or
`scripts/platform-review.sh rota-set "mon-fri 09:00-18:00" "sat 10:00-14:00"` (`rota` shows everyone's hours and who is
on duty now; `rota-set none` clears yours). The queue shows "On duty now: ..." or, in red, "Nobody is on duty now".

- The desk's time is an **IANA time zone**, `ZYVOR_REVIEW_TIMEZONE` (for example `Europe/London` or `Asia/Kolkata`;
  set it on the API **and the worker**, which uses it for automatic assignment). A zone follows daylight saving, and
  shifts are wall-clock: 09:00-18:00 stays 09:00-18:00 when the clocks change. In the hour that does not exist (spring
  forward) nobody's shift covers it, and in the hour that happens twice (fall back) a shift covers it both times. A
  name the server does not know is ignored with a warning, and the fixed offset applies. Without a zone the desk uses
  a **fixed UTC offset**, `ZYVOR_REVIEW_UTC_OFFSET_MINUTES` (default 330, India), which never changes with the seasons.
  Leave and holiday days are days on the same clock. Shifts you saved keep their wall-clock hours when you move the
  desk to a zone.
- A shift that runs past midnight is two shifts (to 24:00, then from 00:00 the next day); the console form takes one
  shift a day, the script and `PUT /v1/platform/rota/me` take any number. Each reviewer sets only their own hours;
  disabling a reviewer removes theirs.
- It **informs and never blocks**: an off-duty reviewer can still decide, and assignment, escalation, the clock and the
  second-reviewer rule ignore the rota. The review target (`ZYVOR_REVIEW_SLA_HOURS`) still counts wall-clock hours, not
  duty hours.
- **Leave and holidays** sit on top of the weekly hours. Book your own leave (Rota page, or `scripts/platform-review.sh
  leave 2026-12-24 2026-12-31 "why"`), or a **desk holiday** for everyone (tick the box, or `holiday 2026-12-25`);
  `leave-remove <id>` / `holiday-remove <id>` take one back. Days are whole local days in the desk's time and include both
  ends (a year at most, nothing already over). On leave, or on a holiday, a reviewer is not on duty however their shifts
  read, so the gauge, the queue strip and automatic assignment all skip them. Anyone may add or lift a desk holiday; leave
  is its owner's to remove. Like the hours it informs and never blocks, and what a reviewer already holds stays with them.
  A holiday with requests waiting trips `ZyvorReviewQueueUnattended` like any other time nobody is on, which is the point.
- Gauge `zyvor_review_on_duty`: how many active reviewers are on duty now, or **-1 when nobody has set hours** (the rota
  is not in use, so an empty desk means nothing). Alert `ZyvorReviewQueueUnattended` fires when it is 0 for 15 minutes
  while requests or changes are waiting.

**Automatic assignment** (`ZYVOR_REVIEW_AUTO_ASSIGN=1` on the **worker**, off by default). Each worker cycle gives every
waiting request and change that nobody holds to an **on-duty** reviewer: the one holding the fewest pending items, the
longest-serving on a tie, and never the reviewer who already gave the first approval (so the second look goes to someone
else, and waits unassigned if only the first approver is on duty). It does nothing when nobody has set hours or nobody is
on duty; those items stay in the "Nobody's" queue. It only fills empty slots: it never moves something a person holds,
a disabled reviewer's items are re-given on the next cycle, and the audit log records it as `system:rota`
(`live_access.assigned` / `live_change.assigned` with `auto: true`). **Mind the trade-off:** a held item can be decided
only by its holder, so an auto-assigned item waits for that person; if they are away, someone takes it over
(`take-over`, or "Take it from ..."). A reviewer who goes off duty keeps what they hold.

**Assignment emails.** A reviewer who has given an address (the Rota page, `scripts/platform-review.sh email you@example.com`,
`PUT /v1/platform/me`, or `zyvor-api platform-reviewer add "Name" you@example.com`) is emailed when a request or change is
**given to them**: by another reviewer (including a take-over that lands on them) or by automatic assignment. Claiming
one yourself sends nothing, and a reviewer with no address, or whose address was cleared, gets nothing. The mail names
who gave it, the business and (for a change) only the kind (a new bank account, changed legal details), with a link to the
desk; no account or identity details. It goes through the same mail queue as every other email, so it leaves only where
the API's SMTP address is set (otherwise it is logged), and a mail that cannot be queued is logged, never an error for the
assignment. The holder is told too when someone **takes an item from them** (a take-over, handing it to a third reviewer, or putting it
back in the queue): "Asha took ... from you; Chitra has it now" (or "it is back in the queue"), so they stop waiting on it.
Giving away or releasing your own item says nothing, and a holder with no address is not mailed. Nothing is sent when a
held item goes overdue to anyone but `ZYVOR_REVIEW_ALERT_EMAIL`. A holder who is disabled, or whose item was released by
their first approval or a decision, is not told: those are not someone taking it.

**A lead can set other people's hours.** The operator makes a lead from the CLI (`platform-reviewer lead <id>`); a lead cannot
promote anyone, and a reviewer cannot make themselves one. A lead replaces another reviewer's weekly shifts with
`PUT /platform/rota/reviewers/{id}` (`rota-set rev_... "mon-fri 09:00-18:00"`); anyone else gets 403. The reviewer is emailed
that their hours changed (who changed them and how many shifts, not the hours), when they have an address, and can change
them back. A lead sets weekly hours only: leave stays each reviewer's own, and a desk holiday stays open to everyone. The
change is logged by the API but not in the audit trail, which is per business and the rota belongs to no business.

A lead can also **book or remove leave for another reviewer** (`leave-for rev_... 2026-12-24 ...`; `POST /platform/rota/days_off`
with `reviewer`), **see coverage** (`coverage [days]`, `GET /platform/rota/coverage`: the hours in the coming days when nobody
is working, with shifts, leave and holidays counted; wall-clock, so a daylight-saving night's odd hour is not modelled),
and **re-balance** (`rebalance`, `POST /platform/rota/rebalance`): everything pending that a reviewer on leave today holds
moves to the on-duty reviewer holding the fewest, never the first approver. Items held by someone who is merely off
shift stay with them, nothing moves while nobody is on duty (409), and an item with nobody it can go to is counted as
`left`. Each move is audited as the lead's, and the new holder and the old one are told.

**Not built:** re-balancing by workload alone (it only moves work off people on leave), and moving work when someone is
only off shift. (A revoke is one person on purpose; giving
access back needs two.)

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
approved. **A different reviewer than the one who took it away has to give it back** (`second_reviewer_required`)
unless `ZYVOR_REVIEW_TWO_PERSON=never`. Taking it away stays one person and immediate, because it protects; giving it
back reopens the risk. Every step is audited (`live_access.revoked`, `.restored`) and raises the same webhook events.

**Not covered:** money already with the provider is not clawed back or held here (do that at Razorpay or Cashfree);
revoking does not sign anyone out; and a business is frozen as a whole, not per provider or per branch.

## Re-review: changes after approval

Once Zyvor has approved a business, its details are locked. Two things can still change, and both are re-reviewed by
Zyvor, not only by a second person in the merchant's own organisation: **where its money can go** (a new bank account)
and **who it is** (its legal details, below).

- adding a bank account to an approved business records a **change** (`live_changes`: who, when, bank, IFSC, last four
  digits; no account number) and **holds that business's live payouts** (`change_awaiting_review`, 409) until a reviewer
  decides. Test mode is not held. Accounts added before approval are part of the first review and flag nothing.
- the reviewer page shows **Changes waiting** above the queue; or `scripts/platform-review.sh changes`,
  `change-approve <id>`, `change-reject <id> "why"`. **Approve** releases the hold. **Reject** needs a reason, removes
  the account, emails the merchant the reason and releases the hold. The record of what it was stays.
- the merchant sees "Zyvor is reviewing a change you made" on Onboarding and by `GET /v1/live_access` (`changes`),
  and is emailed the decision. Audit actions and webhook events: `live_change.approved` / `live_change.rejected`.
- **Legal details.** An admin of an approved business can ask to change its legal name, business type, PAN, GSTIN or
  address (`POST /v1/onboarding/profile_change`, the "Need to change these details?" section on Onboarding), with a
  reason. The new values get the same PAN and GSTIN checks as any others, then wait as a change of kind `profile`: the
  reviewer sees what changes (old to new), the reason and the checks' outcome. **Until a reviewer decides nothing
  changes and live payouts are held** (the same hold as for a bank account). **Approve** writes the new details into
  both the test and live copies; **reject** needs a reason and leaves the old ones. One request can wait at a time. Only
  the last four digits of the PAN are kept, so the reviewer sees the PAN ending, not the PAN. A change of legal name
  does not by itself check that the bank account holder still matches; look at the bank accounts on the review page.
- **Two reviewers for a change.** What was unclear when the change was made is kept (`flagged_because`: no bank check
  ran, a bank/PAN/GSTIN check did not pass). Under `ZYVOR_REVIEW_TWO_PERSON=flagged` (default) a flagged change needs two
  different reviewers: the first approval is recorded and the change stays pending, so **payouts stay held**; a
  different reviewer completes it (`second_reviewer_required` otherwise). `always` asks for two on every change, `never`
  on none. Rejecting is always one reviewer. A reviewer can also **escalate** any pending change that nothing flagged
  (`scripts/platform-review.sh change-escalate <id> "why"`, or "Not sure?" on the change in the queue); that counts under
  every policy, `never` included, and the second reviewer reads the note.
- the gauge `zyvor_live_changes_pending` counts waiting changes; the alert `ZyvorLiveChangeWaiting` fires after 4 hours.

**What this does not do:** it does not re-run KYC. Zyvor keeps only the last four digits of a PAN, so it cannot re-check
a PAN or GSTIN later (a GSTIN that is cancelled next year goes unnoticed), and legal name, PAN, GSTIN and address
cannot be edited at all once approved (the merchant has to ask Zyvor). A held payout is not queued: the merchant
retries after approval. With no reviewer set up a held business stays held, as with live access itself.

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

- Re-balancing held items (the rota with leave, desk holidays and a daylight-saving time zone, assignment, automatic assignment, the clock and the second reviewer exist; see above).
- Re-checking identity later (GSTIN status, name change). A business can ask to change its legal details (see above),
  but nothing re-verifies it on a schedule, and a legal-details change takes one reviewer.
