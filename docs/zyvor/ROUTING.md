# Routing: success-rate signal, circuit breaker and failover

Status: **implemented and tested against the in-process simulator and the local provider mock only.** No real provider
has been used, so the defaults below are reasoned starting points, not tuned values. Nothing here changes a payment
that already exists; it only influences which provider a NEW attempt is sent to.

Code: `crates/zyvor-providers/src/router.rs` (the pure decision), `crates/zyvor-providers/src/decline.rs` (whose fault
a decline was), `crates/zyvor-api/src/health.rs` (signal, breaker, sweeper, failover list, API),
`crates/zyvor-api/src/worker.rs` (runs the sweeper), `crates/zyvor-api/src/payments.rs` (`start_attempt`, the one place
attempts are created; `apply_provider_state`, the one place they are classified). Routing automation (section 6 below):
`crates/zyvor-api/src/routing_auto.rs` (settings, kill switch, cost model, shadow mode, shadow report, scorecard),
`crates/zyvor-api/src/softretry.rs`, `crates/zyvor-providers/src/router.rs` (cost-aware choice).
Tests: router and decline unit tests, `health.rs` unit tests, `crates/zyvor-api/tests/failover.rs`,
`crates/zyvor-api/tests/decline_sweeper.rs`, `crates/zyvor-api/tests/routing_automation.rs`.

## What already existed

Hard filters (provider disabled, not allowed for this payment, capability matrix, rule exclusion), routing rules
(`prefer` / `exclude` by method and amount band), configured fees. Every decision is stored on the attempt (`route`)
with the providers considered. That is unchanged. What follows is added on top.

**One health rule.** The router used to carry a second, older heuristic of its own (any unresolved payment, or more
than half of the last 50 attempts of any method failing, only deprioritised a provider). It could not be switched off,
looked at every method together, never expired, and `GET /v1/routing/health` did not show it, so health could say
"healthy" while routing skipped the provider. It has been removed. The circuit breaker below is now the only health
rule routing applies, and the health endpoint, the router and the sweeper all evaluate it through one function
(`health::evaluate_one`). The consequence to know about: one unresolved payment no longer moves a provider down the
list by itself. It still blocks a retry of that same payment (unchanged), counts as unresolved in the signal and, with
enough samples, counts toward the breaker. A small business that wants an earlier reaction can lower `min_samples`
(minimum 3).

## 1. The success-rate signal

For each tenant, provider and method the API computes, on demand, from `payment_attempts`:

- the newest `window_attempts` resolved attempts (default 100) that are no older than `window_minutes` (default 30);
- `successes` (captured or authorised), `failures` (the provider declined it, or refused the request) and
  `unresolved` (outcome unknown). Merchant cancellations and attempts still in flight are not counted;
- `success_rate_bps` (integer basis points), reported **only once there are at least `min_samples` resolved attempts**
  (default 10). Below that it is `null`, and a `null` rate never changes routing.

Nothing is stored for this: it is derived from the attempts. **Payer declines are left out** of `samples`, `failures`
and the success rate, and reported on their own as `payer_declines` (see "Decline classes" below). A provider whose
payers are often short of money therefore does not look worse than it is, and a business with only payer declines has
no verdict at all. Declines whose cause is not recognised still count as failures, so the default threshold is still a
majority (50%) and not a few percent.

## Decline classes

Each failed attempt carries a `payment_attempts.failure_class` (migration 0079 widened the allowed values):

| Class | Meaning | In the success rate and breaker? | Failover to another provider? |
|---|---|---|---|
| `payer_declined` | the payer's doing: insufficient funds, the payer left or refused | **no**, shown as `payer_declines` | no (the payer being short of money is no reason to change provider) |
| `provider_fault` | the provider or bank side failed, including an unknown outcome (timeout) the provider later confirmed as failed | yes | yes |
| `declined` | the provider said "failed" and the cause is **not recognised** ("unknown"). Every attempt recorded before migration 0079 is here | yes (the conservative choice) | yes |
| `provider_rejected` | the provider refused the request before creating anything | yes | yes |
| `cancelled` | the merchant cancelled it | no | no |

