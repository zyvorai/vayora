# Running Zyvor in a FluxVM lab VM (Apple silicon Mac)

A second, independent place to run the stack and the tests: a Debian VM made with
[FluxVM](https://github.com/zyvorai/zyvor-fluxvm) (`brew install zyvorai/tap/fluxvm`). It has a different
Postgres version (17) and a UTC clock, which is how it found the Indian-business-date bugs fixed in release 3.

What was run by hand and worked (nothing here is automated or run in CI):

```bash
# 1. boot, keep it, share the repo read-only, then grow the 2.8 GB root disk
fluxctl run --name zyvor-lab --keep --cpus 4 --memory-mib 8192 -v "$PWD":/mnt/src:ro -- true
fluxctl stop zyvor-lab && fluxctl disk resize zyvor-lab root --size-gib 40 && fluxctl start zyvor-lab

# 2. reach it (exec needs an agent the image does not have; ssh works)
fluxctl ssh zyvor-lab -- 'uname -m'
```

Inside the VM:

- **apt over HTTPS hangs.** Change the mirror lists first: `sudo sed -i 's#https://#http://#g' /etc/apt/mirrors/*.list`, then
  `apt-get update`. Install `postgresql build-essential pkg-config python3 xz-utils` with `--no-install-recommends`.
  Do not `apt-get install nodejs npm`: the resolver ran past 8 GB and was killed.
- **Rust** with rustup; **Node 22.13 or newer** from the nodejs.org arm64 tarball (pnpm 11 refuses older), then
  `npm i -g pnpm@11.25.0`. Corepack failed to verify its signing key on Node 22.11, so skip it.
- Create a Postgres superuser role and an empty database, copy the source from `/mnt/src` (without `target` and
  `node_modules`), then `OWNER_URL=postgres://<role>:<pass>@127.0.0.1:5432/<db> ./scripts/zyvor-sandbox.sh`.
- Tests: `DATABASE_URL=postgres://<role>:<pass>@127.0.0.1:5432/postgres cargo test --workspace`.
- Console and checkout: `pnpm install --frozen-lockfile`, build as in `deploy/zyvor/Dockerfile.console`, then
  `next start -H 0.0.0.0` under `setsid nohup ... < /dev/null`. Without `setsid` they die when the SSH session ends.
  The VM's address (for example `192.168.64.189`) is reachable from the Mac; the API still binds loopback only.

Run long jobs detached inside the VM and poll; an SSH drop kills foreground work. Never `pkill -f` a pattern that
appears in your own command line.

Not verified: a repeatable one-command script, HTTPS in front of the VM, or any use beyond one Mac.

## Running it as the sandbox: `scripts/fluxvm-lab.sh`

`VM=zyvor-sandbox ./scripts/fluxvm-lab.sh` creates the VM (4 vCPU, 8 GiB, 40 GiB), installs Postgres, Rust, Node and
Caddy, builds the API and web apps, runs the repo's sandbox script to migrate and seed simulated providers, installs one
systemd unit per service (env files in `/etc/zyvor`, mode 600; console and checkout bind loopback) and puts Caddy in
front with its own CA. It prints the addresses and writes a random console password to
`~/.zyvor-lab/<vm>/console-password`.

Verified once, on one Mac: the script ran to the end on a fresh VM, console answered 307 (to login) and checkout 200 over
HTTPS from the Mac, the API reported ready, and after a VM restart every service and Caddy came back on its own.

Limits you should know:
- The certificate comes from Caddy's own CA, so browsers warn and `curl` needs `-k`. The address is a private
  192.168.64.x one, reachable only from the Mac that hosts the VM.
- The script talks to the VM with plain `ssh`/`scp`. `fluxctl ssh` hangs on large commands and never sees EOF on stdin.
- Hard-stopping or force-restarting a VM mid-write damaged files inside it (a Rust toolchain crashed with SIGBUS, a
  `package.json` came back empty, the journal was "uncleanly shut down"). Stop VMs cleanly; if a build step fails
  strangely after a force-stop, delete the toolchain or `node_modules` and rerun. The Rust build is capped at two jobs
  because the first VM stopped answering SSH under a four-job build (a guess at the cause, not proven).
- If every VM on the Mac loses its address, that is the Mac's DHCP service (`bootpd`), not the guest. It needed
  `sudo launchctl kickstart -k system/com.apple.bootpd` run by the person at the Mac.
- `pgrep -x` compares only the first 15 characters of a process name, so `zyvor-provider-mock` is not matched.
