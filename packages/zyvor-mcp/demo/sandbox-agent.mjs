#!/usr/bin/env node
// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Sandbox agent demo: a SCRIPTED agent (no language model is involved) uses the Vayora MCP server over stdio against
// the local sandbox to create a payment link and explain a settlement. Everything is synthetic and nothing moves money.
//
//   OWNER_URL=postgres://you@localhost:5432/zyvor_sandbox ./scripts/zyvor-sandbox.sh      # prints a test key
//   pnpm --filter @zyvor/mcp build
//   ZYVOR_ADMIN_API_KEY=zk_test_... pnpm --filter @zyvor/mcp demo
//
// The admin key is used for set-up only (it mints a narrow key and imports one made-up settlement). The MCP server
// is started with the narrow key, which cannot approve or administer anything, exactly as you should run it.
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import { Zyvor } from "@zyvor/node";
import { fileURLToPath } from "node:url";

const base = (process.env.ZYVOR_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
const adminKey = process.env.ZYVOR_ADMIN_API_KEY;
if (!adminKey?.startsWith("zk_test_")) {
  console.error("Set ZYVOR_ADMIN_API_KEY to a TEST key (zk_test_...) from the sandbox. This demo refuses anything else.");
  process.exit(1);
}
if (!/^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/.test(base)) {
  console.error("This demo only runs against a local sandbox (localhost). Refusing.");
  process.exit(1);
}

const cli = fileURLToPath(new URL("../dist/cli.js", import.meta.url));
const run = Date.now().toString(36);
const say = (title, v) => console.log(`\n== ${title}\n${typeof v === "string" ? v : JSON.stringify(v, null, 2)}`);
const admin = new Zyvor({ apiKey: adminKey, baseUrl: base, actor: "demo-setup" });

// 1. Set-up (admin key): a narrow key for the agent, and one made-up settlement for it to explain.
const narrow = await admin.keys.create({
  name: `mcp-demo-${run}`,
  scopes: ["payments:read", "payments:write", "refunds:write", "recon:read", "recon:write"],
  expiresAt: new Date(Date.now() + 3600_000).toISOString(),
});
const imported = await fetch(`${base}/v1/settlements/import`, {
  method: "POST",
  headers: { authorization: `Bearer ${adminKey}`, "content-type": "application/json", "idempotency-key": `mcp-demo-${run}` },
  body: JSON.stringify({
    provider: "cashfree",
    provider_ref: `synthetic-${run}`,
    utr: `SYNTH${run.toUpperCase()}`,
    settled_on: new Date().toISOString().slice(0, 10),
    net_minor: 48200,
    items: [
      { kind: "payment", provider_ref: `synthetic-pay-${run}`, amount_minor: 50000 },
      { kind: "fee", amount_minor: -1500, description: "Gateway fee (synthetic)" },
      { kind: "fee", amount_minor: -300, description: "GST on fee (synthetic)" },
    ],
  }),
}).then((r) => r.json());
const settlementId = imported.settlement?.id;
if (!settlementId) throw new Error(`could not import the synthetic settlement: ${JSON.stringify(imported)}`);
say("Set-up", `narrow key scopes: ${narrow.key ? "payments:read, payments:write, refunds:write, recon:read, recon:write (no approve, no admin)" : "?"}\nsynthetic settlement: ${settlementId}`);

const connect = async (writes) => {
  const transport = new StdioClientTransport({
    command: process.execPath,
    args: [cli],
    env: { PATH: process.env.PATH ?? "", ZYVOR_API_KEY: narrow.key, ZYVOR_API_URL: base, ZYVOR_ACTOR: "mcp-demo-agent", ...(writes ? { ZYVOR_MCP_ENABLE_WRITES: "1" } : {}) },
    stderr: "inherit",
  });
  const client = new Client({ name: "sandbox-agent-demo", version: "0.1.0" });
  await client.connect(transport);
  return client;
};
const call = async (client, name, args) => {
  const r = await client.callTool({ name, arguments: args });
  const text = r.content[0].text;
  say(`${name} ${JSON.stringify(args)}${r.isError ? "  (refused or failed)" : ""}`, r.isError ? text : JSON.parse(text).data);
  return r.isError ? undefined : JSON.parse(text).data;
};

try {
  // 2. Default mode: read-only. The write tools are not even listed.
  const ro = await connect(false);
  say("Read-only server: tools offered", (await ro.listTools()).tools.map((t) => t.name).join(", "));
  await ro.close();

  // 3. With the flag on, the agent creates a link and explains the settlement.
  const agent = await connect(true);
  const names = (await agent.listTools()).tools.map((t) => t.name);
  say("Write-enabled server: extra tools", names.filter((n) => ["create_payment_link", "request_refund", "confirm_bank_match", "reject_bank_match"].includes(n)).join(", "));
  await call(agent, "whoami", {});
  const link = await call(agent, "create_payment_link", { amount_minor: 49900, description: `Demo: sample fee ${run}`, max_uses: 1 });
  const ex = await call(agent, "explain_settlement", { settlement_id: settlementId });
  if (ex) say("What the agent would tell the user", ex.narrative);

  // 4. A write the agent cannot make by accident: the id must be repeated in `confirm`.
  await call(agent, "request_refund", { intent_id: "pi_demo_not_real", amount_minor: 1000, reason: "demo", confirm: "something else" });
  await agent.close();
  say("Done", `payment link ${link?.id ?? "?"} created; settlement explained. Synthetic data, test key, no real money.`);
} finally {
  await admin.keys.revoke(narrow.id).catch(() => {});
}
