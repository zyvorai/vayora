#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

# One command for every benchmark. SIMULATOR ONLY: no real provider, no real money, no network access beyond localhost.
#   bench/run.sh [--out DIR] [--quick] [--skip-devtime] [--only s1_outage,s3_recon,...]   (--only skips scenario 5)
# Writes DIR/results.json (machine readable) and DIR/results.md (tables), plus one JSON file per scenario.
# Needs: cargo, node 22+, psql and a local Postgres you can create databases in (BENCH_PG_USER/HOST/PORT), curl, git;
# pnpm for the checkout weight (optional) and Chrome for the browser measurement (optional).
# It uses private ports (BENCH_API_PORT 18080, BENCH_MOCK_PORT 19090), a private database and role, and stops only the
# processes it started. Seeds are fixed (BENCH_SEED, default 20261118).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
OUT="" ; QUICK=0 ; DEVTIME=1 ; ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT="$2"; shift 2 ;;
    --quick) QUICK=1; shift ;;
    --skip-devtime) DEVTIME=0; shift ;;
    --only) ONLY="${2//,/ }"; shift 2 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
OUT="${OUT:-$ROOT/bench/results/run-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$OUT"
export BENCH_QUICK="$QUICK" BENCH_SEED="${BENCH_SEED:-20261118}"

if [ -z "${BENCH_BIN_DIR:-}" ]; then
  cargo build --release --locked -q -p zyvor-api -p zyvor-provider-mock
  BIN="${CARGO_TARGET_DIR:-$ROOT/target}/release"
else
  BIN="$BENCH_BIN_DIR"
fi

. "$ROOT/bench/lib/stack.sh"
trap stack_down EXIT
stack_up "$BIN"
node bench/lib/machine.mjs "$OUT/machine.json"
node bench/run-scenarios.mjs $ONLY --out "$OUT"
stack_down
trap - EXIT

if [ "$DEVTIME" = 1 ] && [ -z "$ONLY" ]; then
  echo "== s5 developer time to first payment and checkout weight" >&2
  bash bench/devtime.sh "$OUT"
fi
node bench/report.mjs "$OUT" >/dev/null
echo "results: $OUT/results.json and $OUT/results.md" >&2
cat "$OUT/results.md"
