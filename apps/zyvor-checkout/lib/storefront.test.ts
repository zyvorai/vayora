// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from "vitest";
import { cartLines, cartTotal, clampCart, maxFor, orderErrorKey, phoneOk, type Item } from "./storefront";

const item = (id: string, price: number, available: number | null, sold_out = false): Item => ({ id, name: id, description: "", price_minor: price, image_url: null, available, sold_out });
const items = [item("a", 30_000, 5), item("b", 18_000, null), item("c", 45_000, 0, true)];

describe("storefront cart", () => {
  it("limits each item to what is left, and to the server's cap", () => {
    expect(maxFor(items[0])).toBe(5);
    expect(maxFor(items[1])).toBe(20);
    expect(maxFor(items[2])).toBe(0);
    expect(clampCart({ a: 9, b: 30, c: 1, zz: 3 }, items)).toEqual({ a: 5, b: 20 });
    expect(clampCart({ a: -2, b: 1.9 }, items)).toEqual({ b: 1 });
  });
  it("adds up in whole paise and lists only chosen lines", () => {
    expect(cartTotal({ a: 2, b: 1 }, items)).toBe(78_000);
    expect(cartLines({ a: 2, b: 0 })).toEqual([{ item_id: "a", quantity: 2 }]);
  });
  it("checks a phone number the way the server does", () => {
    for (const ok of ["9876543210", "+91 98765-43210", "(022) 2345 6789"]) expect(phoneOk(ok), ok).toBe(true);
    for (const bad of ["12345", "98765abcde", "++919876543210", ""]) expect(phoneOk(bad), bad).toBe(false);
  });
  it("turns server codes into plain messages", () => {
    expect(orderErrorKey("out_of_stock", 409)).toBe("storeErrStock");
    expect(orderErrorKey("rate_limited", 429)).toBe("storeErrRate");
    expect(orderErrorKey(undefined, 429)).toBe("storeErrRate");
    expect(orderErrorKey("whatever", 500)).toBe("storeErrGeneric");
  });
});
