// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Spawns the built server (dist/cli.js) as a real child process, as an MCP client would, against a small HTTP stand-in
// for the Vayora API. Skipped when the package has not been built (`pnpm build` first; `pnpm -r build` does it).
import { createServer as createHttp, type Server } from "node:http";
import { existsSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const cli = fileURLToPath(new URL("../dist/cli.js", import.meta.url));
const built = existsSync(cli);
const KEY = "zk_test_" + "ef".repeat(32);

let api: Server;
let url = "";
const seen: { url: string; auth?: string }[] = [];

beforeAll(async () => {
  api = createHttp((req, res) => {
    seen.push({ url: req.url ?? "", auth: req.headers.authorization });
    res.setHeader("content-type", "application/json");
    if (req.url === "/v1/me") res.end(JSON.stringify({ tenant_id: "t", env: "test", key_id: "k", actor: "mcp-agent", scopes: ["payments:read", "recon:read"], branch_id: null }));
    else if (req.url?.startsWith("/v1/payment_intents")) res.end(JSON.stringify({ data: [{ id: "pi_1", amount_minor: 100, status: "succeeded", description: "A\nB" }] }));
    else {
      res.statusCode = 404;
      res.end(JSON.stringify({ error: { code: "not_found", message: "nope" } }));
    }
  });
  await new Promise<void>((r) => api.listen(0, "127.0.0.1", r));
  url = `http://127.0.0.1:${(api.address() as { port: number }).port}`;
});
afterAll(() => new Promise<void>((r) => api.close(() => r())));

describe.skipIf(!built)("the built server over stdio", () => {
  it("starts, lists only read tools, and answers a call with the key from its environment", async () => {
    const transport = new StdioClientTransport({ command: process.execPath, args: [cli], env: { PATH: process.env.PATH ?? "", ZYVOR_API_KEY: KEY, ZYVOR_API_URL: url }, stderr: "pipe" });
    const client = new Client({ name: "stdio-test", version: "0" });
    await client.connect(transport);
    try {
      const tools = (await client.listTools()).tools.map((t) => t.name);
      expect(tools).toContain("explain_settlement");
      expect(tools).not.toContain("request_refund");
      const r = await client.callTool({ name: "list_payment_intents", arguments: { limit: 3 } });
      expect(JSON.parse((r.content as { text: string }[])[0].text).data.data[0].description).toBe("A B");
      expect(seen.some((s) => s.auth === `Bearer ${KEY}`)).toBe(true);
    } finally {
      await client.close();
    }
  });

  it("exits with a clear message and no key in it when the key is missing", () => {
    const r = spawnSync(process.execPath, [cli], { env: { PATH: process.env.PATH ?? "" }, encoding: "utf8", timeout: 10_000 });
    expect(r.status).toBe(1);
    expect(r.stderr).toContain("ZYVOR_API_KEY is not set");
  });

  it("refuses to start with writes on when the key can approve", async () => {
    const approver = createHttp((_q, res) => {
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ tenant_id: "t", env: "test", key_id: "k", actor: "a", scopes: ["admin"], branch_id: null }));
    });
    await new Promise<void>((r) => approver.listen(0, "127.0.0.1", r));
    const u = `http://127.0.0.1:${(approver.address() as { port: number }).port}`;
    // async spawn: the stand-in API lives in this process, so the event loop must stay free to answer it
    const r = await new Promise<{ status: number | null; stderr: string }>((resolve) => {
      const child = spawn(process.execPath, [cli], { env: { PATH: process.env.PATH ?? "", ZYVOR_API_KEY: KEY, ZYVOR_API_URL: u, ZYVOR_MCP_ENABLE_WRITES: "1" } });
      let stderr = "";
      child.stderr.on("data", (d) => (stderr += d));
      child.on("close", (status) => resolve({ status, stderr }));
    });
    approver.close();
    expect(r.status).toBe(1);
    expect(r.stderr).toContain("Refusing to start");
    expect(r.stderr).not.toContain(KEY);
  });
});
