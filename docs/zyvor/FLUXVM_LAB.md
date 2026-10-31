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
