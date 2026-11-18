# Routing: success-rate signal, circuit breaker and failover

Status: **implemented and tested against the in-process simulator and the local provider mock only.** No real provider
has been used, so the defaults below are reasoned starting points, not tuned values. Nothing here changes a payment
that already exists; it only influences which provider a NEW attempt is sent to.

Code: `crates/zyvor-providers/src/router.rs` (the pure decision), `crates/zyvor-api/src/health.rs` (signal, breaker,
failover list, API), `crates/zyvor-api/src/payments.rs` (`start_attempt`, the one place attempts are created).
Tests: router unit tests, `health.rs` unit tests, `crates/zyvor-api/tests/failover.rs`.

## What already existed

Hard filters (provider disabled, not allowed for this payment, capability matrix, rule exclusion), routing rules
(`prefer` / `exclude` by method and amount band), configured fees, and a coarse health heuristic (more than half of the
last 50 attempts failed, or any unresolved payment, only deprioritises). Every decision was stored on the attempt
(`route`) with the providers considered. That is all unchanged. What follows is added on top.

## 1. The success-rate signal

For each tenant, provider and method the API computes, on demand, from `payment_attempts`:

- the newest `window_attempts` resolved attempts (default 100) that are no older than `window_minutes` (default 30);
- `successes` (captured or authorised), `failures` (the provider declined it, or refused the request) and
  `unresolved` (outcome unknown). Merchant cancellations and attempts still in flight are not counted;
- `success_rate_bps` (integer basis points), reported **only once there are at least `min_samples` resolved attempts**
  (default 10). Below that it is `null`, and a `null` rate never changes routing.

Nothing is stored for this: it is derived from the attempts. Payer declines (for example insufficient funds) count as
failures, because the attempt record does not say whose fault a decline was. That is why the default threshold is a
majority (50%) and not a few percent. A provider with a high share of genuine payer declines would look worse than it is.

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
  webhook event of the same name (both are in `GET /v1/events/catalogue`), once, in the routing transaction.

Honest limits: transitions are evaluated when a payment is routed (there is no background sweeper), so a quiet tenant is
not marked degraded or recovered until the next payment of that method. `GET /v1/routing/health` shows what the next
payment will see (`state`) next to what is stored (`stored_state`) and never writes. The breaker is not shared across
tenants, and it is per method: a provider degraded for card is still used for UPI.

### What every decision records

`route.considered` lists every provider with `outcome` (`selected`, `not_chosen`, `rejected`, `deprioritised`,
`skipped_degraded`, `skipped_failover`, `probe`), a plain-language `reason`, the estimated fee and the `signal` it was
judged on (`samples`, `successes`, `failures`, `unresolved`, `success_rate_bps`, `min_samples`, `breaker`).
`route.failover_from` and `route.probe` appear when they apply. The same facts go into the `payment_attempt.created`
audit entry.

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
| `GET /v1/routing/health[?method=upi]` | `payments:read` | per provider and method: signal, breaker state, reason, last probe, pending transition; thresholds; recent degrade/recover events. Lists the methods that have attempts in the window or a stored breaker state. |
| `GET /v1/routing/health/settings` | `payments:read` | the tenant's thresholds (defaults when never set) |
| `PUT /v1/routing/health/settings` | `admin` | change thresholds; only fields sent change; validated; audited (`routing_health.settings_updated`) |

Settings (defaults): `breaker_enabled` true, `failover_enabled` true, `window_attempts` 100 (5 to 1000),
`window_minutes` 30 (1 to 1440), `min_samples` 10 (3 to 1000), `degrade_failure_pct` 50, `recover_failure_pct` 20 (below
the degrade value), `cooldown_seconds` 120 (10 to 86400). Branch-limited keys cannot use these routes. All data is
tenant-scoped by row-level security (`routing_health_config`, `provider_health_state`; tested).

Existing tenants get the defaults without a migration step. Note that this turns the breaker and failover on for them:
the breaker's default (at least 10 resolved attempts and at least half failing) is close to the old heuristic, and
failover only changes which provider a retry uses, but turn both off in the settings if you want the old behaviour.

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

## What is not done

No console screen yet (API only). No background sweeper for degrade/recover. No per-provider threshold overrides (one
set per tenant). No cross-tenant or platform-wide signal. Nothing verified against a real provider's behaviour.
