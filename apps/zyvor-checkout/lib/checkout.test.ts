import { describe, expect, it } from "vitest";
import { VPA, failureCopy, money, pollDelay, supportLink, supportRef } from "./checkout";

describe("checkout helpers", () => {
  it("formats money from minor units with Indian grouping", () => {
    expect(money(49900, "INR")).toBe("₹499.00");
    expect(money(12345678, "INR")).toBe("₹1,23,456.78");
    expect(money(5, "INR")).toBe("₹0.05");
    expect(money(1050, "USD")).toBe("$10.50");
  });
  it("validates UPI IDs", () => {
    for (const ok of ["asha@okvayora", "ravi.k-1@ybl"]) expect(VPA.test(ok)).toBe(true);
    for (const bad of ["", "a@b", "nospace @upi", "name@", "@upi", "x y@upi"]) expect(VPA.test(bad)).toBe(false);
  });
  it("never leaks provider internals and always reassures about charges", () => {
    expect(failureCopy("insufficient_funds")).toContain("insufficient funds");
    expect(failureCopy("payer_timeout")).toContain("Nothing was charged");
    expect(failureCopy("HTTP 400: some provider stack trace")).not.toContain("HTTP");
    expect(failureCopy(null)).toContain("Nothing was charged");
  });
  it("polls only while a payment is in flight", () => {
    expect(pollDelay("awaiting_payment")).toBe(2000);
    expect(pollDelay("confirming")).toBe(4000);
    expect(pollDelay("paid")).toBeNull();
    expect(pollDelay("choose_method")).toBeNull();
  });
  it("makes a short support reference that grants no access", () => {
    expect(supportRef("pi_01a18f898f4b75fb8738a1ca6c73235c")).toBe("6C73235C");
    expect(supportRef("pi_abc")).toBe("ABC");
  });
  it("only links to https or a plain email address, and adds the reference", () => {
    expect(supportLink("https://help.example.com/pay", "AB12CD34")).toBe("https://help.example.com/pay?ref=AB12CD34");
    expect(supportLink("mailto:help@example.com", "AB12CD34")).toBe("mailto:help@example.com?subject=Payment%20AB12CD34");
    for (const bad of [undefined, "", "javascript:alert(1)", "http://help.example.com", "https://u:p@help.example.com", "mailto:not an email", "data:text/html,x"]) {
      expect(supportLink(bad, "AB12CD34"), String(bad)).toBeNull();
    }
  });
});
