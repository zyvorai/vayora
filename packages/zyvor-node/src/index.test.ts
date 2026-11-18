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

  it("uploads a document as base64 and downloads the stored file as bytes, not JSON", async () => {
    const f = vi.fn(async (url: string) =>
      String(url).endsWith("/file") ? new Response(new Uint8Array([37, 80, 68, 70, 0, 255]), { status: 200 }) : json(201, { id: "oup_1", read_method: "text", checks: [] }),
    );
    const c = client(f);
    const up = await c.onboarding.uploads.upload({ kind: "gst_certificate", filename: "gst.pdf", data: new Uint8Array([37, 80, 68, 70]) });
    expect(up.id).toBe("oup_1");
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://localhost:8080/v1/onboarding/uploads");
    expect(JSON.parse(init.body as string)).toEqual({ kind: "gst_certificate", filename: "gst.pdf", data_base64: "JVBERg==" });
    const bytes = await c.onboarding.uploads.download("oup_1");
    expect(Array.from(bytes)).toEqual([37, 80, 68, 70, 0, 255]);
  });

  it("renames, reprices and archives a plan with a PATCH that omits what was left out", async () => {
    const f = vi.fn(async () => json(200, {}));
    await client(f).billing.updatePlan("plan_1", { amountMinor: 12500, active: false, keepExisting: true });
    const [url, init] = (f.mock.calls as unknown as [string, RequestInit][])[0];
    expect(url).toBe("http://localhost:8080/v1/plans/plan_1");
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(init.body as string)).toEqual({ amount_minor: 12500, active: false, keep_existing: true });
  });

  it("ends a free trial early or moves its end for one subscription", async () => {
    const f = vi.fn(async () => json(200, {}));
    await client(f).billing.changeTrial("sub_1", { endsOn: "2026-11-20" });
    const [url, init] = (f.mock.calls as unknown as [string, RequestInit][])[0];
    expect(url).toBe("http://localhost:8080/v1/subscriptions/sub_1/trial");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ ends_on: "2026-11-20" });
  });

  it("sets a plan's free trial when creating and editing it", async () => {
    const f = vi.fn(async () => json(200, {}));
    const c = client(f);
    await c.billing.createPlan({ name: "Pro", amountMinor: 90000, intervalUnit: "month", trialDays: 14 });
    await c.billing.updatePlan("plan_1", { trialDays: 0, includedUnits: 100 });
    const calls = f.mock.calls as unknown as [string, RequestInit][];
    expect(JSON.parse(calls[0][1].body as string).trial_days).toBe(14);
    expect(JSON.parse(calls[1][1].body as string)).toEqual({ trial_days: 0, included_units: 100 });
  });

  it("changes a subscription's plan and cancels it", async () => {
    const f = vi.fn(async () => json(200, {}));
    const c = client(f);
    await c.billing.changePlan("sub_1", { planId: "plan_2", quantity: 3 });
    await c.billing.cancel("sub_1");
    const calls = f.mock.calls as unknown as [string, RequestInit][];
    expect(calls[0][0]).toBe("http://localhost:8080/v1/subscriptions/sub_1/change_plan");
    expect(JSON.parse(calls[0][1].body as string)).toEqual({ plan_id: "plan_2", quantity: 3 });
    expect(calls[1][0]).toBe("http://localhost:8080/v1/subscriptions/sub_1/cancel");
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

describe("runs as of a test day", () => {
  it("sends as_of only when asked", async () => {
    const f = vi.fn(async () => json(200, {}));
    const c = client(f);
    await c.billing.run();
    await c.billing.run({ asOf: "2027-01-15" });
    await c.collections.runDunning({ asOf: "2027-01-15" });
    const bodies = f.mock.calls.map((x) => { const [, i] = x as unknown as [string, RequestInit]; return i.body ? JSON.parse(i.body as string) : undefined; });
    expect(bodies[0]?.as_of).toBeUndefined();
    expect(bodies[1]).toEqual({ as_of: "2027-01-15" });
    expect(bodies[2]).toMatchObject({ as_of: "2027-01-15" });
  });
});

describe("payment links, dunning and key rotation", () => {
  it("maps camelCase options to the API's snake_case and the right routes", async () => {
    const f = vi.fn(async () => json(200, {}));
    const c = client(f);
    await c.paymentLinks.create({ description: "Term fee", amountMinor: 125000, maxUses: 40 });
    await c.paymentLinks.cancel("plk_1");
    await c.invoices.send("inv_1");
    await c.collections.runDunning({ channel: "sms", repeatAfterDays: 3 });
    await c.keys.rotate("key_1", 2);
    const seen = f.mock.calls.map((x) => { const [u, i] = x as unknown as [string, RequestInit]; return [i.method, u.replace("http://localhost:8080", ""), i.body ? JSON.parse(i.body as string) : undefined]; });
    expect(seen.map((x) => x.slice(0, 2))).toEqual([
      ["POST", "/v1/payment_links"], ["POST", "/v1/payment_links/plk_1/cancel"], ["POST", "/v1/invoices/inv_1/send"], ["POST", "/v1/collections/dunning/run"], ["POST", "/v1/api_keys/key_1/rotate"],
    ]);
    expect(seen[0][2]).toMatchObject({ description: "Term fee", amount_minor: 125000, max_uses: 40 });
    expect(seen[3][2]).toMatchObject({ channel: "sms", repeat_after_days: 3 });
    expect(seen[4][2]).toEqual({ overlap_hours: 2 });
  });
});

describe("mandates", () => {
  it("maps options to the API's routes and body, with idempotency keys on the writes that need them", async () => {
    const f = vi.fn(async () => json(200, {}));
    const c = client(f);
    await c.mandates.create({ customerId: "cus_1", maxAmountMinor: 500000, frequency: "monthly", endDate: "2027-10-31", idempotencyKey: "m1" });
    await c.mandates.list({ status: "active" });
    await c.mandates.pause("mnd_1");
    await c.mandates.scheduleDebit("mnd_1", { amountMinor: 125000, dueDate: "2026-11-30", idempotencyKey: "d1" });
    const seen = f.mock.calls.map((x) => { const [u, i] = x as unknown as [string, RequestInit]; return [i.method, u.replace("http://localhost:8080", ""), i.body ? JSON.parse(i.body as string) : undefined, (i.headers as Record<string, string>)["idempotency-key"]]; });
    expect(seen.map((x) => [x[0], x[1], x[3]])).toEqual([
      ["POST", "/v1/mandates", "m1"], ["GET", "/v1/mandates?status=active", undefined], ["POST", "/v1/mandates/mnd_1/pause", undefined], ["POST", "/v1/mandates/mnd_1/debits", "d1"],
    ]);
    expect(seen[0][2]).toMatchObject({ customer_id: "cus_1", max_amount_minor: 500000, frequency: "monthly", end_date: "2027-10-31" });
    expect(seen[3][2]).toMatchObject({ amount_minor: 125000, due_date: "2026-11-30" });
  });
  it("rejects a non-positive limit before any request", () => {
    const f = vi.fn();
    expect(() => client(f).mandates.create({ customerId: "c", maxAmountMinor: 0, frequency: "daily", endDate: "2027-01-01" })).toThrow(RangeError);
    expect(f).not.toHaveBeenCalled();
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

describe("signed actor", () => {
  it("signs the actor when given the API's actor secret, and not otherwise", async () => {
    const seen: Record<string, string>[] = [];
    const fetcher = (async (_u: string, init: { headers: Record<string, string> }) => {
      seen.push(init.headers);
      return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
    }) as unknown as typeof fetch;
    await new Zyvor({ apiKey: "zk_test_x", actor: "priya", fetch: fetcher }).branches.list();
    await new Zyvor({ apiKey: "zk_test_x", actor: "priya", actorSecret: "console-actor-secret-0123456789", fetch: fetcher }).branches.list();
    expect(seen[0]["zyvor-actor-signature"]).toBeUndefined();
    const m = /^t=(\d+),v1=([0-9a-f]{64})$/.exec(seen[1]["zyvor-actor-signature"]);
    expect(m).not.toBeNull();
    const { createHmac } = await import("node:crypto");
    expect(m![2]).toBe(createHmac("sha256", "console-actor-secret-0123456789").update(`${m![1]}.priya`).digest("hex"));
  });
});

describe("fee checks, bank statements and routing health", () => {
  it("calls the right verb, path and body", async () => {
    const f = vi.fn(async () => json(200, { ok: true }));
    const c = client(f);
    await c.feeRules.list();
    await c.feeRules.create({ provider: "razorpay", method: "upi", bps: 200, effectiveFrom: "2026-04-01", gstBps: 1800 });
    await c.feeRules.remove("rule_1");
    await c.feeChecks.settings();
    await c.feeChecks.updateSettings({ toleranceMinor: 100 });
    await c.feeChecks.summary();
    await c.feeChecks.findings({ status: "open", provider: "razorpay", limit: 5 });
    await c.feeChecks.run({ since: "2026-04-01" });
    await c.feeChecks.settlement("set_1");
    await c.feeChecks.recheckSettlement("set_1");
    await c.bankStatements.preview("a,b", { layout: "generic", dateFormat: "%d/%m/%Y" });
    await c.bankStatements.importCsv("a,b");
    await c.bankStatements.layouts();
    await c.bankStatements.imports();
    await c.bankStatements.lines({ status: "unmatched", from: "2026-04-01", importId: 3 });
    await c.bankStatements.line(7);
    await c.bankStatements.rematch();
    await c.bankStatements.confirmMatch(9, "ok");
    await c.bankStatements.rejectMatch(9);
    await c.cases.ageing();
    await c.routing.health("upi");
    await c.routing.healthSettings();
    await c.routing.updateHealthSettings({ minSamples: 20, failoverEnabled: false });
    const seen = (f.mock.calls as unknown as [string, RequestInit][]).map(([u, i]) => [i.method, u.replace("http://localhost:8080", ""), i.body ? JSON.parse(i.body as string) : undefined]);
    expect(seen).toEqual([
      ["GET", "/v1/fee_rules", undefined],
      ["POST", "/v1/fee_rules", { provider: "razorpay", method: "upi", bps: 200, effective_from: "2026-04-01", gst_bps: 1800 }],
      ["DELETE", "/v1/fee_rules/rule_1", undefined],
      ["GET", "/v1/fee_checks/settings", undefined],
      ["POST", "/v1/fee_checks/settings", { tolerance_minor: 100 }],
      ["GET", "/v1/fee_checks/summary", undefined],
      ["GET", "/v1/fee_checks/findings?status=open&provider=razorpay&limit=5", undefined],
      ["POST", "/v1/fee_checks/run", { since: "2026-04-01" }],
      ["GET", "/v1/settlements/set_1/fee_check", undefined],
      ["POST", "/v1/settlements/set_1/fee_check", {}],
      ["POST", "/v1/bank_statements/preview", { csv: "a,b", layout: "generic", date_format: "%d/%m/%Y" }],
      ["POST", "/v1/bank_statements/import_csv", { csv: "a,b" }],
      ["GET", "/v1/bank_statements/layouts", undefined],
      ["GET", "/v1/bank_statements/imports", undefined],
      ["GET", "/v1/bank_statements/lines?status=unmatched&from=2026-04-01&import_id=3", undefined],
      ["GET", "/v1/bank_statements/lines/7", undefined],
      ["POST", "/v1/bank_statements/rematch", {}],
      ["POST", "/v1/bank_statements/matches/9/confirm", { note: "ok" }],
      ["POST", "/v1/bank_statements/matches/9/reject", {}],
      ["GET", "/v1/recon/ageing", undefined],
      ["GET", "/v1/routing/health?method=upi", undefined],
      ["GET", "/v1/routing/health/settings", undefined],
      ["PUT", "/v1/routing/health/settings", { min_samples: 20, failover_enabled: false }],
    ]);
  });

  it("raises ZyvorError with the API's code", async () => {
    const f = vi.fn(async () => json(403, { error: { code: "forbidden", message: "admin scope" } }));
    await expect(client(f).routing.updateHealthSettings({ breakerEnabled: false })).rejects.toMatchObject({ status: 403, code: "forbidden" });
  });

  it("runs the new CLI commands", async () => {
    const f = vi.fn(async () => json(200, { data: [] }));
    const out: string[] = [];
    const env = { ZYVOR_API_KEY: "k" };
    for (const argv of [["recon", "ageing"], ["fee-rules", "list"], ["fee-checks", "summary"], ["fee-checks", "run", "--since", "2026-04-01"], ["bank", "lines", "--status", "unmatched"], ["routing", "health", "--method", "upi"]]) {
      expect(await run(argv, env, (s) => out.push(s), f as unknown as typeof fetch)).toBe(0);
    }
    expect((f.mock.calls as unknown as [string][]).map((c) => c[0].replace("http://localhost:8080", ""))).toEqual(["/v1/recon/ageing", "/v1/fee_rules", "/v1/fee_checks/summary", "/v1/fee_checks/run", "/v1/bank_statements/lines?status=unmatched", "/v1/routing/health?method=upi"]);
  });
});
