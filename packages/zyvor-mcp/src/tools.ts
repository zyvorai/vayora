import { createHash } from "node:crypto";
import * as z from "zod/v4";
import type { Zyvor } from "@zyvor/node";
import type { McpConfig } from "./config.js";

/** A refusal the server makes itself (a guard, a mismatched confirm). The message is safe to show the model. */
export class Refusal extends Error {}

export interface ToolContext {
  client: Zyvor;
  config: McpConfig;
  /** Throws Refusal unless this key and configuration may use write tools. Called before every write. */
  guardWrite(): Promise<void>;
}

export interface ToolDef {
  name: string;
  title: string;
  /** What it does and what to expect, in plain words. Scope, mode and an example are appended when registered. */
  description: string;
  /** The Vayora API scope the key needs. */
  scope: string;
  kind: "read" | "write";
  /** Needs a `confirm` argument that repeats the id of the thing acted on. */
  destructive?: boolean;
  input: z.ZodObject<z.ZodRawShape>;
  /** Arguments that must validate against `input`; the first is shown in the description. */
  examples: Record<string, unknown>[];
  run(args: Record<string, unknown>, ctx: ToolContext): Promise<unknown>;
}

function define<S extends z.ZodRawShape>(d: Omit<ToolDef, "input" | "run"> & { input: z.ZodObject<S>; run: (args: z.output<z.ZodObject<S>>, ctx: ToolContext) => Promise<unknown> }): ToolDef {
  return d as unknown as ToolDef;
}

const limit = z.number().int().min(1).max(50).default(20).describe("How many rows to return (1 to 50, default 20).");
const day = (what: string) => z.string().regex(/^\d{4}-\d{2}-\d{2}$/, "use YYYY-MM-DD").describe(what);
const id = (what: string, example: string) => z.string().min(3).max(80).regex(/^[A-Za-z0-9_.:-]+$/, "an id has letters, digits, _ . : - only").describe(`${what} (looks like ${example}).`);

/** `confirm` must repeat the id, so a model cannot trigger a write from a vague instruction in some text it read. */
function confirmEchoes(confirm: string, expected: string, what: string): void {
  if (confirm !== expected) throw new Refusal(`confirm must be exactly "${expected}" (the ${what} you mean to act on). Nothing was changed.`);
}

const cut = <T>(rows: T[] | undefined, n: number): T[] => (rows ?? []).slice(0, n);

// ---------------------------------------------------------------------------------------------------------------
// Read tools. They change nothing in Vayora.
// ---------------------------------------------------------------------------------------------------------------

