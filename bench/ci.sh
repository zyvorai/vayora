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
node "$ROOT/bench/gates.mjs" "$BASE/run1/results.json"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$BASE/run1/results.md" >>"$GITHUB_STEP_SUMMARY"; fi
echo "reports in $BASE"
