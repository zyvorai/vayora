# Routing: success-rate signal, circuit breaker and failover

Status: **implemented and tested against the in-process simulator and the local provider mock only.** No real provider
has been used, so the defaults below are reasoned starting points, not tuned values. Nothing here changes a payment
that already exists; it only influences which provider a NEW attempt is sent to.

Code: `crates/zyvor-providers/src/router.rs` (the pure decision), `crates/zyvor-providers/src/decline.rs` (whose fault
a decline was), `crates/zyvor-api/src/health.rs` (signal, breaker, sweeper, failover list, API),
`crates/zyvor-api/src/worker.rs` (runs the sweeper), `crates/zyvor-api/src/payments.rs` (`start_attempt`, the one place
attempts are created; `apply_provider_state`, the one place they are classified).
Tests: router and decline unit tests, `health.rs` unit tests, `crates/zyvor-api/tests/failover.rs`,
`crates/zyvor-api/tests/decline_sweeper.rs`.

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
  503 and `hang` never answers (both leave the outcome unknown to the caller). The older one-shot switch
  `POST /_sim/behaviour` (`cashfree.pay` = `hang_after_accept`) models a timeout after the provider accepted the payment.
- The in-process `sim` provider already declines by UPI handle (`fail*`) and times out by handle (`uncertain*`).

## 6. The routing rule builder

Status: **implemented and tested against simulators only.** Code: `crates/zyvor-api/src/rulebuilder.rs`. It sits on top of
the router; `zyvor_providers::router::choose` is unchanged. For each payment the rules whose conditions hold are handed
to the router as ordinary `prefer` / `exclude` / `prefer_success_rate` rules, so every hard filter, the circuit breaker,
failover and the "unknown payment blocks retries" rule apply exactly as before. **A rule cannot send a payment somewhere the
router would refuse** (tested: a preference for a provider the payment may not use is ignored).

```json
{ "name": "rupay cards to cashfree",
  "when": { "methods": ["card"], "card_networks": ["rupay"], "min_amount_minor": 100000,
            "currencies": ["INR"], "segments": ["new"],
            "time_window": { "from": "22:00", "to": "06:00", "utc_offset_minutes": 330 } },
  "then": { "prefer": ["cashfree"], "exclude": [], "prefer_success_rate": false } }
```

Every condition is optional and all of them must hold. Unknown field names are refused (a typo is an error, not a rule
that silently does nothing). Order matters: the first `prefer` that names an eligible provider wins, as before.

| Condition | Meaning |
|---|---|
| `methods`, `currencies` | the payment's method and currency |
| `min_amount_minor`, `max_amount_minor` | the amount the payer is charged (after any offer discount) |
| `card_networks` | network class from `instrument.card_network`, a saved card token, or a BIN; an unknown network matches no network rule |
| `segments` | `new` or `returning`; a payment with no customer matches no segment rule |
| `time_window` | `HH:MM` from (inclusive) to (exclusive) in a fixed UTC offset (default +330); `to` earlier than `from` crosses midnight. A fixed offset, not a time-zone database: no daylight saving |

**Validation** (`POST /v1/routing/rule_sets/validate`, and on every save) reports each problem with the rule it is in:
unknown provider, method, currency, card network or segment; amount band the wrong way round; a time window that is
malformed or empty; a rule that does nothing; a provider both preferred and excluded; a card-network condition on a rule
whose methods exclude `card`; more than 50 rules; and a rule that would exclude **every** provider account (matching
payments could not be routed). A provider with no account here is a warning, not an error.

**Versions.** `POST /v1/routing/rule_sets` saves an immutable numbered version (optionally activating it). One active
pointer per business. `POST .../{version}/activate` switches; `POST .../rollback` activates what was active before the
current one (rolling back twice toggles); `POST .../deactivate` goes back to the older `tenant_settings.routing.rules`.
With no active version the settings rules apply exactly as before; with one, it **replaces** them (fees still come from
the settings). Each change is audited (`routing_rules.version_created`, `.activated`, `.rolled_back`, `.deactivated`)
and every attempt records the version and which rules matched (`route.rule_set`). Switching is refused while an
experiment runs.