const readTools: ToolDef[] = [
  define({
    name: "whoami",
    title: "Which key and mode am I using",
    description: "Shows the environment (test or live), the scopes of the API key this server uses, and whether write tools are switched on. Call it first to know what is possible. Never shows the key.",
    scope: "any valid key",
    kind: "read",
    input: z.object({}),
    examples: [{}],
    run: async (_a, { client, config }) => {
      const me = await client.me();
      return {
        env: me.env,
        scopes: me.scopes,
        actor: me.actor,
        branch_pinned: me.branch_id !== null,
        write_tools_enabled: config.writes,
        max_payment_link_minor: config.writes ? config.maxLinkMinor : undefined,
        max_unapproved_refund_minor: config.writes ? config.maxUnapprovedRefundMinor : undefined,
      };
    },
  }),
  define({
    name: "list_payment_intents",
    title: "List payments",
    description: "Lists payment intents (a payment a payer was asked to make), newest first, with status and amount in paise. `q` matches an id exactly or part of a reference or description.",
    scope: "payments:read",
    kind: "read",
    input: z.object({
      status: z.enum(["requires_payment_method", "processing", "succeeded", "failed", "canceled"]).optional().describe("Only payments in this state."),
      q: z.string().max(80).optional().describe("An id, or part of an external reference or description."),
      created_from: day("Created on or after this day (IST)").optional(),
      created_to: day("Created on or before this day (IST)").optional(),
      limit,
    }),
    examples: [{ status: "succeeded", created_from: "2026-11-01", limit: 10 }],
    run: async (a, { client }) => {
      const r = await client.intents.list({ status: a.status, q: a.q, createdFrom: a.created_from, createdTo: a.created_to, limit: a.limit });
      return { data: cut(r.data, a.limit), has_more: r.has_more };
    },
  }),
  define({
    name: "get_payment_intent",
    title: "Get one payment with its attempts",
    description: "One payment intent and each attempt made on it (provider, method, status, failure reason, refunded amount). Provider raw responses and payer action links are left out.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ intent_id: id("The payment intent id", "pi_abc123") }),
    examples: [{ intent_id: "pi_abc123" }],
    run: (a, { client }) => client.intents.retrieve(a.intent_id),
  }),
  define({
    name: "get_payment_timeline",
    title: "Timeline of a payment",
    description: "The ordered story of one payment (intent, attempts, webhooks, ledger postings, refunds, disputes, settlement) with a plain sentence on where it stands now.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ intent_id: id("The payment intent id", "pi_abc123") }),
    examples: [{ intent_id: "pi_abc123" }],
    run: (a, { client }) => client.intents.timeline(a.intent_id),
  }),
  define({
    name: "get_payment_attempt",
    title: "Get one payment attempt",
    description: "One attempt (a try at paying through one provider with one method): its status, failure reason and routing decision. `unknown` means the provider's outcome is not yet known; nothing else can be tried until it resolves.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ attempt_id: id("The attempt id", "pa_abc123") }),
    examples: [{ attempt_id: "pa_abc123" }],
    run: (a, { client }) => client.attempts.retrieve(a.attempt_id),
  }),
  define({
    name: "explain_routing_decision",
    title: "Why was this provider chosen",
    description: "For each attempt on a payment, which providers were considered, which was chosen and why, in the plain sentences the router stored (including providers skipped because their circuit breaker was open). Reads the stored decision; it does not run the router again.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ intent_id: id("The payment intent id", "pi_abc123") }),
    examples: [{ intent_id: "pi_abc123" }],
    run: async (a, { client }) => {
      const r = await client.intents.retrieve(a.intent_id);
      return {
        intent_id: r.intent.id,
        attempts: r.attempts.map((t) => ({ attempt_id: t.id, provider: t.provider, method: t.method, status: t.status, route: t.route })),
        note: "Attempts made before routing explanations existed have no sentence.",
      };
    },
  }),
  define({
    name: "list_refunds",
    title: "List refunds",
    description: "Lists refunds, optionally by status. `pending_approval` means a second person must approve it in the Vayora console; this server cannot approve anything.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ status: z.enum(["pending_approval", "processing", "processed", "failed", "rejected", "unknown"]).optional().describe("Only refunds in this state."), limit }),
    examples: [{ status: "pending_approval" }],
    run: async (a, { client }) => {
      const r = await client.refunds.list({ status: a.status, limit: a.limit });
      return { data: cut(r.data, a.limit), has_more: r.has_more };
    },
  }),
  define({
    name: "list_settlements",
    title: "List settlements",
    description: "Settlements the providers have paid (or are paying), with amounts in paise. Use explain_settlement for the detail of one.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ limit }),
    examples: [{ limit: 10 }],
    run: async (a, { client }) => {
      const r = await client.settlements.list();
      return { data: cut(r.data as unknown[], a.limit), shown_at_most: a.limit };
    },
  }),
  define({
    name: "explain_settlement",
    title: "Explain a settlement",
    description: "Where every rupee of one settlement went: the payments, refunds, fees and adjustments it contains, the bank credit it matched (if any), any difference, and a plain-language narrative. Amounts in paise.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ settlement_id: id("The settlement id", "st_abc123") }),
    examples: [{ settlement_id: "st_abc123" }],
    run: (a, { client }) => client.settlements.explain(a.settlement_id),
  }),
  define({
    name: "list_bank_lines",
    title: "List imported bank statement lines",
    description: "Bank statement lines with their match state: matched, suggested (a person should confirm or reject) or unmatched. Narrations are withheld unless the operator allows them, because they often hold names.",
    scope: "recon:read",
    kind: "read",
    input: z.object({
      status: z.enum(["unmatched", "suggested", "matched"]).optional().describe("Only lines in this match state."),
      direction: z.enum(["credit", "debit"]).optional().describe("Only credits or only debits."),
      from: day("Posted on or after this day").optional(),
      to: day("Posted on or before this day").optional(),
      limit,
    }),
    examples: [{ status: "suggested", limit: 10 }],
    run: async (a, { client }) => {
      const r = await client.bankStatements.lines({ status: a.status, direction: a.direction, from: a.from, to: a.to, limit: a.limit });
      return { data: cut(r.data as unknown[], a.limit) };
    },
  }),
  define({
    name: "get_bank_line",
    title: "Get one bank line with its matches",
    description: "One bank statement line with every match it has had (including rejected ones) and the evidence for each. Use the numeric line id from list_bank_lines.",
    scope: "recon:read",
    kind: "read",
    input: z.object({ line_id: z.number().int().positive().describe("The numeric bank line id.") }),
    examples: [{ line_id: 9 }],
    run: (a, { client }) => client.bankStatements.line(a.line_id),
  }),
  define({
    name: "list_fee_findings",
    title: "List fee check findings",
    description: "Fee findings: settlements where the fee a provider deducted differs from the rate entered from your own contract. Amounts in paise; positive variance means the provider took more.",
    scope: "recon:read",
    kind: "read",
    input: z.object({
      status: z.enum(["open", "reviewed", "all"]).optional().describe("open: not yet resolved (the API default); reviewed: resolved by a person; all."),
      provider: z.string().max(40).optional().describe("A provider name such as razorpay."),
      method: z.string().max(40).optional().describe("A payment method such as upi."),
      limit,
    }),
    examples: [{ status: "open", provider: "razorpay", limit: 10 }],
    run: async (a, { client }) => client.feeChecks.findings({ status: a.status, provider: a.provider, method: a.method, limit: a.limit }),
  }),
  define({
    name: "check_settlement_fees",
    title: "Check one settlement's fees",
    description: "What the fee check says about one settlement against your configured rates. Read only: it records nothing.",
    scope: "recon:read",
    kind: "read",
    input: z.object({ settlement_id: id("The settlement id", "st_abc123") }),
    examples: [{ settlement_id: "st_abc123" }],
    run: (a, { client }) => client.feeChecks.settlement(a.settlement_id),
  }),
  define({
    name: "get_fee_check_summary",
    title: "Fee check summary",
    description: "Totals of over- and under-deducted fees by provider and method, open findings and how many lines have no rule yet.",
    scope: "recon:read",
    kind: "read",
    input: z.object({}),
    examples: [{}],
    run: (_a, { client }) => client.feeChecks.summary(),
  }),
  define({
    name: "get_routing_health",
    title: "Provider health and circuit breakers",
    description: "Per provider and method: recent success rate, circuit-breaker state, whether routing is skipping it now, and recent changes. Payer declines are shown separately and do not count against a provider.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ method: z.enum(["upi", "card", "netbanking", "wallet"]).optional().describe("Only this payment method.") }),
    examples: [{ method: "upi" }],
    run: (a, { client }) => client.routing.health(a.method),
  }),
  define({
    name: "list_cases",
    title: "List reconciliation cases",
    description: "Reconciliation cases (a missing settlement, a mismatch, an unmatched bank credit) with a plain explanation and the amounts involved.",
    scope: "recon:read",
    kind: "read",
    input: z.object({ status: z.enum(["open", "assigned", "resolved"]).optional().describe("Only cases in this state."), kind: z.string().max(40).optional().describe("A case kind, as shown in the list."), limit }),
    examples: [{ status: "open", limit: 10 }],
    run: async (a, { client }) => {
      const r = await client.cases.list({ status: a.status, kind: a.kind });
      return { data: cut(r.data, a.limit), has_more: r.has_more };
    },
  }),
  define({
    name: "get_case_suggestions",
    title: "Likeliest explanation of a case",
    description: "The rule-based explanation of one reconciliation case with its evidence and next steps. Changes nothing; a person resolves the case. The optional AI wording is never requested by this server.",
    scope: "recon:read",
    kind: "read",
    input: z.object({ case_id: id("The case id", "rc_abc123") }),
    examples: [{ case_id: "rc_abc123" }],
    run: (a, { client }) => client.cases.suggestions(a.case_id),
  }),
  define({
    name: "list_payment_links",
    title: "List payment links",
    description: "Payment links with their status, amount, uses and path. Anyone who has a link's path can pay it.",
    scope: "payments:read",
    kind: "read",
    input: z.object({ limit }),
    examples: [{ limit: 10 }],
    run: async (a, { client }) => {
      const r = await client.paymentLinks.list();
      return { data: cut(r.data, a.limit), has_more: r.has_more };
    },
  }),
];

