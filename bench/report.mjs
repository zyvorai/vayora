#!/usr/bin/env node
// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// Merges the per-scenario JSON files in <dir> into results.json and writes results.md (tables).  Usage: node report.mjs <dir>
import { readFileSync, existsSync, writeFileSync } from "node:fs";
const dir = process.argv[2];
const read = (f) => (existsSync(`${dir}/${f}`) ? JSON.parse(readFileSync(`${dir}/${f}`, "utf8")) : null);
const r = {
  generated_at: new Date().toISOString(),
  machine: read("machine.json"),
  s1_outage: read("s1_outage.json"),
  s2_double_charge: read("s2_double_charge.json"),
  s3_recon: read("s3_recon.json"),
  s4_fees: read("s4_fees.json"),
  s5_first_payment: read("s5_first_payment.json"),
  s5_checkout_weight: read("s5_checkout_weight.json"),
};
writeFileSync(`${dir}/results.json`, JSON.stringify(r, null, 2) + "\n");

const L = [];
const table = (head, rows) => { L.push("| " + head.join(" | ") + " |", "|" + head.map(() => "---").join("|") + "|", ...rows.map((x) => "| " + x.join(" | ") + " |"), ""); };
const kb = (b) => (b === null || b === undefined ? "n/a" : (b / 1024).toFixed(1) + " KiB");
const ms = (x) => (x === null || x === undefined ? "n/a" : x >= 10000 ? (x / 1000).toFixed(1) + " s" : x + " ms");

L.push("# Benchmark results (simulator only)", "");
if (r.machine) L.push(`Machine: ${r.machine.cpu_model}, ${r.machine.cpu_cores_logical} logical cores, ${r.machine.memory_gb} GB, ${r.machine.platform}; ${r.machine.postgres}; ${r.machine.rustc}; node ${r.machine.node}; commit ${r.machine.git_commit}.`, "");

