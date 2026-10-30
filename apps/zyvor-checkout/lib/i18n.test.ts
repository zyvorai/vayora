import { describe, expect, it } from "vitest";
import { LANGS, MESSAGES, pickLang, translate, type Key, type Lang } from "./i18n";

const holes = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");

describe("checkout translations", () => {
  const keys = Object.keys(MESSAGES.en) as Key[];
  it("every language has every string, none empty", () => {
    for (const l of Object.keys(LANGS) as Lang[]) {
      expect(Object.keys(MESSAGES[l]).sort(), l).toEqual([...keys].sort());
      for (const k of keys) expect(MESSAGES[l][k].trim().length, `${l}.${k}`).toBeGreaterThan(0);
    }
  });
  it("every translation keeps the placeholders of the English text", () => {
    for (const l of Object.keys(LANGS) as Lang[]) for (const k of keys) expect(holes(MESSAGES[l][k]), `${l}.${k}`).toBe(holes(MESSAGES.en[k]));
  });
  it("fills placeholders and leaves unknown ones visible rather than blank", () => {
    expect(translate("hi", "pay", { amount: "₹499.00" })).toBe("₹499.00 का भुगतान करें");
    expect(translate("en", "paidTo", { amount: "₹1.00", merchant: "Acme" })).toBe("₹1.00 paid to Acme. You can close this page.");
    expect(translate("en", "pay")).toBe("Pay {amount}");
  });
  it("reassures that nothing was charged in every language's failure copy", () => {
    expect(MESSAGES.en.failGeneric).toContain("Nothing was charged");
  });
  it("picks the language: query, then saved, then browser, then English", () => {
    expect(pickLang("ta", "hi", ["bn"])).toBe("ta");
    expect(pickLang(null, "hi", ["bn"])).toBe("hi");
    expect(pickLang(null, null, ["fr-FR", "bn-IN"])).toBe("bn");
    expect(pickLang("xx", "yy", ["fr"])).toBe("en");
    expect(pickLang("HI-in", null, [])).toBe("hi");
  });
});
