export type Phase = "choose_method" | "awaiting_payment" | "confirming" | "paid" | "failed";

export interface Session {
  merchant: string | null;
  amount_minor: number;
  currency: string;
  description: string | null;
  status: string;
  phase: Phase;
  methods: ("upi" | "card" | "netbanking" | "wallet")[];
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
  return phase === "awaiting_payment" ? 2000 : phase === "confirming" ? 4000 : null;
}
