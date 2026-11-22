// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from "vitest";
import { bpsPercent, dayYear, inr, inrCompact, maskKey, parsePercentBps, parseRupees, parseRupeesOrZero, pct } from "./format";

describe("inr", () => {
  it("uses Indian grouping and two decimals", () => {
    expect(inr(0)).toBe("₹0.00");
    expect(inr(49900)).toBe("₹499.00");
    expect(inr(12345678)).toBe("₹1,23,456.78");
    expect(inr(5)).toBe("₹0.05");
  });
});

describe("inrCompact", () => {
  it("abbreviates lakh and crore", () => {
    expect(inrCompact(90000)).toBe("₹900");
    expect(inrCompact(4500000)).toBe("₹45K");
    expect(inrCompact(12000000)).toBe("₹1.2L");
    expect(inrCompact(2500000000)).toBe("₹2.5Cr");
  });
});

describe("parseRupees", () => {
  it("parses exact amounts to paise", () => {
    expect(parseRupees("499")).toBe(49900);
    expect(parseRupees("499.5")).toBe(49950);
    expect(parseRupees("1,200.05")).toBe(120005);
    expect(parseRupees(" 0.01 ")).toBe(1);
  });
  it("rejects anything ambiguous", () => {
    for (const bad of ["", "0", "0.00", "-5", "1e3", "12.345", "abc", "₹5", "1.", ".5"]) expect(parseRupees(bad)).toBeNull();
  });
});

describe("misc", () => {
  it("formats pct and masks keys", () => {
    expect(pct(null)).toBe("n/a");
    expect(pct(1)).toBe("100%");
    expect(pct(0.8333)).toBe("83.3%");
    expect(maskKey("sk_test_abcdef123456")).toBe("sk_test_••••3456");
    expect(maskKey("short")).toBe("••••");
  });
});

describe("bpsPercent", () => {
  it("writes basis points as a percentage without floats", () => {
    expect(bpsPercent(0)).toBe("0%");
    expect(bpsPercent(150)).toBe("1.5%");
    expect(bpsPercent(175)).toBe("1.75%");
    expect(bpsPercent(200)).toBe("2%");
    expect(bpsPercent(5)).toBe("0.05%");
    expect(bpsPercent(10000)).toBe("100%");
  });
});

describe("parsePercentBps", () => {
  it("reads a typed percentage as basis points", () => {
    expect(parsePercentBps("1.5")).toBe(150);
    expect(parsePercentBps(" 18 ")).toBe(1800);
    expect(parsePercentBps("0")).toBe(0);
    expect(parsePercentBps("0.05")).toBe(5);
    expect(parsePercentBps("100")).toBe(10000);
  });
  it("refuses anything else", () => {
    for (const bad of ["", "101", "1.555", "-1", "1,5", "abc", "1%", "100.01", ".5"]) expect(parsePercentBps(bad), bad).toBeNull();
  });
});

describe("parseRupeesOrZero", () => {
  it("allows zero as well as amounts", () => {
    expect(parseRupeesOrZero("0")).toBe(0);
    expect(parseRupeesOrZero("0.00")).toBe(0);
    expect(parseRupeesOrZero("2.50")).toBe(250);
    expect(parseRupeesOrZero("1,200")).toBe(120000);
    expect(parseRupeesOrZero("-1")).toBeNull();
    expect(parseRupeesOrZero("1.234")).toBeNull();
    expect(parseRupeesOrZero("")).toBeNull();
  });
});

describe("dayYear", () => {
  it("writes a day with its year, whatever the time zone", () => {
    expect(dayYear("2026-10-01")).toBe("1 Oct 2026");
    expect(dayYear("2026-10-01T23:59:00Z")).toBe("1 Oct 2026");
    expect(dayYear("not a date")).toBe("not a date");
  });
});
