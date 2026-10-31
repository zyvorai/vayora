#!/usr/bin/env bash
# Builds a Zyvor SANDBOX in a FluxVM Debian VM on an Apple-silicon Mac: Postgres, release builds, systemd services
# for API/worker/simulated providers/console/checkout, and Caddy with its own CA in front (self-signed HTTPS).
#
#   VM=zyvor-sandbox ./scripts/fluxvm-lab.sh          # creates the VM if it does not exist
#   CONSOLE_PASSWORD=... ./scripts/fluxvm-lab.sh      # otherwise a random one is written to $OUT/console-password
#
# Every step here was run by hand first (see docs/zyvor/FLUXVM_LAB.md). The SCRIPT has not yet completed a full run:
# its first attempt got through packages, toolchains, source and database, then the VM stopped answering SSH during the
# Rust build. Treat it as unverified. Providers are SIMULATED. The address is private, so only this Mac reaches it.
set -euo pipefail
cd "$(dirname "$0")/.."
REPO="$PWD"
VM="${VM:-zyvor-sandbox}"
OUT="${OUT:-$HOME/.zyvor-lab/$VM}"; mkdir -p "$OUT"; chmod 700 "$OUT"
command -v fluxctl >/dev/null || { echo "fluxctl not found: brew install zyvorai/tap/fluxvm" >&2; exit 1; }

rs() { fluxctl ssh "$VM" -- "$@" 2>&1 | grep -v 'fluxvm_scheduler' || true; }
vm_ip() { fluxctl get "$VM" 2>/dev/null | grep -o '"guest_ip": "[^"]*"' | cut -d'"' -f4; }
wait_ssh() { local i; for i in $(seq 1 60); do fluxctl ssh "$VM" -- true >/dev/null 2>&1 && return 0; sleep 5; done; echo "no SSH to $VM" >&2; return 1; }

# Run a script inside the VM, detached (an SSH drop must not kill it), and wait for it to finish.
bg() {
  local name="$1" body="$2"
  # `fluxctl ssh` never sees EOF on stdin, so ship the script inside the command line instead
  local enc
  enc="$(printf 'set -e\ntrap "echo FAIL-%s" ERR\n%s\necho DONE-%s\n' "$name" "$body" "$name" | base64 | tr -d '\n')"
  rs "echo $enc | base64 -d > ~/$name.sh" >/dev/null
  rs "setsid nohup bash ~/$name.sh > ~/$name.log 2>&1 < /dev/null & sleep 1; echo started-$name" >/dev/null
  echo "==> $name"
  local out
  while true; do
    out="$(rs "grep -E '^(DONE|FAIL)-$name' ~/$name.log || true")"
    case "$out" in *DONE-*) return 0;; *FAIL-*) echo "step $name failed; last lines:" >&2; rs "tail -15 ~/$name.log" >&2; return 1;; esac
    sleep 10
  done
}

echo "==> VM $VM"
if ! fluxctl get "$VM" >/dev/null 2>&1; then
  fluxctl run --name "$VM" --keep --cpus 4 --memory-mib 8192 -v "$REPO":/mnt/src:ro -- true >/dev/null 2>&1
  fluxctl stop "$VM" >/dev/null 2>&1                       # the image's root disk is 2.8 GB; grow it while stopped
  fluxctl disk resize "$VM" root --size-gib 40 >/dev/null 2>&1
  fluxctl start "$VM" >/dev/null 2>&1
fi
wait_ssh
IP="$(vm_ip)"; [ -n "$IP" ] || { echo "VM has no address" >&2; exit 1; }
H="${IP//./-}.sslip.io"
echo "    address $IP  (hosts console.$H, pay.$H)"

