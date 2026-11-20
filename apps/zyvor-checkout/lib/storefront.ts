// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import type { Key } from "./i18n";

export type Item = { id: string; name: string; description: string; price_minor: number; image_url: string | null; available: number | null; sold_out: boolean };
export type Shop = { slug: string; title: string; description: string; merchant: string | null; currency: string; logo_url: string | null; noindex: boolean; items: Item[] };

/** The most of one item a visitor may add: the server's limit, or what is left if that is fewer. */
export const MAX_QTY = 20;

export function maxFor(item: Item): number {
  return item.sold_out ? 0 : Math.min(MAX_QTY, item.available ?? MAX_QTY);
}

/** Quantities as the visitor set them, dropped to what the page allows (an item may have sold out meanwhile). */
export function clampCart(qty: Record<string, number>, items: Item[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const it of items) {
    const q = Math.min(Math.max(0, Math.floor(qty[it.id] ?? 0)), maxFor(it));
    if (q > 0) out[it.id] = q;
  }
  return out;
}

export function cartTotal(qty: Record<string, number>, items: Item[]): number {
  return items.reduce((sum, it) => sum + it.price_minor * (qty[it.id] ?? 0), 0);
}

export function cartLines(qty: Record<string, number>): { item_id: string; quantity: number }[] {
  return Object.entries(qty).filter(([, q]) => q > 0).map(([item_id, quantity]) => ({ item_id, quantity }));
}

/** A mobile number as the server accepts it: 10 to 15 digits, spaces, dashes and brackets allowed, one leading plus. */
export function phoneOk(raw: string): boolean {
  const t = raw.replace(/[\s\-()]/g, "");
  return /^\+?\d{10,15}$/.test(t);
}

/** Which plain-language message to show for an error code from the server. Never show server wording to a visitor. */
export function orderErrorKey(code: string | undefined, status: number): Key {
  if (code === "out_of_stock" || code === "item_unavailable") return "storeErrStock";
  if (code === "store_busy") return "storeErrBusy";
  if (code === "rate_limited" || status === 429) return "storeErrRate";
  if (code === "invalid_phone") return "storeErrPhone";
  return "storeErrGeneric";
}
