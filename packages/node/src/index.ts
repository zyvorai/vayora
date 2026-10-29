import { createHmac, timingSafeEqual } from "node:crypto";

export interface Order {
  id: string;
  amount_paise: number;
  currency: "INR";
  status: "created" | "paid";
  created_at: string;
}

export interface Payment {
  id: string;
  order_id: string;
  vpa: string;
  amount_paise: number;
  refunded_paise: number;
  status: "captured" | "failed";
  upi_ref: string | null;
  failure_reason: string | null;
  created_at: string;
}

export interface Refund {
  id: string;
  payment_id: string;
  amount_paise: number;
  status: "processed";
  upi_ref: string;
  created_at: string;
}

export interface Page<T> {
  data: T[];
  has_more: boolean;
}

export interface ListParams {
  limit?: number;
  /** id of the last item of the previous page */
  startingAfter?: string;
  status?: string;
}

export interface WebhookEvent {
  id: string;
  type: string;
  status: "pending" | "delivered" | "dead";
  attempts: number;
  next_attempt_at: string;
  delivered_at: string | null;
  last_error: string | null;
  created_at: string;
  payload: { id: string; type: string; created_at: string; data: unknown };
}

export interface Stats {
  captured_count: number;
  failed_count: number;
  /** null until there is at least one attempt */
  success_rate: number | null;
  gross_paise: number;
  refunded_paise: number;
  net_paise: number;
  merchant_payable_paise: number;
  webhooks: { pending: number; delivered: number; dead: number };
  /** last 14 IST days, zero-filled, oldest first */
  daily: { date: string; gross_paise: number; count: number }[];
}

export class VayoraError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export interface VayoraOptions {
  apiKey: string;
  baseUrl?: string;
  fetch?: typeof fetch;
}

const positiveInt = (n: number, name: string) => {
  if (!Number.isSafeInteger(n) || n <= 0) throw new RangeError(`${name} must be a positive integer (paise)`);
};

/** Amounts are integer paise (₹1 = 100). Retrying a write with the same idempotencyKey is safe. */
export class Vayora {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly apiKey: string;

  constructor(options: VayoraOptions) {
    this.apiKey = options.apiKey;
    this.baseUrl = (options.baseUrl ?? "http://localhost:8080").replace(/\/$/, "");
    this.fetchImpl = options.fetch ?? fetch;
  }

  private async request<T>(method: string, path: string, body?: unknown, idempotencyKey?: string): Promise<T> {
    const headers: Record<string, string> = { authorization: `Bearer ${this.apiKey}` };
    if (body !== undefined) headers["content-type"] = "application/json";
    if (idempotencyKey) headers["idempotency-key"] = idempotencyKey;
    const res = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) {
      throw new VayoraError(res.status, json?.error?.code ?? "unknown", json?.error?.message ?? res.statusText);
    }
    return json as T;
  }

  private list<T>(path: string, p: ListParams = {}) {
    const q = new URLSearchParams();
    if (p.limit !== undefined) q.set("limit", String(p.limit));
    if (p.startingAfter) q.set("starting_after", p.startingAfter);
    if (p.status) q.set("status", p.status);
    const qs = q.toString();
    return this.request<Page<T>>("GET", qs ? `${path}?${qs}` : path);
  }

  readonly orders = {
    list: (p?: ListParams) => this.list<Order>("/v1/orders", p),
    create: (p: { amountPaise: number; idempotencyKey: string }) => {
      positiveInt(p.amountPaise, "amountPaise");
      return this.request<Order>("POST", "/v1/orders", { amount_paise: p.amountPaise }, p.idempotencyKey);
    },
    retrieve: (id: string) => this.request<Order>("GET", `/v1/orders/${encodeURIComponent(id)}`),
  };

  readonly payments = {
    list: (p?: ListParams) => this.list<Payment>("/v1/payments", p),
    /** Simulated UPI collect. A declined payment resolves with status "failed" (not an exception). */
    create: (p: { orderId: string; vpa: string; idempotencyKey: string }) =>
      this.request<Payment>("POST", "/v1/payments", { order_id: p.orderId, vpa: p.vpa }, p.idempotencyKey),
    retrieve: (id: string) => this.request<Payment>("GET", `/v1/payments/${encodeURIComponent(id)}`),
  };

  readonly events = {
    list: (p?: ListParams) => this.list<WebhookEvent>("/v1/events", p),
    /** Re-queue a pending or dead webhook. Delivered events can't be retried. */
    retry: (id: string) => this.request<WebhookEvent>("POST", `/v1/events/${encodeURIComponent(id)}/retry`),
  };

  stats() {
    return this.request<Stats>("GET", "/v1/stats");
  }

  readonly refunds = {
    list: (p?: ListParams) => this.list<Refund>("/v1/refunds", p),
    /** Omit amountPaise to refund the remainder. */
    create: (p: { paymentId: string; amountPaise?: number; idempotencyKey: string }) => {
      if (p.amountPaise !== undefined) positiveInt(p.amountPaise, "amountPaise");
      return this.request<Refund>(
        "POST",
        `/v1/payments/${encodeURIComponent(p.paymentId)}/refunds`,
        p.amountPaise === undefined ? {} : { amount_paise: p.amountPaise },
        p.idempotencyKey,
      );
    },
  };
}

/**
 * Verify a `Vayora-Signature` header (`t=<unix>,v1=<hex>`) against the RAW request body.
 * Rejects timestamps older than `toleranceSeconds` to block replays.
 */
export function verifyWebhook(opts: {
  secret: string;
  header: string;
  rawBody: string | Buffer;
  toleranceSeconds?: number;
  nowSeconds?: number;
}): boolean {
  const parts = Object.fromEntries(opts.header.split(",").map((kv) => kv.split("=", 2) as [string, string]));
  const t = Number(parts.t);
  if (!Number.isInteger(t) || !parts.v1) return false;
  const now = opts.nowSeconds ?? Math.floor(Date.now() / 1000);
  if (Math.abs(now - t) > (opts.toleranceSeconds ?? 300)) return false;
  const expected = createHmac("sha256", opts.secret).update(`${t}.`).update(opts.rawBody).digest();
  const given = Buffer.from(parts.v1, "hex");
  return given.length === expected.length && timingSafeEqual(given, expected);
}
