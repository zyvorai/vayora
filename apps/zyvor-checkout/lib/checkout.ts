export type Phase = "choose_method" | "awaiting_payment" | "confirming" | "trying_another_way" | "paid" | "failed";

/** The business's look for a payment link: only what the server judged safe and readable; checked again here. */
export interface Brand {
  accent: { color: string; on_color: string } | null;
  logo_url: string | null;
}

/** What a payment link adds to the payer's pages (all of it plain text or checked addresses). */
export interface LinkBlock {
  message: string | null;
  thank_you: string | null;
  return_url: string | null;
  brand: Brand | null;
  allowed_methods: string[] | null;
  partial: { total_minor: number; paid_minor: number; balance_minor: number; min_part_minor: number | null } | null;
}

export interface Session {
  merchant: string | null;
  amount_minor: number;
  currency: string;
  description: string | null;
  status: string;
  phase: Phase;
  methods: ("upi" | "card" | "netbanking" | "wallet")[];
  /** How UPI works for this payment: a provider-issued QR / intent, or the payer types a UPI ID (fallback). */
  upi_flow?: "intent" | "collect";
  /** Present when the payment was made through a payment link. */
  link?: LinkBlock | null;
  /** True when the attempt shown is a second try on another provider after the first one failed definitively. */
  retried_on_another_provider?: boolean;
  attempt: { id: string; method: string; status: string; failure_reason: string | null; action: { type: string; [k: string]: unknown } | null } | null;
}

export const VPA = /^[A-Za-z0-9._-]{2,64}@[A-Za-z]{2,32}$/;

export function money(minor: number, currency: string): string {
  const major = Math.floor(Math.abs(minor) / 100);
  const frac = String(Math.abs(minor) % 100).padStart(2, "0");
  const sym = currency === "INR" ? "₹" : currency === "USD" ? "$" : `${currency} `;
  return `${minor < 0 ? "-" : ""}${sym}${major.toLocaleString(currency === "INR" ? "en-IN" : "en-US")}.${frac}`;
}

import type { Key } from "./i18n";
import { MESSAGES } from "./i18n";

/** Which plain-language reason to show. Never show provider internals to a customer. */
export function failureKey(reason: string | null): Key {
  const r = (reason ?? "").toLowerCase();
  if (r.includes("insufficient")) return "failFunds";
  if (r.includes("timeout") || r.includes("payer_timeout") || r.includes("expired")) return "failTime";
  if (r.includes("cancel")) return "failCancel";
  return "failGeneric";
}

export const METHOD_KEY: Record<string, Key> = { upi: "mUpi", card: "mCard", netbanking: "mNet", wallet: "mWallet" };

/** English wording, kept for callers outside the page. */
export function failureCopy(reason: string | null): string {
  return MESSAGES.en[failureKey(reason)];
}

export const METHOD_LABEL: Record<string, string> = { upi: "UPI", card: "Card", netbanking: "Net banking", wallet: "Wallet" };

/** How long to wait between status checks for each phase (ms). */
export function pollDelay(phase: Phase): number | null {
  return phase === "awaiting_payment" || phase === "trying_another_way" ? 2000 : phase === "confirming" ? 4000 : null;
}

/** A short code the payer can quote to support. Derived from the payment's id; it grants no access. */
export function supportRef(intentId: string): string {
  return intentId.replace(/^pi_/, "").slice(-8).toUpperCase();
}

/** Where "report a problem" goes. Only https and mailto are accepted, so a bad setting can never become a script link.
 *  The reference is added so support can find the payment. Nothing secret or personal is put in the link. */
export function supportLink(base: string | undefined, ref: string): string | null {
  const b = (base ?? "").trim();
  if (b.startsWith("mailto:")) {
    const addr = b.slice("mailto:".length).split("?")[0];
    return /^[^\s@?&]+@[^\s@?&]+$/.test(addr) ? `mailto:${addr}?subject=${encodeURIComponent(`Payment ${ref}`)}` : null;
  }
  try {
    const u = new URL(b);
    if (u.protocol !== "https:" || u.username || u.password) return null;
    u.searchParams.set("ref", ref);
    return u.toString();
  } catch {
    return null;
  }
}

const HEX = /^#[0-9a-fA-F]{6}$/;

/** Only an https address is ever used for a logo or a return button, so a bad value can never become a script link. */
export function safeHttps(u: string | null | undefined): string | null {
  if (!u) return null;
  try {
    const p = new URL(u);
    return p.protocol === "https:" && !p.username && !p.password ? p.toString() : null;
  } catch {
    return null;
  }
}

/** Card attributes for a branded link: the colour only when it is a plain hex pair (the server already checked that
 *  it is readable); otherwise the page keeps its own look. */
export function cardProps(brand: Brand | null | undefined): { className: string; style?: Record<string, string> } {
  const a = brand?.accent;
  if (a && HEX.test(a.color) && HEX.test(a.on_color)) return { className: "card branded", style: { "--brand": a.color, "--brand-on": a.on_color } };
  return { className: "card" };
}