An unknown decline counts as a provider-side failure on purpose: wrongly excusing a failing provider hides an outage,
while wrongly counting one payer decline only makes the breaker slightly more cautious. A probe that the payer declined
proves nothing about the provider, so it is neither a success nor a failure for the breaker (the next cool-down allows
another probe).

What is recognised (`crates/zyvor-providers/src/decline.rs`), using only strings and codes that appear in this
repository's adapters, simulator, local mock, fixtures and tests:

| Provider | Payer | Provider-side | Everything else |
|---|---|---|---|
| `cashfree` | payment status `USER_DROPPED` (and webhook `PAYMENT_USER_DROPPED_WEBHOOK`); reason `insufficient funds` (the text its subscription tests use) | none | `FAILED`, `CANCELLED`, `VOID` and the mock's "Payment declined by payer bank" are unknown |
| `razorpay` | none shown in the repository | none | all unknown, including the mock's "Payment failed at bank" |
| `phonepe` | none shown in the repository | none | all unknown (`PAYMENT_ERROR`, `PAYMENT_DECLINED`, ...) |
| `sim` (simulator), `sponsor_upi` and custom providers that send the same words | `insufficient_funds`, `declined_by_payer` | `failed_at_provider` | `simulated_failure`, `simulated_token_decline` and any other text are unknown |

Spelling is normalised (case, underscores, hyphens). **None of this was checked against a live provider**: real
providers have many more codes (for example for a bank being down, a card expired, a wrong PIN, a failed 3-D Secure
step), and until a real sandbox run (see [REAL_SANDBOX.md](REAL_SANDBOX.md)) shows what each means, they are unknown and
count against the provider. The Razorpay and PhonePe rows are empty for exactly that reason.

## 2. Routing options

A routing rule may set `prefer_success_rate: true` (optionally `success_rate_margin_bps`, default 200):

```json
{ "rules": [ { "method": "upi", "prefer_success_rate": true } ] }
```

Among the providers still eligible after the hard filters, failover and the breaker, the provider with the best recent
success rate wins **only if** every remaining provider has enough samples and the best leads the next by the margin.
Otherwise the lowest fee decides, as before, and the reason says why (`not enough recent attempts ...`, `success rates
within 200 bps ...`). A `prefer` list that names an eligible provider still wins over this option. A new provider with
no history therefore gets traffic from fee or `prefer`, not from this option.

### Circuit breaker (per tenant, provider and method)

- **Degrade.** When a routed payment is evaluated and the window has at least `min_samples` resolved attempts and at
  least `degrade_failure_pct` (default 50) of them failed or are unresolved, the provider is marked `degraded` for that
  method. Unresolved attempts count against it for the breaker (a run of timeouts is how an outage usually looks).
- **Skip.** A degraded provider is skipped for NEW payments (`outcome: skipped_degraded`). If every eligible provider is
  degraded, routing still chooses among them and says so: a signal never leaves a business unable to take payments.
- **Cool-down and probe.** After `cooldown_seconds` (default 120) one payment may probe the degraded provider
  (`outcome: probe`, `route.probe`). The probe slot is claimed with a single atomic UPDATE, so concurrent payments cannot
  all probe. While a probe is pending or unresolved, no second probe is sent. A failed probe restarts the cool-down.
  A probe is a real customer payment: if you do not want that, turn the breaker off.
- **Recover.** The provider becomes healthy again when a probe succeeds, or when the rolling failure rate is at or below
  `recover_failure_pct` (default 20) with enough samples (for example late webhooks resolved old unknowns). After a
  recovery the window starts afresh, so the failures that caused the degradation are not held against it again.
