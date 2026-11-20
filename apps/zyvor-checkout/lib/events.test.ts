// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, expect, it } from "vitest";
import { EVENT_MESSAGES, evt, type EvtKey } from "./events-i18n";
import { answersBody, formProblems, maxFor, orderErrorKey, seatCount, subtotal, tierNote, type FormField, type PublicTier } from "./events";

const tier = (over: Partial<PublicTier> = {}): PublicTier => ({
  id: "t1", name: "General", description: "", price_minor: 50000, regular_price_minor: 50000, price_kind: "regular", early_bird: null,
  sale: { state: "open", starts_at: null, ends_at: null }, available: 100, sold_out: false, per_order_max: 10, allowed_methods: null, can_buy: true, ...over,
});

describe("event page helpers", () => {
  it("adds up the order at the prices shown", () => {
    const tiers = [tier(), tier({ id: "t2", price_minor: 30000 })];
    expect(subtotal(tiers, { t1: 2, t2: 1 })).toBe(130000);
    expect(seatCount({ t1: 2, t2: 1 })).toBe(3);
    expect(subtotal(tiers, {})).toBe(0);
  });
  it("limits a tier to what is left and the per-order maximum, and to nothing when it cannot be bought", () => {
    expect(maxFor(tier())).toBe(10);
    expect(maxFor(tier({ available: 3 }))).toBe(3);
    expect(maxFor(tier({ can_buy: false }))).toBe(0);
    expect(maxFor(tier({ available: 0, sold_out: true, can_buy: false }))).toBe(0);
  });
  it("explains why a tier cannot be bought, or how few are left", () => {
    expect(tierNote(tier())).toBeNull();
    expect(tierNote(tier({ available: 4 }))).toBe("fewLeft");
    expect(tierNote(tier({ sold_out: true, available: 0 }))).toBe("soldOut");
    expect(tierNote(tier({ sale: { state: "upcoming", starts_at: "2030-01-01T00:00:00Z", ends_at: null } }))).toBe("salesStart");
    expect(tierNote(tier({ sale: { state: "ended", starts_at: null, ends_at: "2020-01-01T00:00:00Z" } }))).toBe("salesEnded");
  });
  const fields: FormField[] = [
    { key: "company", label: "Company", kind: "text", required: true, options: null, min_len: null, max_len: null },
    { key: "mail", label: "Email", kind: "email", required: false, options: null, min_len: null, max_len: null },
    { key: "shirt", label: "Shirt", kind: "select", required: false, options: ["S", "M"], min_len: null, max_len: null },
    { key: "terms", label: "Terms", kind: "checkbox", required: true, options: null, min_len: null, max_len: null },
  ];
  it("checks the registration form like the server does", () => {
    expect(formProblems(fields, {})).toEqual({ company: "required", terms: "required" });
    expect(formProblems(fields, { company: "Acme", terms: true })).toEqual({});
    expect(formProblems(fields, { company: "Acme", terms: true, mail: "nope", shirt: "XL" })).toEqual({ mail: "invalid", shirt: "invalid" });
    expect(formProblems(fields, { company: "  ", terms: true })).toEqual({ company: "required" });
  });
  it("sends only what was answered, with checkboxes as booleans", () => {
    expect(answersBody(fields, { company: " Acme ", terms: true, shirt: "" })).toEqual({ company: "Acme", terms: true });
    expect(answersBody(fields, { company: "Acme" })).toEqual({ company: "Acme", terms: false });
  });
  it("turns server error codes into messages", () => {
    expect(orderErrorKey("sold_out")).toBe("errSoldOut");
    expect(orderErrorKey("coupon_invalid")).toBe("errCoupon");
    expect(orderErrorKey("tier_not_on_sale")).toBe("errNotOnSale");
    expect(orderErrorKey("whatever")).toBe("errGeneric");
    expect(orderErrorKey(undefined)).toBe("errGeneric");
  });
});

describe("event strings", () => {
  const holes = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
  it("only translates keys that exist in English and keeps the placeholders", () => {
    const en = EVENT_MESSAGES.en!;
    for (const [lang, table] of Object.entries(EVENT_MESSAGES)) {
      for (const [k, v] of Object.entries(table!)) {
        expect(en[k as EvtKey], `${lang}.${k} has no English source`).toBeDefined();
        expect(v!.trim().length, `${lang}.${k}`).toBeGreaterThan(0);
        expect(holes(v!), `${lang}.${k}`).toBe(holes(en[k as EvtKey]!));
      }
    }
  });
  it("falls back to English for a string a language does not have", () => {
    expect(evt("hi", "total")).toBe("कुल");
    expect(evt("ta", "total")).toBe("Total");
    expect(evt("hi", "orderPendingBody", { when: "5 pm" })).toContain("5 pm");
    expect(evt("en", "ticketOf", { n: "1", total: "2" })).toBe("Ticket 1 of 2");
  });
});
