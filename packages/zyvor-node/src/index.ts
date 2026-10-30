import { createHmac, randomUUID, timingSafeEqual } from "node:crypto";

export type Currency = "INR" | "USD";
export type Method = "upi" | "card" | "netbanking" | "wallet";
export type AttemptStatus = "created" | "pending" | "authorized" | "captured" | "failed" | "unknown";

export interface PaymentIntent {
  id: string;
  amount_minor: number;
  currency: Currency;
  status: "requires_payment_method" | "processing" | "succeeded" | "failed" | "canceled";
  capture_method: "automatic" | "manual";
  customer_id: string | null;
  invoice_id: string | null;
  external_ref: string | null;
  /** Only present on the response that created the intent. */
  client_secret?: string;
}

export interface PaymentAttempt {
  id: string;
  intent_id: string;
  provider: string;
  method: Method;
  /** `unknown` = the provider's outcome is not known; no new attempt can start until it resolves. */
  status: AttemptStatus;
  amount_minor: number;
  failure_reason: string | null;
  route: { provider: string; reasons: string[]; considered: unknown[] };
  client_action: { type: string; [k: string]: unknown } | null;
  refunded_minor: number;
  reserved_refund_minor: number;
}

export interface Refund {
  id: string;
  attempt_id: string;
  intent_id: string;
  amount_minor: number;
  status: "pending_approval" | "processing" | "processed" | "failed" | "rejected" | "unknown";
  requested_by: string;
  approved_by: string | null;
  external_ref: string | null;
}

export interface ReconCase {
  id: string;
  kind: string;
  status: "open" | "assigned" | "resolved";
  explanation: string;
  expected_minor: number | null;
  actual_minor: number | null;
  assignee: string | null;
}

export interface Page<T> {
  data: T[];
  has_more?: boolean;
}

/** One line of a GST tax invoice. `rateBp` is the slab in percent × 100 (1800 = 18%). */
export interface GstLine { description: string; hsnSac: string; quantity: number; unitPriceMinor: number; rateBp: 0 | 25 | 100 | 150 | 300 | 500 | 1200 | 1800 | 2800 }

export interface PayoutRow { id: string; vendor_id: string; amount_minor: number; currency: string; status: "pending_approval" | "approved" | "paid" | "rejected" | "canceled"; purpose: string; external_ref: string | null; requested_by: string; approved_by: string | null; paid_by: string | null; bank_ref: string | null; created_at: string }

export class ZyvorError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export interface ZyvorOptions {
  apiKey: string;
  baseUrl?: string;
  /** Acts on behalf of a person (shown in the audit trail; used for approval separation of duties). */
  actor?: string;
  fetch?: typeof fetch;
}

const key = (k?: string) => k ?? randomUUID();

/** Amounts are integer minor units (paise). Every payment write takes an idempotency key: reuse it on retry. */
export class Zyvor {
  private readonly base: string;
  private readonly f: typeof fetch;

  constructor(private readonly o: ZyvorOptions) {
    this.base = (o.baseUrl ?? "http://localhost:8080").replace(/\/$/, "");
    this.f = o.fetch ?? fetch;
  }

