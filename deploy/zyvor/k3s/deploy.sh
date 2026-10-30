#!/usr/bin/env bash
# Deploy the Zyvor SANDBOX to a k3s node you can ssh into (needs podman and passwordless sudo for `k3s kubectl`).
#
#   CONSOLE_PASSWORD='...' ./deploy.sh user@host            # CONSOLE_USERNAME defaults to admin
#
# Builds the images on the node (so architecture always matches), imports them into k3s's containerd, and
# creates every secret on the node. The console password is read from your environment and sent over ssh
# stdin: it is never written to this repository or put on a command line.
set -euo pipefail
HOST="${1:?usage: CONSOLE_PASSWORD=... deploy.sh user@host}"
: "${CONSOLE_PASSWORD:?set CONSOLE_PASSWORD}"
USERNAME="${CONSOLE_USERNAME:-admin}"
cd "$(dirname "$0")/../../.."
K="sudo -n k3s kubectl"
R() { ssh -o BatchMode=yes -o ServerAliveInterval=20 -o ServerAliveCountMax=30 "$HOST" "$@"; }

echo "==> uploading source"
R 'rm -rf ~/zyvor-src && mkdir -p ~/zyvor-src'
tar czf - --exclude=target --exclude=node_modules --exclude=.next --exclude=.git --exclude=dist --exclude='.env*' --exclude='*.tsbuildinfo' . | R 'tar xzf - -C ~/zyvor-src'

echo "==> building images on the node (the Rust build takes a few minutes the first time)"
R 'cd ~/zyvor-src && podman build -q -t localhost/zyvor-api:dev -f deploy/zyvor/Dockerfile . \
 && podman build -q -t localhost/zyvor-checkout:dev --build-arg ZYVOR_API_URL=http://zyvor-api:8080 -f deploy/zyvor/Dockerfile.checkout . \
 && podman build -q -t localhost/zyvor-console:dev -f deploy/zyvor/Dockerfile.console .'
for img in zyvor-api zyvor-checkout zyvor-console; do
  R "podman save localhost/$img:dev | sudo -n k3s ctr images import -" >/dev/null
done

echo "==> namespace and secrets"
R "$K create namespace zyvor --dry-run=client -o yaml | $K apply -f -" >/dev/null
R "$K -n zyvor get secret zyvor-secrets >/dev/null 2>&1 || { \
  OWNER=\$(openssl rand -hex 16); APP=\$(openssl rand -hex 16); \
  $K -n zyvor create secret generic zyvor-secrets \
    --from-literal=pg-owner-password=\$OWNER --from-literal=pg-app-password=\$APP \
    --from-literal=migrate-database-url=postgres://postgres:\$OWNER@postgres:5432/zyvor \
    --from-literal=database-url=postgres://zyvor_app:\$APP@postgres:5432/zyvor \
    --from-literal=master-key=\$(openssl rand -hex 32); }"

echo "==> applying manifest"
R "$K delete job zyvor-migrate -n zyvor --ignore-not-found" >/dev/null
R "$K apply -f -" < deploy/zyvor/k3s/zyvor.yaml >/dev/null
R "$K -n zyvor rollout status statefulset/postgres --timeout=240s"
R "$K -n zyvor wait --for=condition=complete job/zyvor-migrate --timeout=240s"
R "$K -n zyvor rollout restart deploy/zyvor-api deploy/zyvor-worker deploy/zyvor-checkout >/dev/null; $K -n zyvor rollout status deploy/zyvor-api --timeout=240s"

echo "==> sandbox tenant and simulated providers"
if ! R "$K -n zyvor get secret zyvor-console >/dev/null 2>&1"; then
  OUT="$(R "$K -n zyvor exec deploy/zyvor-api -- zyvor-api bootstrap 'Sandbox Org' 2>/dev/null")"
  KEY="$(echo "$OUT" | grep -A1 'test tenant' | tail -1 | awk '{print $2}')"
  [ -n "$KEY" ] || { echo "bootstrap failed"; exit 1; }
  for p in 'cashfree|{"client_id":"cf","client_secret":"cfs"}|cf_hook' 'razorpay|{"key_id":"rzp","key_secret":"rzs"}|rz_hook'; do
    IFS='|' read -r name creds hook <<<"$p"
    R "$K -n zyvor exec deploy/zyvor-api -- curl -fsS localhost:8080/v1/provider_accounts -H 'Authorization: Bearer $KEY' -H 'content-type: application/json' -d '{\"provider\":\"$name\",\"credentials\":$creds,\"webhook_secret\":\"$hook\",\"base_url\":\"http://provider-mock:9090\"}'" >/dev/null
  done
  printf %s "$CONSOLE_PASSWORD" | R "$K -n zyvor create secret generic zyvor-console \
    --from-literal=api-key=$KEY --from-literal=username=$USERNAME \
    --from-literal=session-secret=\$(openssl rand -hex 32) --from-file=password=/dev/stdin" >/dev/null
fi
R "$K -n zyvor rollout restart deploy/zyvor-console >/dev/null; $K -n zyvor rollout status deploy/zyvor-console --timeout=240s; $K -n zyvor rollout status deploy/zyvor-checkout --timeout=240s"
echo "==> up. Console: http://${HOST#*@}:30303   Checkout: http://${HOST#*@}:30302"
