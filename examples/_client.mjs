// Shared by the examples: a client for the sandbox the scripts talk to.
//   ZYVOR_API_KEY=zk_test_... ZYVOR_API_URL=http://localhost:8080 node examples/campus-fees.mjs
// (scripts/zyvor-sandbox.sh prints a key). Build the SDK first: pnpm --filter @zyvor/node build
import { Zyvor } from "../packages/zyvor-node/dist/index.js";

export function client() {
  const apiKey = process.env.ZYVOR_API_KEY;
  if (!apiKey) throw new Error("set ZYVOR_API_KEY (the sandbox script prints one)");
  return new Zyvor({ apiKey, baseUrl: process.env.ZYVOR_API_URL ?? "http://localhost:8080", actor: "example-script" });
}

/** A short unique suffix so the example can be run again without clashing with its own earlier records. */
export const run = Date.now().toString(36);

export const rupees = (minor) => `₹${(minor / 100).toLocaleString("en-IN", { minimumFractionDigits: 2 })}`;
export const say = (title, value) => console.log(`\n== ${title}\n${typeof value === "string" ? value : JSON.stringify(value, null, 2)}`);