  private async req<T>(method: string, path: string, body?: unknown, idem?: string): Promise<T> {
    const h: Record<string, string> = { authorization: `Bearer ${this.o.apiKey}` };
    if (body !== undefined) h["content-type"] = "application/json";
    if (idem) h["idempotency-key"] = idem;
    if (this.o.actor) h["zyvor-actor"] = this.o.actor;
    const res = await this.f(`${this.base}${path}`, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw new ZyvorError(res.status, json?.error?.code ?? "unknown", json?.error?.message ?? res.statusText);
    return json as T;
  }

  private qs(p: Record<string, string | number | undefined>) {
    const q = new URLSearchParams();
    for (const [k, v] of Object.entries(p)) if (v !== undefined) q.set(k, String(v));
    const s = q.toString();
    return s ? `?${s}` : "";
  }

  readonly intents = {
    create: (p: { amountMinor: number; currency?: Currency; customerId?: string; invoiceId?: string; externalRef?: string; description?: string; allowedProviders?: string[]; captureMethod?: "automatic" | "manual"; idempotencyKey?: string }) => {
      if (!Number.isSafeInteger(p.amountMinor) || p.amountMinor <= 0) throw new RangeError("amountMinor must be a positive integer (paise)");
      return this.req<PaymentIntent>(
        "POST",
        "/v1/payment_intents",
        { amount_minor: p.amountMinor, currency: p.currency ?? "INR", customer_id: p.customerId, invoice_id: p.invoiceId, external_ref: p.externalRef, description: p.description, allowed_providers: p.allowedProviders, capture_method: p.captureMethod },
        key(p.idempotencyKey),
      );
    },
    retrieve: (id: string) => this.req<{ intent: PaymentIntent; attempts: PaymentAttempt[] }>("GET", `/v1/payment_intents/${encodeURIComponent(id)}`),
    list: (p: { limit?: number; startingAfter?: string; status?: string } = {}) =>
      this.req<Page<PaymentIntent>>("GET", `/v1/payment_intents${this.qs({ limit: p.limit, starting_after: p.startingAfter, status: p.status })}`),
    attempt: (intentId: string, p: { method: Method; vpa?: string; idempotencyKey?: string }) =>
      this.req<PaymentAttempt>("POST", `/v1/payment_intents/${encodeURIComponent(intentId)}/attempts`, { method: p.method, vpa: p.vpa }, key(p.idempotencyKey)),
    refund: (intentId: string, p: { amountMinor?: number; reason?: string; externalRef?: string; idempotencyKey?: string }) =>
      this.req<Refund>("POST", `/v1/payment_intents/${encodeURIComponent(intentId)}/refunds`, { amount_minor: p.amountMinor, reason: p.reason, external_ref: p.externalRef }, key(p.idempotencyKey)),
  };

  readonly attempts = {
    retrieve: (id: string) => this.req<PaymentAttempt>("GET", `/v1/payment_attempts/${encodeURIComponent(id)}`),
    /** Ask the provider where this payment stands. Safe to call any number of times. */
    refresh: (id: string) => this.req<PaymentAttempt>("POST", `/v1/payment_attempts/${encodeURIComponent(id)}/refresh`),
    cancel: (id: string) => this.req<PaymentAttempt>("POST", `/v1/payment_attempts/${encodeURIComponent(id)}/cancel`),
    capture: (id: string) => this.req<PaymentAttempt>("POST", `/v1/payment_attempts/${encodeURIComponent(id)}/capture`),
  };

  readonly refunds = {
    list: (p: { status?: string; limit?: number } = {}) => this.req<Page<Refund>>("GET", `/v1/refunds${this.qs({ status: p.status, limit: p.limit })}`),
    approve: (id: string) => this.req<Refund>("POST", `/v1/refunds/${encodeURIComponent(id)}/approve`),
    reject: (id: string) => this.req<Refund>("POST", `/v1/refunds/${encodeURIComponent(id)}/reject`),
    retry: (id: string) => this.req<Refund>("POST", `/v1/refunds/${encodeURIComponent(id)}/retry`),
    refresh: (id: string) => this.req<Refund>("POST", `/v1/refunds/${encodeURIComponent(id)}/refresh`),
  };

  readonly settlements = {
    list: () => this.req<Page<Record<string, unknown>>>("GET", "/v1/settlements"),
    /** "Explain every rupee": payments, refunds, fees, adjustments, bank match and a plain-language narrative. */
    explain: (id: string) => this.req<{ narrative: string; difference_minor: number; difference_breakdown: { reason: string; amount_minor: number }[]; [k: string]: unknown }>("GET", `/v1/settlements/${encodeURIComponent(id)}/explain`),
    sync: (provider: string, date: string) => this.req<{ imported: string[] }>("POST", "/v1/settlements/sync", { provider, date }),
  };

  readonly cases = {
    list: (p: { status?: string; kind?: string } = {}) => this.req<Page<ReconCase>>("GET", `/v1/recon/cases${this.qs({ status: p.status, kind: p.kind })}`),
    assign: (id: string, assignee: string) => this.req("POST", `/v1/recon/cases/${encodeURIComponent(id)}/assign`, { assignee }),
    resolve: (id: string, resolution: string) => this.req("POST", `/v1/recon/cases/${encodeURIComponent(id)}/resolve`, { resolution }),
  };

  readonly billing = {
    createPlan: (p: { name: string; amountMinor: number; intervalUnit: "day" | "week" | "month" | "year"; intervalCount?: number; usageUnitMinor?: number; dueDays?: number; currency?: Currency }) =>
      this.req<{ id: string }>("POST", "/v1/plans", { name: p.name, amount_minor: p.amountMinor, interval_unit: p.intervalUnit, interval_count: p.intervalCount, usage_unit_minor: p.usageUnitMinor, due_days: p.dueDays, currency: p.currency }),
    plans: () => this.req<Page<Record<string, unknown>>>("GET", "/v1/plans"),
    subscribe: (p: { customerId: string; planId: string; quantity?: number; startDate?: string }) =>
      this.req<{ id: string; next_bill_date: string }>("POST", "/v1/subscriptions", { customer_id: p.customerId, plan_id: p.planId, quantity: p.quantity, start_date: p.startDate }),
    subscriptions: () => this.req<Page<Record<string, unknown>>>("GET", "/v1/subscriptions"),
    pause: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/pause`),
    resume: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/resume`),
    cancel: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/cancel`),
    /** Retry-safe when `reference` is set: the same reference is recorded once. */
    recordUsage: (id: string, p: { quantity: number; reference?: string; note?: string }) => this.req<{ recorded: boolean }>("POST", `/v1/subscriptions/${encodeURIComponent(id)}/usage`, p),
    run: () => this.req<{ invoices_created: number }>("POST", "/v1/billing/run"),
    creditNote: (invoiceId: string, p: { amountMinor: number; reason: string }) => this.req<{ id: string }>("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/credit_notes`, { amount_minor: p.amountMinor, reason: p.reason }),
    voidInvoice: (invoiceId: string) => this.req("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/void`),
    statement: (customerId: string) => this.req<{ lines: Record<string, unknown>[]; balance_minor: Record<string, number> }>("GET", `/v1/customers/${encodeURIComponent(customerId)}/statement`),
  };