- **Record.** Each degrade and recovery writes an audit entry (`provider.degraded`, `provider.recovered`) and queues a
  webhook event of the same name (both are in `GET /v1/events/catalogue`), once, in the transaction that made the change.

### The health sweeper

Transitions used to be evaluated only when a payment was routed, so a quiet business was not marked degraded or
recovered until its next payment. The worker (`worker::run_cycle`, step 3e) now re-evaluates every enabled provider and
method of each business that has attempts in the window or a stored breaker state, and records any transition through
the same code a routed payment uses. So failures that were settled late (the resolver, a webhook) degrade a provider
without waiting for a payment, and late successes recover it.

- **Safe with several workers.** One advisory lock per business (`pg_try_advisory_xact_lock`, held for the sweep's
  transaction) means one replica sweeps a business at a time; the others skip it and move on (`skipped_busy`).
- **Exactly once.** A transition is a conditional update (`state <> 'degraded'` to degrade, `state = 'degraded'` to
  recover); the audit entry and the event are written only when that update changed a row. A routed payment evaluating
  the same state at the same instant blocks on the row, then changes nothing.
- **Idempotent.** Running it again changes nothing unless the evidence changed.
- **It never starts a payment and never takes the probe slot.** Only a real payment can probe; the sweeper does not.
- **What it does not do.** Evidence that ages out of the window does not recover a degraded provider: only a successful
  probe, or a window with enough samples that is at or below `recover_failure_pct`, does. A provider left degraded by a
  quiet business shows `probe_due` once its cool-down is over, and the next payment tests it. Time is the database
  clock; the test clock in `clock.rs` is a calendar date and is not used for the minutes-long window, so tests move time
  by changing stored timestamps.

`GET /v1/routing/health` shows what the next payment will see (`state`) next to what is stored (`stored_state`) and never
writes. Each method row also says what routing does with it (`routing_effect`: `normal`, `skipped`, `probe_next` or
`disabled`, with `routing_effect_text`), and the response carries `routing_health_rule`. A provider or method with no
attempts in the window has no row and is used normally. The breaker is not shared across tenants, and it is per method:
a provider degraded for card is still used for UPI.

### What every decision records

`route.considered` lists every provider with `outcome` (`selected`, `not_chosen`, `rejected`, `skipped_degraded`,
`skipped_failover`, `probe`), a `reason`, an `explanation`, the estimated fee and the `signal` it was judged on
(`samples`, `successes`, `failures`, `payer_declines`, `unresolved`, `success_rate_bps`, `min_samples`, `breaker`).
`route.explanation` repeats the chosen provider's sentence. `route.failover_from` and `route.probe` appear when they
apply. The same facts go into the `payment_attempt.created` audit entry.

**`explanation`** is one plain-language sentence a person can read in a console or an API client, written from the same
facts as the decision, for example:

- "Chosen because it has the lowest estimated fee (INR 5.00)."
- "Chosen because it has the best recent success rate (95.0% against 75.0% for razorpay)."
- "Chosen because it is the only provider left after skipping razorpay (degraded by the circuit breaker)."
- "Chosen as a test: razorpay was degraded, its cool-down is over, and this payment checks whether it works again."
- "Skipped because the circuit breaker has it degraded: 18 of its last 20 payments failed or are unresolved."
- "Not chosen: cashfree was used instead, which has the lowest estimated fee (INR 5.00)."

The reason `only eligible provider` is now used only when exactly one provider could take the payment at all. When other
providers were set aside (failed this payment already, or degraded), the reason names them instead (`only provider left
after skipping razorpay (degraded by the circuit breaker)`). Attempts routed before this change have no `explanation`.

## 3. Failover, and why it cannot double-charge

When the next attempt on a payment is created (`POST /v1/payment_intents/{id}/attempts`, or the checkout equivalent),
providers that **definitively failed this payment already** are skipped if another eligible provider exists:

