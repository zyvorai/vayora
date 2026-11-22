#!/usr/bin/env node
// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { serveStdio } from "@modelcontextprotocol/server/stdio";
import { Zyvor } from "@zyvor/node";
import { ConfigError, describeConfig, loadConfig } from "./config.js";
import { FORBIDDEN_WRITE_SCOPES, createServer, makeWriteGuard } from "./server.js";

// stdout is the protocol channel: everything a person should read goes to stderr, and never includes the key.
async function main() {
  const config = loadConfig();
  const client = new Zyvor({ apiKey: config.apiKey, baseUrl: config.baseUrl, actor: config.actor });
  if (config.writes) {
    try {
      await makeWriteGuard(client, config)();
    } catch (e) {
      console.error(`Refusing to start with write tools on: ${(e as Error).message}`);
      process.exit(1);
    }
  } else {
    try {
      const me = await client.me();
      const risky = me.scopes.filter((s) => (FORBIDDEN_WRITE_SCOPES as readonly string[]).includes(s));
      if (risky.length) console.error(`Warning: the API key holds ${risky.join(", ")}. This server is read-only, but a key with only payments:read and recon:read is safer.`);
    } catch {
      console.error("Warning: could not reach the Vayora API at start-up; tools will report errors until it is reachable.");
    }
  }
  console.error(describeConfig(config));
  serveStdio(() => createServer({ config, client }), { onerror: (e) => console.error(`mcp: ${e.message}`) });
}

main().catch((e) => {
  console.error(e instanceof ConfigError ? `Configuration error: ${e.message}` : "Failed to start the Vayora MCP server.");
  process.exit(1);
});
