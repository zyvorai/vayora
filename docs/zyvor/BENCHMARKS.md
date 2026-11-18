# Benchmarks

Status: **simulator only.** Every number on this page was produced by `bench/run.sh` on one laptop, against the local
provider mock (`zyvor-provider-mock`) and synthetic data. **No real payment provider, bank, card network, UPI app or
customer was involved, and no real money moved.** Read [What this does not show](#what-this-does-not-show) before
quoting anything.

## What this is for

To let anyone check, on their own machine, a few specific behaviours of Vayora's routing, idempotency, reconciliation and
fee checks, and to put a number on each so that a change that makes one worse is visible. It is a regression harness and
a method, not a league table.

- The "baseline" in scenario 1 is **a payment pinned to a single provider with no Vayora routing** (the intent's
  `allowed_providers` names one provider). It is not any named provider's product and says nothing about one.
- **This page makes no claim that Vayora is better than, faster than or more reliable than any named payment
  provider, aggregator or orchestrator**, and none should be derived from it. A simulator that fails when told to fail
  cannot show how real providers behave.
- Numbers that come from here may be quoted elsewhere (README, website, decks) only with the label "simulator", the
  scenario and the sample size, and without comparison to a named company.

## Run it

```
bench/run.sh                  # everything, about 9 minutes with a cold build of the two binaries
bench/run.sh --quick          # smaller samples, about 3 minutes
bench/run.sh --only s1_outage,s3_recon --out /tmp/bench
bench/ci.sh                   # two full runs + reproducibility check + hard gates (not wired into a workflow yet)
node bench/check-repro.mjs run1/results.json run2/results.json
```

Needs `cargo`, Node 22 or later (no npm packages), `psql` and a local Postgres where you may create databases
(`BENCH_PG_USER`, `BENCH_PG_HOST`, `BENCH_PG_PORT`), `curl` and `git`; `pnpm` and a Chrome/Chromium
(`BENCH_CHROME`) are optional (checkout weight, browser measurement). The harness builds the API and the mock, creates
a private database and a private database role, listens on private ports (`BENCH_API_PORT` 18080, `BENCH_MOCK_PORT`
19090; scenario 5 uses 18081, 19091 and 13002), and stops **only the processes it started** and drops only what it
created. It does not use `pkill`. Output goes to `bench/results/run-<time>/`: `results.json`, `results.md`, and one JSON
per scenario. `bench/results/reference/` holds the run quoted below.

Everything is seeded (`BENCH_SEED`, default 20261118): amounts, UTRs, narration wordings, which rows get which injected
error, which calls fail in a brown-out. Payments in scenario 1 run one after another so the failure pattern is exactly
the same on every run; scenario 2 is deliberately parallel.

## The machine

The reference run was measured on: Apple M6, 12 logical cores, 16 GB RAM, macOS (darwin 27.3, arm64), PostgreSQL 14.24
(Homebrew), rustc 1.100, Node 26.11, Chrome 157 for the browser measurement. API, mock, Postgres, the load generator
and the browser all ran on this one machine, with other work going on in the background, so absolute times are only
indicative. The harness records its machine in `machine.json` on every run.

## Method

### 1. Payment success during an outage and a brown-out

- Two fresh tenants per cell ("arms"), both with the same two simulated providers. **Baseline arm:** every intent is
  pinned to one provider (`allowed_providers: ["razorpay"]`), so there is no routing, failover or breaker to help.
  **Vayora arm:** the intent may use either provider; failover, the success-rate signal and the circuit breaker are on
  with their default settings. Razorpay is made the cheaper provider for UPI (invented fee rates), so it is the primary
  in both arms. The names are only labels for two simulated endpoints.
- 20 healthy payments first (so each arm has the same history), then the fault is switched on in the mock and 40
  payments run (`BENCH_QUICK` uses fewer). The fault is a sticky `POST /_sim/outage` on the primary: `reject` (HTTP 400,
  nothing created: a definitive answer), `unavailable` (HTTP 503) or `hang` (no answer until the API's 1 s provider
  timeout: the outcome is unknown). `fail_pct` makes it a **brown-out**: only 30%, 60% or 100% of order-creation calls
  fail, chosen by a seeded hash so every run fails the same calls.
  *This `fail_pct` and `seed` switch is new in this change; without it the mock could only fail every call.*
- One payment is a customer journey with the same policy in both arms: create the intent; start an attempt; up to 3
  attempts, retrying only after a definite failure (the API refuses any other retry); an `unknown` outcome is
  asked about once (`POST /v1/payment_attempts/{id}/refresh`) and, if still unknown, the payment stops there, which is
  the rule. The payer always approves a pending attempt; payer declines are not part of this scenario. The "status
  lookup" column says whether the provider's lookup works during the outage; with `no` the lookup is not called, which
  models a lookup that fails too and leaves every in-flight payment `unknown`.
- Reported: payments captured (success rate), payments that ended `unknown` and `failed`, payments with an unknown
  first outcome, how many were captured through the alternate provider, total attempts, end-to-end latency per payment
  (p50, p95, max), **time to failover** in two senses (the first capture on the alternate provider, measured from the
  moment the outage began; and how many payments passed before a payment's *first* attempt was already sent to the
  alternate, which is the breaker having opened), and the primary's breaker state at the end.

### 2. Double-charge check

One tenant, both providers allowed, the client as the adversary: after a provider accepts a payment but times out, it
sends retries (sequential and 5 in parallel, with new keys); during outages (503 and hang) it retries in parallel, asks
the provider, fails over and approves late; it sends 8 parallel requests with the same idempotency key and 8 with
distinct keys; the provider delivers each webhook up to 3 times (also in parallel); intent creation is replayed 5 times
per key in parallel and once with the same key and a different body; and a seeded chaos phase mixes random faults on
either provider, random parallelism and a mix of repeated and fresh keys. 120 intents and about 1000 attempt
requests in a full run.

The audit uses **two independent sources**: the platform's own records (intents with more than one `captured` attempt)
and the simulated provider's records (successful payments per provider order, mapped back to intents). The second is
the one that would correspond to money leaving a payer. The scenario also fails if any request returned something it
should not (for example a second live attempt being accepted). The payer approves every attempt the platform shows as
pending, which is what a customer would do.

### 3. Reconciliation

For each of 8 statement layouts (`hdfc`, `icici`, `sbi`, `axis`, `kotak`, `generic_split`, `generic_signed`,
`generic_drcr`) a fresh tenant gets 60 random settlements (made-up UTRs in five styles, random amounts and dates) and
one synthetic CSV written in that layout (headings and date and amount styles as in
[BANK_RECONCILIATION.md](BANK_RECONCILIATION.md); account numbers are zeros). The ground truth is known for every row.
Injected: references in the reference column; references only in the narration in seven wordings; a credit short by a bank
charge; no credit at all; references broken by a space, cut off or masked; credits with no reference (including two pairs
of twins with identical amount and date); a row duplicated; near-miss references on credits that belong to nothing
(one digit added, removed or changed); unrelated credits; and debits. After import the scenario reads back every stored
line and scores it: **confirmed correctly, confirmed wrongly, suggested (and whether the truth is among the suggestions),
unmatched**. "Auto-confirmed wrong" counts any credit the system confirmed to a settlement other than its true one,
and **any confirmation of a credit that has no true settlement, duplicates included**. Also scored: settlements with no
credit flagged by `POST /v1/recon/scan`, short credits flagged as amount mismatches, and whether re-uploading the same file or an
overlapping half adds rows.

### 4. Fee-variance detection

200 payments are created and captured through the API (100 on each provider's simulated path). Two fee rules are
entered (invented rates: 1.75% + 18% GST, and 1.5% + 300 paise flat, capped at 12,000 paise, + 18% GST; 12% of the
payments are chosen so the percentage lands exactly on half a paisa, to exercise rounding). Settlements are then
imported in which each payment's fee line equals what the rule expects, except for a random 25% where an over- or
under-deduction of 1, 2, 3, 5, 10, 25, 118, 500 or 2000 paise is injected. The platform's findings are scored
against the injected truth at tolerances of 0, 2 and 10 paise ("should flag" means the injected error is larger than
the tolerance). The harness computes the expected fee itself with the formula in [FEE_CHECKS.md](FEE_CHECKS.md) and also
reports any line where the platform's expected fee differs from it.

### 5. Developer time to first payment and checkout weight (local)

`bench/devtime.sh` clones the committed `HEAD` into a temp directory (a clean checkout), builds `zyvor-api` and
`zyvor-provider-mock` in release mode **with an empty target directory** (a cold build, from the crates already cached on
the machine; downloads are not timed), creates a database, migrates, starts both services, creates an organisation,
connects both simulated providers, creates an intent, starts an attempt, has the simulated payer approve, and stops the
clock when the attempt reads `captured`. Each stage is timed. Then it installs and builds `apps/zyvor-checkout`, serves
it with `next start` and measures one payment page two ways: **curl accounting** (bytes sent for the HTML and every
`/_next/static` file it references, with gzip/brotli accepted) and **headless Chrome through the DevTools protocol**
(every request the page actually makes, cache disabled, with and without a throttle of 1.6 Mbit/s down, 750 kbit/s up and
562.5 ms round trip, Chrome's "Fast 3G" preset values). These are local numbers: the server is on the same machine, so
network time is only what the throttle adds.

## Results

The run below is `bench/results/reference/` (the first of two consecutive full runs; the second agreed within the tolerances
in [Reproducibility](#reproducibility-and-tolerances)). Latencies are for one machine and indicative only; the counts in
sections 2 to 4 are deterministic or nearly so. Section 1 uses 40 payments per cell: a share such as "92.5%" is 37 of 40, not a
precise rate.

### 1. Success during an outage or brown-out (40 payments per cell after 20 healthy ones)

| Primary provider fault | Failing calls | Status lookup | Baseline captured | Vayora captured | Baseline unknown | Vayora unknown | Vayora: payments until routed away | Vayora: first capture on alternate | Baseline p50 | Vayora p50 |
|---|---|---|---|---|---|---|---|---|---|---|
| reject | 0% | yes | 40/40 (100%) | 40/40 (100%) | 0 | 0 | never | n/a | 19 ms | 9 ms |
| reject | 30% | yes | 40/40 (100%) | 40/40 (100%) | 0 | 0 | never | 11 ms | 9 ms | 7 ms |
| reject | 60% | yes | 32/40 (80%) | 40/40 (100%) | 0 | 0 | never | 21 ms | 11 ms | 11 ms |
| reject | 100% | yes | 0/40 (0%) | 40/40 (100%) | 0 | 0 | 21 | 9 ms | 11 ms | 9 ms |
| unavailable | 30% | yes | 37/40 (92.5%) | 40/40 (100%) | 0 | 0 | never | 20 ms | 10 ms | 9 ms |
| unavailable | 100% | yes | 0/40 (0%) | 40/40 (100%) | 0 | 0 | 21 | 12 ms | 21 ms | 13 ms |
| unavailable | 100% | no | 0/40 (0%) | 39/40 (97.5%) | 40 | 1 | 2 | 18 ms | 9 ms | 13 ms |
| hang | 30% | yes | 40/40 (100%) | 40/40 (100%) | 0 | 0 | never | 1039 ms | 39 ms | 14 ms |
| hang | 100% | yes | 0/40 (0%) | 40/40 (100%) | 0 | 0 | 21 | 1027 ms | 3055 ms | 1017 ms |
| hang | 100% | no | 0/40 (0%) | 39/40 (97.5%) | 40 | 1 | 2 | 1021 ms | 1014 ms | 4 ms |

### 2. Double-charge check

| Measure | Value |
|---|---|
| Requests sent | 1143 |
| Attempt requests | 1023 |
| Refused with 409 | 697 |
| Same-key replays | 240 |
| Webhook deliveries (incl. duplicates) | 53 |
| Intents with attempts | 120 |
| Intents paid at the (simulated) provider | 92 |
| Intents with more than one captured attempt (platform records) | 0 |
| Intents paid more than once (provider records) | 0 |
| Paid at provider but no captured attempt in platform | 0 |
| Unexpected observations | 0 |
| Pass | true |

### 3. Reconciliation (8 layouts, 60 random settlements each, plus twins and noise)

| Measure | Value |
|---|---|
| Credit lines that have a true settlement | 467 |
| Confirmed automatically and correct | 311 |
| Confirmed automatically and WRONG | 0 |
| Suggested for a person (true settlement among suggestions) | 156 |
| Suggested, true settlement missing from the suggestions | 0 |
| Left unmatched although a true settlement exists | 0 |
| Confirmed-correct share of all lines with a true settlement | 66.6% |
| Confirmed-correct share where a readable reference is present | 100% |
| Confirmed or suggested share | 100% |
| Missing credits injected / flagged | 45 / 45 |
| Short credits injected / amount-mismatch cases | 41 / 41 |
| Rows added when the same file is uploaded again | 0 |

By kind of line (all layouts):

| Kind of line | Lines | confirmed_correct | confirmed_WRONG | suggested_incl_truth | suggested_without_truth | suggested_no_truth_exists | unmatched | unmatched_correctly |
|---|---|---|---|---|---|---|---|---|
| near_miss_reference | 48 | 0 | 0 | 0 | 0 | 0 | 0 | 48 |
| unrelated_credit | 64 | 0 | 0 | 0 | 0 | 0 | 0 | 64 |
| degraded_reference | 45 | 10 | 0 | 35 | 0 | 0 | 0 | 0 |
| exact_ref_column | 126 | 126 | 0 | 0 | 0 | 0 | 0 | 0 |
| ref_in_narration | 134 | 134 | 0 | 0 | 0 | 0 | 0 | 0 |
| duplicated_row | 41 | 0 | 0 | 41 | 0 | 0 | 0 | 0 |
| duplicate_of_a_real_credit | 41 | 0 | 0 | 0 | 0 | 41 | 0 | 0 |
| no_reference_exact_amount | 48 | 0 | 0 | 48 | 0 | 0 | 0 | 0 |
| short_credit | 41 | 41 | 0 | 0 | 0 | 0 | 0 | 0 |
| twin_no_reference | 32 | 0 | 0 | 32 | 0 | 0 | 0 | 0 |

### 4. Fee-variance detection (200 payments, 49 with an injected error)

| Tolerance (paise) | Should flag | True pos. | False pos. | False neg. | Precision | Recall | Over found | Under found | Expected fee differs from benchmark's |
|---|---|---|---|---|---|---|---|---|---|
| 0 | 49 | 49 | 0 | 0 | 100% | 100% | 25 of 25 | 24 of 24 | 0 |
| 2 | 42 | 42 | 0 | 0 | 100% | 100% | 22 of 22 | 20 of 20 | 0 |
| 10 | 25 | 25 | 0 | 0 | 100% | 100% | 10 of 10 | 15 of 15 | 0 |

### 5. Developer time to first payment (local)

| Stage | Seconds |
|---|---|
| clone | 0.2 |
| build | 83.6 |
| database migrations and services start | 1.5 |
| tenant key and provider setup | 0.1 |
| intent attempt payer approves captured | 0 |
| TOTAL to first captured payment | 85.4 |
| Total excluding compile | 1.8000000000000114 |

### 5b. Checkout page weight (local)

| Measure | Value |
|---|---|
| curl: HTML + static files, transferred | 187.9 KiB |
| curl: decoded | 617.4 KiB |
| curl: JS / CSS / font / HTML transferred | 182.9 KiB / 1.9 KiB / 0.0 KiB / 3.0 KiB |
| browser: all requests, transferred | 154.1 KiB in 10 requests |
| browser: load event, no throttling | 65 ms |
| browser: load event, throttled (fast-3g-like) | 2043 ms |

How to read section 1: "Baseline" and "Vayora" are the two arms described above. "Payments until routed away" is how many
payments of the outage window had passed before a payment's first attempt already went to the alternate provider (the breaker
having opened); "never" means it did not open in 40 payments. "First capture on alternate" is the time from switching the
fault on to the first payment captured through the other provider, which in the `hang` rows is one provider timeout (1 s by
configuration). Latencies are per payment, from creating the intent to captured or giving up. In section 5 the cold build took 83.6 s in this run
and 67.9 s and 57.5 s in two earlier runs of the same code: it depends on what else the machine is doing (other builds were
running here), so quote it as a range, "about one to one and a half minutes on this laptop", and not as a figure.

## Findings worth reading

These are observations from the run above, not marketing. Some are limits of the product.

1. **Failover works per payment, the breaker works per window, and they do different jobs.** With the primary
   rejecting everything, every payment succeeded in the Vayora arm because each payment fails over to the alternate on its
   retry. The breaker opened only after about 20 failed payments in the 100% cells, because its window still held the 20
   healthy payments from before the outage and it needs at least half of the window to fail. Until it opened, each new
   payment spent one wasted attempt at the primary. In the 30% and 60% brown-outs the breaker never opened (the failure
   rate in the window never reached 50%); success came entirely from per-payment failover, at the price of extra attempts.
2. **A payment in flight when a provider stops answering stays `unknown`, by design.** When the lookup also fails
   (the "no" rows) the Vayora arm still ends one payment `unknown` (the first one) and then routes everything else to the
   alternate after a single unresolved payment, whereas the baseline ends all of them `unknown`. That is the
   "unknown payment blocks retries" rule doing what it is for, and it is also why success is not 100% there.
3. **`GET /v1/routing/health` and the routing decision can disagree.** In the "no lookup" rows the primary was already being
   skipped after one unresolved payment (the older unresolved-payment heuristic), yet the breaker state reported at the end
   was `healthy`. This is the known inconsistency listed in the routing plan; the benchmark makes it visible.
4. **Duplicate rows make both copies unresolved.** A bank row that appears twice with the same reference is never
   confirmed to the settlement (`reference_ambiguous`): both rows become suggestions for a person. That is the safe
   outcome (no wrong confirmation, no double count) but it costs the genuine credit its automatic match.
5. **The matcher is more forgiving than its documentation about references split by a space.** A reference
   broken by one space inside the narration (`NEFT CR-ICIC4 3927022424-ACME`) was confirmed to the right settlement in
   a share of the "degraded reference" rows (see the table); cut-off and masked references were only ever suggested through
   amount and date. The wrong-confirmation count stayed at the figure in the table, but this joining of adjacent tokens is not described in
   BANK_RECONCILIATION.md and has not been tested against adversarial narrations beyond the near-miss rows here.
6. **Fee detection is arithmetic, and the score only says the arithmetic agrees.** Precision and recall of 100% mean the
   platform computed the same expected fee as an independent implementation of the documented formula (including half-up
   rounding, a cap and GST) and flagged exactly the injected differences. It says nothing about whether the rule a
   merchant types in matches their real contract, or how a real provider rounds.
7. **Checkout weight depends on how it is counted.** curl sums every static file the HTML references (including the
   fallback bundle for browsers without module support); the browser only fetches what it uses, so it transfers less.
   Both are listed. Load time under the throttle is dominated by the round trips, not by the bytes.
8. **The harness paces itself.** The API limits requests per client address (300 per second sustained for authenticated
   calls) and has no setting for it, so the load generator keeps under 250 requests per second rather than changing the
   product. The run records how many requests were rate limited and retried (see `requests_that_were_rate_limited_and_retried`
   in the JSON; it was 0 in the reference run).

9. **Slow tails in the `hang` cells.** In the 100% `hang` cell the baseline arm's slowest payment took 10.0 s in the reference
   run (7.1 s in an earlier run), where three 1 s provider timeouts predict about 3.1 s. The cause was not investigated (it
   could be the API, the mock holding 30 s sleeps, or the machine) and it did not show in the Vayora arm of the same run
   beyond 2.0 s. Do not read anything into a single tail value.

## Reproducibility and tolerances

The harness is deterministic where it can be and states a tolerance where it cannot. `bench/check-repro.mjs` compares two
`results.json` files number by number and fails when any difference exceeds the rule for that number:

| Kind of number | Where | Rule |
|---|---|---|
| exact | reconciliation, fee-variance, the double-charge counts that must be zero, all text | identical |
| outage | scenario 1 shares and counts | percentages within 5 points, counts within 3 payments |
| volume | scenario 2 request counters and intents paid | within 10% (floor 8): parallel races vary the number of refusals |
| timing | anything in ms or seconds, scenario 5 stages | within 50% (floor 250 ms, or 2 s for seconds) |
| tail | p95 and max latency | within 100% (floor 250 ms): one slow request moves them |
| bytes | checkout weight | within 2% |

Run `bench/ci.sh` to run the suite twice back to back and check. The two consecutive full runs behind this page
agreed within these tolerances. **That is after the tolerances were tightened by experience, which you should know:** the
first pair of full runs failed the check on a baseline latency tail in the `hang` cell (p95 3.1 s in one run, 6.9 s in the
other) and on the number of intents paid in the chaos phase of scenario 2 (1075 vs 1077; parallel timing changes it). The
second pair failed on the healthy control cell's baseline tail (p95 411 ms in one run, 9 ms in the other; the slowest request
1.4 s, probably a cold start). The "tail" and "volume" rules above were written after seeing those, and the audit counts
were then scoped to each scenario's own tenants. The numbers that must be zero and the deterministic scenarios were identical
in every run. Treat latency tails in particular as noisy.

`bench/ci.sh` also applies hard gates, which are about correctness of the simulator runs and not about performance:
zero double charges in the platform's records **and** the provider's, no unexpected observation in scenario 2, auto-confirmed
wrong reconciliation matches no more than `BENCH_MAX_WRONG_MATCHES` (default 0), every layout imports, and fee-variance
precision and recall at least `BENCH_MIN_FEE_PCT` (default 100). It is **not** wired into `.github/workflows`: add a job that
calls `bench/ci.sh` (a nightly schedule is the intent).

## What this does not show

- **Anything about a real provider.** Failure shapes, latency, how a real gateway answers a retry, what a real 503 or
  timeout means, issuer and UPI-handle effects, soft declines and settlement behaviour are all invented here. The mock
  fails exactly when and how it is told to.
- **Anything about real success rates.** No traffic, no real payers, no payer declines, no mix of methods or banks.
  "100% captured" under a simulated outage is a statement about routing logic, not about the world.
- **Vayora's cost of a failover in real life.** A real provider that times out may have taken the money; the platform's
  rule (never open a second attempt until the first is resolved) holds here, but how often real providers resolve quickly
  is not measured.
- **Real bank statements.** The eight layouts are written from the headings those banks' downloads are commonly shown with
  and were never compared with a real export. The injected errors are the ones we thought of; real narrations are messier.
  A match rate here is not a match rate on your statements. Run a preview on a real file first.
- **Real fee schedules.** Rates are invented; the check proves the arithmetic, not the contract.
- **Scale and concurrency limits.** A few hundred requests per second at most, one machine, one database, one API replica,
  sequential payments in scenario 1. It is not a load test.
- **Real developer experience.** "Time to first payment" is a scripted run by a machine that already has Rust and the
  crates cached, a running Postgres and no documentation to read. It says how long the code takes to build and run, not
  how long a person takes to integrate.
- **Real page performance.** Checkout weight is for a local production build with the API on the same machine; load times use
  a throttle preset, not a real phone on a real network.
- **Comparisons.** There is no comparison with any other product or provider, and none should be inferred.

## Files

- `bench/run.sh`, `bench/ci.sh`, `bench/devtime.sh`, `bench/check-repro.mjs`, `bench/report.mjs`
- `bench/scenarios/s1_outage.mjs` to `s4_fees.mjs`, `bench/lib/*` (helpers, bank layout writers, the stack launcher)
- `bench/results/reference/`: the run quoted here (`results.json`, `results.md`, per-scenario JSON, `machine.json`)
- Mock changes: `crates/zyvor-provider-mock` (`fail_pct` and `seed` on `/_sim/outage`, `razorpay_captured_payments_by_order` on
  `/_sim/stats`), documented in [ROUTING.md](ROUTING.md).
