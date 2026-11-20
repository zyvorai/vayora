// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Scenario 4: fee-variance detection with known ground truth.
// Real (simulated) captured payments are created through the API; settlements are then imported whose fee lines equal the
// fee our own invented fee rules expect, except for payments where a known over- or under-deduction is injected.
// The rates below are arbitrary round numbers made up for this benchmark. They are not any provider's, network's or
// tax authority's rates. We then ask the platform which payments it flags and score it against the injected truth.
import { env, newTenant, ok, rng, subseed, rate, round, sql, createIntent, startAttempt, payerApproves, getAttempt } from "../lib/common.mjs";

const PER_PROVIDER = env.quick ? 30 : 100;
const RULES = {
  razorpay: { method: "upi", bps: 175, flat_minor: 0, cap_minor: null, gst_bps: 1800 },
  cashfree: { method: "upi", bps: 150, flat_minor: 300, cap_minor: 12000, gst_bps: 1800 },
};
const halfUp = (num, den) => Math.floor((2 * num + den) / (2 * den)); // integer round half up, as the docs describe
function expectedFee(amount, r) {
  let fee = halfUp(amount * r.bps, 10000) + r.flat_minor;
  if (r.cap_minor != null) fee = Math.min(fee, r.cap_minor);
  return fee + halfUp(fee * r.gst_bps, 10000);
}
const MAGNITUDES = [1, 2, 3, 5, 10, 25, 118, 500, 2000];

/** An amount whose percentage part lands exactly on a half paise, so rounding rules matter. */
function boundaryAmount(rand, r) {
  for (let i = 0; i < 20000; i++) {
    const a = rand.int(1_000, 400_000);
    if ((a * r.bps) % 10000 === 5000) return a;
  }
  return 10_000;
}

async function pay(t, provider, amount) {
  const intent = await createIntent(t, { amount, pinned: [provider] });
  const r = await startAttempt(t, intent);
  if (r.status !== 201) throw new Error("attempt failed " + r.text);
  await payerApproves(t, r.body);
  const a = await getAttempt(t, r.body.id);
  if (a.status !== "captured") throw new Error(`not captured: ${a.status}`);
  return a;
}

export async function run() {
  const t = await newTenant("s4");
  const rand = rng(subseed("s4"));
  for (const [provider, r] of Object.entries(RULES)) {
    await ok(t.key, "POST", "/v1/fee_rules", { provider, method: r.method, bps: r.bps, flat_minor: r.flat_minor, ...(r.cap_minor != null ? { cap_minor: r.cap_minor } : {}), gst_bps: r.gst_bps, effective_from: "2020-01-01", note: "benchmark rule (invented rates)" });
  }
  const truth = new Map(); // attempt id -> { injected (signed paise), expected, provider }
  const today = new Date().toISOString().slice(0, 10);
  let settlementNo = 0;
  for (const provider of Object.keys(RULES)) {
    const rule = RULES[provider];
    const payments = [];
    for (let i = 0; i < PER_PROVIDER; i++) {
      const amount = rand() < 0.12 ? boundaryAmount(rand, rule) : rand.pick([rand.int(1_000, 20_000), rand.int(20_000, 150_000), rand.int(150_000, 900_000)]);
      payments.push({ a: await pay(t, provider, amount), amount });
    }
    for (let off = 0; off < payments.length; off += 25) {
      const items = [];
      for (const { a, amount } of payments.slice(off, off + 25)) {
        const exp = expectedFee(amount, rule);
        const inject = rand() < 0.25 ? rand.pick([-1, 1]) * rand.pick(MAGNITUDES) : 0;
        const actual = exp + inject;
        truth.set(a.id, { injected: inject, expected: exp, amount, provider });
        items.push({ kind: "payment", provider_ref: a.provider_payment_ref, amount_minor: amount });
        items.push({ kind: "fee", provider_ref: a.provider_payment_ref, amount_minor: -actual });
      }
      await ok(t.key, "POST", "/v1/settlements/import", { provider, provider_ref: `stl_fee_${++settlementNo}`, utr: `FEEUTR${String(settlementNo).padStart(6, "0")}`, settled_on: today, items });
    }
  }

  const rows = () => sql(`SELECT attempt_id, status, expected_minor, actual_minor, variance_minor FROM fee_check_lines WHERE tenant_id = '${t.id}'`).map(([att, status, exp, act, v]) => ({ att, status, exp: Number(exp), act: Number(act), v: Number(v) }));
  const results = [];
  for (const tol of [0, 2, 10]) {
    await ok(t.key, "POST", "/v1/fee_checks/settings", { tolerance_minor: tol });
    await ok(t.key, "POST", "/v1/fee_checks/run", { limit: 500 });
    const lines = rows();
    let tp = 0, fp = 0, fn = 0, tn = 0, expectedMismatch = 0, overFlagged = 0, underFlagged = 0, overTruth = 0, underTruth = 0, dirWrong = 0;
    const missed = [], falseAlarms = [];
    for (const [att, g] of truth) {
      const line = lines.find((l) => l.att === att);
      const shouldFlag = Math.abs(g.injected) > tol;
      const flagged = line?.status === "flagged";
      if (line && line.exp !== g.expected) expectedMismatch++; // the platform's expected fee differs from our own calculation
      if (g.injected > tol) overTruth++;
      if (g.injected < -tol) underTruth++;
      if (shouldFlag && flagged) { tp++; if ((line.v > 0) !== (g.injected > 0)) dirWrong++; if (g.injected > 0) overFlagged++; else underFlagged++; }
      else if (shouldFlag && !flagged) { fn++; missed.push({ injected: g.injected, status: line?.status ?? "no line" }); }
      else if (!shouldFlag && flagged) { fp++; falseAlarms.push({ injected: g.injected, variance: line.v }); }
      else tn++;
    }
    const summary = await ok(t.key, "GET", "/v1/fee_checks/summary");
    results.push({
      tolerance_minor: tol,
      payments: truth.size,
      should_flag: tp + fn,
      true_positives: tp,
      false_positives: fp,
      false_negatives: fn,
      true_negatives: tn,
      precision_pct: rate(tp, tp + fp),
      recall_pct: rate(tp, tp + fn),
      over_deductions_found: `${overFlagged} of ${overTruth}`,
      under_deductions_found: `${underFlagged} of ${underTruth}`,
      direction_reported_wrongly: dirWrong,
      lines_where_platform_expected_fee_differs_from_the_benchmarks_own_calculation: expectedMismatch,
      open_findings_reported_by_summary: summary.open_findings ?? summary.open ?? null,
      missed: missed.slice(0, 10),
      false_alarms: falseAlarms.slice(0, 10),
    });
    console.error(`  s4 tolerance ${tol}: precision ${results.at(-1).precision_pct}% recall ${results.at(-1).recall_pct}%`);
  }
  const injectedTotal = [...truth.values()].filter((g) => g.injected !== 0).length;
  return {
    description: "Fee-variance detection on injected over/under deductions (simulator; invented rates)",
    rules_used: RULES,
    payments: truth.size,
    payments_with_an_injected_deduction_error: injectedTotal,
    injected_magnitudes_paise: MAGNITUDES,
    results,
  };
}
