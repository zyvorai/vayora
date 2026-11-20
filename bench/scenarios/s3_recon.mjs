// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Scenario 3: reconciliation of synthetic provider settlements against synthetic bank statements.
// For each statement layout a fresh tenant gets the same kinds of settlements (made-up UTRs and amounts) and a statement
// with KNOWN ground truth and injected problems: exact references, references only in the narration (several wordings),
// a credit short by a bank charge, a missing credit, narrations with the reference garbled or cut off, credits with no
// reference at all (some of them indistinguishable twins), duplicated rows, near-miss references on unrelated credits,
// and unrelated credits. Nothing here is a real bank layout from a real account: see docs/zyvor/BANK_RECONCILIATION.md.
import { env, newTenant, ok, call, rng, subseed, rate, round, sql, sleep } from "../lib/common.mjs";
import { LAYOUTS, writeStatement } from "../lib/bank.mjs";

const S = env.quick ? 24 : 60; // settlements per layout
const LAYOUT_NAMES = Object.keys(LAYOUTS);
const CATS = [
  ["exact_ref_column", 24],
  ["ref_in_narration", 26],
  ["short_credit", 8],
  ["missing_credit", 8],
  ["degraded_reference", 10],
  ["no_reference_exact_amount", 8],
  ["duplicated_row", 8],
]; // weights

const pad = (n, w) => String(n).padStart(w, "0");
function isoDay(day) { return `2026-10-${pad(day, 2)}`; }
function addDays(isoStr, n) { const d = new Date(isoStr + "T00:00:00Z"); d.setUTCDate(d.getUTCDate() + n); return d.toISOString().slice(0, 10); }
function pickWeighted(rand, table) {
  const total = table.reduce((s, [, w]) => s + w, 0);
  let x = rand() * total;
  for (const [name, w] of table) { x -= w; if (x < 0) return name; }
  return table[0][0];
}

function genUtr(rand, used) {
  for (;;) {
    const kind = rand.pick(["num12", "hdfc", "axis", "sbi", "icici"]);
    const digits = (n) => Array.from({ length: n }, () => rand.int(0, 9)).join("");
    const u = kind === "num12" ? digits(12) : kind === "hdfc" ? "HDFCR" + digits(10) : kind === "axis" ? "AXISN" + digits(10) : kind === "sbi" ? "SBIN" + digits(11) : "ICIC" + digits(11);
    if (!used.has(u)) { used.add(u); return u; }
  }
}
// narrations with the reference written in the ways bank statements tend to write it (invented wordings)
const NARR = [
  (u) => `NEFT CR-${u}-ACME PAYMENTS PVT LTD`,
  (u) => `UPI/${u}/SETTLEMENT/ACME`,
  (u) => `IMPS-${u}-ACME-SETTLEMENT`,
  (u) => `${u.toLowerCase()} neft inward remittance`,
  (u) => `UTR${u} MERCHANT SETTLEMENT`,
  (u) => `NEFT_${u}_ACME`,
  (u) => `RTGS CR ${u} ACME PAYMENTS`,
];
const DEGRADED = [
  (u) => `NEFT CR-${u.slice(0, 5)} ${u.slice(5)}-ACME`, // the bank broke the reference with a space
  (u) => `NEFT CR-${u.slice(0, u.length - 3)}...`, // the narration was cut off
  (u) => `NEFT CR-${u.slice(0, 3)}*${u.slice(-3)}-ACME`, // masked
];
const PLAIN = ["NEFT CR-ACME PAYMENTS PVT LTD", "SETTLEMENT CREDIT", "NEFT INWARD REMITTANCE ACME", "BY TRANSFER ACME PAY"];
const UNRELATED = ["CASH DEPOSIT BRANCH", "CHQ DEP CLG", "NEFT CR-GREENFIELD TRADERS", "UPI/CUSTOMER REFUND REVERSAL", "INTEREST CREDIT"];

