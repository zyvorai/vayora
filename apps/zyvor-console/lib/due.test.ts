import { describe, expect, it } from "vitest";
import { dueLabel } from "./due";

const now = Date.parse("2026-11-01T12:00:00Z");
const at = (hours: number) => new Date(now + hours * 3_600_000).toISOString();

describe("dueLabel", () => {
  it("says nothing without a usable date", () => {
    expect(dueLabel("", now)).toEqual({ text: "", late: false });
  });
  it("counts down while there is time", () => {
    expect(dueLabel(at(5), now)).toEqual({ text: "due in 5h", late: false });
    expect(dueLabel(at(0.5), now)).toEqual({ text: "due in under 1h", late: false });
  });
  it("says how late it is once past, in days after two days", () => {
    expect(dueLabel(at(-3), now)).toEqual({ text: "overdue by 3h", late: true });
    expect(dueLabel(at(-72), now)).toEqual({ text: "overdue by 3d", late: true });
  });
});
