// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

export type Sale = "upcoming" | "open" | "ended";

export interface PublicTier {
  id: string;
  name: string;
  description: string;
  price_minor: number;
  regular_price_minor: number;
  price_kind: "regular" | "early_bird";
  early_bird: { price_minor: number; starts_at: string | null; ends_at: string } | null;
  sale: { state: Sale; starts_at: string | null; ends_at: string | null };
  available: number;
  sold_out: boolean;
  per_order_max: number;
  allowed_methods: string[] | null;
  can_buy: boolean;
}

export interface FormField {
  key: string;
  label: string;
  kind: "text" | "email" | "phone" | "select" | "checkbox";
  required: boolean;
  options: string[] | null;
  min_len: number | null;
  max_len: number | null;
}

export interface PublicEvent {
  id: string;
  merchant: string | null;
  name: string;
  description: string;
  mode: "venue" | "online";
  venue: string | null;
  starts_at: string;
  ends_at: string | null;
  status: "on_sale" | "closed" | "cancelled";
  currency: string;
  on_sale: boolean;
  hold_minutes: number;
  tiers: PublicTier[];
  form_fields: FormField[];
}

export interface OrderTicket {
  id: string;
  code: string;
  status: "issued" | "checked_in" | "refunded" | "void";
  tier: string;
  checked_in_at: string | null;
  qr_payload: string | null;
  qr_path: string;
}

export interface OrderView {
  order_id: string;
  status: "pending" | "paid" | "expired" | "cancelled" | "refunded" | "needs_refund";
  merchant: string | null;
  event: { id: string; name: string; mode: "venue" | "online"; venue: string | null; starts_at: string; ends_at: string | null; status: string; online_url: string | null };
  buyer: { name: string; email: string };
  quantity: number;
  total_minor: number;
  currency: string;
  intent_id: string | null;
  hold_expires_at: string | null;
  tickets: OrderTicket[];
}

/** Total in minor units for the chosen quantities, at the prices on screen. */
export function subtotal(tiers: PublicTier[], qty: Record<string, number>): number {
  return tiers.reduce((sum, t) => sum + t.price_minor * (qty[t.id] ?? 0), 0);
}

export function seatCount(qty: Record<string, number>): number {
  return Object.values(qty).reduce((a, b) => a + b, 0);
}

/** The most of a tier a buyer can pick: what is left, and the per-order limit. */
export function maxFor(t: PublicTier): number {
  return t.can_buy ? Math.max(0, Math.min(t.available, t.per_order_max)) : 0;
}

/** Which note goes under a tier: why it cannot be bought, or how few are left. */
export function tierNote(t: PublicTier): "soldOut" | "salesStart" | "salesEnded" | "fewLeft" | null {
  if (t.sale.state === "upcoming") return "salesStart";
  if (t.sale.state === "ended") return "salesEnded";
  if (t.sold_out) return "soldOut";
  return t.available <= 10 ? "fewLeft" : null;
}

/** Check the registration answers the same way the server does, so the buyer sees the problem before ordering. Returns field key -> problem. */
export function formProblems(fields: FormField[], answers: Record<string, string | boolean>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of fields) {
    const v = answers[f.key];
    if (f.kind === "checkbox") {
      if (f.required && v !== true) out[f.key] = "required";
      continue;
    }
    const text = typeof v === "string" ? v.trim() : "";
    if (!text) {
      if (f.required) out[f.key] = "required";
      continue;
    }
    const n = [...text].length;
    if (n < (f.min_len ?? 0) || n > (f.max_len ?? 500)) out[f.key] = "length";
    else if (f.kind === "email" && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(text)) out[f.key] = "invalid";
    else if (f.kind === "phone" && !(/^[\d+\s()-]+$/.test(text) && text.replace(/\D/g, "").length >= 7 && text.replace(/\D/g, "").length <= 15)) out[f.key] = "invalid";
    else if (f.kind === "select" && !(f.options ?? []).includes(text)) out[f.key] = "invalid";
  }
  return out;
}

/** The answers to send: checkboxes as booleans, empty optional text left out. */
export function answersBody(fields: FormField[], answers: Record<string, string | boolean>): Record<string, string | boolean> {
  const out: Record<string, string | boolean> = {};
  for (const f of fields) {
    const v = answers[f.key];
    if (f.kind === "checkbox") out[f.key] = v === true;
    else if (typeof v === "string" && v.trim()) out[f.key] = v.trim();
  }
  return out;
}

/** A date and time in India, in the buyer's language. */
export function when(iso: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short", timeZone: "Asia/Kolkata" }).format(new Date(iso)) + " IST";
}

export const LOCALE: Record<string, string> = { en: "en-IN", hi: "hi-IN", mr: "mr-IN", ta: "ta-IN", te: "te-IN", bn: "bn-IN" };

/** The error code the server sent, as the message key to show. */
export function orderErrorKey(code: string | undefined): "errSoldOut" | "errNotOnSale" | "errCoupon" | "errPending" | "errGeneric" {
  switch (code) {
    case "sold_out":
    case "too_many_tickets":
      return "errSoldOut";
    case "event_not_on_sale":
    case "tier_not_on_sale":
      return "errNotOnSale";
    case "coupon_invalid":
      return "errCoupon";
    case "too_many_pending_orders":
      return "errPending";
    default:
      return "errGeneric";
  }
}
