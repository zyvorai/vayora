// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, expect, it } from "vitest";
import { ConfigError, describeConfig, loadConfig } from "./config";

const KEY = "zk_test_" + "ab".repeat(32);

describe("config comes from the environment only", () => {
  it("needs a key that looks like a Vayora key", () => {
    expect(() => loadConfig({})).toThrow(ConfigError);
    expect(() => loadConfig({ ZYVOR_API_KEY: "sk_live_nope" })).toThrow(/does not look like/);
  });
  it("is read-only unless the flag is set, and the flag must be explicit", () => {
    expect(loadConfig({ ZYVOR_API_KEY: KEY }).writes).toBe(false);
    expect(loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_MCP_ENABLE_WRITES: "0" }).writes).toBe(false);
    expect(loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_MCP_ENABLE_WRITES: "maybe" }).writes).toBe(false);
    expect(loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_MCP_ENABLE_WRITES: "1" }).writes).toBe(true);
    expect(loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_MCP_ENABLE_WRITES: "true" }).writes).toBe(true);
  });
  it("defaults to the local sandbox and refuses plain http elsewhere", () => {
    expect(loadConfig({ ZYVOR_API_KEY: KEY }).baseUrl).toBe("http://localhost:8080");
    expect(() => loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_API_URL: "http://api.example.com" })).toThrow(/https/);
    expect(loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_API_URL: "https://api.example.com/" }).baseUrl).toBe("https://api.example.com");
  });
  it("validates the limits", () => {
    expect(() => loadConfig({ ZYVOR_API_KEY: KEY, ZYVOR_MCP_MAX_LINK_MINOR: "ten" })).toThrow(/whole number/);
    expect(loadConfig({ ZYVOR_API_KEY: KEY }).maxUnapprovedRefundMinor).toBe(0);
  });
  it("never prints the key", () => {
    expect(describeConfig(loadConfig({ ZYVOR_API_KEY: KEY }))).not.toContain(KEY);
  });
});
