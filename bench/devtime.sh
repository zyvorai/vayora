#!/usr/bin/env bash
# Copyright 2026 Zyvor AI Labs · https://zyvor.dev
# SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

# Scenario 5: developer time to first payment, from a CLEAN checkout, and checkout page weight. LOCAL numbers only.
#   bench/devtime.sh <out_dir>
# Clones the committed HEAD into a temp dir, builds the API and the simulated provider with an EMPTY target dir (a cold
# build; set BENCH_DEVTIME_BIN_DIR to reuse an existing build and skip it), starts a private database and both services,
# and times each step up to the first captured sandbox payment. Then builds and serves the checkout app and measures it.
# The crate registry is whatever is already on this machine (this does not time downloads unless cargo has to fetch).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:?usage: devtime.sh <out_dir>}"
mkdir -p "$OUT"
. "$ROOT/bench/lib/stack.sh"
export BENCH_API_PORT="${BENCH_API_PORT:-18081}" BENCH_MOCK_PORT="${BENCH_MOCK_PORT:-19091}"
CHECKOUT_PORT="${BENCH_CHECKOUT_PORT:-13002}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/vayora-devtime.XXXXXX")"
NEXT_PID=""
cleanup() {
  [ -n "$NEXT_PID" ] && kill "$NEXT_PID" 2>/dev/null || true
  stack_down
  rm -rf "$WORK"
}
trap cleanup EXIT
now() { node -p 'Date.now()'; }

export BENCH_T_START="$(now)"
git clone -q --local --no-hardlinks "$ROOT" "$WORK/checkout"
export BENCH_T_CLONED="$(now)"
if [ -n "${BENCH_DEVTIME_BIN_DIR:-}" ]; then
  BIN="$BENCH_DEVTIME_BIN_DIR"
else
  (cd "$WORK/checkout" && CARGO_TARGET_DIR="$WORK/target" cargo build --release --locked -q -p zyvor-api -p zyvor-provider-mock)
  BIN="$WORK/target/release"
fi
export BENCH_T_BUILT="$(now)"
stack_up "$BIN"
export BENCH_T_STACK_UP="$(now)"
node "$ROOT/bench/lib/firstpayment.mjs" "$OUT/s5_first_payment.json"

# checkout app: install, production build, serve, measure (not part of the time-to-first-payment total)
if command -v pnpm >/dev/null 2>&1; then
  c0="$(now)"
  (cd "$WORK/checkout" && pnpm install --frozen-lockfile --filter @zyvor/checkout... >"$WORK/pnpm.log" 2>&1 \
     && ZYVOR_API_URL="$BENCH_API" pnpm --filter @zyvor/checkout build >>"$WORK/pnpm.log" 2>&1) || { echo "checkout build failed; see $WORK/pnpm.log" >&2; cat "$WORK/pnpm.log" | tail -20 >&2; exit 1; }
  c1="$(now)"
  (cd "$WORK/checkout/apps/zyvor-checkout" && exec ./node_modules/.bin/next start -p "$CHECKOUT_PORT" >"$WORK/next.log" 2>&1) &
  NEXT_PID=$!
  for _ in $(seq 1 60); do curl -fs "http://127.0.0.1:$CHECKOUT_PORT/" >/dev/null 2>&1 && break; sleep 0.5; done
  node "$ROOT/bench/lib/checkout_weight.mjs" "http://127.0.0.1:$CHECKOUT_PORT" "$OUT/s5_checkout_weight.json"
  node -e "const f=process.argv[1];const fs=require('fs');const j=JSON.parse(fs.readFileSync(f));j.checkout_install_and_build_seconds=Math.round(($c1-$c0)/100)/10;fs.writeFileSync(f,JSON.stringify(j,null,2)+'\n')" "$OUT/s5_checkout_weight.json"
else
  echo '{"skipped":"pnpm not installed"}' >"$OUT/s5_checkout_weight.json"
fi
