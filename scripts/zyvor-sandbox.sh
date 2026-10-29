#!/usr/bin/env bash
# Local SANDBOX without Docker: migrates a fresh database, starts the API (as a restricted non-superuser role),
# worker and the SIMULATED provider mock, connects both providers and prints a ready-to-open checkout URL.
#
#   OWNER_URL=postgres://you@localhost:5432/zyvor_sandbox ./scripts/zyvor-sandbox.sh
#   (the database must exist; the script creates the restricted role `zyvor_app` if missing)
set -euo pipefail
cd "$(dirname "$0")/.."
: "${OWNER_URL:?set OWNER_URL to a superuser/owner connection string for an EMPTY database}"
APP_PASS="${APP_PASS:-sandbox-app-secret}"
APP_URL="$(echo "$OWNER_URL" | sed -E "s#//[^@/]*@#//zyvor_app:${APP_PASS}@#")"
export ZYVOR_MASTER_KEY="${ZYVOR_MASTER_KEY:-$(printf 'ab%.0s' $(seq 1 32))}"   # sandbox-only key
OUT="${OUT:-/tmp/zyvor-sandbox}"; mkdir -p "$OUT"
cargo build -q -p zyvor-api -p zyvor-provider-mock

psql "$OWNER_URL" -q -c "DO \$\$ BEGIN CREATE ROLE zyvor_app LOGIN PASSWORD '${APP_PASS}'; EXCEPTION WHEN duplicate_object THEN ALTER ROLE zyvor_app LOGIN PASSWORD '${APP_PASS}'; END \$\$;"
ZYVOR_MIGRATE_DATABASE_URL="$OWNER_URL" DATABASE_URL="$APP_URL" ./target/debug/zyvor-api migrate
psql "$OWNER_URL" -q -c "GRANT USAGE ON SCHEMA public TO zyvor_app; GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO zyvor_app; GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO zyvor_app;"

pkill -f target/debug/zyvor-provider-mock 2>/dev/null || true; pkill -f 'target/debug/zyvor-api' 2>/dev/null || true; pkill -f target/debug/zyvor-worker 2>/dev/null || true
PORT=9090 MOCK_CASHFREE_WEBHOOK_SECRET=cf_hook MOCK_RAZORPAY_WEBHOOK_SECRET=rz_hook nohup ./target/debug/zyvor-provider-mock >"$OUT/mock.log" 2>&1 &
DATABASE_URL="$APP_URL" ZYVOR_PROVIDER_TIMEOUT_SECS=2 ZYVOR_BIND=127.0.0.1:8080 nohup ./target/debug/zyvor-api >"$OUT/api.log" 2>&1 &
DATABASE_URL="$APP_URL" nohup ./target/debug/zyvor-worker >"$OUT/worker.log" 2>&1 &
for _ in $(seq 1 30); do curl -fs localhost:8080/readyz >/dev/null && break; sleep 0.5; done

B="$(DATABASE_URL="$APP_URL" ./target/debug/zyvor-api bootstrap "Sandbox Campus" 2>/dev/null)"
KEY="$(echo "$B" | grep -A1 'test tenant' | tail -1 | awk '{print $2}')"; TEN="$(echo "$B" | grep 'test tenant' | awk '{print $3}')"
A="Authorization: Bearer $KEY"; J="content-type: application/json"
curl -fs localhost:8080/v1/provider_accounts -H "$A" -H "$J" -d '{"provider":"cashfree","credentials":{"client_id":"cf","client_secret":"cfs"},"webhook_secret":"cf_hook","base_url":"http://127.0.0.1:9090"}' >/dev/null
curl -fs localhost:8080/v1/provider_accounts -H "$A" -H "$J" -d '{"provider":"razorpay","credentials":{"key_id":"rzp","key_secret":"rzs"},"webhook_secret":"rz_hook","base_url":"http://127.0.0.1:9090"}' >/dev/null
I="$(curl -fs localhost:8080/v1/payment_intents -H "$A" -H "$J" -H "Idempotency-Key: sandbox-$(date +%s)" -d '{"amount_minor":249900,"currency":"INR","allowed_providers":["cashfree"],"description":"Term 2 tuition fee","external_ref":"FEE-STU-2231-T2"}')"
IID="$(echo "$I" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')"; SEC="$(echo "$I" | python3 -c 'import sys,json;print(json.load(sys.stdin)["client_secret"])')"
echo "$KEY" >"$OUT/key"; echo "$TEN" >"$OUT/tenant"; echo "$IID" >"$OUT/intent"
cat <<MSG

SANDBOX READY (simulated providers: no real money moves)
  API            http://localhost:8080      key: $KEY
  tenant         $TEN
  checkout page  http://localhost:3002/pay/$TEN/$IID?s=$SEC
                 (start it with: ZYVOR_API_URL=http://localhost:8080 pnpm --filter @zyvor/checkout dev)
  payer approves: curl -s -X POST localhost:9090/_sim/cashfree/pay -H 'content-type: application/json' -d '{"order_id":"<attempt id>","outcome":"SUCCESS"}'
                  then POST the returned webhook body/headers to /v1/webhooks/cashfree/$TEN
MSG
