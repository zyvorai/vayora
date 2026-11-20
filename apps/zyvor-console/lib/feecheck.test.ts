// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { describe, expect, it } from "vitest";
import { feeLine, feeRuleBody, feeRunBody, feeSettingsBody, groupLabel, ruleSentence, runSentence, varianceText } from "./feecheck";

const form = (o: Record<string, string>) => ({ get: (k: string) => (k in o ? o[k] : null) }) as unknown as FormData;
const rule = { provider: "Razorpay", method: "upi", percent: "1.5", effective_from: "2026-04-01" };

describe("feeRuleBody", () => {
  it("turns percentages and rupees into basis points and paise", () => {
    const r = feeRuleBody(form({ ...rule, flat: "2.50", cap: "100", gst: "18", refund_returns_fee: "on", note: " from the contract " }));
    expect(r).toEqual({ ok: true, body: { provider: "razorpay", method: "upi", bps: 150, flat_minor: 250, cap_minor: 10000, gst_bps: 1800, refund_returns_fee: true, effective_from: "2026-04-01", note: "from the contract" } });
  });
  it("leaves out what was left empty, and means no refund give-back unless ticked", () => {
    const r = feeRuleBody(form(rule));
    expect(r).toEqual({ ok: true, body: { provider: "razorpay", method: "upi", bps: 150, flat_minor: 0, cap_minor: undefined, gst_bps: undefined, refund_returns_fee: false, effective_from: "2026-04-01", note: undefined } });
  });
  it("allows a zero percentage and a zero flat fee, because a method may be free", () => {
    const r = feeRuleBody(form({ ...rule, percent: "0", flat: "0" }));
    expect(r.ok && [r.body.bps, r.body.flat_minor]).toEqual([0, 0]);
  });
  it("never invents a rate: the percentage must be typed", () => {
    expect(feeRuleBody(form({ ...rule, percent: "" }))).toMatchObject({ ok: false });
  });
  it("refuses what the API would refuse, with a sentence", () => {
    const bads: Record<string, string>[] = [{ provider: "" }, { method: "cash" }, { percent: "101" }, { percent: "1.555" }, { percent: "abc" }, { flat: "-2" }, { flat: "1.234" }, { cap: "x" }, { gst: "150" }, { effective_from: "" }, { effective_from: "2026-02-30" }, { effective_from: "01/04/2026" }];
    for (const bad of bads) {
      const r = feeRuleBody(form({ ...rule, ...bad }));
      expect(r.ok, JSON.stringify(bad)).toBe(false);
      expect(!r.ok && r.error.length).toBeGreaterThan(10);
    }
  });
});

describe("feeSettingsBody", () => {
  it("reads the tolerance in rupees as paise and trims the owner", () => {
    expect(feeSettingsBody(form({ tolerance: "0.50", owner: " asha@shop.example " }))).toEqual({ ok: true, body: { tolerance_minor: 50, owner: "asha@shop.example" } });
  });
  it("an empty owner clears it (the API reads an empty string as no owner), an empty tolerance is zero", () => {
    expect(feeSettingsBody(form({ tolerance: "", owner: "" }))).toEqual({ ok: true, body: { tolerance_minor: 0, owner: "" } });
  });
  it("refuses a negative or odd tolerance", () => {
    expect(feeSettingsBody(form({ tolerance: "-1" })).ok).toBe(false);
    expect(feeSettingsBody(form({ tolerance: "1.234" })).ok).toBe(false);
  });
});

describe("feeRunBody", () => {
  it("takes an optional start day", () => {
    expect(feeRunBody(form({ since: "" }))).toEqual({ ok: true, body: { since: undefined } });
    expect(feeRunBody(form({ since: "2026-10-01" }))).toEqual({ ok: true, body: { since: "2026-10-01" } });
    expect(feeRunBody(form({ since: "yesterday" })).ok).toBe(false);
  });
});

describe("words", () => {
  it("describes a rule in one line", () => {
    expect(ruleSentence({ bps: 150, flat_minor: 0, cap_minor: null, gst_bps: null })).toBe("1.5%");
    expect(ruleSentence({ bps: 200, flat_minor: 250, cap_minor: 10000, gst_bps: 1800 })).toBe("2% + ₹2.50 flat, at most ₹100.00 before tax, plus 18% GST on the fee");
    expect(ruleSentence({ bps: 0, flat_minor: 0, cap_minor: 0, gst_bps: 0 })).toBe("0%, at most ₹0.00 before tax, plus 0% GST on the fee");
  });
  it("says which way a difference goes", () => {
    expect(varianceText(730)).toBe("₹7.30 more than your rate");
    expect(varianceText(-5)).toBe("₹0.05 less than your rate");
    expect(varianceText(0)).toBe("Matches your rate");
  });
  it("says a settlement's fee differs only when it is flagged", () => {
    expect(feeLine({ status: "flagged", message: "Fee differs from your configured rate by ₹7.30" })).toBe("Fee differs from your configured rate by ₹7.30");
    expect(feeLine({ status: "ok" })).toBeNull();
    expect(feeLine({ status: "no_rule" })).toBeNull();
    expect(feeLine(null)).toBeNull();
  });
  it("sums up a run, including the case of nothing to check", () => {
    expect(runSentence({ settlements_checked: 0, flagged: 0, ok: 0, no_rule: 0 })).toBe("There were no settlements to check.");
    expect(runSentence({ settlements_checked: 1, flagged: 1, ok: 0, no_rule: 0 })).toBe("Checked 1 settlement: 1 with a difference, 0 matching your rates, 0 with no rule to compare against.");
  });
  it("labels a provider and method", () => {
    expect(groupLabel({ provider: "razorpay", method: "upi" })).toBe("Razorpay · UPI");
  });
});