| Previous attempt on this payment | Failover to another provider? |
|---|---|
| failed, provider refused the request before creating anything (`provider_rejected`) | yes |
| failed, provider answered "failed" (`declined`, by API response, webhook or lookup) | yes |
| failed, cancelled by the merchant (`cancelled`) | no: a late capture is still possible; plain retry rules apply |
| **unknown** (timeout, 5xx, dropped connection) | **never**: no new attempt of any kind until it is resolved |
| pending, created, authorised, captured | no new attempt at all |

The guards, none of which failover can bypass because they all run before routing in `start_attempt`:

1. Under a row lock on the intent, any attempt that is `created`, `pending`, `authorized`, `unknown` or `captured`
   refuses a new attempt (`attempt_in_progress`, `payment_outcome_unresolved`). So a second live attempt cannot exist
   next to an unresolved first one, and two simultaneous requests produce exactly one attempt.
2. An unknown outcome is resolved only by asking the provider (`POST /v1/payment_attempts/{id}/refresh`, the
   resolver, or a webhook). Only when that says `failed` is the payment eligible for another provider.
3. The idempotency key is claimed before anything else. Replaying a key returns the same attempt (re-read at its
   current state) and is never routed again.
4. The database allows at most one captured attempt per intent (`one_capture_per_intent`).

If failover is switched off, or no other provider is eligible, the next attempt simply goes to the same provider as a
plain retry always did (the decision says `no other eligible provider`). Residual risk, not new but worth stating: a
provider that reported "failed" and later captures anyway would be a provider bug; the existing late-capture handling
and the unique-capture index apply, and it would need a refund.

## 4. API

| Route | Scope | Purpose |
|---|---|---|
| `GET /v1/routing/health[?method=upi]` | `payments:read` | per provider and method: signal, `payer_declines`, breaker state, what routing does with it (`routing_effect`), reason, last probe, pending transition; thresholds; recent degrade/recover events. Lists the methods that have attempts in the window or a stored breaker state. |
| `GET /v1/routing/health/settings` | `payments:read` | the tenant's thresholds (defaults when never set) |
| `PUT /v1/routing/health/settings` | `admin` | change thresholds; only fields sent change; validated; audited (`routing_health.settings_updated`) |
| `GET /v1/routing/automation` | `payments:read` | shadow, soft retry, cost-aware and kill-switch settings, and `effective` (what acts right now) |
| `PUT /v1/routing/automation` | `admin` | change those settings; one audit entry per automation switched, plus `routing_automation.settings_updated` |
| `POST /v1/routing/automation/pause`, `.../resume` | `admin` | the kill switch; idempotent; audited |
| `GET /v1/routing/shadow/report[?days=7&method=upi]` | `payments:read` | shadow versus applied over a window |
| `GET /v1/routing/scorecard[?days=7&method=upi]` | `payments:read` | per provider and method scorecard |

Settings (defaults): `breaker_enabled` true, `failover_enabled` true, `window_attempts` 100 (5 to 1000),
`window_minutes` 30 (1 to 1440), `min_samples` 10 (3 to 1000), `degrade_failure_pct` 50, `recover_failure_pct` 20 (below
the degrade value), `cooldown_seconds` 120 (10 to 86400). Branch-limited keys cannot use these routes. All data is
tenant-scoped by row-level security (`routing_health_config`, `provider_health_state`; tested).

Existing tenants get the defaults without a migration step. Note that this turns the breaker and failover on for them:
failover only changes which provider a retry uses, and the breaker's default (at least 10 resolved attempts and at least
half failing) is close to the router's old heuristic, which it replaces. Turn the breaker off in the settings and routing
ignores provider health completely; turn failover off to keep retrying the same provider.

## 5. Making the simulators fail

