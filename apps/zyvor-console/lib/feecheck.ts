// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { bpsPercent, inr, parsePercentBps, parseRupeesOrZero } from "@/lib/format";
import { methodLabel, providerLabel } from "@/lib/labels";

/** What the fee-check endpoints send back (see docs/zyvor/FEE_CHECKS.md). Amounts are whole paise. */
export type FeeRule = { id: string; provider: string; method: string; bps: number; flat_minor: number; cap_minor: number | null; gst_bps: number | null; refund_returns_fee: boolean; effective_from: string; note: string; created_by: string };
export type FeeSettings = { tolerance_minor: number; owner: string | null };
export type FeeGroup = { provider: string; method: string; open_findings: number; settlements: number; over_deducted_minor: number; under_deducted_minor: number; total_variance_minor: number };
export type FeeSummary = { rules_configured: number; open_findings: number; open_cases: number; over_deducted_minor: number; under_deducted_minor: number; total_variance_minor: number; total_absolute_variance_minor: number; by_provider_method: FeeGroup[]; lines_without_a_rule: number; message: string | null; note: string };
export type FeeFinding = { settlement_id: string; attempt_id: string; provider: string; method: string; settled_on: string; payment_minor: number; actual_minor: number; expected_minor: number; variance_minor: number; direction: "over_deducted" | "under_deducted"; tolerance_minor: number; case_id: string | null; case_status: string | null; owner: string | null };
export type FeeRun = { settlements_checked: number; flagged: number; ok: number; no_rule: number };

export const FEE_METHODS = ["upi", "card", "netbanking", "wallet"] as const;
type FormLike = { get(name: string): FormDataEntryValue | null };
const val = (f: FormLike, k: string) => String(f.get(k) ?? "").trim();
type Parsed<T> = { ok: true; body: T } | { ok: false; error: string };

const isDay = (s: string) => /^\d{4}-\d{2}-\d{2}$/.test(s) && !Number.isNaN(Date.parse(`${s}T00:00:00Z`)) && new Date(`${s}T00:00:00Z`).toISOString().startsWith(s);

/** The form for a new fee rule, as the body the API takes. Percentages and rupees as people write them; paise and
 *  basis points out. Nothing is filled in for the person: every rate must come from their contract. */
export function feeRuleBody(f: FormLike): Parsed<{ provider: string; method: string; bps: number; flat_minor: number; cap_minor?: number; gst_bps?: number; refund_returns_fee: boolean; effective_from: string; note?: string }> {
  const provider = val(f, "provider").toLowerCase();
  if (!provider) return { ok: false, error: "Say which provider the rule is for." };
  const method = val(f, "method");
  if (!(FEE_METHODS as readonly string[]).includes(method)) return { ok: false, error: "Choose a payment method." };
  const pct = val(f, "percent");
  if (pct === "") return { ok: false, error: "Enter the percentage from your contract. Enter 0 if this method has no percentage fee." };
  const bps = parsePercentBps(pct);
  if (bps === null) return { ok: false, error: "The percentage must be a number from 0 to 100 with at most two decimals, such as 1.5." };
  const flatRaw = val(f, "flat");
  const flat = flatRaw === "" ? 0 : parseRupeesOrZero(flatRaw);
  if (flat === null) return { ok: false, error: "The flat fee must be an amount in rupees, such as 2 or 2.50." };
  const capRaw = val(f, "cap");
  const cap = capRaw === "" ? undefined : parseRupeesOrZero(capRaw);
  if (cap === null) return { ok: false, error: "The most a fee can be must be an amount in rupees, or empty for no limit." };
  const gstRaw = val(f, "gst");
  const gst = gstRaw === "" ? undefined : parsePercentBps(gstRaw);
  if (gst === null) return { ok: false, error: "GST on the fee must be a percentage from 0 to 100, or empty if your contract has none." };
  const from = val(f, "effective_from");
  if (!isDay(from)) return { ok: false, error: "Choose the first day the rate applies." };
  const note = val(f, "note");
  return { ok: true, body: { provider, method, bps, flat_minor: flat, cap_minor: cap, gst_bps: gst, refund_returns_fee: f.get("refund_returns_fee") === "on", effective_from: from, note: note || undefined } };
}

/** Tolerance in rupees and the owner, as the settings body. Both optional: only what is filled in changes. */
export function feeSettingsBody(f: FormLike): Parsed<{ tolerance_minor: number; owner: string }> {
  const raw = val(f, "tolerance");
  const tol = raw === "" ? 0 : parseRupeesOrZero(raw);
  if (tol === null) return { ok: false, error: "The tolerance must be an amount in rupees, such as 0 or 0.50." };
  return { ok: true, body: { tolerance_minor: tol, owner: val(f, "owner") } };
}

/** "Run the check" form: how far back, as a day, and how many settlements at most. */
export function feeRunBody(f: FormLike): Parsed<{ since?: string; limit?: number }> {
  const since = val(f, "since");
  if (since && !isDay(since)) return { ok: false, error: "Choose a valid start day, or leave it empty." };
  return { ok: true, body: { since: since || undefined } };
}

/** The rule in one line: "1.5% + ₹2.00 flat, at most ₹100.00, plus 18% GST on the fee". */
export function ruleSentence(r: Pick<FeeRule, "bps" | "flat_minor" | "cap_minor" | "gst_bps">): string {
  const parts = [bpsPercent(r.bps)];
  if (r.flat_minor > 0) parts[0] += ` + ${inr(r.flat_minor)} flat`;
  if (r.cap_minor !== null && r.cap_minor !== undefined) parts.push(`at most ${inr(r.cap_minor)} before tax`);
  if (r.gst_bps !== null && r.gst_bps !== undefined) parts.push(`plus ${bpsPercent(r.gst_bps)} GST on the fee`);
  return parts.join(", ");
}

/** Which way a difference goes, in words a person can act on. Positive variance: the provider took more. */
export function varianceText(variance: number): string {
  if (variance === 0) return "Matches your rate";
  return variance > 0 ? `${inr(variance)} more than your rate` : `${inr(-variance)} less than your rate`;
}

export const directionLabel = (d: string) => (d === "over_deducted" ? "Took more than your rate" : "Took less than your rate");

/** The line for a settlement page or a case: nothing when there is nothing to say. */
export function feeLine(check: { status: string; message?: string } | null | undefined): string | null {
  if (!check || check.status !== "flagged") return null;
  return check.message ?? "Fee differs from your configured rate. A person should check it against the contract.";
}

export function runSentence(r: FeeRun): string {
  if (r.settlements_checked === 0) return "There were no settlements to check.";
  const bits = [`${r.flagged} with a difference`, `${r.ok} matching your rates`, `${r.no_rule} with no rule to compare against`];
  return `Checked ${r.settlements_checked} settlement${r.settlements_checked === 1 ? "" : "s"}: ${bits.join(", ")}.`;
}

export const groupLabel = (g: { provider: string; method: string }) => `${providerLabel(g.provider)} · ${methodLabel(g.method)}`;
