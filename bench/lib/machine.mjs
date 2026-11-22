// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// Records what the numbers were measured on. Usage: node machine.mjs <out.json>
import os from "node:os";
import { execFileSync } from "node:child_process";
import { writeJson } from "./common.mjs";
const run = (cmd, args, opts = {}) => { try { return execFileSync(cmd, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], ...opts }).trim(); } catch { return null; } };
const pgUrl = process.env.BENCH_OWNER_URL;
await writeJson(process.argv[2], {
  platform: `${os.platform()} ${os.release()} ${os.arch()}`,
  cpu_model: os.cpus()[0]?.model ?? null,
  cpu_cores_logical: os.cpus().length,
  memory_gb: Math.round(os.totalmem() / 2 ** 30),
  node: process.version,
  rustc: run("rustc", ["--version"]),
  postgres: pgUrl ? run("psql", [pgUrl, "-X", "-At", "-c", "select split_part(version(), ' on ', 1)"]) : null,
  git_commit: run("git", ["rev-parse", "--short", "HEAD"]),
  git_tree_clean: run("git", ["status", "--porcelain", "--", "bench", "crates"]) === "",
  note: "everything runs on this one machine: API, simulated provider mock, Postgres, load generator and (for scenario 5) the browser",
});