- Local provider mock (`zyvor-provider-mock`): `POST /_sim/outage {"provider": "razorpay"|"cashfree", "mode": ...}`,
  sticky until `"off"`. `reject` answers HTTP 400 on order creation (definitive: nothing created), `unavailable` answers
  503 and `hang` never answers (both leave the outcome unknown to the caller). Add `"fail_pct": 30` (0 to 100, default
  100) for a **brown-out**: only that share of order-creation calls fails and the rest are served normally. Which calls
  fail is fixed by `"seed"` (default 0), so a run repeats exactly. `GET /_sim/stats` also reports
  `razorpay_captured_payments_by_order` next to the Cashfree equivalent, so a test can tell a double charge from one.
  The benchmark harness ([BENCHMARKS.md](BENCHMARKS.md)) uses these. The older one-shot switch
  `POST /_sim/behaviour` (`cashfree.pay` = `hang_after_accept`) models a timeout after the provider accepted the payment.
- The in-process `sim` provider already declines by UPI handle (`fail*`) and times out by handle (`uncertain*`).

## 6. Routing automation: shadow mode, soft retry, cost-aware routing, the kill switch

Status: **implemented and tested against the in-process simulator and the local provider mock only** (13 integration
tests in `tests/routing_automation.rs`, router unit tests, cost-model unit tests). Nothing was run against a real
provider. **Everything here is off until a business turns it on**, and one switch turns it all off again. Migration 0084.

### The kill switch

`POST /v1/routing/automation/pause` (admin) stops, for that business only and at once: shadow mode, soft retry,
cost-aware routing, failover and the circuit breaker's effect on routing. New payments are then routed by the
business's own rules and the lowest configured fee, as before any automation existed. Details: the health signal is
still computed and shown but every provider reads as taking payments, nothing is recorded as degraded or recovered,
the background sweeper skips the business, and `GET /v1/routing/health` says `automation_paused: true` and
`routing_effect: paused`. Each individual setting keeps its value, so `resume` puts things back exactly as they were.
Pause and resume are idempotent (a second pause writes no second entry) and audited (`routing_automation.paused`,
`routing_automation.resumed`, with the reason and who). Each switch inside `PUT /v1/routing/automation` also writes its
own entry (`routing_automation.shadow_enabled`, `soft_retry_disabled`, `cost_aware_enabled`, ...). Failover and the
breaker are still set in `/v1/routing/health/settings` (audited there). The kill switch is per business and never
affects another.

### Shadow mode

With `shadow_enabled`, every NEW attempt also records the decision a **candidate policy** would have made, next to the
decision that was applied (`routing_shadow_decisions`, linked to the attempt). The candidate policy is
`shadow_policy`: `{"rules": [...routing rules...], "cost_aware": true}`. Without `rules` it uses the business's live
rules; `cost_aware` defaults to true, so the usual question ("what if I turned on cost-aware routing") needs no policy
at all. The shadow decision sees exactly what the real one saw (same providers, health and fees) and carries the same
explanations.

Shadow mode **never changes routing and never calls a provider**: it is a second call to the same pure function that
chooses a provider, run after the real decision and the attempt were written, inside a savepoint, so a problem in it
rolls back only the shadow row and never fails a payment. It records no probe and claims no probe slot.

`GET /v1/routing/shadow/report?days=7&method=upi` (1 to 90 days): `decisions`, `agreed`, `differed`,
`agreement_rate_bps`, `differences` (applied provider versus shadow provider, with counts), `estimated_fee`
(`difference_minor` is applied minus shadow: positive means the shadow choice would have cost less; only decisions where
both providers have a cost basis are compared), `outcome_by_provider` (what happened to the payments that were really
sent, split by whether shadow would have chosen another provider) and the 20 latest differences with both sentences.
**Limit, stated in the response:** shadow mode cannot know how the other provider would have done. The outcomes are
those of the provider that really took each payment. The fee difference is an estimate from the cost model below, not a
settlement figure.

### Cost-aware routing

Off by default (`cost_aware_enabled`). When on, the lowest-cost provider among those still eligible (after the hard
filters, failover and the circuit breaker, and after any `prefer` rule or `prefer_success_rate` decision) wins, subject
to guardrails. A cost-aware decision never overrides a hard filter, failover, the breaker, or a rule that names a
provider.