function build(layout, seedName) {
  const rand = rng(subseed(`s3:${seedName}`));
  const used = new Set();
  const settlements = [];
  const rows = [];
  const truthCat = [];
  let netBase = 100_00;
  const push = (r, truth, cat) => { rows.push(r); truthCat.push({ truth, cat }); };

  for (let i = 0; i < S; i++) {
    const cat = pickWeighted(rand, CATS);
    const utr = genUtr(rand, used);
    const day = rand.int(1, 24);
    const net = rand.int(5_000, 900_000) * 1 + (i % 7); // paise; plenty of distinct values
    settlements.push({ i, utr, settled_on: isoDay(day), net, cat, gross: net + Math.round(net * 0.0236) });
    const posted = addDays(isoDay(day), rand.int(0, 2));
    const base = { date: posted, debit: null };
    if (cat === "exact_ref_column") push({ ...base, narration: NARR[0](utr), reference: utr, credit: net }, i, cat);
    else if (cat === "ref_in_narration") push({ ...base, narration: rand.pick(NARR)(utr), reference: "", credit: net }, i, cat);
    else if (cat === "short_credit") push({ ...base, narration: rand.pick(NARR)(utr), reference: rand() < 0.5 ? utr : "", credit: net - rand.pick([1180, 2360, 5900, 100]) }, i, cat);
    else if (cat === "degraded_reference") push({ ...base, narration: rand.pick(DEGRADED)(utr), reference: "", credit: net }, i, cat);
    else if (cat === "no_reference_exact_amount") push({ ...base, narration: rand.pick(PLAIN), reference: "", credit: net }, i, cat);
    else if (cat === "duplicated_row") {
      const r = { ...base, narration: NARR[0](utr), reference: utr, credit: net };
      push(r, i, cat);
      push({ ...r }, null, "duplicate_of_a_real_credit");
    }
    // missing_credit: no row at all
  }
  // two pairs of twin settlements: same amount, same day, no reference on either credit (cannot be told apart)
  for (let k = 0; k < 2; k++) {
    const net = 777_000 + k * 1111;
    const day = 25 + k;
    for (const tag of ["a", "b"]) {
      const i = settlements.length;
      const utr = genUtr(rand, used);
      settlements.push({ i, utr, settled_on: isoDay(day), net, cat: "twin_" + tag, gross: net + 1000 });
      push({ date: isoDay(day), debit: null, narration: PLAIN[1], reference: "", credit: net }, i, "twin_no_reference");
    }
  }
  // near-miss references on credits that belong to NO settlement (a wrong confirmation here is a false match)
  const base = settlements.filter((s) => s.cat !== "missing_credit");
  for (let k = 0; k < 6; k++) {
    const s = base[rand.int(0, base.length - 1)];
    const m = [s.utr + "7", s.utr.slice(0, -1), s.utr.slice(0, -1) + String((Number(s.utr.slice(-1)) + 1) % 10)][k % 3];
    if (used.has(m)) continue;
    push({ date: isoDay(rand.int(1, 27)), debit: null, narration: `NEFT CR-${m}-OTHER PARTY`, reference: k % 2 ? m : "", credit: rand.int(5_000, 900_000) + 13 }, null, "near_miss_reference");
  }
  // unrelated credits, and debits (charges, transfers) that are not settlements
  for (let k = 0; k < 8; k++) push({ date: isoDay(rand.int(1, 27)), debit: null, narration: rand.pick(UNRELATED), reference: "", credit: rand.int(1_000, 400_000) + 7 }, null, "unrelated_credit");
  for (let k = 0; k < 6; k++) push({ date: isoDay(rand.int(1, 27)), debit: rand.int(1_000, 90_000), narration: rand.pick(["BANK CHARGES", "NEFT DR-VENDOR PAYMENT", "ATM WDL"]), reference: "", credit: null }, null, "debit");
  // a late closing entry so the statement covers the whole period (the missing-credit check needs that)
  push({ date: "2026-11-10", debit: 118, narration: "BANK CHARGES", reference: "", credit: null }, null, "debit");

  // shuffle in a fixed way, then order by date as a bank would (ties keep the shuffled order)
  const order = rand.shuffle(rows.map((_, i) => i)).sort((a, b) => (rows[a].date < rows[b].date ? -1 : rows[a].date > rows[b].date ? 1 : 0));
  return { settlements, rows: order.map((i) => rows[i]), truthCat: order.map((i) => truthCat[i]) };
}

