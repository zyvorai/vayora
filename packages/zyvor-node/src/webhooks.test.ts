// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { createHmac } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { MAX_V1_ENTRIES, WebhookReplayGuard, Zyvor, verifyWebhook, verifyWebhookDetailed } from "./index";

const secret = "whsec_test_vector";
const mac = (s: string, t: number, b: string) => createHmac("sha256", s).update(`${t}.${b}`).digest("hex");
const body = JSON.stringify({ id: "evt_test_1", type: "payment.captured", amount_minor: 50000 });
const header = (t: number, b = body, s = secret) => `t=${t},v1=${mac(s, t, b)}`;

describe("verifyWebhookDetailed", () => {
  it("matches the signature vector the Rust signer is tested against (zyvor-api outbox::tests)", () => {
    expect(mac(secret, 1_700_000_000, body)).toBe("b7d274b460eaf81fcd9b75301d5db186d1062bbd2402f6606d0e144a56709030");
    const h = `t=1700000000,v1=b7d274b460eaf81fcd9b75301d5db186d1062bbd2402f6606d0e144a56709030`;
    expect(verifyWebhookDetailed({ secret, header: h, rawBody: body, nowSeconds: 1_700_000_100 })).toEqual({ ok: true, timestamp: 1_700_000_000 });
    expect(verifyWebhook({ secret, header: h, rawBody: body, nowSeconds: 1_700_000_100 })).toBe(true);
  });

  it("names the reason", () => {
    const base = { secret, rawBody: body, nowSeconds: 1000 };
    expect(verifyWebhookDetailed({ ...base, header: header(1000) }).ok).toBe(true);
    expect(verifyWebhookDetailed({ ...base, header: header(1000), rawBody: body + " " })).toEqual({ ok: false, reason: "signature_mismatch" });
    expect(verifyWebhookDetailed({ ...base, header: header(1000, body, "other") })).toEqual({ ok: false, reason: "signature_mismatch" });
    expect(verifyWebhookDetailed({ ...base, header: header(1000), nowSeconds: 1301 })).toEqual({ ok: false, reason: "timestamp_outside_tolerance" });
    expect(verifyWebhookDetailed({ ...base, header: header(1000), nowSeconds: 1300 }).ok).toBe(true);
    // a timestamp from the future is as stale as one from the past
    expect(verifyWebhookDetailed({ ...base, header: header(2000) })).toEqual({ ok: false, reason: "timestamp_outside_tolerance" });
    for (const bad of ["", "junk", "t=1000", `v1=${mac(secret, 1000, body)}`, "t=,v1=ab", `t=1e3,v1=${mac(secret, 1000, body)}`, `t=-5,v1=${mac(secret, -5, body)}`, `t=1000,v1=zz`, `t=1000,v1=abc`]) {
      expect(verifyWebhookDetailed({ ...base, header: bad }), bad).toEqual({ ok: false, reason: "malformed_header" });
    }
  });

  it("accepts a header carrying two signatures when either secret matches (rotation) and ignores other schemes", () => {
    const rotated = `t=1000,v1=${mac("old", 1000, body)},v1=${mac(secret, 1000, body)},v0=${"00".repeat(32)}`;
    expect(verifyWebhookDetailed({ secret, header: rotated, rawBody: body, nowSeconds: 1000 }).ok).toBe(true);
    expect(verifyWebhookDetailed({ secret: "old", header: rotated, rawBody: body, nowSeconds: 1000 }).ok).toBe(true);
    expect(verifyWebhookDetailed({ secret: "neither", header: rotated, rawBody: body, nowSeconds: 1000 })).toEqual({ ok: false, reason: "signature_mismatch" });
    // the old verifier keeps its documented behaviour: the last v1 entry is the one that counts
    expect(verifyWebhook({ secret: "old", header: rotated, rawBody: body, nowSeconds: 1000 })).toBe(false);
  });

  it("refuses hostile headers cheaply", () => {
    const now = { secret, rawBody: body, nowSeconds: 1000 };
    expect(verifyWebhookDetailed({ ...now, header: "a=b,".repeat(5000) })).toEqual({ ok: false, reason: "malformed_header" });
    const many = `t=1000,${Array.from({ length: MAX_V1_ENTRIES + 1 }, () => `v1=${mac("x", 1000, body)}`).join(",")},v1=${mac(secret, 1000, body)}`;
    expect(verifyWebhookDetailed({ ...now, header: many })).toEqual({ ok: false, reason: "malformed_header" });
    expect(verifyWebhookDetailed({ ...now, header: `t=1000,t=1000,v1=${mac(secret, 1000, body)}` })).toEqual({ ok: false, reason: "malformed_header" });
    // a huge body is just hashed
    const big = "x".repeat(5_000_000);
    expect(verifyWebhookDetailed({ ...now, rawBody: big, header: header(1000, big) }).ok).toBe(true);
  });

  it("never lets a zero or negative tolerance switch the replay check off", () => {
    for (const tol of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() => verifyWebhookDetailed({ secret, header: header(1000), rawBody: body, toleranceSeconds: tol })).toThrow(RangeError);
    }
  });

  it("any single changed byte of the body, the timestamp or the signature is refused (deterministic fuzz)", () => {
    let x = 12345;
    const rnd = (n: number) => ((x = (x * 1103515245 + 12345) & 0x7fffffff) % n);
    for (let i = 0; i < 300; i++) {
      const b = Array.from({ length: 1 + rnd(200) }, () => String.fromCharCode(32 + rnd(90))).join("");
      const t = 1_000_000 + rnd(1000);
      const h = header(t, b);
      expect(verifyWebhookDetailed({ secret, header: h, rawBody: b, nowSeconds: t }).ok).toBe(true);
      const k = rnd(b.length);
      const tampered = b.slice(0, k) + String.fromCharCode(b.charCodeAt(k) ^ 1) + b.slice(k + 1);
      expect(verifyWebhookDetailed({ secret, header: h, rawBody: tampered, nowSeconds: t }).ok).toBe(false);
      expect(verifyWebhookDetailed({ secret, header: h.replace(`t=${t}`, `t=${t + 1}`), rawBody: b, nowSeconds: t }).ok).toBe(false);
      const junk = Array.from({ length: rnd(60) }, () => String.fromCharCode(rnd(256))).join("");
      expect(verifyWebhookDetailed({ secret, header: junk, rawBody: b, nowSeconds: t }).ok).toBe(false);
    }
  });
});

