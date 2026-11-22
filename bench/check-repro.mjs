#!/usr/bin/env node
// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// Compares two results.json files and fails if any number differs by more than its documented tolerance.
//   node check-repro.mjs <run1/results.json> <run2/results.json>
// Tolerances (see docs/zyvor/BENCHMARKS.md):
//   exact       counts and rates in the deterministic scenarios (reconciliation, fee checks, double-charge audit flags)
//   outage      scenario 1 shares: +-5 percentage points, counts +-3 payments, "payments until routed away" +-3
//   timing      anything in ms / seconds, and scenario 5: relative 50% with a floor of 250 ms (or 2 s for seconds)
//   tail        p95 and max latency: relative 100% (a single slow request moves them) with a floor of 250 ms
//   bytes       page weight: relative 2%
//   volume      scenario 2 request counters (parallel races vary slightly): relative 10% with a floor of 8
import { readFileSync } from "node:fs";
const [a, b] = process.argv.slice(2).map((f) => JSON.parse(readFileSync(f, "utf8")));
const IGNORE = /(^|\.)(generated_at|machine|wall_seconds|window_seconds|description|note|version|files|file_list|checkout_install_and_build_seconds|missed|false_alarms)(\.|$)/;
const problems = [];
function rule(path) {
  if (/latency_ms\.(p95|max)$/.test(path)) return "tail";
  if (/bytes|transferred/.test(path) && path.includes("s5_checkout_weight")) return "bytes";
  if (/load_event_ms|s5_first_payment|ms_until|latency_ms|_ms$|seconds/.test(path) && !/tolerance_minor/.test(path)) return "timing";
  if (path.startsWith("s1_outage")) return "outage";
  if (/^s2_double_charge\.audit\./.test(path) && !/more_than|no_captured|not_recorded/.test(path)) return "volume";
  if (path.startsWith("s2_double_charge.requests") || /^s2_double_charge\.(attempts_recorded|provider_orders_created)/.test(path)) return "volume";
  return "exact";
}
function cmp(x, y, path) {
  if (IGNORE.test(path)) return;
  if (x === null || y === null || typeof x !== typeof y) { if (x !== y) problems.push(`${path}: ${JSON.stringify(x)} vs ${JSON.stringify(y)}`); return; }
  if (Array.isArray(x)) {
    if (x.length !== y.length) return problems.push(`${path}: length ${x.length} vs ${y.length}`);
    x.forEach((v, i) => cmp(v, y[i], `${path}[${i}]`));
  } else if (typeof x === "object") {
    for (const k of new Set([...Object.keys(x), ...Object.keys(y)])) cmp(x[k], y[k], path ? `${path}.${k}` : k);
  } else if (typeof x === "number") {
    const d = Math.abs(x - y), kind = rule(path);
    const secs = /seconds/.test(path);
    const ok = kind === "exact" ? d === 0
      : kind === "bytes" ? d <= 0.02 * Math.max(x, y)
      : kind === "tail" ? d <= Math.max(1.0 * Math.max(x, y), 250)
      : kind === "timing" ? d <= Math.max(0.5 * Math.max(x, y), secs ? 2 : 250)
      : kind === "volume" ? d <= Math.max(0.1 * Math.max(x, y), 8)
      : /rate_pct|share|_pct/.test(path) ? d <= 5 : d <= 3;
    if (!ok) problems.push(`${path}: ${x} vs ${y} (${kind})`);
  } else if (x !== y) problems.push(`${path}: ${JSON.stringify(x)} vs ${JSON.stringify(y)}`);
}
cmp(a, b, "");
if (problems.length) {
  console.error(`NOT reproducible within tolerance: ${problems.length} difference(s)`);
  for (const p of problems.slice(0, 40)) console.error("  " + p);
  process.exit(1);
}
console.log("two runs agree within the documented tolerances");
