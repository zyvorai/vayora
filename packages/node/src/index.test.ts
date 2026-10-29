import { describe, expect, it, vi } from "vitest";
import { Vayora } from "./index";

describe("orders.create", () => {
  it("sends paise and the idempotency key", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ id: "order_1", amount_paise: 5000, currency: "INR", status: "created" })));
    const v = new Vayora({ fetch: f as unknown as typeof fetch });
    const o = await v.orders.create({ amountPaise: 5000, idempotencyKey: "k1" });
    expect(o.id).toBe("order_1");
    const [, init] = f.mock.calls[0] as unknown as [string, RequestInit];
    expect((init.headers as Record<string, string>)["idempotency-key"]).toBe("k1");
    expect(init.body).toBe('{"amount_paise":5000}');
  });

  it("rejects non-integer amounts", async () => {
    await expect(new Vayora().orders.create({ amountPaise: 10.5, idempotencyKey: "k" })).rejects.toThrow(RangeError);
  });
});
