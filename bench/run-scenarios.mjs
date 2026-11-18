#!/usr/bin/env node
// Runs the scenarios named on the command line (default: all) and writes one JSON file per scenario into --out.
import { writeJson, env } from "./lib/common.mjs";
const args = process.argv.slice(2);
const outDir = args[args.indexOf("--out") + 1];
const names = args.filter((a, i) => !a.startsWith("--") && args[i - 1] !== "--out");
const all = ["s1_outage", "s2_double_charge", "s3_recon", "s4_fees"];
for (const n of names.length ? names : all) {
  const t0 = Date.now();
  console.error(`== ${n}`);
  const { run } = await import(`./scenarios/${n}.mjs`);
  const result = await run();
  result.seed = env.seed;
  result.wall_seconds = Math.round((Date.now() - t0) / 100) / 10;
  await writeJson(`${outDir}/${n}.json`, result);
}