**Dry run.** `POST /v1/routing/dry_run` answers "which provider would this payment take": the full decision
(`considered`, explanations, signals), the rules that matched and the ones that did not (with why), and the experiment
arm. It can try unsaved `rules`, an older `version`, an `at` time for time-of-day rules, a `card_network` or `card_bin`,
and a `customer_id` or `segment`. It creates nothing and claims no circuit-breaker probe (`would_probe` says whether a
real payment would be the probe). Tested: attempts and audit entries do not change, and the answer equals what a real
attempt then does.

## 7. Routing experiments (A/B)

Status: **implemented and tested against simulators only.** Code: `crates/zyvor-api/src/experiments.rs`.

A running experiment sends a share of payments (1 to 50 percent, optionally one method only) through a saved variant
version instead of the active one (the control). The split is by hash of the experiment's salt and the **intent id**:
every attempt on a payment is in the same arm, retries and failovers included. One experiment per business at a time. The
variant is an ordinary version, so every router guard applies to it. If its rules leave a payment with no provider, the
control's rules route it and the attempt is marked `fell_back` and left out of the results, so an experiment cannot make a
payment fail that would have been routed.

**Guardrails** (documented, not tuned on real traffic):

- nothing is concluded until **both** arms have `min_sample` resolved attempts (default 100, at least 20);
- the variant is stopped **automatically** only when it is clearly worse: its success rate is at least `min_drop_bps`
  below the control's (default 300, three percentage points) **and** a one-sided two-proportion z-test gives
  z >= 2.33 (about 99% one-sided; stored as `z_x100`);
- a variant that looks better is only reported (`variant_better`); nothing is promoted automatically;
- what counts is the same as for routing health: successes are captured or authorised attempts, failures are provider-side
  declines and declines of unknown cause, an unknown outcome counts against the arm, payer declines and merchant
  cancellations are shown but left out of the rates;
- checking repeatedly raises the chance of a false stop, so the threshold is deliberately strict. This is a safety stop
  against a clearly broken variant, not a statistical study. It does not adjust for amount mix, method mix or time of day.

The worker evaluates the running experiment each cycle and `POST /v1/routing/experiments/{id}/evaluate` does the same on
demand; a read (`GET`) never stops anything and shows `would_auto_stop`. A stop (by a person or automatic) writes an audit
entry, queues `routing.experiment_stopped`, and an automatic one also opens a case (`routing_experiment_stopped`) and
records the reason (both rates, the gap, z, the sample sizes). The control rules are in force again at once.
`GET /v1/routing/experiments/{id}` returns per arm the stats, providers used and the verdict.

## 8. Provider and bank downtime alerts

Status: **implemented and tested against seeded history and simulators only.** Code: `crates/zyvor-api/src/alerts.rs`.

The worker (step 3f) and `POST /v1/routing/alerts/evaluate` read the last `window_minutes` (default 30) of attempts and
look at three kinds of group: a provider for one method, a **bank** within it and a **UPI handle** within it, where
attempts carry one (`instrument.bank`, the handle of the typed UPI ID, or `instrument.upi_handle`). A group whose failures
(provider-side and unknown-cause declines, plus unknown outcomes; never payer declines or cancellations) reach
`open_failure_pct` (default 50) of at least `min_samples` (default 20) opens **one** alert: a case (`provider_alert`), a
`provider.alert_opened` event for the webhook outbox, an audit entry and, when `email_to` is set, an email through the
existing mail queue. It sits beside the circuit breaker, which steers routing per provider and method; alerts also cover
banks and handles, and tell people. Alerts are **on by default** for every business (no row in `provider_alert_config` means the defaults; turn them off with `enabled: false`), and a business that subscribes to all webhook events will now receive the new event types. Bank and handle alerts need the caller to send them (see OFFERS.md, `instrument`);
no attempt is classed by a bank or handle unless one was given.