  readonly invoices = {
    list: (status?: string) => this.req<Page<{ id: string; number: string; amount_minor: number; paid_minor: number; outstanding_minor: number; status: string; due_date: string | null }>>("GET", `/v1/invoices${this.qs({ status })}`),
    retrieve: (id: string) => this.req<Record<string, unknown>>("GET", `/v1/invoices/${encodeURIComponent(id)}`),
    /** Give `amountMinor`, or GST `lines` and the server computes tax and total (needs `gst.gstin` in settings). */
    create: (p: { number: string; amountMinor?: number; customerId?: string; dueDate?: string; taxMinor?: number; externalRef?: string; lines?: GstLine[]; placeOfSupply?: string; buyerGstin?: string; reverseCharge?: boolean }) =>
      this.req<{ id: string; amount_minor: number; tax_minor: number }>("POST", "/v1/invoices", { number: p.number, amount_minor: p.amountMinor, customer_id: p.customerId, due_date: p.dueDate, tax_minor: p.taxMinor, external_ref: p.externalRef, lines: p.lines?.map((l) => ({ description: l.description, hsn_sac: l.hsnSac, quantity: l.quantity, unit_price_minor: l.unitPriceMinor, rate_bp: l.rateBp })), place_of_supply: p.placeOfSupply, buyer_gstin: p.buyerGstin, reverse_charge: p.reverseCharge }),
    /** CGST/SGST/IGST breakdown of a GST invoice. */
    lines: (id: string) => this.req<Record<string, unknown>>("GET", `/v1/invoices/${encodeURIComponent(id)}/lines`),
  };