bg packages '
sudo -n sed -i "s#https://#http://#g" /etc/apt/mirrors/*.list      # apt over HTTPS hangs in this image
echo "Acquire::ForceIPv4 true;" | sudo -n tee /etc/apt/apt.conf.d/99ipv4 >/dev/null
sudo -n apt-get update -qq
sudo -n DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends postgresql build-essential pkg-config python3 xz-utils caddy >/dev/null
'

bg toolchains '
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1
curl -fsSL https://nodejs.org/dist/v22.20.0/node-v22.20.0-linux-arm64.tar.xz | sudo -n tar -xJ -C /usr/local --strip-components=1
sudo -n env PATH=/usr/local/bin:$PATH npm i -g pnpm@11.25.0 >/dev/null 2>&1
'

bg source '
rm -rf ~/zyvor; mkdir -p ~/zyvor
cd /mnt/src && tar cf - --exclude=target --exclude=node_modules --exclude=.next --exclude=.git --exclude=dist . | tar xf - -C ~/zyvor
'

bg database '
sudo -n -u postgres psql -qc "DO \$\$ BEGIN CREATE ROLE mini SUPERUSER LOGIN PASSWORD '"'"'lab-owner-pass'"'"'; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$"
sudo -n -u postgres psql -tAc "select 1 from pg_database where datname = '"'"'zyvor_sandbox'"'"'" | grep -q 1 || sudo -n -u postgres createdb -O mini zyvor_sandbox
'

# Migrates, creates the restricted role, a tenant and simulated provider accounts (the repo sandbox script), then
# we replace its hand-started processes with services.
bg sandbox '
. ~/.cargo/env; cd ~/zyvor
OWNER_URL=postgres://mini:lab-owner-pass@127.0.0.1:5432/zyvor_sandbox OUT=$HOME/sbx ./scripts/zyvor-sandbox.sh >/dev/null
cargo build --release -p zyvor-api -p zyvor-provider-mock 2>&1 | tail -1
'

bg web '
export PATH=/usr/local/bin:$PATH; cd ~/zyvor
pnpm install --frozen-lockfile >/dev/null 2>&1
pnpm --filter @zyvor/node build >/dev/null 2>&1
ZYVOR_API_KEY=build CONSOLE_PASSWORD=build CONSOLE_SESSION_SECRET=build pnpm --filter @zyvor/console build >/dev/null 2>&1
ZYVOR_API_URL=http://127.0.0.1:8080 pnpm --filter @zyvor/checkout build >/dev/null 2>&1
'

PW="${CONSOLE_PASSWORD:-$(openssl rand -hex 8)}"
[ -f "$OUT/console-password" ] && [ -z "${CONSOLE_PASSWORD:-}" ] && PW="$(cat "$OUT/console-password")"
umask 077; printf '%s\n' "$PW" > "$OUT/console-password"
SS="$(openssl rand -hex 24)"; AS="$(openssl rand -hex 32)"

bg services "
H=$H
sudo mkdir -p /etc/zyvor /opt/zyvor/bin
for n in zyvor-api zyvor-worker zyvor-provider-mock; do pid=\$(pgrep -x \"\${n:0:15}\" || true); [ -n \"\$pid\" ] && kill \$pid || true; done
sudo cp ~/zyvor/target/release/zyvor-api ~/zyvor/target/release/zyvor-worker ~/zyvor/target/release/zyvor-provider-mock /opt/zyvor/bin/
APPURL=postgres://zyvor_app:sandbox-app-secret@127.0.0.1:5432/zyvor_sandbox
MK=\$(printf 'ab%.0s' \$(seq 1 32))
KEY=\$(cat ~/sbx/key)
w() { sudo tee /etc/zyvor/\$1 >/dev/null; }
printf 'DATABASE_URL=%s\nZYVOR_MASTER_KEY=%s\nZYVOR_BIND=127.0.0.1:8080\nZYVOR_PROVIDER_TIMEOUT_SECS=2\nZYVOR_ACTOR_SECRET=$AS\n' \"\$APPURL\" \"\$MK\" | w api.env
printf 'DATABASE_URL=%s\nZYVOR_MASTER_KEY=%s\nZYVOR_ALLOW_PRIVATE_TARGETS=1\n' \"\$APPURL\" \"\$MK\" | w worker.env
printf 'PORT=9090\nMOCK_RAZORPAY_WEBHOOK_SECRET=rz_hook\nMOCK_CASHFREE_WEBHOOK_SECRET=cf_hook\n' | w mock.env
printf 'ZYVOR_API_URL=http://127.0.0.1:8080\nZYVOR_API_KEY=%s\nCONSOLE_USER=console\nCONSOLE_USERNAME=admin\nCONSOLE_PASSWORD=$PW\nCONSOLE_SESSION_SECRET=$SS\nZYVOR_ACTOR_SECRET=$AS\nCHECKOUT_PUBLIC_URL=https://pay.\$H\n' \"\$KEY\" | w console.env
printf 'ZYVOR_API_URL=http://127.0.0.1:8080\n' | w checkout.env
sudo chmod 600 /etc/zyvor/*.env
unit() {
  sudo tee /etc/systemd/system/\$1.service >/dev/null <<U
[Unit]
Description=\$2
After=network.target postgresql.service
[Service]
User=mini
WorkingDirectory=\$3
EnvironmentFile=/etc/zyvor/\$5
Environment=PATH=/usr/local/bin:/usr/bin:/bin
ExecStart=\$4
Restart=always
RestartSec=2
NoNewPrivileges=true
ProtectSystem=full
[Install]
WantedBy=multi-user.target
U
}
unit zyvor-mock 'Zyvor simulated providers' /opt/zyvor /opt/zyvor/bin/zyvor-provider-mock mock.env
unit zyvor-api 'Zyvor API' /opt/zyvor /opt/zyvor/bin/zyvor-api api.env
unit zyvor-worker 'Zyvor worker' /opt/zyvor /opt/zyvor/bin/zyvor-worker worker.env
unit zyvor-console 'Zyvor console' /home/mini/zyvor/apps/zyvor-console '/usr/local/bin/pnpm exec next start -H 127.0.0.1 -p 3000' console.env
unit zyvor-checkout 'Zyvor checkout' /home/mini/zyvor/apps/zyvor-checkout '/usr/local/bin/pnpm exec next start -H 127.0.0.1 -p 3002' checkout.env
sudo tee /etc/caddy/Caddyfile >/dev/null <<C
{
	local_certs
}
console.\$H {
	tls internal
	reverse_proxy 127.0.0.1:3000
}
pay.\$H {
	tls internal
	reverse_proxy 127.0.0.1:3002
}
C
sudo systemctl daemon-reload
sudo systemctl enable --now zyvor-mock zyvor-api zyvor-worker zyvor-console zyvor-checkout >/dev/null 2>&1
sudo systemctl restart caddy
"

echo "==> checking from this Mac"
sleep 8
ok=1
for h in console pay; do
  code="$(curl -sk -m 20 -o /dev/null -w '%{http_code}' --resolve "$h.$H:443:$IP" "https://$h.$H/" || true)"
  echo "    https://$h.$H -> $code"
  case "$code" in 200|307) ;; *) ok=0;; esac
done
echo "    API ready: $(rs 'curl -s -m 5 localhost:8080/readyz' | tr -d '\r' | head -1)"
[ "$ok" = 1 ] || { echo "not healthy" >&2; exit 1; }
cat <<MSG

SANDBOX UP (simulated providers; self-signed certificate, so browsers warn and curl needs -k)
  console  https://console.$H   user admin   password in $OUT/console-password
  checkout https://pay.$H
  shell    fluxctl ssh $VM
  note     if the Mac resolves sslip.io names to a private address poorly, add:  $IP console.$H pay.$H  to /etc/hosts
MSG
