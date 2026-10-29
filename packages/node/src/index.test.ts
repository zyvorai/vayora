import { createHmac } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { Vayora, VayoraError, verifyWebhook } from "./index";

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });

describe("client", () => {
  it("sends auth, paise and idempotency key", async () => {
    const f = vi.fn(async () => json(201, { id: "order_1" }));
    const v = new Vayora({ apiKey: "sk_test", fetch: f as unknown as typeof fetch });
    await v.orders.create({ amountPaise: 5000, idempotencyKey: "k1" });
    const [url, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    const h = init.headers as Record<string, string>;
    expect(url).toBe("http://localhost:8080/v1/orders");
    expect(h.authorization).toBe("Bearer sk_test");
    expect(h["idempotency-key"]).toBe("k1");
    expect(init.body).toBe('{"amount_paise":5000}');
  });

  it("rejects non-integer amounts before any request", () => {
    const f = vi.fn();
    const v = new Vayora({ apiKey: "k", fetch: f as unknown as typeof fetch });
    expect(() => v.orders.create({ amountPaise: 10.5, idempotencyKey: "k" })).toThrow(RangeError);
    expect(f).not.toHaveBeenCalled();
  });

  it("turns API errors into VayoraError", async () => {
    const f = vi.fn(async () => json(409, { error: { code: "order_already_paid", message: "paid" } }));
    const v = new Vayora({ apiKey: "k", fetch: f as unknown as typeof fetch });
    await expect(v.payments.create({ orderId: "o", vpa: "a@upi", idempotencyKey: "k" })).rejects.toMatchObject({
      status: 409,
      code: "order_already_paid",
    });
    expect(VayoraError).toBeDefined();
  });

  it("omits amount for a full refund", async () => {
    const f = vi.fn(async () => json(201, {}));
    const v = new Vayora({ apiKey: "k", fetch: f as unknown as typeof fetch });
    await v.refunds.create({ paymentId: "pay_1", idempotencyKey: "r" });
    expect((f.mock.calls[0] as unknown as [string, RequestInit])[1].body).toBe("{}");
  });
});

describe("verifyWebhook", () => {
  const secret = "whsec";
  const body = '{"type":"payment.captured"}';
  const sign = (t: number, b = body) => `t=${t},v1=${createHmac("sha256", secret).update(`${t}.${b}`).digest("hex")}`;

  it("accepts a valid signature", () => {
    expect(verifyWebhook({ secret, header: sign(1000), rawBody: body, nowSeconds: 1100 })).toBe(true);
  });
  it("rejects tampering, wrong secret and stale timestamps", () => {
    expect(verifyWebhook({ secret, header: sign(1000), rawBody: body + " ", nowSeconds: 1000 })).toBe(false);
    expect(verifyWebhook({ secret: "x", header: sign(1000), rawBody: body, nowSeconds: 1000 })).toBe(false);
    expect(verifyWebhook({ secret, header: sign(1000), rawBody: body, nowSeconds: 9000 })).toBe(false);
    expect(verifyWebhook({ secret, header: "junk", rawBody: body })).toBe(false);
  });
});
