#!/usr/bin/env bash
# Bring up deploy/zyvor/docker-compose.yml, wait on /readyz and Envoy, bootstrap a test org.
# Requires Docker. Tears the stack down on exit unless KEEP=1.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
compose=(docker compose -f "$root/deploy/zyvor/docker-compose.yml")
export ZYVOR_MASTER_KEY="${ZYVOR_MASTER_KEY:-$(openssl rand -hex 32)}"

cleanup() {
  if [[ "${KEEP:-}" == "1" ]]; then
    echo "KEEP=1: leaving stack up (ZYVOR_MASTER_KEY=$ZYVOR_MASTER_KEY)"
    return
  fi
  "${compose[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> certs"
"$root/deploy/zyvor/envoy/gen-certs.sh"

echo "==> compose up"
"${compose[@]}" up -d --build

echo "==> wait for API /readyz via Envoy"
ok=0
for _ in $(seq 1 90); do
  if curl -fsSk https://localhost:8443/readyz >/dev/null 2>&1; then
    ok=1
    break
  fi
  sleep 2
done
if [[ "$ok" != "1" ]]; then
  echo "API did not become ready" >&2
  "${compose[@]}" logs --tail=80 api migrate grants || true
  exit 1
fi

echo "==> bootstrap"
out="$("${compose[@]}" run --rm api zyvor-api bootstrap "Compose Smoke" 2>&1)"
echo "$out"
key="$(echo "$out" | grep -Eo 'zk_test_[A-Za-z0-9]+' | head -n1 || true)"
if [[ -z "$key" ]]; then
  echo "bootstrap did not print a test key" >&2
  exit 1
fi

echo "==> signed call"
curl -fsSk -H "Authorization: Bearer $key" -H "Content-Type: application/json" \
  https://localhost:8443/v1/me | grep -q tenant_id

echo "compose smoke ok"
