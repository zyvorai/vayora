import { createHmac } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { run, rupeesToMinor } from "./cli";
import { Zyvor, ZyvorError, verifyWebhook } from "./index";

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
const client = (f: unknown) => new Zyvor({ apiKey: "zk_test_x", actor: "priya", fetch: f as typeof fetch });

describe("client", () => {
  it("sends auth, actor and an idempotency key, in minor units", async () => {
    const f = vi.fn(async () => json(201, { id: "pi_1" }));
    await client(f).intents.create({ amountMinor: 49900, externalRef: "FEE-1" });
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    const h = init.headers as Record<string, string>;
    expect(url).toBe("http://localhost:8080/v1/payment_intents");
    expect(h.authorization).toBe("Bearer zk_test_x");
    expect(h["zyvor-actor"]).toBe("priya");
    expect(h["idempotency-key"]).toMatch(/[0-9a-f-]{36}/);
    expect(JSON.parse(init.body as string)).toMatchObject({ amount_minor: 49900, currency: "INR", external_ref: "FEE-1" });
  });

  it("reuses a caller-supplied idempotency key so retries are safe", async () => {
    const f = vi.fn(async () => json(201, {}));
    await client(f).intents.refund("pi_1", { amountMinor: 100, idempotencyKey: "ret-7" });
    expect(((f.mock.calls[0] as unknown as [string, RequestInit])[1].headers as Record<string, string>)["idempotency-key"]).toBe("ret-7");
  });

  it("rejects bad amounts before any request", () => {
    const f = vi.fn();
    expect(() => client(f).intents.create({ amountMinor: 10.5 })).toThrow(RangeError);
    expect(() => client(f).intents.create({ amountMinor: 0 })).toThrow(RangeError);
    expect(f).not.toHaveBeenCalled();
  });

  it("turns API errors into ZyvorError", async () => {
    const f = vi.fn(async () => json(409, { error: { code: "payment_outcome_unresolved", message: "resolve first" } }));
    await expect(client(f).intents.attempt("pi_1", { method: "upi" })).rejects.toMatchObject({ status: 409, code: "payment_outcome_unresolved" });
    expect(ZyvorError).toBeDefined();
  });

  it("builds list queries", async () => {
    const f = vi.fn(async () => json(200, { data: [] }));
    await client(f).refunds.list({ status: "failed" });
    await client(f).cases.list({ status: "open", kind: "fee_variance" });
    expect((f.mock.calls[0] as unknown as [string])[0]).toBe("http://localhost:8080/v1/refunds?status=failed");
    expect((f.mock.calls[1] as unknown as [string])[0]).toBe("http://localhost:8080/v1/recon/cases?status=open&kind=fee_variance");
  });
});

describe("verifyWebhook", () => {
  const secret = "whsec";
  const body = '{"type":"payment.captured"}';
  const sig = (t: number, b = body) => `t=${t},v1=${createHmac("sha256", secret).update(`${t}.${b}`).digest("hex")}`;
  it("accepts valid, rejects tampered, wrong-secret and stale", () => {
    expect(verifyWebhook({ secret, header: sig(1000), rawBody: body, nowSeconds: 1100 })).toBe(true);
    expect(verifyWebhook({ secret, header: sig(1000), rawBody: body + " ", nowSeconds: 1000 })).toBe(false);
    expect(verifyWebhook({ secret: "x", header: sig(1000), rawBody: body, nowSeconds: 1000 })).toBe(false);
    expect(verifyWebhook({ secret, header: sig(1000), rawBody: body, nowSeconds: 9000 })).toBe(false);
    expect(verifyWebhook({ secret, header: "junk", rawBody: body })).toBe(false);
  });
});

describe("cli", () => {
  it("parses rupees exactly", () => {
    expect(rupeesToMinor("499")).toBe(49900);
    expect(rupeesToMinor("499.5")).toBe(49950);
    for (const bad of ["", "1e3", "12.345", "-5", "₹5"]) expect(() => rupeesToMinor(bad)).toThrow();
  });

  it("creates an intent from rupees and approves with the actor header", async () => {
    const f = vi.fn(async () => json(201, { id: "pi_9" }));
    const lines: string[] = [];
    const code = await run(["intents", "create", "250.50", "--external-ref", "SO-1"], { ZYVOR_API_KEY: "zk_test_x", ZYVOR_ACTOR: "arjun" }, (s) => lines.push(s), f as unknown as typeof fetch);
    expect(code).toBe(0);
    expect(JSON.parse((f.mock.calls[0] as unknown as [string, RequestInit])[1].body as string).amount_minor).toBe(25050);
    expect(lines.join("")).toContain("pi_9");
    expect(await run(["intents", "create", "x"], { ZYVOR_API_KEY: "k" }, () => {}, f as unknown as typeof fetch)).toBe(1);
    expect(await run(["intents", "get", "pi_1"], {}, () => {})).toBe(2);
  });
});