describe("WebhookReplayGuard", () => {
  it("reports a repeated event id once seen, and forgets it after the ttl", () => {
    const g = new WebhookReplayGuard({ ttlSeconds: 100 });
    expect(g.seen("evt_1", 0)).toBe(false);
    expect(g.seen("evt_1", 50)).toBe(true);
    expect(g.seen("evt_2", 60)).toBe(false);
    expect(g.seen("evt_1", 100)).toBe(false); // expired at 100, so it is new again
    expect(g.seen("evt_2", 100)).toBe(true);
  });
  it("is bounded", () => {
    const g = new WebhookReplayGuard({ maxEntries: 3 });
    for (let i = 0; i < 10; i++) g.seen(`e${i}`, 0);
    expect(g.size).toBe(3);
    expect(g.seen("e9", 0)).toBe(true);
    expect(g.seen("e0", 0)).toBe(false);
  });
  it("rejects nonsense options", () => {
    expect(() => new WebhookReplayGuard({ ttlSeconds: 0 })).toThrow(RangeError);
    expect(() => new WebhookReplayGuard({ maxEntries: 1.5 })).toThrow(RangeError);
  });
});

describe("ledger.verify", () => {
  it("calls GET /v1/ledger/verify", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ ok: true, trial_balance: [] }), { status: 200 }));
    const r = await new Zyvor({ apiKey: "zk_test_x", fetch: f as unknown as typeof fetch }).ledger.verify();
    expect(r.ok).toBe(true);
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://localhost:8080/v1/ledger/verify");
    expect(init.method).toBe("GET");
  });
});
