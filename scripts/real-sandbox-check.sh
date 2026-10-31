#!/usr/bin/env bash
# Walk a TEST environment through a real provider's sandbox from the command line (see docs/zyvor/REAL_SANDBOX.md).
#
#   ZYVOR_API_URL=https://api.example.com ZYVOR_API_KEY=zk_test_... scripts/real-sandbox-check.sh check
#   ... pay cashfree [amount_minor]        start a payment with that provider; prints what the payer must do
#   ... status <payment_id>                where the payment stands (attempts included)
#   ... refund <payment_id> <amount_minor> refund part or all of a captured payment
#   ... accounts                           the connected provider accounts (no secrets are ever returned)
#
# Never prints the key. Refuses a live key: this script is for test environments only.
set -euo pipefail

: "${ZYVOR_API_URL:?set ZYVOR_API_URL to the API address}"
: "${ZYVOR_API_KEY:?set ZYVOR_API_KEY to a zk_test_ admin key (Developers or API keys page)}"
case "$ZYVOR_API_KEY" in zk_test_*) ;; *) echo "refusing: this script only takes a zk_test_ key" >&2; exit 2;; esac
base="${ZYVOR_API_URL%/}"

call() { # method path [json-body] [idempotency-key]
  local m="$1" p="$2" b="${3:-}" k="${4:-}"
  local args=(-sS -X "$m" "$base$p" -H "authorization: Bearer $ZYVOR_API_KEY" -H "content-type: application/json")
  if [ -n "$k" ]; then args+=(-H "idempotency-key: $k"); fi
  if [ -n "$b" ]; then args+=(-d "$b"); fi
  curl "${args[@]}"
}
pretty() { python3 -c 'import sys,json; print(json.dumps(json.load(sys.stdin), indent=2))'; }
field() { python3 -c "import sys,json; print(json.load(sys.stdin)$1)"; }

case "${1:-}" in
  accounts) call GET /v1/provider_accounts | python3 -c '
import sys, json
for a in json.load(sys.stdin)["data"]:
    print("%-10s enabled=%-5s address=%s  webhook=%s" % (a["provider"], a["enabled"], a["base_url"] or "provider default", a["webhook_url"]))' ;;
  check)
    for p in razorpay cashfree; do
      printf '%-9s ' "$p"
      out=$(call POST "/v1/provider_accounts/$p/check" || true)
      echo "$out" | python3 -c '
import sys,json
try:
  d=json.load(sys.stdin)
  print(("OK    " if d.get("ok") else "FAIL  ")+d.get("detail", d.get("error",{}).get("message","?")))
except Exception: print("no answer (is this provider connected?)")'
    done ;;
  pay)
    provider="${2:?usage: pay <razorpay|cashfree> [amount_minor]}"; amount="${3:-1000}"
    key="rs-$(date +%s)-$RANDOM"
    id=$(call POST /v1/payment_intents "{\"amount_minor\":$amount,\"currency\":\"INR\",\"allowed_providers\":[\"$provider\"],\"external_ref\":\"real-sandbox-$key\"}" "$key" | field '["id"]')
    echo "payment: $id"
    vpa="${REAL_SANDBOX_VPA:-}"   # a test UPI id from the provider's own docs, if the account has no UPI intent
    body='{"method":"upi"}'
    if [ -n "$vpa" ]; then body="{\"method\":\"upi\",\"vpa\":\"$vpa\"}"; fi
    call POST "/v1/payment_intents/$id/attempts" "$body" "$key-a" | pretty
    echo "Complete the payment with the provider's TEST instrument, then run: $0 status $id" ;;
  status) call GET "/v1/payment_intents/${2:?usage: status <payment_id>}" | python3 -c '
import sys,json
d=json.load(sys.stdin); print("payment:", d["intent"]["status"])
for a in d["attempts"]: print(" attempt", a["provider"], a["method"], a["status"], a.get("failure_reason") or "")' ;;
  refund) call POST "/v1/payment_intents/${2:?usage: refund <payment_id> <amount_minor>}/refunds" "{\"amount_minor\":${3:?amount_minor}}" "rf-$(date +%s)-$RANDOM" | pretty ;;
  *) sed -n '2,10p' "$0"; exit 1 ;;
esac