  /** Vendors you pay and the payouts to them. Zyvor controls and records; the transfer is made from your own bank. */
  readonly payouts = {
    createVendor: (p: { name: string; accountNumber?: string; ifsc?: string; vpa?: string; externalRef?: string }) =>
      this.req<{ id: string; name: string }>("POST", "/v1/vendors", { name: p.name, account_number: p.accountNumber, ifsc: p.ifsc, vpa: p.vpa, external_ref: p.externalRef }),
    vendors: () => this.req<Page<{ id: string; name: string; external_ref: string | null; account_last4: string | null; ifsc: string | null; vpa: string | null; archived: boolean }>>("GET", "/v1/vendors"),
    /** Full bank details for whoever makes the transfer (needs `payouts:approve`; every reveal is audited). */
    vendorDetails: (id: string) => this.req<{ id: string; details: { account_number: string | null; ifsc: string | null; vpa: string | null } }>("GET", `/v1/vendors/${encodeURIComponent(id)}/details`),
    /** Repeating a request with the same `externalRef` returns the original payout. */
    create: (p: { vendorId: string; amountMinor: number; purpose: string; currency?: string; externalRef?: string }) =>
      this.req<PayoutRow>("POST", "/v1/payouts", { vendor_id: p.vendorId, amount_minor: p.amountMinor, purpose: p.purpose, currency: p.currency, external_ref: p.externalRef }),
    list: (status?: PayoutRow["status"]) => this.req<Page<PayoutRow>>("GET", `/v1/payouts${this.qs({ status })}`),
    approve: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/approve`),
    reject: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/reject`),
    /** Record the bank reference (UTR) of the transfer you made; posts to the ledger once. */
    markPaid: (id: string, bankRef: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/mark_paid`, { bank_ref: bankRef }),
    cancel: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/cancel`),
  };

  readonly gst = {
    /** GSTR-1 style summary for `YYYY-MM`: a preparation aid, Zyvor does not file returns. */
    gstr1: (period: string) => this.req<Record<string, unknown>>("GET", `/v1/gst/gstr1${this.qs({ period })}`),
  };

  readonly milestones = {
    list: () => this.req<Page<Record<string, unknown>>>("GET", "/v1/milestones"),
    create: (p: { customerId: string; name: string; amountMinor: number; dueDays?: number; externalRef?: string }) =>
      this.req<{ id: string }>("POST", "/v1/milestones", { customer_id: p.customerId, name: p.name, amount_minor: p.amountMinor, due_days: p.dueDays, external_ref: p.externalRef }),
    /** Accept the milestone and invoice it. Calling twice returns the same invoice. */
    complete: (id: string) => this.req<{ invoice_id: string }>("POST", `/v1/milestones/${encodeURIComponent(id)}/complete`),
    cancel: (id: string) => this.req("POST", `/v1/milestones/${encodeURIComponent(id)}/cancel`),
  };

  readonly disputes = {
    /** Holds the disputed amount on the payment so it cannot also be refunded. */
    open: (p: { attemptId: string; reason: string; amountMinor?: number; providerRef?: string; respondBy?: string }) =>
      this.req<{ id: string }>("POST", "/v1/disputes", { attempt_id: p.attemptId, reason: p.reason, amount_minor: p.amountMinor, provider_ref: p.providerRef, respond_by: p.respondBy }),
    list: (status?: "open" | "under_review" | "won" | "lost") => this.req<Page<{ id: string; attempt_id: string; amount_minor: number; reason: string; status: string; respond_by: string | null; evidence_count: number }>>("GET", `/v1/disputes${this.qs({ status })}`),
    addEvidence: (id: string, p: { note: string; url?: string }) => this.req("POST", `/v1/disputes/${encodeURIComponent(id)}/evidence`, p),
    /** `lost` books the loss on the ledger and re-opens the paid invoice; `won` releases the hold. */
    resolve: (id: string, outcome: "won" | "lost") => this.req("POST", `/v1/disputes/${encodeURIComponent(id)}/resolve`, { outcome }),
  };

  readonly collections = {
    ageing: (asOf?: string) => this.req<{ as_of: string; data: { currency: string; bucket: string; outstanding_minor: number; count: number; invoices: { id: string; number: string; customer_id: string | null; due_date: string | null; outstanding_minor: number; days_past_due: number }[] }[] }>("GET", `/v1/collections/ageing${this.qs({ as_of: asOf })}`),
    promise: (invoiceId: string, p: { promisedMinor: number; promisedDate: string; note?: string }) =>
      this.req<{ id: string }>("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/promises`, { promised_minor: p.promisedMinor, promised_date: p.promisedDate, note: p.note }),
    promises: (status?: "open" | "kept" | "broken" | "canceled") => this.req<Page<Record<string, unknown>>>("GET", `/v1/collections/promises${this.qs({ status })}`),
    cancelPromise: (id: string) => this.req("POST", `/v1/collections/promises/${encodeURIComponent(id)}/cancel`),
    /** Draft a reminder for a person to review. `ai: true` asks the optional assistant to word it (needs it enabled server-side). */
    draft: (invoiceId: string, p: { channel?: "email" | "sms" | "whatsapp" | "call" | "other"; tone?: "friendly" | "firm"; ai?: boolean } = {}) =>
      this.req<{ id: string; message: string; source: "template" | "ai"; status: string }>("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/reminders/draft`, p),
    drafts: (status?: "pending" | "approved" | "rejected") => this.req<Page<{ id: string; invoice_id: string; invoice_number: string; channel: string; message: string; source: "template" | "ai"; status: string; created_by: string; created_at: string }>>("GET", `/v1/collections/drafts${this.qs({ status })}`),
    /** Sends nothing itself: approving logs the reminder and emits `invoice.reminder`. Pass `message` to edit first. */
    approveDraft: (id: string, message?: string) => this.req<{ id: string; reminder_id: string }>("POST", `/v1/collections/drafts/${encodeURIComponent(id)}/approve`, message ? { message } : {}),
    rejectDraft: (id: string) => this.req("POST", `/v1/collections/drafts/${encodeURIComponent(id)}/reject`),
    /** Remind every overdue invoice with no live promise to pay and no recent reminder. Safe to repeat. */
    runDunning: (p: { channel?: "email" | "sms" | "whatsapp" | "call" | "other"; minDaysOverdue?: number; repeatAfterDays?: number } = {}) =>
      this.req<{ reminded: number; reminder_ids: string[] }>("POST", "/v1/collections/dunning/run", { channel: p.channel, min_days_overdue: p.minDaysOverdue, repeat_after_days: p.repeatAfterDays }),
    reminders: (invoiceId: string) => this.req<Page<Record<string, unknown>>>("GET", `/v1/invoices/${encodeURIComponent(invoiceId)}/reminders`),
    remind: (invoiceId: string, p: { channel: "email" | "sms" | "whatsapp" | "call" | "other"; message?: string }) => this.req("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/reminders`, p),
  };

  readonly paymentLinks = {
    /** One URL, many payers: opening it mints a fresh payment intent. Give `amountMinor` or `invoiceId`. */
    create: (p: { description?: string; amountMinor?: number; currency?: string; invoiceId?: string; customerId?: string; maxUses?: number; expiresAt?: string }) =>
      this.req<{ id: string; token: string; path: string }>("POST", "/v1/payment_links", { description: p.description, amount_minor: p.amountMinor, currency: p.currency, invoice_id: p.invoiceId, customer_id: p.customerId, max_uses: p.maxUses, expires_at: p.expiresAt }),
    list: () => this.req<Page<{ id: string; path: string; description: string | null; amount_minor: number | null; currency: string; invoice_id: string | null; max_uses: number | null; uses: number; status: "active" | "expired" | "used_up" | "canceled"; created_at: string }>>("GET", "/v1/payment_links"),
    cancel: (id: string) => this.req("POST", `/v1/payment_links/${encodeURIComponent(id)}/cancel`),
  };

  /** Named console users with roles, Argon2id passwords and TOTP. Needs an admin key. */
  readonly consoleUsers = {
    create: (p: { username: string; password: string; role: "admin" | "finance" | "viewer" }) => this.req<{ id: string; username: string; role: string }>("POST", "/v1/console/users", p),
    list: () => this.req<Page<{ id: string; username: string; role: "admin" | "finance" | "viewer"; mfa_enrolled: boolean; last_login_at: string | null; locked: boolean; disabled: boolean }>>("GET", "/v1/console/users"),
    disable: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/disable`),
    enable: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/enable`),
    setPassword: (id: string, password: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/password`, { password }),
    mfaBegin: (id: string) => this.req<{ secret: string; otpauth_uri: string }>("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/begin`),
    mfaConfirm: (id: string, otp: string) => this.req<{ recovery_codes: string[] }>("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/confirm`, { otp }),
    mfaReset: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/reset`),
    /** Throws ZyvorError: `otp_required` (401) when a code is needed, `account_locked` (423) after repeated failures. */
    authenticate: (p: { username: string; password: string; otp?: string }) => this.req<{ user: { id: string; username: string; role: "admin" | "finance" | "viewer" }; mfa_enrolled: boolean }>("POST", "/v1/console/authenticate", p),
  };

  readonly audit = {
    /** Recompute the tamper-evident chain; `ok: false` names the first broken seal. */
    verify: () => this.req<{ ok: boolean; seals_checked: number; sealed_events?: number; unsealed_events?: number; reason?: string; broken_seal?: number }>("GET", "/v1/audit_events/verify"),
    seal: () => this.req<{ sealed_events: number }>("POST", "/v1/audit_events/seal"),
  };

  /** Key hygiene. */
  readonly keys = {
    /** Mint a replacement with the same scopes; the old key keeps working for `overlapHours` (default 24). */
    rotate: (id: string, overlapHours?: number) => this.req<{ id: string; key: string }>("POST", `/v1/api_keys/${encodeURIComponent(id)}/rotate`, { overlap_hours: overlapHours }),
  };

  readonly webhooks = {
    /** Every event type with its meaning. Choose which to receive with the `webhook_events` setting. */
    catalogue: () => this.req<Page<{ type: string; description: string }>>("GET", "/v1/events/catalogue"),
    outbox: (status?: string) => this.req<Page<Record<string, unknown>>>("GET", `/v1/outbox${this.qs({ status })}`),
    replay: (eventId: string) => this.req("POST", `/v1/outbox/${encodeURIComponent(eventId)}/replay`),
  };
}

/**
 * Verify a `Zyvor-Signature` header (`t=<unix>,v1=<hex hmac-sha256("{t}.{body}")>`) against the RAW body.
 * Webhooks are delivered at least once: also de-duplicate on `event.id`.
 */
export function verifyWebhook(opts: { secret: string; header: string; rawBody: string | Buffer; toleranceSeconds?: number; nowSeconds?: number }): boolean {
  const parts = Object.fromEntries(opts.header.split(",").map((kv) => kv.split("=", 2) as [string, string]));
  const t = Number(parts.t);
  if (!Number.isInteger(t) || !parts.v1) return false;
  const now = opts.nowSeconds ?? Math.floor(Date.now() / 1000);
  if (Math.abs(now - t) > (opts.toleranceSeconds ?? 300)) return false;
  const expected = createHmac("sha256", opts.secret).update(`${t}.`).update(opts.rawBody).digest();
  const given = Buffer.from(parts.v1, "hex");
  return given.length === expected.length && timingSafeEqual(given, expected);
}