**Cost of one payment** (`routing_auto::CostModel`), in this order of trust, whole paise, integer arithmetic, rounded
half up. The sentence names the source:

1. `observed`: the fees the provider actually deducted (settlement fee lines, the same source as
   `GET /v1/analytics/cost`, last 90 days), as a share of the volume they were deducted on, applied to this amount. Used
   only once at least `cost_min_fee_samples` captured payments have a fee line (default 30).
2. `fee_rule`: the merchant's own contract rule in force today (`/v1/fee_rules`: percentage, flat, cap, GST on the fee).
3. `configured`: the routing fee table (`/v1/settings`).
4. No basis: the provider has no cost and is treated as having no data (below).

**Choice among providers** (`router::cost_choice`), with `floor` = `cost_success_floor_bps` (default 8000, 80%) and
`premium` = `cost_premium_bps` (default 500, 5%):

- A provider is *qualified* only if its success rate is known (at least the health `min_samples` resolved attempts) and
  is at least `floor`. Payer declines are not counted in that rate (decline classes above).
- If any provider is qualified: the cheapest qualified provider wins (ties go to the better success rate, then the
  name), **unless** another qualified provider has a success rate at least 200 bps better than the cheapest's and costs
  no more than `cheapest cost x (10000 + premium) / 10000`: then that one wins, and the sentence says what was traded.
  Cheaper providers that were left out (below the floor, or too little data) are named in the sentence.
- If none is qualified: providers with no success-rate data may be chosen on cost (they are not known to be below the
  floor); if every provider has data and every one is below the floor, **the best success rate wins, never the cheapest**.

So a provider below the floor is never chosen only for being cheaper, and a provider with too little data cannot beat a
proven one on price. The trade-off: a new provider gets traffic on cost only while nobody is proven; to bring one in
earlier, use a `prefer` rule or lower `min_samples`. Example sentences: "Chosen because it has the lowest estimated
cost (INR 5.00, from your fee rule) among providers with a success rate of at least 80.0% (left out of the cost
comparison: cashfree (cheaper at INR 2.50, but its success rate is below your 80.0% floor))." and "Chosen because it has
a better recent success rate (97.0% against 88.0% for razorpay) and costs INR 0.40 more (...), within the 5.0% extra you
allow for reliability."

The defaults (floor, premium, samples) are reasoned starting points, **not tuned on real traffic**.

### Soft-decline retry on another provider