async function runLayout(layout) {
  const t = await newTenant(`s3 ${layout}`);
  const { settlements, rows, truthCat } = build(layout, layout);
  const sid = new Map(); // settlement index -> platform id
  for (const s of settlements) {
    const r = await ok(t.key, "POST", "/v1/settlements/import", {
      provider: s.i % 2 ? "cashfree" : "razorpay",
      provider_ref: `stl_${layout}_${s.i}`,
      utr: s.utr,
      settled_on: s.settled_on,
      items: [
        { kind: "payment", provider_ref: `pay_${layout}_${s.i}`, amount_minor: s.gross },
        { kind: "fee", provider_ref: `pay_${layout}_${s.i}`, amount_minor: -(s.gross - s.net) },
      ],
    });
    sid.set(s.i, r.settlement.id);
  }
  const csv = writeStatement(layout, rows);
  const imp = await call(t.key, "POST", "/v1/bank_statements/import_csv", { csv, filename: `${layout}.csv`, layout });
  if (imp.status !== 200 && imp.status !== 201) return { layout, import_failed: true, status: imp.status, error: imp.text.slice(0, 400) };
  const importId = imp.body.import_id ?? imp.body.id;
  const lines = (await ok(t.key, "GET", `/v1/bank_statements/lines?import_id=${importId}&limit=200`)).data;

  // map each statement row to the stored line: same date, amount and narration, in file order
  const keyOf = (date, amount, narr) => `${date}|${amount}|${narr}`;
  const queues = new Map();
  for (const l of lines.slice().sort((a, b) => a.line_no - b.line_no)) {
    const k = keyOf(l.posted_on, l.amount_minor, l.narration);
    (queues.get(k) ?? queues.set(k, []).get(k)).push(l);
  }
  const rowLine = rows.map((r) => queues.get(keyOf(r.date, r.credit ?? -r.debit, r.narration))?.shift() ?? null);
  const parsed = rowLine.filter(Boolean).length;

  const sidToIdx = new Map([...sid].map(([i, id]) => [id, i]));
  const cat = {};
  const bump = (c, k) => { (cat[c] ??= { lines: 0 })[k] = ((cat[c][k] ?? 0) + 1); cat[c].lines++; };
  let confirmedCorrect = 0, confirmedWrong = 0, truthLines = 0, suggestedWithTruth = 0, suggestedWithoutTruth = 0, unmatchedTruth = 0, noiseUnmatched = 0, noiseSuggested = 0;
  const wrongDetail = [];
  rows.forEach((r, k) => {
    const { truth, cat: c } = truthCat[k];
    if (r.credit == null) return; // debits are not part of credit matching
    const l = rowLine[k];
    if (!l) { bump(c, "not_parsed"); return; }
    const m = l.match && l.match.subject_type === "settlement" ? sidToIdx.get(l.match.subject_id) : l.match ? "other" : undefined;
    const sug = (l.suggestions ?? []).filter((x) => x.subject_type === "settlement").map((x) => sidToIdx.get(x.subject_id));
    if (truth !== null) truthLines++;
    if (l.status === "matched") {
      if (truth !== null && m === truth) { confirmedCorrect++; bump(c, "confirmed_correct"); }
      else { confirmedWrong++; bump(c, "confirmed_WRONG"); wrongDetail.push({ layout, category: c, narration: r.narration, rule: l.match?.rule, truth }); }
    } else if (l.status === "suggested") {
      if (truth !== null) { if (sug.includes(truth)) { suggestedWithTruth++; bump(c, "suggested_incl_truth"); } else { suggestedWithoutTruth++; bump(c, "suggested_without_truth"); } }
      else { noiseSuggested++; bump(c, "suggested_no_truth_exists"); }
    } else {
      if (truth !== null) { unmatchedTruth++; bump(c, "unmatched"); } else { noiseUnmatched++; bump(c, "unmatched_correctly"); }
    }
  });

  // missing-credit detection and short-credit detection
  await call(t.key, "POST", "/v1/recon/scan?older_than_days=3");
  const noCredit = new Set(sql(`SELECT subject_id FROM recon_cases WHERE tenant_id = '${t.id}' AND kind = 'settlement_no_bank_credit'`).map((x) => sidToIdx.get(x[0])));
  const mismatch = sql(`SELECT subject_id FROM recon_cases WHERE tenant_id = '${t.id}' AND kind = 'bank_amount_mismatch'`).length;
  const trulyMissing = settlements.filter((s) => s.cat === "missing_credit").map((s) => s.i);
  const trulyShort = rows.filter((r, k) => truthCat[k].cat === "short_credit").length;
  const missingFlagged = trulyMissing.filter((i) => noCredit.has(i)).length;
  // flagged although some credit explains it (a flag is expected only for settlements with no confirmed bank match)
  const confirmedSettlements = new Set(lines.filter((l) => l.status === "matched" && l.match?.subject_type === "settlement").map((l) => sidToIdx.get(l.match.subject_id)));
  const flaggedButConfirmed = [...noCredit].filter((i) => confirmedSettlements.has(i)).length;

  // re-upload of the identical file, and of an overlapping file (every row from the middle on): must add nothing new
  const again = await ok(t.key, "POST", "/v1/bank_statements/import_csv", { csv, filename: `${layout}-again.csv`, layout });
  const half = rows.slice(Math.floor(rows.length / 2));
  const overlap = await ok(t.key, "POST", "/v1/bank_statements/import_csv", { csv: writeStatement(layout, half), filename: `${layout}-overlap.csv`, layout });
  const afterConfirmed = (await ok(t.key, "GET", `/v1/bank_statements/lines?status=matched&limit=200`)).data.length;

  return {
    layout,
    rows_in_file: rows.length,
    lines_parsed: parsed,
    credits: rows.filter((r) => r.credit != null).length,
    credit_lines_with_a_true_settlement: truthLines,
    confirmed_correct: confirmedCorrect,
    confirmed_wrong: confirmedWrong,
    suggested_including_truth: suggestedWithTruth,
    suggested_not_including_truth: suggestedWithoutTruth,
    unmatched_but_a_true_settlement_exists: unmatchedTruth,
    unrelated_credits_unmatched: noiseUnmatched,
    unrelated_credits_with_a_suggestion: noiseSuggested,
    by_category: cat,
    wrong_detail: wrongDetail,
    settlements: settlements.length,
    settlements_truly_without_credit: trulyMissing.length,
    missing_credit_flagged: missingFlagged,
    settlements_flagged_no_credit_although_a_credit_is_confirmed: flaggedButConfirmed,
    short_credits_injected: trulyShort,
    amount_mismatch_cases: mismatch,
    reupload_same_file: { added: again.added, duplicates: again.duplicates },
    reupload_overlapping_half: { rows: half.length, added: overlap.added, duplicates: overlap.duplicates },
    matched_lines_after_reuploads: afterConfirmed,
    matched_lines_before_reuploads: lines.filter((l) => l.status === "matched").length,
  };
}

