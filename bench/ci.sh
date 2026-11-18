#!/usr/bin/env bash
# CI entry point (not wired into .github/workflows: add a job that calls this script).
#   bench/ci.sh [--quick]
# Runs the whole suite twice, checks the two runs agree within the documented tolerances, applies hard gates, and writes a
# markdown summary (to $GITHUB_STEP_SUMMARY when set). Exit code 1 on any failure.
# Gates (override with env): double charges must be 0 in both the platform and provider records; auto-confirmed wrong
# reconciliation matches <= BENCH_MAX_WRONG_MATCHES (default 0); fee-variance precision and recall >= BENCH_MIN_FEE_PCT (default 100).
# These gates are about correctness of the simulator runs, NOT performance claims.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARGS=("$@")
BASE="${BENCH_CI_OUT:-$ROOT/bench/results/ci-$(date +%Y%m%d-%H%M%S)}"
"$ROOT/bench/run.sh" --out "$BASE/run1" ${ARGS[@]+"${ARGS[@]}"} >/dev/null
"$ROOT/bench/run.sh" --out "$BASE/run2" ${ARGS[@]+"${ARGS[@]}"} >/dev/null
node "$ROOT/bench/check-repro.mjs" "$BASE/run1/results.json" "$BASE/run2/results.json"
node - "$BASE/run1/results.json" <<'JS'
const r = JSON.parse(require("fs").readFileSync(process.argv[2], "utf8"));
const maxWrong = Number(process.env.BENCH_MAX_WRONG_MATCHES ?? 0), minFee = Number(process.env.BENCH_MIN_FEE_PCT ?? 100);
const bad = [];
const a = r.s2_double_charge?.audit;
if (!a || a.intents_with_more_than_one_captured_attempt_in_platform !== 0 || a.intents_paid_more_than_once_at_provider !== 0) bad.push("double charge audit not zero");
if (!r.s2_double_charge?.pass) bad.push("double-charge scenario reported unexpected observations");
if ((r.s3_recon?.totals.auto_confirmed_wrong ?? 1) > maxWrong) bad.push(`auto-confirmed wrong matches ${r.s3_recon?.totals.auto_confirmed_wrong} > ${maxWrong}`);
if (r.s3_recon?.totals.layouts_that_failed_to_import) bad.push("a bank layout failed to import");
for (const x of r.s4_fees?.results ?? []) if (x.precision_pct < minFee || x.recall_pct < minFee) bad.push(`fee variance at tolerance ${x.tolerance_minor}: precision ${x.precision_pct}% recall ${x.recall_pct}%`);
if (bad.length) { console.error("BENCHMARK GATES FAILED:\n  " + bad.join("\n  ")); process.exit(1); }
console.log("benchmark gates passed");
JS
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$BASE/run1/results.md" >>"$GITHUB_STEP_SUMMARY"; fi
echo "reports in $BASE"
