// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { createHmac } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { callJson, query, refusalText } from "./restcall";

const reply = (status: number, body: unknown) => vi.fn(async () => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } }));

describe("callJson", () => {
  it("sends the key, the JSON body and a signed actor", async () => {
    const f = reply(200, { ok: true });
    const r = await callJson<{ ok: boolean }>({ base: "http://api.test/", key: "sk_test_x", actor: "asha", actorSecret: "s3cret", fetch: f as unknown as typeof fetch, now: () => 1_700_000_000_000 }, "POST", "/v1/fee_rules", { bps: 150 });
    expect(r).toEqual({ ok: true, status: 200, body: { ok: true } });
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://api.test/v1/fee_rules");
    const h = init.headers as Record<string, string>;
    expect(h.authorization).toBe("Bearer sk_test_x");
    expect(h["content-type"]).toBe("application/json");
    expect(h["zyvor-actor"]).toBe("asha");
    const sig = createHmac("sha256", "s3cret").update("1700000000.asha").digest("hex");
    expect(h["zyvor-actor-signature"]).toBe(`t=1700000000,v1=${sig}`);
    expect(init.body).toBe(JSON.stringify({ bps: 150 }));
  });

  it("sends no body header on a GET and no signature without a secret", async () => {
    const f = reply(200, {});
    await callJson({ base: "http://api.test", key: "k", actor: "asha", fetch: f as unknown as typeof fetch }, "GET", "/v1/routing/health");
    const h = (f.mock.calls[0] as unknown as [string, RequestInit])[1].headers as Record<string, string>;
    expect(h["content-type"]).toBeUndefined();
    expect(h["zyvor-actor-signature"]).toBeUndefined();
  });

  it("turns a refusal into a value with the API's code and message", async () => {
    const f = reply(409, { error: { code: "duplicate_rule", message: "a rule already starts on that date" } });
    const r = await callJson({ base: "http://api.test", key: "k", fetch: f as unknown as typeof fetch }, "POST", "/v1/fee_rules", {});
    expect(r).toMatchObject({ ok: false, status: 409, code: "duplicate_rule", message: "a rule already starts on that date" });
  });

  it("keeps the whole body of a refusal, so line-level errors survive", async () => {
    const body = { error: { code: "invalid_statement", message: "no", errors: [{ line: 3, column: "Date", message: "bad date" }] } };
    const r = await callJson({ base: "http://api.test", key: "k", fetch: reply(422, body) as unknown as typeof fetch }, "POST", "/v1/bank_statements/import_csv", {});
    expect(!r.ok && r.body).toEqual(body);
  });

  it("says so when the API cannot be reached", async () => {
    const r = await callJson({ base: "http://api.test", key: "k", fetch: (async () => { throw new Error("down"); }) as unknown as typeof fetch }, "GET", "/x");
    expect(r).toMatchObject({ ok: false, status: 0, code: "unreachable" });
  });
});

describe("refusalText", () => {
  it("is plain for the common statuses", () => {
    expect(refusalText(403, "insufficient scope")).toBe("Your sign-in is not allowed to do that.");
    expect(refusalText(500, "boom")).toMatch(/Try again/);
    expect(refusalText(422, "tolerance_minor cannot be negative")).toBe("tolerance_minor cannot be negative");
    expect(refusalText(422, undefined)).toBe("The request was refused.");
  });
});

describe("query", () => {
  it("keeps only values that are set", () => {
    expect(query({ status: "open", provider: "", method: undefined, limit: 100 })).toBe("?status=open&limit=100");
    expect(query({ a: undefined })).toBe("");
  });
});