// ---------------------------------------------------------------------------------------------------------------
// Write tools. Registered only when ZYVOR_MCP_ENABLE_WRITES is set; each still passes the API's own checks.
// ---------------------------------------------------------------------------------------------------------------

const writeTools: ToolDef[] = [
  define({
    name: "create_payment_link",
    title: "Create a payment link",
    description:
      "WRITES. Creates a fixed-amount payment link a payer can open and pay. Calling it again creates another link. The amount is capped by the operator (ZYVOR_MCP_MAX_LINK_MINOR). INR only. It collects money into the business's own provider account; it sends nothing to anyone. Share the returned path yourself or ask the user to.",
    scope: "payments:write",
    kind: "write",
    input: z.object({
      amount_minor: z.number().int().min(100).describe("Amount in paise: 49900 is Rs 499.00. At least 100."),
      description: z.string().min(3).max(120).describe("What the payment is for, shown to the payer. Plain text."),
      max_uses: z.number().int().min(1).max(10_000).optional().describe("How many times the link can be paid (default: unlimited)."),
      expires_at: z.string().max(40).optional().describe("When the link stops working, ISO 8601, e.g. 2026-12-31T18:29:59Z."),
      customer_id: id("An existing customer id to attach", "cus_abc123").optional(),
    }),
    examples: [{ amount_minor: 49900, description: "Term 2 tuition fee", max_uses: 1 }],
    run: async (a, ctx) => {
      await ctx.guardWrite();
      if (a.amount_minor > ctx.config.maxLinkMinor) throw new Refusal(`amount_minor ${a.amount_minor} is above the limit this server allows (${ctx.config.maxLinkMinor} paise, set by ZYVOR_MCP_MAX_LINK_MINOR). Nothing was created.`);
      const link = await ctx.client.paymentLinks.create({ amountMinor: a.amount_minor, currency: "INR", description: a.description, maxUses: a.max_uses, expiresAt: a.expires_at, customerId: a.customer_id });
      return { id: link.id, path: link.path, amount_minor: a.amount_minor, note: "The link is live. The path is relative to the business's checkout address." };
    },
  }),
  define({
    name: "request_refund",
    title: "Request a refund (approval policy applies)",
    description:
      "WRITES, DESTRUCTIVE. Asks Vayora to refund part or all of a captured payment, through the normal approval policy. Refunds above the tenant's approval threshold wait as pending_approval until a second person with the approve scope approves them in the console; this server cannot approve. A refund at or below the threshold would start at once without a second person, so this server refuses it unless the operator set ZYVOR_MCP_MAX_UNAPPROVED_REFUND_MINOR at least that high (default 0: refused). `confirm` must repeat intent_id. Repeating the same request within ten minutes returns the same refund.",
    scope: "refunds:write",
    kind: "write",
    destructive: true,
    input: z.object({
      intent_id: id("The payment intent to refund", "pi_abc123"),
      amount_minor: z.number().int().positive().describe("Amount to refund in paise. Required; it must not exceed what is refundable."),
      reason: z.string().min(3).max(200).describe("Why, in plain words. Shown to the approver."),
      confirm: z.string().describe("Repeat intent_id exactly to confirm you mean this payment."),
    }),
    examples: [{ intent_id: "pi_abc123", amount_minor: 25000, reason: "Customer returned the item", confirm: "pi_abc123" }],
    run: async (a, ctx) => {
      confirmEchoes(a.confirm, a.intent_id, "intent_id");
      await ctx.guardWrite();
      // Fail closed: read the threshold the API will apply. If it cannot be read, nothing is requested.
      let threshold: number;
      try {
        threshold = (await ctx.client.settings.get()).refund_approval_threshold_minor;
      } catch {
        throw new Refusal("Could not read the refund approval threshold, so no refund was requested (this server refuses when it cannot tell whether approval will be needed).");
      }
      if (a.amount_minor <= threshold && a.amount_minor > ctx.config.maxUnapprovedRefundMinor) {
        throw new Refusal(
          `A refund of ${a.amount_minor} paise is at or below the approval threshold (${threshold} paise), so Vayora would process it immediately without a second person. This server only requests refunds that wait for approval, unless the operator raises ZYVOR_MCP_MAX_UNAPPROVED_REFUND_MINOR (now ${ctx.config.maxUnapprovedRefundMinor}). Nothing was requested.`,
        );
      }
      const bucket = Math.floor(Date.now() / 600_000);
      const key = `mcp-refund-${createHash("sha256").update(`${a.intent_id}|${a.amount_minor}|${a.reason}|${bucket}`).digest("hex").slice(0, 24)}`;
      const refund = await ctx.client.intents.refund(a.intent_id, { amountMinor: a.amount_minor, reason: a.reason, idempotencyKey: key });
      return {
        refund_id: refund.id,
        status: refund.status,
        amount_minor: refund.amount_minor,
        next: refund.status === "pending_approval" ? "Waiting for a second person with the refunds:approve scope to approve it in the Vayora console." : `Status is ${refund.status}.`,
      };
    },
  }),
  define({
    name: "confirm_bank_match",
    title: "Confirm a suggested bank match",
    description:
      "WRITES, DESTRUCTIVE. Accepts a suggested match between a bank line and a settlement, recording it as confirmed. It moves no money and posts nothing to the ledger, but a confirmed match closes the reconciliation for that pair. Check the evidence with get_bank_line first. `confirm` must repeat match_id.",
    scope: "recon:write",
    kind: "write",
    destructive: true,
    input: z.object({
      match_id: z.number().int().positive().describe("The numeric match id shown on a suggested bank line."),
      note: z.string().max(200).optional().describe("Why it is right, for the audit trail."),
      confirm: z.string().describe("Repeat match_id exactly, as text."),
    }),
    examples: [{ match_id: 12, note: "UTR and amount agree", confirm: "12" }],
    run: async (a, ctx) => {
      confirmEchoes(a.confirm, String(a.match_id), "match_id");
      await ctx.guardWrite();
      return ctx.client.bankStatements.confirmMatch(a.match_id, a.note);
    },
  }),
  define({
    name: "reject_bank_match",
    title: "Reject a suggested bank match",
    description:
      "WRITES, DESTRUCTIVE. Rejects a suggested match between a bank line and a settlement; that pair is not suggested again. Moves no money. `confirm` must repeat match_id.",
    scope: "recon:write",
    kind: "write",
    destructive: true,
    input: z.object({
      match_id: z.number().int().positive().describe("The numeric match id shown on a suggested bank line."),
      note: z.string().max(200).optional().describe("Why it is wrong, for the audit trail."),
      confirm: z.string().describe("Repeat match_id exactly, as text."),
    }),
    examples: [{ match_id: 12, note: "Amount differs by a fee", confirm: "12" }],
    run: async (a, ctx) => {
      confirmEchoes(a.confirm, String(a.match_id), "match_id");
      await ctx.guardWrite();
      return ctx.client.bankStatements.rejectMatch(a.match_id, a.note);
    },
  }),
];

export const READ_TOOLS: readonly ToolDef[] = readTools;
export const WRITE_TOOLS: readonly ToolDef[] = writeTools;
export const ALL_TOOLS: readonly ToolDef[] = [...readTools, ...writeTools];

/** The text a model reads for one tool: what it does, the scope, the mode and a worked example. */
export function fullDescription(t: ToolDef): string {
  const mode = t.kind === "read" ? "Read only: changes nothing in Vayora." : t.destructive ? "Write tool (needs ZYVOR_MCP_ENABLE_WRITES). Hard to undo; needs `confirm`." : "Write tool (needs ZYVOR_MCP_ENABLE_WRITES).";
  return `${t.description}\n\nAPI scope needed: ${t.scope}. ${mode}\nExample arguments: ${JSON.stringify(t.examples[0])}`;
}
