export interface Order {
  id: string;
  amount_paise: number;
  currency: "INR";
  status: string;
}

export interface VayoraOptions {
  baseUrl?: string;
  fetch?: typeof fetch;
}

export class Vayora {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: VayoraOptions = {}) {
    this.baseUrl = (options.baseUrl ?? "http://localhost:8080").replace(/\/$/, "");
    this.fetchImpl = options.fetch ?? fetch;
  }

  readonly orders = {
    /** Amount is integer paise (₹1 = 100). Retrying with the same idempotencyKey is safe. */
    create: async (params: { amountPaise: number; idempotencyKey: string }): Promise<Order> => {
      if (!Number.isSafeInteger(params.amountPaise) || params.amountPaise <= 0) {
        throw new RangeError("amountPaise must be a positive integer");
      }
      const res = await this.fetchImpl(`${this.baseUrl}/v1/orders`, {
        method: "POST",
        headers: { "content-type": "application/json", "idempotency-key": params.idempotencyKey },
        body: JSON.stringify({ amount_paise: params.amountPaise }),
      });
      if (!res.ok) throw new Error(`Vayora API error ${res.status}: ${await res.text()}`);
      return (await res.json()) as Order;
    },
  };
}
