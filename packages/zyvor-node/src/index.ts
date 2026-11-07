import { createHmac, randomUUID, timingSafeEqual } from "node:crypto";
import type { DeleteTestClockResponse, GetInstallmentPlansByIdResponse, GetInvoicesByIdEinvoiceResponse, GetInvoicesByIdLinesResponse, GetInvoicesByIdRemindersResponse, GetInvoicesByIdResponse, GetMailSuppressionsResponse, GetMandatesByIdDebitsResponse, GetMilestonesResponse, GetPlansResponse, GetSettlementBatchesByIdResponse, GetSettlementsResponse, GetSubscriptionsResponse, GetTestClockResponse, PostMandatesByIdDebitsResponse, PostSettlementsByIdSimulateBankResponse, PostTestClockResponse } from "./types.js";

export type Currency = "INR" | "USD";
/** A UPI AutoPay mandate. `authorization` carries the link the payer approves. */
export type Mandate = { id: string; customer_id: string; status: string; max_amount_minor: number; frequency: string; start_date: string; end_date: string; authorization?: Record<string, unknown> | null; [k: string]: unknown };
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
  reason: string | null;
  failure_reason: string | null;
  created_at: string;
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

export interface RiskRules { max_intent_amount_minor?: number | null; max_attempts_per_intent?: number | null; max_failed_per_customer_per_hour?: number | null; blocked_vpas?: string[]; score_block_below?: number | null }

export interface Settings {
  risk: RiskRules;
  refund_approval_threshold_minor: number;
  payout_approval_threshold_minor: number;
  /** Email each payer a receipt when a payment is captured (needs the customer's email). */
  payment_receipts: boolean;
  /** Where replies to the emails sent for this business go; null when none is set. */
  reply_to_email: string | null;
  /** Email payment reminders whose channel is email to the customer; off by default. */
  reminder_emails: boolean;
  /** Email each subscription invoice to its customer when billing raises it; off by default. */
  invoice_emails: boolean;
  /** `#rrggbb` for the button and top edge of payer emails; null: the default blue. */
  email_accent: string | null;
  /** An https address of a logo shown beside the business name in payer emails. */
  email_logo_url: string | null;
  outbound_webhook_url: string | null;
  webhook_events: string[];
  dunning: { enabled: boolean; channel: "email" | "sms" | "whatsapp" | "call" | "other"; min_days_overdue: number; repeat_after_days: number; requires_approval: boolean };
  gst: { gstin: string | null; legal_name: string | null; address1?: string | null; location?: string | null; pincode?: string | null };
  /** Allowed return_url origins (`https://shop.example.com`). Empty = any https URL that passes SSRF checks. */
  return_url_origins?: string[];
  env: string;
}

/** The filters a Payments view can hold. `from` and `to` are days (IST, YYYY-MM-DD). */
export interface SavedViewQuery { status?: string; q?: string; from?: string; to?: string }
export interface SavedView { id: string; name: string; query: SavedViewQuery; shared: boolean; mine: boolean; owner: string }

/** Only the fields you send change. `outbound_webhook_secret` is write-only. */
export interface SettingsPatch {
  refund_approval_threshold_minor?: number;
  payout_approval_threshold_minor?: number;
  payment_receipts?: boolean;
  /** One plain email address; an empty string clears it. */
  reply_to_email?: string;
  reminder_emails?: boolean;
  invoice_emails?: boolean;
  email_accent?: string;
  email_logo_url?: string;
  outbound_webhook_url?: string;
  outbound_webhook_secret?: string;
  webhook_events?: string[];
  dunning?: Partial<Settings["dunning"]>;
  gst?: { gstin: string; legal_name?: string; address1?: string; location?: string; pincode?: string };
  /** Limits that stop a payment. Sending `risk` replaces the whole set, so include what you want to keep. */
  risk?: RiskRules;
  /** Replace the whole allowlist. Empty array clears it. */
  return_url_origins?: string[];
}

export interface PayoutRow { id: string; vendor_id: string; amount_minor: number; currency: string; status: "pending_approval" | "approved" | "paid" | "rejected" | "canceled"; purpose: string; external_ref: string | null; requested_by: string; approved_by: string | null; paid_by: string | null; bank_ref: string | null; created_at: string }

/** What was read from an uploaded document, and how it compares with the profile (`result`: match, mismatch, not_found or not_checked). */
export type OnboardingUpload = {
  id: string; kind: string; filename: string; content_type: string; size_bytes: number;
  read_method: "text" | "ocr" | "none";
  facts: { gstin?: string | null; pan_last4?: string | null; pan_in_gstin?: boolean | null; cin?: string | null; legal_name?: string | null; ifsc?: string | null; /** OCR misread a character and it was corrected by position; worth confirming on the file */ ocr_corrected?: boolean };
  checks: { name: string; result: "match" | "mismatch" | "not_found" | "not_checked"; detail: string }[];
  uploaded_by: string; created_at: string;
};