export async function run() {
  const layouts = [];
  for (const l of LAYOUT_NAMES) {
    layouts.push(await runLayout(l));
    const r = layouts[layouts.length - 1];
    console.error(`  s3 ${l}: confirmed ${r.confirmed_correct} correct / ${r.confirmed_wrong} wrong of ${r.credit_lines_with_a_true_settlement}`);
  }
  const ok_ = layouts.filter((l) => !l.import_failed);
  const sum = (k) => ok_.reduce((s, l) => s + l[k], 0);
  const truth = sum("credit_lines_with_a_true_settlement");
  const referenceBearing = ok_.reduce((s, l) => s + ["exact_ref_column", "ref_in_narration", "short_credit", "duplicated_row"].reduce((a, c) => a + (l.by_category[c]?.lines ?? 0), 0), 0);
  const refConfirmed = ok_.reduce((s, l) => s + ["exact_ref_column", "ref_in_narration", "short_credit"].reduce((a, c) => a + (l.by_category[c]?.confirmed_correct ?? 0), 0), 0);
  const refMatchable = ok_.reduce((s, l) => s + ["exact_ref_column", "ref_in_narration", "short_credit"].reduce((a, c) => a + (l.by_category[c]?.lines ?? 0), 0), 0);
  const totals = {
    layouts_tested: LAYOUT_NAMES.length,
    layouts_that_failed_to_import: layouts.length - ok_.length,
    credit_lines_with_a_true_settlement: truth,
    confirmed_correct: sum("confirmed_correct"),
    auto_confirmed_wrong: sum("confirmed_wrong"),
    suggested_including_truth: sum("suggested_including_truth"),
    suggested_not_including_truth: sum("suggested_not_including_truth"),
    unmatched_but_a_true_settlement_exists: sum("unmatched_but_a_true_settlement_exists"),
    unrelated_credits_with_a_suggestion: sum("unrelated_credits_with_a_suggestion"),
    unrelated_credits_unmatched: sum("unrelated_credits_unmatched"),
    match_rate_pct_confirmed_correct_of_lines_with_truth: rate(sum("confirmed_correct"), truth),
    match_rate_pct_when_the_reference_is_present_and_readable: rate(refConfirmed, refMatchable),
    resolved_or_suggested_pct: rate(sum("confirmed_correct") + sum("suggested_including_truth"), truth),
    reference_bearing_lines_in_scope: referenceBearing,
    missing_credits_injected: sum("settlements_truly_without_credit"),
    missing_credits_flagged: sum("missing_credit_flagged"),
    short_credits_injected: sum("short_credits_injected"),
    amount_mismatch_cases_opened: sum("amount_mismatch_cases"),
    reupload_added_rows_total: ok_.reduce((s, l) => s + l.reupload_same_file.added, 0),
  };
  const byCategory = {};
  for (const l of ok_) for (const [c, v] of Object.entries(l.by_category)) for (const [k, n] of Object.entries(v)) (byCategory[c] ??= {})[k] = (byCategory[c][k] ?? 0) + n;
  totals.by_category = byCategory;
  return { description: "Reconciliation on synthetic statements with injected errors (simulator, made-up data)", settlements_per_layout: S, totals, layouts };
}
