#!/usr/bin/env node
// Hard gates for CI (correctness of the simulator runs, not performance). Usage: node gates.mjs <results.json>
import { readFileSync } from "node:fs";
const r = JSON.parse(readFileSync(process.argv[2], "utf8"));
const maxWrong = Number(process.env.BENCH_MAX_WRONG_MATCHES ?? 0), minFee = Number(process.env.BENCH_MIN_FEE_PCT ?? 100);
const bad = [];
const a = r.s2_double_charge?.audit;
if (!a || a.intents_with_more_than_one_captured_attempt_in_platform !== 0 || a.intents_paid_more_than_once_at_provider !== 0) bad.push("double charge audit not zero");
if (!r.s2_double_charge?.pass) bad.push("double-charge scenario reported unexpected observations");
if ((r.s3_recon?.totals.auto_confirmed_wrong ?? 1) > maxWrong) bad.push(`auto-confirmed wrong matches ${r.s3_recon?.totals.auto_confirmed_wrong} > ${maxWrong}`);
if (r.s3_recon?.totals.layouts_that_failed_to_import) bad.push("a bank layout failed to import");
for (const x of r.s4_fees?.results ?? []) if (x.precision_pct < minFee || x.recall_pct < minFee) bad.push(`fee variance at tolerance ${x.tolerance_minor}: precision ${x.precision_pct}% recall ${x.recall_pct}%`);
if (bad.length) { console.error("BENCHMARK GATES FAILED:\n  " + bad.join("\n  ")); process.exit(1); }
console.log("benchmark gates passed");