export type Customer = { id: string; name: string; email: string | null; phone: string | null; external_ref?: string | null; gstin: string | null; address1: string | null; location: string | null; pincode: string | null };

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
  /** The API's `ZYVOR_ACTOR_SECRET`. When the API has one, it believes `actor` only if the request is signed with it. */
  actorSecret?: string;
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

  private headers(hasBody: boolean, idem?: string): Record<string, string> {
    const h: Record<string, string> = { authorization: `Bearer ${this.o.apiKey}` };
    if (hasBody) h["content-type"] = "application/json";
    if (idem) h["idempotency-key"] = idem;
    if (this.o.actor) {
      h["zyvor-actor"] = this.o.actor;
      if (this.o.actorSecret) {
        const t = Math.floor(Date.now() / 1000);
        h["zyvor-actor-signature"] = `t=${t},v1=${createHmac("sha256", this.o.actorSecret).update(`${t}.${this.o.actor}`).digest("hex")}`;
      }
    }
    return h;
  }

  /** A response that is a file, not JSON. */
  private async bytes(path: string): Promise<Uint8Array> {
    const res = await this.f(`${this.base}${path}`, { method: "GET", headers: this.headers(false) });
    if (!res.ok) {
      const json = await res.json().catch(() => null);
      throw new ZyvorError(res.status, json?.error?.code ?? "unknown", json?.error?.message ?? res.statusText);
    }
    return new Uint8Array(await res.arrayBuffer());
  }

  private async req<T>(method: string, path: string, body?: unknown, idem?: string): Promise<T> {
    const h = this.headers(body !== undefined, idem);
    const res = await this.f(`${this.base}${path}`, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw new ZyvorError(res.status, json?.error?.code ?? "unknown", json?.error?.message ?? res.statusText);
    return json as T;
  }

  /** A request whose body is raw bytes (an image), not JSON. */
  private async rawReq<T>(method: string, path: string, body?: Uint8Array): Promise<T> {
    const h = { ...this.headers(false), ...(body ? { "content-type": "application/octet-stream" } : {}) };
    const res = await this.f(`${this.base}${path}`, { method, headers: h, body: body as BodyInit | undefined });
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
    create: (p: { amountMinor: number; currency?: Currency; customerId?: string; invoiceId?: string; externalRef?: string; description?: string; allowedProviders?: string[]; captureMethod?: "automatic" | "manual"; idempotencyKey?: string; /** Marketplace shares owed to vendors out of this payment (sum at most the amount). */ splits?: { vendorId: string; amountMinor: number }[]; /** Attribute the payment to a branch, or to a counter within it (which implies its branch). */ branchId?: string; counterId?: string }) => {
      if (!Number.isSafeInteger(p.amountMinor) || p.amountMinor <= 0) throw new RangeError("amountMinor must be a positive integer (paise)");
      return this.req<PaymentIntent>(
        "POST",
        "/v1/payment_intents",
        { amount_minor: p.amountMinor, currency: p.currency ?? "INR", branch_id: p.branchId, counter_id: p.counterId, customer_id: p.customerId, invoice_id: p.invoiceId, external_ref: p.externalRef, description: p.description, allowed_providers: p.allowedProviders, capture_method: p.captureMethod, splits: p.splits?.map((x) => ({ vendor_id: x.vendorId, amount_minor: x.amountMinor })) },
        key(p.idempotencyKey),
      );
    },
    retrieve: (id: string) => this.req<{ intent: PaymentIntent; attempts: PaymentAttempt[] }>("GET", `/v1/payment_intents/${encodeURIComponent(id)}`),
    /** One ordered story of the payment, with a plain sentence on where it stands. */
    timeline: (id: string) => this.req<{ payment_id: string; status: string; standing: string; events: { at: string; kind: "intent" | "attempt" | "webhook" | "ledger" | "refund" | "dispute" | "settlement"; title: string; detail: Record<string, unknown> }[] }>("GET", `/v1/payment_intents/${encodeURIComponent(id)}/timeline`),
    /** `q` matches a payment id exactly, or part of its reference or description. */
    list: (p: { limit?: number; startingAfter?: string; status?: string; q?: string; /** Created on or after this day (IST, YYYY-MM-DD). */ createdFrom?: string; /** Created on or before this day (IST, YYYY-MM-DD). */ createdTo?: string } = {}) =>
      this.req<Page<PaymentIntent>>("GET", `/v1/payment_intents${this.qs({ limit: p.limit, starting_after: p.startingAfter, status: p.status, q: p.q, created_from: p.createdFrom, created_to: p.createdTo })}`),
    attempt: (intentId: string, p: { method: Method; vpa?: string; returnUrl?: string; upiFlow?: string; cardTokenId?: string; idempotencyKey?: string }) =>
      this.req<PaymentAttempt>("POST", `/v1/payment_intents/${encodeURIComponent(intentId)}/attempts`, { method: p.method, vpa: p.vpa, return_url: p.returnUrl, upi_flow: p.upiFlow, card_token_id: p.cardTokenId }, key(p.idempotencyKey)),
    refund: (intentId: string, p: { amountMinor?: number; reason?: string; externalRef?: string; idempotencyKey?: string }) =>
      this.req<Refund>("POST", `/v1/payment_intents/${encodeURIComponent(intentId)}/refunds`, { amount_minor: p.amountMinor, reason: p.reason, external_ref: p.externalRef }, key(p.idempotencyKey)),
  };

  readonly attempts = {
    retrieve: (id: string) => this.req<PaymentAttempt>("GET", `/v1/payment_attempts/${encodeURIComponent(id)}`),
    /** Ask the provider where this payment stands. Safe to call any number of times. */
    refresh: (id: string) => this.req<PaymentAttempt>("POST", `/v1/payment_attempts/${encodeURIComponent(id)}/refresh`),
    /** Test mode only: deliver the provider's late update for a pending simulated payment. */
    simulateEvent: (id: string, outcome: "captured" | "failed") => this.req<PaymentAttempt>("POST", `/v1/payment_attempts/${encodeURIComponent(id)}/simulate_event`, { outcome }),
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
    list: () => this.req<GetSettlementsResponse>("GET", "/v1/settlements"),
    /** "Explain every rupee": payments, refunds, fees, adjustments, bank match and a plain-language narrative. */
    explain: (id: string) => this.req<{ narrative: string; difference_minor: number; difference_breakdown: { reason: string; amount_minor: number }[]; [k: string]: unknown }>("GET", `/v1/settlements/${encodeURIComponent(id)}/explain`),
    sync: (provider: string, date: string) => this.req<{ imported: string[] }>("POST", "/v1/settlements/sync", { provider, date }),
    /** Test mode: add the bank credit a simulated settlement would produce, optionally `shortMinor` less than its net. */
    simulateBank: (id: string, shortMinor = 0) => this.req<PostSettlementsByIdSimulateBankResponse>("POST", `/v1/settlements/${encodeURIComponent(id)}/simulate_bank`, { short_minor: shortMinor }),
  };

  readonly cases = {
    list: (p: { status?: string; kind?: string } = {}) => this.req<Page<ReconCase>>("GET", `/v1/recon/cases${this.qs({ status: p.status, kind: p.kind })}`),
    /** The likeliest explanation with its evidence and next steps. Changes nothing; a person resolves the case. */
    suggestions: (id: string, opts: { ai?: boolean } = {}) => this.req<{ case_id: string; kind: string; summary: string; ai_summary: string | null; evidence: Record<string, unknown>[]; next_steps: string[]; note: string }>("GET", `/v1/recon/cases/${encodeURIComponent(id)}/suggestions${opts.ai ? "?ai=true" : ""}`),
    assign: (id: string, assignee: string) => this.req("POST", `/v1/recon/cases/${encodeURIComponent(id)}/assign`, { assignee }),
    resolve: (id: string, resolution: string) => this.req("POST", `/v1/recon/cases/${encodeURIComponent(id)}/resolve`, { resolution }),
  };

  readonly billing = {
    createPlan: (p: { name: string; amountMinor: number; intervalUnit: "day" | "week" | "month" | "year"; intervalCount?: number; usageUnitMinor?: number; dueDays?: number; trialDays?: number; includedUnits?: number; includedPerSeat?: boolean; currency?: Currency }) =>
      this.req<{ id: string }>("POST", "/v1/plans", { name: p.name, amount_minor: p.amountMinor, interval_unit: p.intervalUnit, interval_count: p.intervalCount, usage_unit_minor: p.usageUnitMinor, due_days: p.dueDays, trial_days: p.trialDays, included_units: p.includedUnits, included_per_seat: p.includedPerSeat, currency: p.currency }),
    plans: () => this.req<GetPlansResponse>("GET", "/v1/plans"),
    /** Rename a plan, change its price (from each subscriber's next invoice), set the free trial new subscribers get (`trialDays`), the free units per period on a metered plan (`includedUnits`) or archive it (`active: false`). With a new price, `keepExisting: true` leaves today's subscribers on the price they pay now. */
    updatePlan: (id: string, p: { name?: string; amountMinor?: number; active?: boolean; trialDays?: number; includedUnits?: number; includedPerSeat?: boolean; keepExisting?: boolean }) =>
      this.req<{ id: string; name: string; amount_minor: number; active: boolean; trial_days: number; included_units: number; included_per_seat: boolean; subscribers_kept_on_old_price: number }>("PATCH", `/v1/plans/${encodeURIComponent(id)}`, { name: p.name, amount_minor: p.amountMinor, active: p.active, trial_days: p.trialDays, included_units: p.includedUnits, included_per_seat: p.includedPerSeat, keep_existing: p.keepExisting }),
    subscribe: (p: { customerId: string; planId: string; quantity?: number; startDate?: string }) =>
      this.req<{ id: string; next_bill_date: string; trial_ends_on: string | null }>("POST", "/v1/subscriptions", { customer_id: p.customerId, plan_id: p.planId, quantity: p.quantity, start_date: p.startDate }),
    subscriptions: () => this.req<GetSubscriptionsResponse>("GET", "/v1/subscriptions"),
    pause: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/pause`),
    resume: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/resume`),
    cancel: (id: string) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/cancel`),
    /** An upgrade is charged now for the unused part of the period at the price difference; a downgrade applies from the next invoice. */
    /** `followPlanPrice: true` (same plan) ends a kept price: the plan's current price applies from the next invoice. */
    changePlan: (id: string, p: { planId: string; quantity?: number; followPlanPrice?: boolean }) => this.req("POST", `/v1/subscriptions/${encodeURIComponent(id)}/change_plan`, { plan_id: p.planId, quantity: p.quantity, follow_plan_price: p.followPlanPrice }),
    /** Retry-safe when `reference` is set: the same reference is recorded once. */
    /** End a running free trial early (`endsOn` today) or move its end, for one subscriber. Only a subscription still in its trial can change. */
    changeTrial: (id: string, p: { endsOn: string }) => this.req<{ id: string; trial_ends_on: string; next_bill_date: string }>("POST", `/v1/subscriptions/${encodeURIComponent(id)}/trial`, { ends_on: p.endsOn }),
    recordUsage: (id: string, p: { quantity: number; reference?: string; note?: string }) => this.req<{ recorded: boolean }>("POST", `/v1/subscriptions/${encodeURIComponent(id)}/usage`, p),
    /** `asOf` (YYYY-MM-DD) runs as if it were that day; test mode only. */
    run: (p: { asOf?: string } = {}) => this.req<{ invoices_created: number }>("POST", "/v1/billing/run", { as_of: p.asOf }),
    creditNote: (invoiceId: string, p: { amountMinor: number; reason: string }) => this.req<{ id: string }>("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/credit_notes`, { amount_minor: p.amountMinor, reason: p.reason }),
    voidInvoice: (invoiceId: string) => this.req("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/void`),
    statement: (customerId: string) => this.req<{ lines: Record<string, unknown>[]; balance_minor: Record<string, number> }>("GET", `/v1/customers/${encodeURIComponent(customerId)}/statement`),
  };

  readonly invoices = {
    list: (status?: string) => this.req<Page<{ id: string; number: string; amount_minor: number; paid_minor: number; outstanding_minor: number; status: string; due_date: string | null; customer_id: string | null; customer_name: string | null }>>("GET", `/v1/invoices${this.qs({ status })}`),
    retrieve: (id: string) => this.req<GetInvoicesByIdResponse>("GET", `/v1/invoices/${encodeURIComponent(id)}`),
    /** Email an invoice to its customer: what it is made of, what is owed and by when (once per invoice per day; needs a customer with an email address and an invoice that is still owed). */
    send: (id: string) => this.req<{ id: string; queued: boolean }>("POST", `/v1/invoices/${encodeURIComponent(id)}/send`),
    /** Give `amountMinor`, or GST `lines` and the server computes tax and total (needs `gst.gstin` in settings). */
    create: (p: { number: string; amountMinor?: number; customerId?: string; dueDate?: string; taxMinor?: number; externalRef?: string; lines?: GstLine[]; placeOfSupply?: string; buyerGstin?: string; reverseCharge?: boolean }) =>
      this.req<{ id: string; amount_minor: number; tax_minor: number }>("POST", "/v1/invoices", { number: p.number, amount_minor: p.amountMinor, customer_id: p.customerId, due_date: p.dueDate, tax_minor: p.taxMinor, external_ref: p.externalRef, lines: p.lines?.map((l) => ({ description: l.description, hsn_sac: l.hsnSac, quantity: l.quantity, unit_price_minor: l.unitPriceMinor, rate_bp: l.rateBp })), place_of_supply: p.placeOfSupply, buyer_gstin: p.buyerGstin, reverse_charge: p.reverseCharge }),
    /** Void an invoice nothing has been paid or credited against. */
    void: (id: string) => this.req("POST", `/v1/invoices/${encodeURIComponent(id)}/void`),
    /** CGST/SGST/IGST breakdown of a GST invoice. */
    lines: (id: string) => this.req<GetInvoicesByIdLinesResponse>("GET", `/v1/invoices/${encodeURIComponent(id)}/lines`),
  };

  /** Vendors you pay and the payouts to them. Zyvor controls and records; the transfer is made from your own bank. */
  readonly payouts = {
    createVendor: (p: { name: string; accountNumber?: string; ifsc?: string; vpa?: string; externalRef?: string }) =>
      this.req<{ id: string; name: string }>("POST", "/v1/vendors", { name: p.name, account_number: p.accountNumber, ifsc: p.ifsc, vpa: p.vpa, external_ref: p.externalRef }),
    vendors: () => this.req<Page<{ id: string; name: string; external_ref: string | null; account_last4: string | null; ifsc: string | null; vpa: string | null; archived: boolean }>>("GET", "/v1/vendors"),
    /** Full bank details for whoever makes the transfer (needs `payouts:approve`; every reveal is audited). */
    vendorDetails: (id: string) => this.req<{ id: string; details: { account_number: string | null; ifsc: string | null; vpa: string | null } }>("GET", `/v1/vendors/${encodeURIComponent(id)}/details`),
    /** Repeating a request with the same `externalRef` returns the original payout. `fromBalance` pays out of what the vendor is owed from split payments. */
    create: (p: { vendorId: string; amountMinor: number; purpose: string; currency?: string; externalRef?: string; fromBalance?: boolean }) =>
      this.req<PayoutRow>("POST", "/v1/payouts", { vendor_id: p.vendorId, amount_minor: p.amountMinor, purpose: p.purpose, currency: p.currency, external_ref: p.externalRef, from_balance: p.fromBalance }),
    /** Up to 200 payouts, all created or none. Safe to repeat with the same references. */
    createBulk: (items: { vendorId: string; amountMinor: number; purpose: string; externalRef?: string; fromBalance?: boolean }[]) =>
      this.req<{ count: number; already_existed: number; new_total_minor: number; data: PayoutRow[] }>("POST", "/v1/payouts/bulk", { payouts: items.map((p) => ({ vendor_id: p.vendorId, amount_minor: p.amountMinor, purpose: p.purpose, external_ref: p.externalRef, from_balance: p.fromBalance })) }),
    /** What a vendor is owed from split payments, and how much is free to pay out. */
    vendorBalance: (id: string) => this.req<{ vendor_id: string; balances: { currency: string; owed_minor: number; available_minor: number }[]; awaiting_capture_minor: number }>("GET", `/v1/vendors/${encodeURIComponent(id)}/balance`),
    list: (status?: PayoutRow["status"]) => this.req<Page<PayoutRow>>("GET", `/v1/payouts${this.qs({ status })}`),
    approve: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/approve`),
    reject: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/reject`),
    /** Record the bank reference (UTR) of the transfer you made; posts to the ledger once. */
    markPaid: (id: string, bankRef: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/mark_paid`, { bank_ref: bankRef }),
    /** Acquire mode: instruct the sponsor payout rail (falls back to sim rail without ZYVOR_PAYOUT_RAIL_URL). */
    sendRail: (id: string, p: { rail?: "imps" | "neft" | "upi"; accountNumber?: string; ifsc?: string; vpa?: string; beneficiaryName?: string } = {}) =>
      this.req<{ payout_id: string; status: string; bank_ref: string }>("POST", `/v1/payouts/${encodeURIComponent(id)}/send_rail`, { rail: p.rail, account_number: p.accountNumber, ifsc: p.ifsc, vpa: p.vpa, beneficiary_name: p.beneficiaryName }),
    cancel: (id: string) => this.req<PayoutRow>("POST", `/v1/payouts/${encodeURIComponent(id)}/cancel`),
  };

  /** Orchestrate vs acquire money mode and settlement delay. */
  readonly moneyMode = {
    get: () => this.req<{ money_mode: string; acquire_kyc_status: string; settlement_delay_days: number }>("GET", "/v1/money_mode"),
    set: (p: { moneyMode: "orchestrate" | "acquire"; settlementDelayDays?: number }) =>
      this.req<{ money_mode: string }>("PUT", "/v1/money_mode", { money_mode: p.moneyMode, settlement_delay_days: p.settlementDelayDays }),
  };

  readonly acquireKyc = {
    get: () => this.req<{ organisation_id: string; acquire_kyc_status: string; acquire_kyc_approved_at: string | null }>("GET", "/v1/acquire_kyc"),
    set: (status: "not_started" | "pending" | "approved" | "rejected" | "re_kyc_due") =>
      this.req<{ organisation_id: string; acquire_kyc_status: string }>("PUT", "/v1/acquire_kyc", { status }),
  };

  /** Acquire-mode settlement batches (escrow → merchant bank via payout rail). */
  readonly settlementBatches = {
    list: () => this.req<{ data: Record<string, unknown>[] }>("GET", "/v1/settlement_batches"),
    get: (id: string) => this.req<GetSettlementBatchesByIdResponse>("GET", `/v1/settlement_batches/${encodeURIComponent(id)}`),
    open: (currency = "INR") => this.req<{ id: string; status: string; currency: string }>("POST", "/v1/settlement_batches", { currency }),
    addItem: (id: string, p: { kind: "capture" | "refund" | "fee" | "hold"; reference: string; amountMinor: number }) =>
      this.req("POST", `/v1/settlement_batches/${encodeURIComponent(id)}/items`, { kind: p.kind, reference: p.reference, amount_minor: p.amountMinor }),
    release: (id: string) => this.req("POST", `/v1/settlement_batches/${encodeURIComponent(id)}/release`, {}),
  };

  readonly escrow = {
    proof: () => this.req<{ escrow_available_net: number; open_payables_net: number; balanced: boolean }>("GET", "/v1/escrow/proof"),
  };

  /** Network card tokens only — never send PAN/CVV. */
  readonly cardTokens = {
    list: () => this.req<{ tokens: { id: string; last4: string; brand: string | null }[] }>("GET", "/v1/card_tokens"),
    store: (p: { networkTokenRef: string; last4: string; brand?: string; customerId?: string; expMonth?: number; expYear?: number }) =>
      this.req("POST", "/v1/card_tokens", { network_token_ref: p.networkTokenRef, last4: p.last4, brand: p.brand, customer_id: p.customerId, exp_month: p.expMonth, exp_year: p.expYear }),
    revoke: (id: string) => this.req("POST", `/v1/card_tokens/${encodeURIComponent(id)}/revoke`, {}),
  };

  readonly customers = {
    list: (externalRef?: string) => this.req<Page<Customer>>("GET", `/v1/customers${this.qs({ external_ref: externalRef })}`),
    get: (id: string) => this.req<Customer>("GET", `/v1/customers/${encodeURIComponent(id)}`),
    /** Correct details. A field left out stays; an empty string clears it. Issued invoices keep the GSTIN they were issued with. */
    update: (id: string, p: { name?: string; email?: string; phone?: string; gstin?: string; address1?: string; location?: string; pincode?: string }) => this.req<Customer>("PATCH", `/v1/customers/${encodeURIComponent(id)}`, p),
    create: (p: { name: string; email?: string; phone?: string; externalRef?: string; gstin?: string; address1?: string; location?: string; pincode?: string }) =>
      this.req<{ id: string }>("POST", "/v1/customers", { name: p.name, email: p.email, phone: p.phone, external_ref: p.externalRef, gstin: p.gstin, address1: p.address1, location: p.location, pincode: p.pincode }),
  };

  /** UPI AutoPay. The payer approves once; you schedule debits, each with a pre-debit notice. Simulator only until a live provider supports it. */
  readonly mandates = {
    create: (p: { customerId: string; maxAmountMinor: number; frequency: "daily" | "weekly" | "fortnightly" | "monthly" | "bimonthly" | "quarterly" | "half_yearly" | "yearly" | "as_presented"; endDate: string; startDate?: string; provider?: string; externalRef?: string; idempotencyKey?: string }) => {
      if (!Number.isSafeInteger(p.maxAmountMinor) || p.maxAmountMinor <= 0) throw new RangeError("maxAmountMinor must be a positive integer (paise)");
      return this.req<Mandate>("POST", "/v1/mandates", { customer_id: p.customerId, max_amount_minor: p.maxAmountMinor, frequency: p.frequency, start_date: p.startDate, end_date: p.endDate, provider: p.provider, external_ref: p.externalRef }, key(p.idempotencyKey));
    },
    list: (p: { status?: string; customerId?: string } = {}) => this.req<Page<Mandate>>("GET", `/v1/mandates${this.qs({ status: p.status, customer_id: p.customerId })}`),
    get: (id: string) => this.req<Mandate & { debits?: Record<string, unknown>[] }>("GET", `/v1/mandates/${encodeURIComponent(id)}`),
    /** Ask the provider where it stands (approved, declined, revoked by the payer). */
    refresh: (id: string) => this.req<Mandate>("POST", `/v1/mandates/${encodeURIComponent(id)}/refresh`, {}),
    pause: (id: string) => this.req<Mandate>("POST", `/v1/mandates/${encodeURIComponent(id)}/pause`, {}),
    resume: (id: string) => this.req<Mandate>("POST", `/v1/mandates/${encodeURIComponent(id)}/resume`, {}),
    revoke: (id: string) => this.req<Mandate>("POST", `/v1/mandates/${encodeURIComponent(id)}/revoke`, {}),
    /** Schedule a debit for an India-time day far enough ahead for the payer's notice. */
    scheduleDebit: (id: string, p: { amountMinor: number; dueDate: string; invoiceId?: string; idempotencyKey?: string }) => {
      if (!Number.isSafeInteger(p.amountMinor) || p.amountMinor <= 0) throw new RangeError("amountMinor must be a positive integer (paise)");
      return this.req<PostMandatesByIdDebitsResponse>("POST", `/v1/mandates/${encodeURIComponent(id)}/debits`, { amount_minor: p.amountMinor, due_date: p.dueDate, invoice_id: p.invoiceId }, key(p.idempotencyKey));
    },
    debits: (id: string) => this.req<GetMandatesByIdDebitsResponse>("GET", `/v1/mandates/${encodeURIComponent(id)}/debits`),
    /** Sandbox aid: run the mandate pass now (the worker does this on its own). */
    run: () => this.req<Record<string, unknown>>("POST", "/v1/mandates/run", {}),
  };

  readonly fx = {
    /** Record the INR-per-USD rate you use for a day (e.g. "83.2150"). One per day, never edited. Needs an admin key. */
    setRate: (p: { currency: "USD"; rate: string; asOf?: string; note?: string }) => this.req<{ id: string }>("POST", "/v1/fx/rates", { currency: p.currency, rate: p.rate, as_of: p.asOf, note: p.note }),
    rates: () => this.req<Page<{ id: string; currency: string; rate: string; as_of: string; note: string | null; set_by: string }>>("GET", "/v1/fx/rates"),
    convert: (p: { amountMinor: number; from: "INR" | "USD"; to: "INR" | "USD"; asOf?: string }) => this.req<{ amount_minor: number; currency: string; rate: string | null; rate_date?: string }>("GET", `/v1/fx/convert${this.qs({ amount_minor: p.amountMinor, from: p.from, to: p.to, as_of: p.asOf })}`),
    exposure: (p: { asOf?: string } = {}) => this.req<{ as_of: string; rate_date: string | null; outstanding_usd_minor: number; complete: boolean; booked_inr_minor: number | null; current_inr_minor: number | null; unrealised_inr_minor: number | null; data: Array<{ invoice_id: string; number: string; outstanding_usd_minor: number; raised_on: string; booked_inr_minor: number | null; current_inr_minor: number | null; unrealised_inr_minor: number | null }> }>("GET", `/v1/fx/exposure${this.qs({ as_of: p.asOf })}`),
  };

  /** Named sets of Payments filters. Personal unless `shared`. */
  readonly savedViews = {
    list: () => this.req<{ data: SavedView[] }>("GET", "/v1/saved_views?scope=payments"),
    create: (p: { name: string; query: SavedViewQuery; shared?: boolean }) => this.req<SavedView>("POST", "/v1/saved_views", { scope: "payments", name: p.name, query: p.query, shared: p.shared ?? false }),
    delete: (id: string) => this.req<{ ok: boolean }>("DELETE", `/v1/saved_views/${encodeURIComponent(id)}`),
  };

  /** Addresses that asked (through the unsubscribe link) to stop a business's automatic emails. */
  readonly mailSuppressions = {
    list: () => this.req<GetMailSuppressionsResponse>("GET", "/v1/mail_suppressions"),
    /** Send automatic emails to the address again. */
    remove: (email: string) => this.req<{ email: string; removed: boolean }>("DELETE", `/v1/mail_suppressions/${encodeURIComponent(email)}`),
  };
  readonly testClock = {
    /** The date this test environment sees, and whether its test clock is set. */
    get: () => this.req<GetTestClockResponse>("GET", "/v1/test_clock"),
    /** Test environments only: move the clock forward to `date` (YYYY-MM-DD) or by `advanceDays` (send one). Billing, reminders and dates follow it. */
    set: (to: { date: string } | { advanceDays: number }) =>
      this.req<PostTestClockResponse>("POST", "/v1/test_clock", "date" in to ? { date: to.date } : { advance_days: to.advanceDays }),
    /** Test environments only: back to real time. */
    clear: () => this.req<DeleteTestClockResponse>("DELETE", "/v1/test_clock"),
  };
  readonly testData = {
    /** Test environments only: clears payments, invoices, customers and the ledger; keeps settings, keys, providers, branches, vendors, plans, coupons. */
    reset: () => this.req<{ ok: boolean; removed: Record<string, number> }>("POST", "/v1/test_data/reset", { confirm: "reset test data" }),
  };

  readonly emailLogo = {
    /** Upload a PNG, JPEG, GIF or WebP image (up to 200 KB) as the logo in payer emails. Needs ZYVOR_PUBLIC_API_URL on the server. */
    upload: (image: Uint8Array) => this.rawReq<{ email_logo_url: string }>("POST", "/v1/settings/logo", image),
    /** Remove the uploaded logo and its address. */
    remove: () => this.rawReq<{ email_logo_url: null }>("DELETE", "/v1/settings/logo"),
  };

  readonly settings = {
    get: () => this.req<Settings>("GET", "/v1/settings"),
    update: (patch: SettingsPatch) => this.req<{ ok: boolean }>("POST", "/v1/settings", patch),
  };

  /** Bank statement import for reconciliation (JSON lines, CSV, MT940 or CAMT.053). */
  readonly bankStatements = {
    importCsv: (csv: string) => this.req("POST", "/v1/bank_statements/import_csv", { csv }),
    importMt940: (text: string) => this.req("POST", "/v1/bank_statements/import_mt940", { text }),
    importCamt: (text: string) => this.req("POST", "/v1/bank_statements/import_camt", { text }),
  };

  readonly coupons = {
    /** `percent` takes `value` in basis points (1000 = 10%); `flat` in minor units. Redeemed on fixed-amount payment links. */
    create: (p: { code: string; kind: "percent" | "flat"; value: number; maxUses?: number; expiresAt?: string }) =>
      this.req<{ id: string; code: string }>("POST", "/v1/coupons", { code: p.code, kind: p.kind, value: p.value, max_uses: p.maxUses, expires_at: p.expiresAt }),
    list: () => this.req<Page<{ id: string; code: string; kind: "percent" | "flat"; value: number; max_uses: number | null; uses: number; expires_at: string | null; status: "active" | "expired" | "used_up" | "deactivated" }>>("GET", "/v1/coupons"),
    deactivate: (id: string) => this.req("POST", `/v1/coupons/${encodeURIComponent(id)}/deactivate`),
  };

  readonly onboarding = {
    get: () => this.req<{ status: "not_started" | "draft" | "in_review" | "approved" | "rejected"; profile: { legal_name: string; business_type: string; pan: string; gstin: string | null; address: string; submitted_by: string | null; reviewed_by: string | null; review_note: string | null } | null; documents: { id: string; kind: string; reference: string; status: string }[]; /** uploaded files, what was read from each and how it compares with the profile */ uploads: OnboardingUpload[]; document_reader_available: boolean; bank_accounts: { id: string; holder_name: string; ifsc: string; account_last4: string; verified: boolean }[]; /** `true` when the deployment runs identity and bank checks itself */ automated_checks: boolean; /** why a submission went to a person */ review_reasons: string[]; checks: { kind: "pan" | "gstin" | "bank"; subject: string; outcome: "pass" | "fail" | "inconclusive"; detail: string }[] }>("GET", "/v1/onboarding"),
    saveProfile: (p: { legalName: string; businessType: string; pan: string; gstin?: string; address: string }) =>
      this.req("PUT", "/v1/onboarding", { legal_name: p.legalName, business_type: p.businessType, pan: p.pan, gstin: p.gstin, address: p.address }),
    /** After approval the details are locked; ask Zyvor to change them. Nothing changes (and live payouts are held) until a reviewer decides. */
    requestProfileChange: (p: { legalName: string; businessType: string; pan: string; gstin?: string; address: string; reason: string }) =>
      this.req<{ id: string; status: "pending"; summary: string }>("POST", "/v1/onboarding/profile_change", { legal_name: p.legalName, business_type: p.businessType, pan: p.pan, gstin: p.gstin, address: p.address, reason: p.reason }),
    addDocument: (p: { kind: string; reference: string }) => this.req<{ id: string }>("POST", "/v1/onboarding/documents", p),
    /** Document files: stored encrypted, read for the facts on them and compared with the profile. PDF, PNG or JPEG, up to 5 MB. */
    uploads: {
      list: () => this.req<{ data: OnboardingUpload[]; reader_available: boolean }>("GET", "/v1/onboarding/uploads"),
      upload: (p: { kind: string; filename: string; data: Uint8Array }) =>
        this.req<OnboardingUpload>("POST", "/v1/onboarding/uploads", { kind: p.kind, filename: p.filename, data_base64: Buffer.from(p.data).toString("base64") }),
      delete: (id: string) => this.req<{ id: string; deleted: boolean }>("DELETE", `/v1/onboarding/uploads/${encodeURIComponent(id)}`),
      download: (id: string) => this.bytes(`/v1/onboarding/uploads/${encodeURIComponent(id)}/file`),
    },
    addBankAccount: (p: { holderName: string; ifsc: string; accountNumber: string }) =>
      this.req<{ id: string; account_last4: string; check: { outcome: "pass" | "fail" | "inconclusive"; detail: string } | null }>("POST", "/v1/onboarding/bank_accounts", { holder_name: p.holderName, ifsc: p.ifsc, account_number: p.accountNumber }),
    verifyBankAccount: (id: string) => this.req("POST", `/v1/onboarding/bank_accounts/${encodeURIComponent(id)}/verify`),
    /** `automatic: true` means every check passed and the business was approved without a person. */
    submit: () => this.req<{ status: "approved" | "in_review"; automatic: boolean; reasons: string[] }>("POST", "/v1/onboarding/submit"),
    review: (p: { decision: "approve" | "reject"; note?: string }) => this.req("POST", "/v1/onboarding/review", p),
  };

  readonly installments = {
    /** Split an agreed total into dated invoices: exact (remainder on the last), all or nothing, idempotent by `externalRef`. */
    create: (p: { customerId: string; name: string; totalMinor: number; instalments: number; firstDueDate: string; numberPrefix: string; everyUnit?: "month" | "day"; every?: number; currency?: string; externalRef?: string }) =>
      this.req<{ id: string; remaining_minor: number; invoices: { invoice_id: string; number: string; amount_minor: number; due_date: string }[] }>("POST", "/v1/installment_plans", { customer_id: p.customerId, name: p.name, total_minor: p.totalMinor, instalments: p.instalments, first_due_date: p.firstDueDate, number_prefix: p.numberPrefix, every_unit: p.everyUnit, every: p.every, currency: p.currency, external_ref: p.externalRef }),
    retrieve: (id: string) => this.req<GetInstallmentPlansByIdResponse>("GET", `/v1/installment_plans/${encodeURIComponent(id)}`),
  };

  readonly gst = {
    /** GSTR-1 style summary for `YYYY-MM`: a preparation aid, Zyvor does not file returns. */
    /** DRAFT e-invoice JSON (IRP schema 1.1) for a B2B tax invoice. Not submitted, no IRN, not checked against the portal. */
    einvoice: (invoiceId: string) => this.req<GetInvoicesByIdEinvoiceResponse>("GET", `/v1/invoices/${invoiceId}/einvoice`),
    gstr1: (period: string) => this.req<Record<string, unknown>>("GET", `/v1/gst/gstr1${this.qs({ period })}`),
  };

  readonly milestones = {
    list: () => this.req<GetMilestonesResponse>("GET", "/v1/milestones"),
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
    list: (status?: "open" | "under_review" | "won" | "lost") => this.req<Page<{ id: string; attempt_id: string; intent_id: string; amount_minor: number; reason: string; status: string; respond_by: string | null; evidence_count: number; created_at: string }>>("GET", `/v1/disputes${this.qs({ status })}`),
    addEvidence: (id: string, p: { note: string; url?: string }) => this.req("POST", `/v1/disputes/${encodeURIComponent(id)}/evidence`, p),
    /** `lost` books the loss on the ledger and re-opens the paid invoice; `won` releases the hold. */
    resolve: (id: string, outcome: "won" | "lost") => this.req("POST", `/v1/disputes/${encodeURIComponent(id)}/resolve`, { outcome }),
  };

  readonly collections = {
    ageing: (asOf?: string) => this.req<{ as_of: string; total_outstanding_inr_minor?: number | null; data: { currency: string; bucket: string; outstanding_minor: number; count: number; invoices: { id: string; number: string; customer_id: string | null; due_date: string | null; outstanding_minor: number; days_past_due: number }[] }[] }>("GET", `/v1/collections/ageing${this.qs({ as_of: asOf })}`),
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
    runDunning: (p: { channel?: "email" | "sms" | "whatsapp" | "call" | "other"; minDaysOverdue?: number; repeatAfterDays?: number; asOf?: string } = {}) =>
      this.req<{ reminded: number; reminder_ids: string[] }>("POST", "/v1/collections/dunning/run", { channel: p.channel, min_days_overdue: p.minDaysOverdue, repeat_after_days: p.repeatAfterDays, as_of: p.asOf }),
    reminders: (invoiceId: string) => this.req<GetInvoicesByIdRemindersResponse>("GET", `/v1/invoices/${encodeURIComponent(invoiceId)}/reminders`),
    remind: (invoiceId: string, p: { channel: "email" | "sms" | "whatsapp" | "call" | "other"; message?: string }) => this.req("POST", `/v1/invoices/${encodeURIComponent(invoiceId)}/reminders`, p),
  };

  readonly branches = {
    list: () => this.req<Page<{ id: string; name: string }>>("GET", "/v1/branches"),
    /** Branches a person can be tied to: read from the organisation's home environment, as invite checks them. */
    assignable: () => this.req<Page<{ id: string; name: string }>>("GET", "/v1/console/branches"),
    create: (name: string) => this.req<{ id: string; name: string }>("POST", "/v1/branches", { name }),
  };

  readonly cash = {
    /** Cash taken (`sale`), handed back (`refund`) or put in the till (`float`). Never edited: correct with an opposite entry. */
    add: (p: { branchId: string; counterId?: string; kind: "sale" | "refund" | "float"; amountMinor: number; note?: string }) =>
      this.req<{ id: string }>("POST", "/v1/cash_entries", { branch_id: p.branchId, counter_id: p.counterId, kind: p.kind, amount_minor: p.amountMinor, note: p.note }),
  };

  readonly closings = {
    /** Close a branch's IST day (`YYYY-MM-DD`): expected cash and digital totals against the counted cash. */
    create: (p: { branchId: string; day: string; countedCashMinor: number; note?: string }) =>
      this.req<{ id: string; expected_cash_minor: number; counted_cash_minor: number; difference_minor: number; digital: { method: string; payments: number; net_minor: number }[]; status: string }>("POST", "/v1/closings", { branch_id: p.branchId, day: p.day, counted_cash_minor: p.countedCashMinor, note: p.note }),
    list: (p: { branchId?: string; status?: "pending_signoff" | "signed_off" } = {}) =>
      this.req<Page<{ id: string; branch_id: string; day: string; expected_cash_minor: number; counted_cash_minor: number; difference_minor: number; digital: { method: string; payments: number; net_minor: number }[]; note: string | null; counted_by: string; signed_off_by: string | null; status: "pending_signoff" | "signed_off" }>>("GET", `/v1/closings${this.qs({ branch_id: p.branchId, status: p.status })}`),
    /** A different person from whoever counted. */
    signOff: (id: string) => this.req("POST", `/v1/closings/${encodeURIComponent(id)}/signoff`),
  };

  readonly insights = {
    /** What is owed by due week, plus overdue, promised and payouts waiting. A schedule, not a prediction. */
    cashflow: (weeks?: number) => this.req<{ as_of: string; currency: string; weeks: { starts: string; due_minor: number }[]; overdue_minor: number; due_later_minor: number; no_due_date_minor: number; promised_minor: number; payouts_waiting_minor: number; daily_receipts_run_rate_minor: number; note: string }>("GET", `/v1/insights/cashflow${this.qs({ weeks })}`),
    /** Refund patterns worth a person's look; not findings of wrongdoing. */
    refundAnomalies: () => this.req<{ refunds_last_7_days: number; weekly_average_before: number; flags: { code: string; message: string }[]; note: string }>("GET", "/v1/insights/refund_anomalies"),
  };

  readonly counters = {
    create: (p: { branchId: string; name: string }) => this.req<{ id: string; branch_id: string; name: string }>("POST", "/v1/counters", { branch_id: p.branchId, name: p.name }),
    list: (branchId?: string) => this.req<Page<{ id: string; branch_id: string; name: string; active: boolean }>>("GET", `/v1/counters${this.qs({ branch_id: branchId })}`),
    /** Also cancels the counter's QR links. */
    deactivate: (id: string) => this.req("POST", `/v1/counters/${encodeURIComponent(id)}/deactivate`),
    /** Captured money per IST day, counter and method, net of refunds. Dates are YYYY-MM-DD. */
    branchSummary: (branchId: string, p: { from?: string; to?: string } = {}) =>
      this.req<{ branch: { id: string; name: string }; from: string; to: string; data: { day: string; counter_id: string | null; method: string; currency: string; payments: number; gross_minor: number; refunded_minor: number; net_minor: number }[] }>("GET", `/v1/branches/${encodeURIComponent(branchId)}/summary${this.qs({ from: p.from, to: p.to })}`),
  };

  readonly paymentLinks = {
    /** One URL, many payers: opening it mints a fresh payment intent. Give `amountMinor` or `invoiceId`. */
    create: (p: { description?: string; amountMinor?: number; currency?: string; invoiceId?: string; customerId?: string; maxUses?: number; expiresAt?: string; splits?: { vendorId: string; amountMinor: number }[]; /** A fee the payer covers, in basis points (200 = 2%, at most 1000). Fixed-amount links only. */ convenienceFeeBp?: number; /** Print this link's QR at a counter: payments are attributed to it and its branch. */ counterId?: string; branchId?: string; /** The payer types the amount (a counter QR). Bound it with min/max; no amount, invoice, splits or fee. */ openAmount?: boolean; minAmountMinor?: number; maxAmountMinor?: number; /** Email the customer once in the last day before `expiresAt` (needs `customerId` and `expiresAt`). */ remindBeforeExpiry?: boolean }) =>
      this.req<{ id: string; token: string; path: string }>("POST", "/v1/payment_links", { open_amount: p.openAmount, min_amount_minor: p.minAmountMinor, max_amount_minor: p.maxAmountMinor, counter_id: p.counterId, branch_id: p.branchId, description: p.description, amount_minor: p.amountMinor, currency: p.currency, invoice_id: p.invoiceId, customer_id: p.customerId, max_uses: p.maxUses, expires_at: p.expiresAt, splits: p.splits?.map((x) => ({ vendor_id: x.vendorId, amount_minor: x.amountMinor })), convenience_fee_bp: p.convenienceFeeBp, remind_before_expiry: p.remindBeforeExpiry }),
    /** SVG QR code of the link's public checkout address (needs `ZYVOR_CHECKOUT_PUBLIC_URL` on the API). */
    qrUrl: (id: string) => `${this.base}/v1/payment_links/${encodeURIComponent(id)}/qr.svg`,
    list: () => this.req<Page<{ id: string; path: string; description: string | null; amount_minor: number | null; currency: string; invoice_id: string | null; max_uses: number | null; uses: number; status: "active" | "expired" | "used_up" | "canceled"; created_at: string; can_email: boolean }>>("GET", "/v1/payment_links"),
    cancel: (id: string) => this.req("POST", `/v1/payment_links/${encodeURIComponent(id)}/cancel`),
    /** Email an active link to the customer it was made for (once per link per day). */
    send: (id: string) => this.req("POST", `/v1/payment_links/${encodeURIComponent(id)}/send`),
  };

  /** Named console users with roles, Argon2id passwords and TOTP. Needs an admin key. */
  readonly consoleUsers = {
    create: (p: { username: string; password: string; role: "admin" | "finance" | "viewer" | "cashier"; branchId?: string }) => this.req<{ id: string; username: string; role: string; branch_id: string | null }>("POST", "/v1/console/users", { username: p.username, password: p.password, role: p.role, branch_id: p.branchId }),
    list: () => this.req<Page<{ id: string; username: string; role: "admin" | "finance" | "viewer" | "cashier"; branch_id: string | null; mfa_enrolled: boolean; last_login_at: string | null; locked: boolean; disabled: boolean }>>("GET", "/v1/console/users"),
    /** Email someone a one-time link to join. The link works once and expires in 7 days. */
    invite: (p: { email: string; role: "admin" | "finance" | "viewer" | "cashier"; branchId?: string }) => this.req<{ id: string; email: string; role: string; expires_at: string }>("POST", "/v1/console/invitations", { email: p.email, role: p.role, branch_id: p.branchId }),
    invitations: () => this.req<Page<{ id: string; email: string; role: string; expires_at: string; invited_by: string | null }>>("GET", "/v1/console/invitations"),
    revokeInvitation: (id: string) => this.req("POST", `/v1/console/invitations/${encodeURIComponent(id)}/revoke`),
    disable: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/disable`),
    enable: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/enable`),
    setPassword: (id: string, password: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/password`, { password }),
    mfaBegin: (id: string) => this.req<{ secret: string; otpauth_uri: string }>("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/begin`),
    mfaConfirm: (id: string, otp: string) => this.req<{ recovery_codes: string[] }>("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/confirm`, { otp }),
    mfaReset: (id: string) => this.req("POST", `/v1/console/users/${encodeURIComponent(id)}/mfa/reset`),
    /** Throws ZyvorError: `otp_required` (401) when a code is needed, `account_locked` (423) after repeated failures. */
    authenticate: (p: { username: string; password: string; otp?: string }) => this.req<{ user: { id: string; username: string; role: "admin" | "finance" | "viewer" | "cashier"; branch_id?: string | null }; mfa_enrolled: boolean }>("POST", "/v1/console/authenticate", p),
  };

  readonly audit = {
    /** Recompute the tamper-evident chain; `ok: false` names the first broken seal. */
    verify: () => this.req<{ ok: boolean; seals_checked: number; sealed_events?: number; unsealed_events?: number; reason?: string; broken_seal?: number }>("GET", "/v1/audit_events/verify"),
    seal: () => this.req<{ sealed_events: number }>("POST", "/v1/audit_events/seal"),
  };

  /** Key hygiene. */
  readonly keys = {
    list: () => this.req<Page<{ id: string; name: string; prefix: string; scopes: string[]; branch_id: string | null; created_at: string; revoked_at: string | null; expires_at: string | null; last_used_at: string | null; allowed_cidrs: string[] }>>("GET", "/v1/api_keys"),
    revoke: (id: string) => this.req("POST", `/v1/api_keys/${encodeURIComponent(id)}/revoke`),
    /** `branchId` pins the key to one branch (a till or a manager's key); such a key cannot hold `admin`. The secret is shown once. */
    create: (p: { name: string; scopes: string[]; branchId?: string; expiresAt?: string; allowedCidrs?: string[] }) =>
      this.req<{ id: string; key: string }>("POST", "/v1/api_keys", { name: p.name, scopes: p.scopes, branch_id: p.branchId, expires_at: p.expiresAt, allowed_cidrs: p.allowedCidrs }),
    /** Mint a replacement with the same scopes; the old key keeps working for `overlapHours` (default 24). */
    rotate: (id: string, overlapHours?: number) => this.req<{ id: string; key: string }>("POST", `/v1/api_keys/${encodeURIComponent(id)}/rotate`, { overlap_hours: overlapHours }),
  };

  readonly providers = {
    list: () => this.req<Page<{ id: string; provider: string; enabled: boolean; label: string; base_url: string | null; webhook_url: string; capabilities: { methods?: string[] } | null }>>("GET", "/v1/provider_accounts"),
    /** Test environments only: an in-process provider whose outcome the payer's UPI handle chooses. */
    enableSimulator: () => this.req<{ id: string }>("POST", "/v1/provider_accounts", { provider: "sim", credentials: {}, webhook_secret: "sim-unused" }),
    /** Store these credentials for the provider, replacing its account if there is one, and turn it on. Leave `baseUrl` out for the provider's own test or live address. A Razorpay key must match the environment (rzp_test_ in test, rzp_live_ in live). */
    connect: (provider: "razorpay" | "cashfree" | "custom" | "phonepe" | "sponsor_upi", p: { credentials: Record<string, string | boolean>; webhookSecret: string; label?: string; baseUrl?: string }) =>
      this.req<{ id: string; provider: string; enabled: boolean; replaced: boolean }>("PUT", `/v1/provider_accounts/${provider}`, { credentials: p.credentials, webhook_secret: p.webhookSecret, label: p.label, base_url: p.baseUrl }),
    /** Ask the provider whether it accepts the credentials. Read-only; a refusal comes back as `ok: false`. */
    check: (provider: string) => this.req<{ provider: string; ok: boolean; detail: string; env: string }>("POST", `/v1/provider_accounts/${encodeURIComponent(provider)}/check`),
    setEnabled: (provider: string, enabled: boolean) => this.req("POST", `/v1/provider_accounts/${encodeURIComponent(provider)}/enabled`, { enabled }),
  };

  /** Zyvor reviews every business before live mode opens. Ask once your business details are approved. */
  readonly liveAccess = {
    get: () => this.req<{ status: "not_requested" | "pending" | "approved" | "rejected" | "revoked"; note: string | null; requested_at?: string; reviewed_at?: string | null; round?: number; /** when to expect an answer: a target, not a guarantee */ expected_by?: string | null; sla_hours?: number; onboarding_approved: boolean; /** changes made after approval that Zyvor reviews (a new bank account, new legal details); payouts are held while one is pending */ changes?: { id: string; kind: "bank_account" | "profile"; summary: string; status: "pending" | "approved" | "rejected"; note: string | null; expected_by?: string | null }[] }>("GET", "/v1/live_access"),
    request: () => this.req<{ status: "pending" | "approved" }>("POST", "/v1/live_access"),
  };

  readonly webhooks = {
    /** Every event type with its meaning. Choose which to receive with the `webhook_events` setting. */
    catalogue: () => this.req<Page<{ type: string; description: string }>>("GET", "/v1/events/catalogue"),
    outbox: (status?: string) => this.req<Page<Record<string, unknown>>>("GET", `/v1/outbox${this.qs({ status })}`),
    /** Per-attempt delivery history for one outbound event (newest first). */
    deliveries: (eventId: string) => this.req<Page<{ id: number; attempt_no: number; ok: boolean; http_status: number | null; error: string | null; latency_ms: number | null; created_at: string }>>("GET", `/v1/outbox/${encodeURIComponent(eventId)}/deliveries`),
    replay: (eventId: string) => this.req("POST", `/v1/outbox/${encodeURIComponent(eventId)}/replay`),
    /** Queue a `webhook.test` event for your webhook endpoint. Needs a webhook URL in settings. */
    sendTest: () => this.req<{ id: string; status: string }>("POST", "/v1/outbox/test"),
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
