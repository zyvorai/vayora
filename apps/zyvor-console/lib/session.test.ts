import { describe, expect, it } from "vitest";
import { createSession, passwordMatches, verifySession } from "./session";

describe("session", () => {
  it("accepts a fresh token and rejects tampering, wrong secret and expiry", async () => {
    const t = await createSession("s3cret", 1_000_000);
    expect(await verifySession("s3cret", t, 1_000_000)).toBe(true);
    expect(await verifySession("other", t, 1_000_000)).toBe(false);
    const [exp, sig] = t.split(".");
    expect(await verifySession("s3cret", `${Number(exp) + 99999}.${sig}`, 1_000_000)).toBe(false);
    expect(await verifySession("s3cret", t, 1_000_000 + 13 * 3600 * 1000)).toBe(false);
    for (const bad of [undefined, "", "x", "1.", ".abc", "abc.def"]) expect(await verifySession("s3cret", bad, 0)).toBe(false);
  });
  it("compares passwords without length shortcuts", async () => {
    expect(await passwordMatches("k", "hunter2", "hunter2")).toBe(true);
    expect(await passwordMatches("k", "hunter2", "hunter")).toBe(false);
  });
});
