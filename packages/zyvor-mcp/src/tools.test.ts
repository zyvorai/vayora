// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as z from "zod/v4";
import { describe, expect, it } from "vitest";
import { ALL_TOOLS, READ_TOOLS, WRITE_TOOLS, fullDescription } from "./tools";

describe("every tool definition", () => {
  it("has a unique snake_case name and exactly one kind", () => {
    const names = ALL_TOOLS.map((t) => t.name);
    expect(new Set(names).size).toBe(names.length);
    for (const t of ALL_TOOLS) expect(t.name).toMatch(/^[a-z][a-z0-9_]{2,40}$/);
    expect(READ_TOOLS.every((t) => t.kind === "read")).toBe(true);
    expect(WRITE_TOOLS.every((t) => t.kind === "write")).toBe(true);
  });

  it("has an input schema that converts to JSON Schema, with every property described", () => {
    for (const t of ALL_TOOLS) {
      const js = z.toJSONSchema(t.input) as { type: string; properties?: Record<string, { description?: string }> };
      expect(js.type, t.name).toBe("object");
      for (const [k, p] of Object.entries(js.properties ?? {})) expect(p.description, `${t.name}.${k} needs a description`).toBeTruthy();
      expect(Object.keys(js.properties ?? {}).length, `${t.name} schema stays small`).toBeLessThanOrEqual(7);
    }
  });

  it("has examples that validate against its own schema", () => {
    for (const t of ALL_TOOLS) {
      expect(t.examples.length, t.name).toBeGreaterThan(0);
      for (const ex of t.examples) {
        const r = t.input.safeParse(ex);
        expect(r.success, `${t.name}: ${JSON.stringify(ex)} ${r.success ? "" : r.error.message}`).toBe(true);
      }
    }
  });

  it("rejects unknown shapes: wrong types fail validation", () => {
    const list = ALL_TOOLS.find((t) => t.name === "list_payment_intents")!;
    expect(list.input.safeParse({ limit: 500 }).success).toBe(false);
    expect(list.input.safeParse({ created_from: "yesterday" }).success).toBe(false);
    const link = ALL_TOOLS.find((t) => t.name === "create_payment_link")!;
    expect(link.input.safeParse({ amount_minor: 499.5, description: "x y z" }).success).toBe(false);
    expect(link.input.safeParse({ amount_minor: -5, description: "x y z" }).success).toBe(false);
  });

  it("documents its scope and mode in the description a model reads", () => {
    for (const t of ALL_TOOLS) {
      const d = fullDescription(t);
      expect(d, t.name).toContain(`API scope needed: ${t.scope}`);
      expect(d, t.name).toContain("Example arguments:");
      expect(d.length, t.name).toBeLessThan(1400);
      expect(d, t.name).toContain(t.kind === "read" ? "Read only" : "Write tool");
    }
  });

  it("never takes a credential, key or secret as an argument", () => {
    for (const t of ALL_TOOLS) for (const k of Object.keys(t.input.shape)) expect(k, `${t.name}.${k}`).not.toMatch(/key|secret|token|password|authorization|credential|actor/i);
  });

  it("makes every destructive tool require a confirm that its example echoes", () => {
    const destructive = ALL_TOOLS.filter((t) => t.destructive);
    expect(destructive.map((t) => t.name).sort()).toEqual(["confirm_bank_match", "reject_bank_match", "request_refund"]);
    for (const t of destructive) {
      expect(Object.keys(t.input.shape), t.name).toContain("confirm");
      const ex = t.examples[0] as Record<string, unknown>;
      const idKey = t.name === "request_refund" ? "intent_id" : "match_id";
      expect(String(ex.confirm)).toBe(String(ex[idKey]));
      expect(t.kind).toBe("write");
    }
    for (const t of READ_TOOLS) expect(Object.keys(t.input.shape), t.name).not.toContain("confirm");
  });

  it("offers nothing that manages keys, approves, or pays out", () => {
    for (const t of ALL_TOOLS) expect(t.name, t.name).not.toMatch(/key|approve|payout|rotate|webhook|setting|provider_account|vendor/);
  });

  it("is listed, with its scope, in docs/zyvor/AI_INTEGRATION.md", () => {
    const doc = readFileSync(fileURLToPath(new URL("../../../docs/zyvor/AI_INTEGRATION.md", import.meta.url)), "utf8");
    for (const t of ALL_TOOLS) {
      const row = doc.split("\n").find((l) => l.startsWith(`| \`${t.name}\``));
      expect(row, `${t.name} is missing from the docs table`).toBeTruthy();
      if (t.scope !== "any valid key") expect(row, `${t.name} scope in docs`).toContain(`\`${t.scope}\``);
    }
  });
});