const s1 = r.s1_outage;
if (s1) {
  L.push(`## 1. Success during an outage or brown-out (${s1.payments_per_cell} payments per cell after ${s1.healthy_warmup_payments} healthy ones)`, "");
  table(["Primary provider fault", "Failing calls", "Status lookup", "Baseline captured", "Vayora captured", "Baseline unknown", "Vayora unknown", "Vayora: payments until routed away", "Vayora: first capture on alternate", "Baseline p50", "Vayora p50"],
    s1.cells.map((c) => [c.mode, `${c.fail_pct}%`, c.lookup_available ? "yes" : "no", `${c.baseline.captured}/${c.baseline.payments} (${c.baseline.success_rate_pct}%)`, `${c.vayora.captured}/${c.vayora.payments} (${c.vayora.success_rate_pct}%)`, c.baseline.ended_unknown, c.vayora.ended_unknown, c.vayora.payments_until_first_one_routed_away_from_primary ?? "never", ms(c.vayora.ms_until_first_capture_on_alternate), ms(c.baseline.latency_ms.p50), ms(c.vayora.latency_ms.p50)]));
}
const s2 = r.s2_double_charge;
if (s2) {
  L.push("## 2. Double-charge check", "");
  table(["Measure", "Value"], [
    ["Requests sent", s2.requests.requests], ["Attempt requests", s2.requests.attempt_posts], ["Refused with 409", s2.requests.refused_409], ["Same-key replays", s2.requests.same_key_replays], ["Webhook deliveries (incl. duplicates)", s2.requests.webhook_deliveries],
    ["Intents with attempts", s2.audit.intents_with_attempts], ["Intents paid at the (simulated) provider", s2.audit.intents_paid_at_provider],
    ["Intents with more than one captured attempt (platform records)", s2.audit.intents_with_more_than_one_captured_attempt_in_platform],
    ["Intents paid more than once (provider records)", s2.audit.intents_paid_more_than_once_at_provider],
    ["Paid at provider but no captured attempt in platform", s2.audit.intents_paid_at_provider_but_no_captured_attempt_in_platform],
    ["Unexpected observations", s2.unexpected_observations.length], ["Pass", s2.pass]]);
}
const s3 = r.s3_recon;
if (s3) {
  const t = s3.totals;
  L.push(`## 3. Reconciliation (${t.layouts_tested} layouts, ${s3.settlements_per_layout} random settlements each, plus twins and noise)`, "");
  table(["Measure", "Value"], [
    ["Credit lines that have a true settlement", t.credit_lines_with_a_true_settlement], ["Confirmed automatically and correct", t.confirmed_correct], ["Confirmed automatically and WRONG", t.auto_confirmed_wrong],
    ["Suggested for a person (true settlement among suggestions)", t.suggested_including_truth], ["Suggested, true settlement missing from the suggestions", t.suggested_not_including_truth], ["Left unmatched although a true settlement exists", t.unmatched_but_a_true_settlement_exists],
    ["Confirmed-correct share of all lines with a true settlement", `${t.match_rate_pct_confirmed_correct_of_lines_with_truth}%`], ["Confirmed-correct share where a readable reference is present", `${t.match_rate_pct_when_the_reference_is_present_and_readable}%`],
    ["Confirmed or suggested share", `${t.resolved_or_suggested_pct}%`], ["Missing credits injected / flagged", `${t.missing_credits_injected} / ${t.missing_credits_flagged}`], ["Short credits injected / amount-mismatch cases", `${t.short_credits_injected} / ${t.amount_mismatch_cases_opened}`], ["Rows added when the same file is uploaded again", t.reupload_added_rows_total]]);
  L.push("By kind of line (all layouts):", "");
  const keys = ["confirmed_correct", "confirmed_WRONG", "suggested_incl_truth", "suggested_without_truth", "suggested_no_truth_exists", "unmatched", "unmatched_correctly"];
  table(["Kind of line", "Lines", ...keys], Object.entries(t.by_category).map(([c, v]) => [c, v.lines, ...keys.map((k) => v[k] ?? 0)]));
}
const s4 = r.s4_fees;
if (s4) {
  L.push(`## 4. Fee-variance detection (${s4.payments} payments, ${s4.payments_with_an_injected_deduction_error} with an injected error)`, "");
  table(["Tolerance (paise)", "Should flag", "True pos.", "False pos.", "False neg.", "Precision", "Recall", "Over found", "Under found", "Expected fee differs from benchmark's"],
    s4.results.map((x) => [x.tolerance_minor, x.should_flag, x.true_positives, x.false_positives, x.false_negatives, `${x.precision_pct}%`, `${x.recall_pct}%`, x.over_deductions_found, x.under_deductions_found, x.lines_where_platform_expected_fee_differs_from_the_benchmarks_own_calculation]));
}
const s5 = r.s5_first_payment;
if (s5) {
  L.push("## 5. Developer time to first payment (local)", "");
  table(["Stage", "Seconds"], [...Object.entries(s5.stages_seconds).map(([k, v]) => [k.replaceAll("_", " "), v]), ["TOTAL to first captured payment", s5.total_seconds_to_first_captured_payment], ["Total excluding compile", s5.total_seconds_excluding_compile]]);
}
const w = r.s5_checkout_weight;
if (w && w.curl_accounting) {
  L.push("## 5b. Checkout page weight (local)", "");
  const c = w.curl_accounting;
  const rows = [["curl: HTML + static files, transferred", kb(c.html_js_css_font_transferred_bytes)], ["curl: decoded", kb(c.html_js_css_font_decoded_bytes)], ["curl: JS / CSS / font / HTML transferred", ["js", "css", "font", "html"].map((k) => kb(c.by_kind_transferred_bytes[k])).join(" / ")]];
  const b = w.headless_browser;
  if (b?.unthrottled && !b.unthrottled.skipped) rows.push(["browser: all requests, transferred", `${kb(b.unthrottled.transferred_bytes_total)} in ${b.unthrottled.requests} requests`], ["browser: load event, no throttling", ms(b.unthrottled.load_event_ms)], ["browser: load event, throttled (" + b.throttled_profile.name + ")", ms(b.throttled?.load_event_ms)]);
  else rows.push(["browser", b?.note ?? "not run"]);
  table(["Measure", "Value"], rows);
}
writeFileSync(`${dir}/results.md`, L.join("\n"));
console.log(L.join("\n"));
