import { describe, expect, it } from "vitest";
import { parseBulk } from "./bulk";

const vendors = [{ id: "ven_1", name: "Sharma Stationers" }, { id: "ven_2", name: "City Couriers" }];

describe("bulk payout lines", () => {
  it("reads vendor names or ids, rupees to minor units, and skips a header and comments", () => {
    const r = parseBulk("vendor, amount, purpose, reference\n# october\nsharma stationers, 12,500, x, R1\nven_2, 34.5, Deliveries, R2", vendors);
    // a comma inside the amount would shift the columns (and pay ₹12), so it is refused rather than guessed
    expect(r).toEqual({ error: "Line 3: more than four columns. Write amounts without commas (12500, not 12,500)." });
    const ok = parseBulk("vendor,amount,purpose,reference\n# october\nsharma stationers,12500,Supplies,R1\nven_2,₹34.50,Deliveries,R2", vendors);
    expect(ok).toEqual({ items: [{ vendorId: "ven_1", amountMinor: 1_250_000, purpose: "Supplies", externalRef: "R1" }, { vendorId: "ven_2", amountMinor: 3_450, purpose: "Deliveries", externalRef: "R2" }] });
  });
  it("names the line that is wrong and never guesses", () => {
    expect(parseBulk("nobody,100,x,R1", vendors)).toEqual({ error: 'Line 1: no vendor called "nobody".' });
    expect(parseBulk("ven_1,abc,x,R1", vendors)).toEqual({ error: 'Line 1: "abc" is not an amount.' });
    expect(parseBulk("ven_1,0,x,R1", vendors)).toEqual({ error: 'Line 1: "0" is not an amount.' });
    expect(parseBulk("ven_1,5,,R1", vendors)).toEqual({ error: "Line 1: add a purpose." });
    expect(parseBulk("ven_1,5,x", vendors)).toEqual({ error: "Line 1: add a reference so repeating the upload cannot pay twice." });
    expect(parseBulk("  \n# only a comment", vendors)).toEqual({ error: "Paste at least one line." });
  });
  it("caps a run at 200", () => {
    const lines = Array.from({ length: 201 }, (_, i) => `ven_1,1,x,R${i}`).join("\n");
    expect(parseBulk(lines, vendors)).toEqual({ error: "At most 200 payouts at a time." });
  });
});
