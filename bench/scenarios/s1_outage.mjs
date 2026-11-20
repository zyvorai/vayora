// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Scenario 1: payment success during a provider outage and a brown-out, with and without Vayora routing.
//   arm "baseline": every payment is pinned to ONE provider (allowed_providers = ["razorpay"]), i.e. no Vayora routing
//   arm "vayora":   the payment may use either provider; failover, success-rate signal and circuit breaker are on (defaults)
// The primary provider (razorpay, the cheaper one in this setup) is made to fail by the SIMULATED provider mock.
// Payments run one after another so the seeded failure pattern is exactly reproducible.
import { env, stats, newTenant, setRoutingFees, outage, rate, pct, round, subseed, ok, auditDoubleCharges, now, sleep } from "../lib/common.mjs";
import { journey } from "../lib/journey.mjs";

const N = env.quick ? 16 : 40; // payments inside the outage window
const WARM = env.quick ? 10 : 20; // healthy payments before it starts

const cells = [
  { mode: "reject", pct: 0, lookup: true, label: "control, healthy" },
  { mode: "reject", pct: 30, lookup: true },
  { mode: "reject", pct: 60, lookup: true },
  { mode: "reject", pct: 100, lookup: true },
  { mode: "unavailable", pct: 30, lookup: true },
  { mode: "unavailable", pct: 100, lookup: true },
  { mode: "unavailable", pct: 100, lookup: false },
  { mode: "hang", pct: 30, lookup: true },
  { mode: "hang", pct: 100, lookup: true },
  { mode: "hang", pct: 100, lookup: false },
];
const only = process.env.BENCH_S1_ONLY; // for debugging, e.g. "hang:100:true"
const quickCells = cells.filter((c) => c.pct === 0 || (c.pct === 100 && c.lookup) || (c.mode === "reject" && c.pct === 60));

const tenants = [];
async function arm(cell, name) {
  const t = await newTenant(`s1 ${name}`);
  tenants.push(t.id);
  await setRoutingFees(t);
  const pinned = name === "baseline" ? ["razorpay"] : null;
  const o = { pinned, lookup: cell.lookup };
  await outage("razorpay", "off");
  for (let i = 0; i < WARM; i++) await journey(t, o); // healthy history, identical for both arms
  const seed = subseed(`s1:${cell.mode}:${cell.pct}`);
  await outage("razorpay", cell.mode, cell.pct, seed);
  const start = now();
  const logs = [];
  let firstAltAt = null, firstAltIdx = null, firstCapViaAlt = null;
  for (let i = 0; i < N; i++) {
    const l = await journey(t, o);
    logs.push(l);
    const firstProvider = l.attempts[0]?.provider;
    if (firstAltIdx === null && firstProvider && firstProvider !== "razorpay") { firstAltIdx = i + 1; firstAltAt = now() - start; }
    if (firstCapViaAlt === null && l.end === "captured" && l.via && l.via !== "razorpay") firstCapViaAlt = now() - start;
  }
  await outage("razorpay", "off");
  const count = (f) => logs.filter(f).length;
  const lat = logs.map((l) => l.ms);
  const health = await ok(t.key, "GET", "/v1/routing/health?method=upi");
  const rzRow = health.providers?.find((p) => p.provider === "razorpay")?.methods?.find((m) => m.method === "upi");
  const captured = count((l) => l.end === "captured");
  return {
    payments: N,
    captured,
    success_rate_pct: rate(captured, N),
    ended_failed: count((l) => l.end === "failed"),
    ended_unknown: count((l) => l.end === "unknown"),
    payments_needing_a_retry: count((l) => l.attempts.length > 1),
    payments_with_an_unknown_first_outcome: count((l) => l.unknownSeen > 0),
    captured_via_alternate_provider: count((l) => l.end === "captured" && l.via !== "razorpay"),
    attempts_total: logs.reduce((s, l) => s + l.attempts.length, 0),
    payments_until_first_one_routed_away_from_primary: firstAltIdx,
    ms_until_first_one_routed_away_from_primary: firstAltAt === null ? null : round(firstAltAt, 0),
    ms_until_first_capture_on_alternate: firstCapViaAlt === null ? null : round(firstCapViaAlt, 0),
    latency_ms: { p50: round(pct(lat, 50), 0), p95: round(pct(lat, 95), 0), max: round(Math.max(...lat), 0) },
    window_seconds: round((now() - start) / 1000, 1),
    primary_breaker_state_at_end: rzRow?.state ?? null,
  };
}

export async function run() {
  const out = { description: "Success during outage/brown-out, baseline (pinned to one provider) vs Vayora routing", payments_per_cell: N, healthy_warmup_payments: WARM, cells: [] };
  for (const cell of only ? cells.filter((c) => `${c.mode}:${c.pct}:${c.lookup}` === only) : env.quick ? quickCells : cells) {
    const baseline = await arm(cell, "baseline");
    const vayora = await arm(cell, "vayora");
    out.cells.push({ mode: cell.mode, fail_pct: cell.pct, lookup_available: cell.lookup, baseline, vayora });
    console.error(`  s1 ${cell.mode} ${cell.pct}% lookup=${cell.lookup}: baseline ${baseline.success_rate_pct}%  vayora ${vayora.success_rate_pct}%`);
  }
  out.double_charge_audit_over_this_scenario = await auditDoubleCharges(tenants);
  out.requests_that_were_rate_limited_and_retried = stats.rate_limited_retries;
  return out;
}
