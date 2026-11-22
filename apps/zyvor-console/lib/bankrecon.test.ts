// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from "vitest";
import { MAX_STATEMENT_BYTES, ageingRows, caseKindLabel, evidenceFacts, importSentence, lineAmount, problemText, problemsOf, ruleLabel, statementBody, subjectLabel, waitingText } from "./bankrecon";

const form = (o: Record<string, string>) => ({ get: (k: string) => (k in o ? o[k] : null) }) as unknown as FormData;

describe("statementBody", () => {
  it("defaults to auto and sends only what was set", () => {
    expect(statementBody(form({ csv: "a,b\n1,2", filename: "oct.csv" }))).toEqual({ ok: true, body: { csv: "a,b\n1,2", filename: "oct.csv", layout: "auto" } });
  });
  it("sends a custom mapping, date style and separator", () => {
    const r = statementBody(form({ csv: "x", layout: "generic_split", map_date: " Posted ", map_credit: "Money in", map_reference: "#5", map_debit: "", date_format: "%d.%m.%Y", delimiter: ";" }));
    expect(r).toEqual({ ok: true, body: { csv: "x", layout: "generic_split", mapping: { date: "Posted", credit: "Money in", reference: "#5" }, date_format: "%d.%m.%Y", delimiter: ";" } });
  });
  it("refuses an empty file", () => {
    expect(statementBody(form({ csv: "  \n " }))).toMatchObject({ ok: false });
  });
  it("refuses a date style or separator that is not on the list", () => {
    expect(statementBody(form({ csv: "x", date_format: "%m/%d/%Y" })).ok).toBe(false);
    expect(statementBody(form({ csv: "x", delimiter: "x" })).ok).toBe(false);
  });
  it("refuses a file over the size the console sends", () => {
    expect(statementBody(form({ csv: "a".repeat(MAX_STATEMENT_BYTES + 1) })).ok).toBe(false);
  });
});

describe("problems", () => {
  it("names the line and the column", () => {
    expect(problemText({ line: 12, column: "Date", message: "31/31/2026 is not a date" })).toBe("Line 12 (Date): 31/31/2026 is not a date");
    expect(problemText({ line: 4, message: "both debit and credit are filled in" })).toBe("Line 4: both debit and credit are filled in");
    expect(problemText({ line: 0, message: "no header row found" })).toBe("no header row found");
  });
  it("reads the list out of a refused import", () => {
    const body = { error: { code: "invalid_statement", errors: [{ line: 3, column: "Date", message: "bad" }, { message: "whole file" }, "junk"], warnings: [{ line: 9, message: "balance" }] } };
    expect(problemsOf(body)).toEqual([{ line: 3, column: "Date", message: "bad" }, { line: 0, column: null, message: "whole file" }]);
    expect(problemsOf(body, "warnings")).toEqual([{ line: 9, column: null, message: "balance" }]);
    expect(problemsOf(null)).toEqual([]);
    expect(problemsOf({ error: { errors: "no" } })).toEqual([]);
  });
});

describe("match wording", () => {
  it("labels each rule the API can record and passes unknown ones through readably", () => {
    expect(ruleLabel("amount_date_window")).toMatch(/No reference matched/);
    expect(ruleLabel("reference_amount_differs")).toMatch(/amount is different/);
    expect(ruleLabel("something_new")).toBe("something new");
    expect(subjectLabel("settlement", "stl_1")).toBe("Settlement stl_1");
    expect(caseKindLabel("unmatched_bank_credit")).toBe("Money arrived and nothing explains it");
  });
  it("shows money in or out without a minus sign", () => {
    expect(lineAmount({ amount_minor: 123450 })).toEqual({ text: "₹1,234.50", direction: "in" });
    expect(lineAmount({ amount_minor: -5000 })).toEqual({ text: "₹50.00", direction: "out" });
  });
});

describe("evidenceFacts", () => {
  it("lists only what the API recorded, for an amount-and-date suggestion", () => {
    const f = evidenceFacts({
      rule: "amount_date_window", line: { posted_on: "2026-10-03", amount_minor: 98000, narration: "NEFT", reference: null },
      settlement: { net_minor: 98000, settled_on: "2026-10-01", utr: "UTR1" }, amount_equal: true, days_apart: 2, window_days: 3, competing_candidates: 1, note: "A person has to confirm this.",
    });
    expect(f).toEqual([
      { label: "Bank line", value: "₹980.00 on 3 Oct 2026" },
      { label: "Settlement", value: "₹980.00 settled 1 Oct 2026" },
      { label: "Settlement UTR", value: "UTR1" },
      { label: "Amount", value: "Same" },
      { label: "Days apart", value: "2" },
      { label: "Looked within", value: "3 days" },
      { label: "Possible matches found", value: "1" },
      { label: "Note", value: "A person has to confirm this." },
    ]);
  });
  it("shows a reference match with the person who confirmed it", () => {
    const f = evidenceFacts({ key: "UTR998877", amount_equal: false, subject: { amount_minor: 5000, reference: "UTR998877" }, line: { amount_minor: -4000, posted_on: "2026-10-05" }, confirmed_by_person: "asha" });
    expect(f.map((x) => x.label)).toEqual(["Bank line", "We recorded", "Reference matched", "Amount", "Confirmed by"]);
    expect(f.find((x) => x.label === "Amount")?.value).toBe("Different");
  });
  it("copes with nothing or the wrong shape", () => {
    expect(evidenceFacts(null)).toEqual([]);
    expect(evidenceFacts("x")).toEqual([]);
    expect(evidenceFacts({ line: { amount_minor: "lots" }, days_apart: "2" })).toEqual([{ label: "Bank line", value: "" }]);
  });
});

describe("ageing and sentences", () => {
  it("lays the buckets out in the order the API lists them, with zeros where there are none", () => {
    const rows = ageingRows({ bucket_days: ["0-1", "2-3", "4-7", "8-14", "15+"], open: 3, unassigned: 1, oldest_days: 9, by_kind: { unmatched_bank_credit: { buckets: { "0-1": 1, "8-14": 2 }, open: 3, unassigned: 1, oldest_days: 9, amount_minor: 50000 } } });
    expect(rows).toEqual([{ kind: "unmatched_bank_credit", open: 3, unassigned: 1, oldest_days: 9, amount_minor: 50000, buckets: [1, 0, 0, 2, 0] }]);
    expect(ageingRows({ bucket_days: [], open: 0, unassigned: 0, oldest_days: 0, by_kind: {} })).toEqual([]);
  });
  it("says how long a case has waited", () => {
    expect(waitingText(0)).toBe("Opened today");
    expect(waitingText(1)).toBe("Waiting 1 day");
    expect(waitingText(5)).toBe("Waiting 5 days");
  });
  it("sums up an import, calling suggestions what they are", () => {
    expect(importSentence({ import_id: 1, rows: 10, added: 8, duplicates: 2, settlements_rechecked: 1, matched: { confirmed: 3, suggested: 2 }, cases_opened: 1 })).toBe("8 new lines added, 2 already held and skipped, 3 matched by reference, 2 suggested for a person to check, 1 case opened.");
    expect(importSentence({ import_id: 1, rows: 1, added: 1, duplicates: 0, settlements_rechecked: 0, matched: { confirmed: 0, suggested: 0 }, cases_opened: 0 })).toBe("1 new line added, 0 matched by reference, 0 suggested for a person to check.");
  });
});
