import { describe, expect, it } from "vitest";
import { cell, toCsv } from "./csv";

describe("csv", () => {
  it("quotes commas, quotes and newlines", () => {
    expect(cell('a,"b"\nc')).toBe('"a,""b""\nc"');
  });
  it("defuses formulas but leaves plain negative numbers alone", () => {
    expect(cell("=HYPERLINK(\"x\")")).toBe("\"'=HYPERLINK(\"\"x\"\")\"");
    expect(cell("@SUM(A1)")).toBe("'@SUM(A1)");
    expect(cell("-5000")).toBe("-5000");
    expect(cell("-cmd|calc")).toBe("'-cmd|calc");
  });
  it("renders empties and rows", () => {
    expect(toCsv(["a", "b"], [[1, null], ["x", undefined]])).toBe("a,b\r\n1,\r\nx,\r\n");
  });
});
