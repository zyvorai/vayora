#!/usr/bin/env bash
# Adds what the iPhone merchant app's inbox screens need to a sandbox made by zyvor-sandbox.sh:
# one refund and one payout waiting for approval, one open dispute, one reminder draft (INV-1001 must be overdue,
# as startdata-style seeding leaves it). Usage: API=http://localhost:8081 KEY=zk_test_... scripts/seed-sandbox-ios.sh
# Set APPROVER_KEY_OUT=<file> to also mint a second key (for a successful approval) and write it there.
# Needs the simulated Cashfree mock (SIM_URL, default http://localhost:9090) the sandbox script starts, and TENANT (the
# tenant id the sandbox script prints). Test environment only. Prints ids, never the key.
set -euo pipefail
API=${API:-http://localhost:8081}; KEY=${KEY:?set KEY to a test admin key}; TENANT=${TENANT:?set TENANT to the sandbox tenant id}; SIM_URL=${SIM_URL:-http://localhost:9090}
case "$KEY" in zk_test_*) ;; *) echo "refusing: not a test key" >&2; exit 1;; esac
H="Authorization: Bearer $KEY"; J='content-type: application/json'
# POSTs that create money records need an Idempotency-Key; a failure prints the server's error and stops
post() {
  local out; out=$(curl -s -w '\n%{http_code}' "$API/v1$1" -H "$H" -H "$J" -H "Idempotency-Key: seed-$RANDOM-$RANDOM-$(date +%s)" -d "$2")
  local code=${out##*$'\n'} body=${out%$'\n'*}
  case "$code" in 2*) printf '%s' "$body";; *) echo "seed step $1 failed ($code): $body" >&2; exit 1;; esac
}
jq_() { python3 -c "import sys,json;print(json.load(sys.stdin)$1)"; }

# every refund and the dunning run need a second person's yes; payouts already do by default
post /settings '{"refund_approval_threshold_minor":0,"dunning":{"enabled":true,"channel":"email","min_days_overdue":1,"repeat_after_days":7,"requires_approval":true}}' >/dev/null

# a paid payment to refund and dispute
PI=$(post /payment_intents '{"amount_minor":50000,"currency":"INR","description":"Seed sale"}' | jq_ '["id"]')
AT=$(post "/payment_intents/$PI/attempts" '{"method":"upi","upi_flow":"collect","vpa":"asha@upi"}' | jq_ '["id"]')
# the simulated payer approves at the provider mock, and its webhook is delivered to the API as a real one would be
SIM=$(curl -sf -X POST "$SIM_URL/_sim/cashfree/pay" -H "$J" -d "{\"order_id\":\"$AT\",\"outcome\":\"SUCCESS\"}")
printf '%s' "$SIM" | python3 -c '
import sys, json, urllib.request
w = json.load(sys.stdin)["webhook"]
req = urllib.request.Request(sys.argv[1], data=w["body"].encode(), headers=w["headers"], method="POST")
urllib.request.urlopen(req).read()' "$API/v1/webhooks/cashfree/$TENANT"
RF=$(post "/payment_intents/$PI/refunds" '{"amount_minor":2500,"reason":"Seed refund"}' | jq_ '["id"]')
DP=$(post /disputes "{\"attempt_id\":\"$AT\",\"amount_minor\":45000,\"reason\":\"Customer says they were charged twice\",\"respond_by\":\"$(python3 -c 'import datetime;print(datetime.date.today()+datetime.timedelta(days=2))')\"}" | jq_ '["id"]')

# a payout waiting for approval
VD=$(post /vendors '{"name":"Seed Supplier","vpa":"supplier@upi"}' | jq_ '["id"]')
PO=$(post /payouts "{\"vendor_id\":\"$VD\",\"amount_minor\":120000,\"purpose\":\"Supplier invoice 88\"}" | jq_ '["id"]')

# a reminder draft for the overdue invoice
post /collections/dunning/run '{}' >/dev/null || true
# a second person, so an approval can succeed (the person who asked cannot approve): its key goes to a file, never the screen
if [ -n "${APPROVER_KEY_OUT:-}" ]; then
  ( umask 077; post /api_keys '{"name":"ios approver","scopes":["payments:read","refunds:approve","payouts:approve","disputes:write"]}' | jq_ '["key"]' > "$APPROVER_KEY_OUT" )
  echo "approver key written to $APPROVER_KEY_OUT"
fi
echo "refund=$RF dispute=$DP payout=$PO"