De-duplication and flap control:

- at most one open alert per provider, method, kind and value (a partial unique index), however often it is checked, and
  from however many worker replicas (an advisory lock per business);
- hysteresis: it opens at `open_failure_pct` and resolves only at or below `resolve_failure_pct` (default 20, always
  lower) with at least `min_samples` attempts, and not before it has been open `hold_seconds` (default 300);
- after it resolves, the failures that caused it are not counted again: only attempts made since can reopen it;
- a recurrence within `flap_window_seconds` (default 900) of the resolution **reopens the same alert quietly**
  (`flap_count` goes up, the case reopens, an audit entry is written, but no new event or email); a later one raises a new
  alert;
- a provider-level alert also resolves when the circuit breaker has recovered that provider and method, and any alert
  resolves after 24 hours with no attempts at all (a skipped provider gets no traffic to prove it is back);
- resolving writes `provider.alert_resolved`, closes the case and emails once.

## 9. API (rule builder, experiments, alerts)

| Route | Scope | Purpose |
|---|---|---|
| `POST /v1/routing/rule_sets/validate` | `routing:read` | check rules, nothing saved |
| `POST /v1/routing/rule_sets` | `routing:write` | save a version (validated); `activate: true` to use it |
| `GET /v1/routing/rule_sets`, `GET .../{version}` | `routing:read` | versions, the active one, the previous one |
| `POST .../{version}/activate`, `POST .../rollback`, `POST .../deactivate` | `routing:write` | switch (refused while an experiment runs) |
| `POST /v1/routing/dry_run` | `routing:read` | which provider would this payment take |
| `POST /v1/routing/experiments`, `GET`, `GET .../{id}` | `routing:write` / `routing:read` | start, list, results |
| `POST .../{id}/stop`, `POST .../{id}/evaluate` | `routing:write` | stop; run the guardrail now |
| `GET /v1/routing/alerts[?status=]`, `GET .../{id}` | `routing:read` | alerts, one alert with its case |
| `POST .../{id}/ack`, `PUT .../settings`, `POST .../evaluate` | `routing:write` | acknowledge; thresholds and email; check now |
| `GET /v1/routing/alerts/settings` | `routing:read` | thresholds |

New scopes: `offers:read`, `offers:write`, `routing:read`, `routing:write` (`admin` has all; `finance` console roles get
the offers pair and `routing:read`, viewers get both read scopes). New events in the catalogue: `provider.alert_opened`,
`provider.alert_resolved`, `routing.experiment_stopped`. New tables (migration 0086) are all tenant-scoped by row-level
security and tested as the app role: `routing_rule_versions`, `routing_rule_active`, `routing_experiments`,
`routing_experiment_assignments`, `provider_alert_config`, `provider_alerts` (and the offers tables, see OFFERS.md).
Tests: `crates/zyvor-api/tests/routing_rules.rs`, `crates/zyvor-api/tests/provider_alerts.rs`, unit tests in the three modules.

## What is not done

The console has a **Routing health** page (success rate or "not enough data", saved and current state, recent changes, and limits that only an admin can change); it was looked at in a browser against a stand-in API with made-up data, not against a running Zyvor API. No per-provider threshold overrides
(one set per tenant). No cross-tenant or platform-wide signal. The decline classifier knows only the codes listed above;
Razorpay and PhonePe have none, and no failure code of any provider was verified against a live account. Existing
declines recorded before migration 0079 stay `declined` (unknown) and keep counting against the provider until they age
out of the window. The sweeper and the classifier are tested against simulators only.

Rule builder, experiments and alerts (sections 6 to 8): no console page yet (the API and SDKs only); the time-of-day
condition uses a fixed UTC offset; an experiment compares two saved versions of the one rule set, not arbitrary
configurations, and has no stratification or multiple-testing correction; the alerts see a bank or UPI handle only when
the caller sends one (a provider that reports the bank itself is not read), and nothing was checked against a real provider
outage or a real bank's downtime pattern. Thresholds are reasoned starting points, not tuned values.
