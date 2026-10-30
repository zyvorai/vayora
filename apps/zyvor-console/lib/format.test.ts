import { describe, expect, it } from "vitest";
import { inr, inrCompact, maskKey, parseRupees, pct } from "./format";

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
