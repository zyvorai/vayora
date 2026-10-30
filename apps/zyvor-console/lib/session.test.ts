import { describe, expect, it } from "vitest";
import { allows, tillOnly, createSession, passwordMatches, readSession, verifySession } from "./session";

const who = { i: "usr_1", u: "priya", r: "finance" as const, m: true };

describe("session", () => {
  it("round-trips who is signed in and rejects tampering, wrong secret and expiry", async () => {
    const t = await createSession("s3cret", who, 1_000_000);
    expect(await readSession("s3cret", t, 1_000_000)).toMatchObject(who);
    expect(await verifySession("other", t, 1_000_000)).toBe(false);
    // swapping the payload for an admin one keeps the old signature and must fail
    const [, sig] = t.split(".");
    const forged = (await createSession("s3cret", { ...who, r: "admin" }, 1_000_000)).split(".")[0];
    expect(await readSession("s3cret", `${forged}.${sig}`, 1_000_000)).toBeNull();
    expect(await verifySession("s3cret", t, 1_000_000 + 13 * 3600 * 1000)).toBe(false);
    for (const bad of [undefined, "", "x", "1.", ".abc", "abc.def", "e30.00"]) expect(await verifySession("s3cret", bad, 0)).toBe(false);
  });
  it("orders roles", () => {
    expect(allows("admin", "finance") && allows("finance", "finance") && allows("finance", "viewer")).toBe(true);
    expect(allows("viewer", "finance") || allows("finance", "admin")).toBe(false);
    expect(allows("cashier", "cashier") && allows("viewer", "cashier")).toBe(true);
    expect(allows("cashier", "viewer") || allows("cashier", "finance")).toBe(false);
    expect(tillOnly("/closing") && tillOnly("/account") && !tillOnly("/payments") && !tillOnly("/closingx")).toBe(true);
  });
  it("compares passwords without length shortcuts", async () => {
    expect(await passwordMatches("k", "hunter2", "hunter2")).toBe(true);
    expect(await passwordMatches("k", "hunter2", "hunter")).toBe(false);
  });
});