Off by default (`soft_retry_enabled`; `soft_retry_max_attempts` 2 to 5 counting the first attempt, default 2;
`soft_retry_window_seconds` 30 to 86400 from the payment's first attempt, default 600). When an attempt fails
definitively on the provider's side, Zyvor itself starts the next attempt on another provider so the payer is not sent
back to the start. The failover rules above still decide which provider; this only removes the need for the payer or the
merchant to start the retry.

**What is retried.** Only failure class `provider_fault` or `provider_rejected`. Not `payer_declined` (the payer's
doing), not `declined` (cause not recognised, treated as unknown), not `cancelled`, and never an `unknown` outcome. An
unknown outcome that the provider later confirms as failed becomes `provider_fault` and is then eligible; the retry
starts when that answer is applied (the resolver, `POST .../refresh`, or a webhook), never before.

**Conditions, all checked under the row lock on the payment** (`softretry::guard`, then `check_alternate`), on top of
the guards every attempt already passes:

- the failed attempt is the newest on the payment and has not been decided before;
- attempts so far are fewer than the limit and the window has not passed;
- another provider is eligible (hard filters, the payment's `allowed_providers`, rules), has not been used for this
  payment, and is not degraded (a due probe is not used for a retry);
- the failed attempt was created with the request stored (only while soft retry was on; never for a stored card token or
  a mandate debit). The stored UPI ID or return URL lives in `payment_attempts.retry_input` only for the retry, and is
  cleared when the retry is decided, when the payment is paid, and after a day.

When any of these fails for good (no alternate, limit, window, degraded alternate) the decision is recorded once as
`skipped` with the reason (`routing_soft_retries`, audit `routing.soft_retry_skipped`); nothing changes for the payer
and the checkout stops saying "trying another way".

**Why it cannot double-charge.** The retry is created by `payments::start_attempt_with`, the same function every
attempt goes through. Under a row lock on the payment it refuses a new attempt while any attempt is `created`,
`pending`, `authorized`, `unknown` or `captured`; the soft-retry guards only add conditions. It is idempotent three ways:
`routing_soft_retries.from_attempt_id` is UNIQUE, the retry uses the idempotency key `softretry:<failed attempt id>`, and
the newest-attempt check fails for any second caller. It runs from four places (the attempt request, the resolver and
refresh, a provider webhook in the background, and the worker's backstop sweep) and any number of them can race. Tests
count live attempts and provider orders: eight concurrent retriers plus four manual attempts on one failure create
exactly one new attempt and one order, three rounds in a row; a retry that itself goes `unknown` blocks every further
attempt; payer declines, unknown causes, cancellations and pending or unresolved payments are never retried.
Residual risk, not new (failover has it too): a provider that reported "failed" and later captures anyway. The existing
late-capture handling and the one-capture-per-payment index apply, and it would need a refund.

**What the payer sees.** The checkout session shows phase `trying_another_way` (with `retried_on_another_provider` once
the new attempt exists) while a retry is due: "Trying another way. The first attempt did not go through, and you will
not be asked to pay twice." The strings exist in the six checkout languages, **not reviewed by native speakers**. The
response of the original attempt request is always that original (failed) attempt (the request waits for the retry's
provider call, so in the worst case its response time is two provider calls); the new attempt appears on the
payment and in the session. `route.soft_retry` on the new attempt says which attempt it followed and why.

### Provider scorecard

`GET /v1/routing/scorecard?days=7&method=upi`, per provider and method over the window:
`success_rate_bps` (successes over successes + provider failures + unresolved; payer declines and merchant
cancellations are left out; null below the health `min_samples`), the counts behind it, `failure_classes` (how failed
attempts split by class), `median_seconds_to_resolve` (creation to last update, for failed attempts and for captured
attempts no refund has touched, because a refund moves the last-update time), `unresolved_now` (attempts with an unknown
outcome right now, whatever their age), `observed_fee` (observed effective rate, payments counted, whether cost-aware
routing would use it) next to `configured_fee`, the breaker state, and `last_degraded_at` / `last_recovered_at` from the
audit entries. The console shows it on the Routing health page.

### Not done in this section

Shadow mode compares decisions, not outcomes (see above). The fee basis is only as good as the merchant's rules and
imported settlements. No per-provider or per-method overrides of the cost settings, no A/B experiments, no time-of-day or
issuer-level signals. A webhook-triggered retry runs in the background of the webhook request, so a process that stops
at that moment leaves it to the worker's next cycle. No event is emitted for a soft retry (the audit trail has it).
Everything was run against simulators only; the decline classes that decide what is retried are the ones listed in the
decline table, none verified against a live provider.

## What is not done

The console has a **Routing health** page, now also with Automation (kill switch and settings), shadow mode and scorecard sections added in section 6 (those sections are unit-tested but were not looked at in a browser). The health part (success rate or "not enough data", saved and current state, recent changes, and limits that only an admin can change); it was looked at in a browser against a stand-in API with made-up data, not against a running Zyvor API. No per-provider threshold overrides
(one set per tenant). No cross-tenant or platform-wide signal. The decline classifier knows only the codes listed above;
Razorpay and PhonePe have none, and no failure code of any provider was verified against a live account. Existing
declines recorded before migration 0079 stay `declined` (unknown) and keep counting against the provider until they age
out of the window. The sweeper and the classifier are tested against simulators only.
