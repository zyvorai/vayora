# Benchmark results (simulator only)

Machine: Apple M6, 12 logical cores, 16 GB, darwin 27.3.0 arm64; PostgreSQL 14.24 (Homebrew); rustc 1.100.0 (0ef1bc386 2026-11-09); node v26.11.1; commit 52b0ea1.

## 1. Success during an outage or brown-out (40 payments per cell after 20 healthy ones)

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

## 2. Double-charge check

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

## 3. Reconciliation (8 layouts, 60 random settlements each, plus twins and noise)

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

## 4. Fee-variance detection (200 payments, 49 with an injected error)

| Tolerance (paise) | Should flag | True pos. | False pos. | False neg. | Precision | Recall | Over found | Under found | Expected fee differs from benchmark's |
|---|---|---|---|---|---|---|---|---|---|
| 0 | 49 | 49 | 0 | 0 | 100% | 100% | 25 of 25 | 24 of 24 | 0 |
| 2 | 42 | 42 | 0 | 0 | 100% | 100% | 22 of 22 | 20 of 20 | 0 |
| 10 | 25 | 25 | 0 | 0 | 100% | 100% | 10 of 10 | 15 of 15 | 0 |

## 5. Developer time to first payment (local)

| Stage | Seconds |
|---|---|
| clone | 0.2 |
| build | 83.6 |
| database migrations and services start | 1.5 |
| tenant key and provider setup | 0.1 |
| intent attempt payer approves captured | 0 |
| TOTAL to first captured payment | 85.4 |
| Total excluding compile | 1.8000000000000114 |

## 5b. Checkout page weight (local)

| Measure | Value |
|---|---|
| curl: HTML + static files, transferred | 187.9 KiB |
| curl: decoded | 617.4 KiB |
| curl: JS / CSS / font / HTML transferred | 182.9 KiB / 1.9 KiB / 0.0 KiB / 3.0 KiB |
| browser: all requests, transferred | 154.1 KiB in 10 requests |
| browser: load event, no throttling | 65 ms |
| browser: load event, throttled (fast-3g-like) | 2043 ms |
