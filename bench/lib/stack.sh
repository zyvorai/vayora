#!/usr/bin/env bash
# Starts a private Postgres database, the Zyvor API and the SIMULATED provider mock on ports of its own, and tears down
# ONLY what it started. Source it:   . bench/lib/stack.sh; stack_up <bin_dir>; ...; (stack_down runs on exit)
#
# Environment (all optional): BENCH_PG_USER, BENCH_PG_HOST, BENCH_PG_PORT, BENCH_API_PORT, BENCH_MOCK_PORT, BENCH_TMP.
# It never uses pkill: it kills the process ids it recorded, nothing else.

STACK_PIDS=()
STACK_DB=""
STACK_ROLE=""
STACK_OWNER_URL=""
STACK_OWN_TMP=""

stack_up() {
  local bin="$1"
  local user="${BENCH_PG_USER:-$(id -un)}" host="${BENCH_PG_HOST:-127.0.0.1}" port="${BENCH_PG_PORT:-5432}"
  local api_port="${BENCH_API_PORT:-18080}" mock_port="${BENCH_MOCK_PORT:-19090}"
  local tmp
  if [ -n "${BENCH_TMP:-}" ]; then tmp="$BENCH_TMP"; mkdir -p "$tmp"; else tmp="$(mktemp -d "${TMPDIR:-/tmp}/vayora-bench.XXXXXX")"; STACK_OWN_TMP="$tmp"; fi
  STACK_TMP="$tmp"
  STACK_DB="vayora_bench_$$_$RANDOM"
  STACK_ROLE="bench_app_$$"
  local admin_url="postgres://$user@$host:$port/postgres"
  STACK_OWNER_URL="postgres://$user@$host:$port/$STACK_DB"
  local app_pass="bench-local-only"
  local app_url="postgres://$STACK_ROLE:$app_pass@$host:$port/$STACK_DB"

  for p in "$api_port" "$mock_port"; do
    if (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null; then
      echo "port $p is already in use; set BENCH_API_PORT / BENCH_MOCK_PORT" >&2; return 1
    fi
  done

  psql "$admin_url" -X -q -c "CREATE DATABASE \"$STACK_DB\"" >/dev/null
  psql "$admin_url" -X -q -c "CREATE ROLE \"$STACK_ROLE\" LOGIN PASSWORD '$app_pass'" >/dev/null
  export ZYVOR_MASTER_KEY="${ZYVOR_MASTER_KEY:-$(printf 'ab%.0s' $(seq 1 32))}"   # a throwaway local key, not a secret
  ZYVOR_MIGRATE_DATABASE_URL="$STACK_OWNER_URL" DATABASE_URL="$app_url" "$bin/zyvor-api" migrate >"$tmp/migrate.log" 2>&1
  psql "$STACK_OWNER_URL" -X -q -c "GRANT USAGE ON SCHEMA public TO \"$STACK_ROLE\"; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO \"$STACK_ROLE\"; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO \"$STACK_ROLE\";" >/dev/null

  PORT="$mock_port" MOCK_CASHFREE_WEBHOOK_SECRET=cf_hook MOCK_RAZORPAY_WEBHOOK_SECRET=rz_hook \
    "$bin/zyvor-provider-mock" >"$tmp/mock.log" 2>&1 &
  STACK_PIDS+=($!)
  DATABASE_URL="$app_url" ZYVOR_BIND="127.0.0.1:$api_port" ZYVOR_PROVIDER_TIMEOUT_SECS=1 ZYVOR_ALLOW_PRIVATE_TARGETS=1 \
    ZYVOR_RATE_KEY_PER_SEC=100000 ZYVOR_RATE_KEY_BURST=100000 ZYVOR_RATE_PUBLIC_PER_SEC=100000 ZYVOR_RATE_PUBLIC_BURST=100000 \
    "$bin/zyvor-api" >"$tmp/api.log" 2>&1 &
  STACK_PIDS+=($!)
  local i
  for i in $(seq 1 60); do
    curl -fs "http://127.0.0.1:$api_port/readyz" >/dev/null 2>&1 && break
    sleep 0.5
  done
  curl -fs "http://127.0.0.1:$api_port/readyz" >/dev/null || { echo "API did not become ready; see $tmp/api.log" >&2; return 1; }
  for i in $(seq 1 60); do
    curl -fs "http://127.0.0.1:$mock_port/_sim/stats" >/dev/null 2>&1 && break
    sleep 0.25
  done
  curl -fs "http://127.0.0.1:$mock_port/_sim/stats" >/dev/null || { echo "mock did not become ready; see $tmp/mock.log" >&2; return 1; }
  export BENCH_API="http://127.0.0.1:$api_port" BENCH_MOCK="http://127.0.0.1:$mock_port"
  export BENCH_OWNER_URL="$STACK_OWNER_URL" BENCH_APP_URL="$app_url" BENCH_BIN_DIR="$bin"
}

stack_down() {
  local pid
  for pid in "${STACK_PIDS[@]:-}"; do
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  done
  for pid in "${STACK_PIDS[@]:-}"; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null || true
  done
  STACK_PIDS=()
  if [ -n "$STACK_DB" ]; then
    local user="${BENCH_PG_USER:-$(id -un)}" host="${BENCH_PG_HOST:-127.0.0.1}" port="${BENCH_PG_PORT:-5432}"
    local admin_url="postgres://$user@$host:$port/postgres"
    psql "$admin_url" -X -q -c "DROP DATABASE IF EXISTS \"$STACK_DB\" WITH (FORCE)" >/dev/null 2>&1 || true
    psql "$admin_url" -X -q -c "DROP ROLE IF EXISTS \"$STACK_ROLE\"" >/dev/null 2>&1 || true
    STACK_DB=""
  fi
  if [ -n "$STACK_OWN_TMP" ]; then rm -rf "$STACK_OWN_TMP"; STACK_OWN_TMP=""; fi
}
