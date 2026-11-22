// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

// All configuration comes from the environment of the process that starts the server (the MCP client's `env` block).
// There is deliberately no tool argument, and no log line, that carries the API key.

export class ConfigError extends Error {}

export interface McpConfig {
  /** From ZYVOR_API_KEY. Never printed, never returned by a tool. */
  readonly apiKey: string;
  readonly baseUrl: string;
  /** Name recorded in the audit trail for what the agent does. Not a secret and not trusted by the API on its own. */
  readonly actor: string;
  /** ZYVOR_MCP_ENABLE_WRITES: register the write tools at all. Off unless set. */
  readonly writes: boolean;
  /** ZYVOR_MCP_ALLOW_LIVE_WRITES: a live key may use the write tools. Off unless set. */
  readonly allowLiveWrites: boolean;
  /** ZYVOR_MCP_MAX_LINK_MINOR: largest payment link the agent may create, in paise. */
  readonly maxLinkMinor: number;
  /** ZYVOR_MCP_MAX_UNAPPROVED_REFUND_MINOR: a refund that the API would start without a second person's approval is refused above this. Default 0. */
  readonly maxUnapprovedRefundMinor: number;
  /** ZYVOR_MCP_SHOW_NARRATIONS: show (redacted) bank narrations. Off unless set. */
  readonly showNarrations: boolean;
}

const truthy = (v: string | undefined) => v !== undefined && /^(1|true|yes|on)$/i.test(v.trim());

function intEnv(env: NodeJS.ProcessEnv, name: string, def: number, min: number, max: number): number {
  const raw = env[name];
  if (raw === undefined || raw.trim() === "") return def;
  if (!/^\d{1,15}$/.test(raw.trim())) throw new ConfigError(`${name} must be a whole number of paise`);
  const n = Number(raw.trim());
  if (n < min || n > max) throw new ConfigError(`${name} must be between ${min} and ${max}`);
  return n;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): McpConfig {
  const apiKey = env.ZYVOR_API_KEY?.trim();
  if (!apiKey) throw new ConfigError("ZYVOR_API_KEY is not set. Put it in the env block of your MCP client's server entry.");
  if (!/^zk_(test|live)_[A-Za-z0-9]+$/.test(apiKey)) throw new ConfigError("ZYVOR_API_KEY does not look like a Vayora key (zk_test_... or zk_live_...)");
  const baseUrl = (env.ZYVOR_API_URL?.trim() || "http://localhost:8080").replace(/\/$/, "");
  try {
    const u = new URL(baseUrl);
    if (u.protocol !== "https:" && !/^(localhost|127\.0\.0\.1|\[::1\])$/.test(u.hostname)) throw new ConfigError("ZYVOR_API_URL must be https (plain http is accepted only for localhost)");
  } catch (e) {
    if (e instanceof ConfigError) throw e;
    throw new ConfigError("ZYVOR_API_URL is not a valid URL");
  }
  const actor = (env.ZYVOR_ACTOR?.trim() || "mcp-agent").slice(0, 64);
  return {
    apiKey,
    baseUrl,
    actor,
    writes: truthy(env.ZYVOR_MCP_ENABLE_WRITES),
    allowLiveWrites: truthy(env.ZYVOR_MCP_ALLOW_LIVE_WRITES),
    maxLinkMinor: intEnv(env, "ZYVOR_MCP_MAX_LINK_MINOR", 1_000_000, 100, 100_000_000),
    maxUnapprovedRefundMinor: intEnv(env, "ZYVOR_MCP_MAX_UNAPPROVED_REFUND_MINOR", 0, 0, 100_000_000),
    showNarrations: truthy(env.ZYVOR_MCP_SHOW_NARRATIONS),
  };
}

/** What may be said about the configuration in a log or a tool result: never the key. */
export function describeConfig(c: McpConfig): string {
  const env = c.apiKey.startsWith("zk_live_") ? "live" : "test";
  return `Vayora MCP: ${c.baseUrl}, ${env} key, ${c.writes ? "write tools ON" : "read-only"}, actor ${c.actor}`;
}
