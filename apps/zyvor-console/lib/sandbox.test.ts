import { describe, expect, it } from "vitest";
import { METHODS, OUTCOMES, handleFor, methodFor } from "./sandbox";

describe("sandbox outcomes", () => {
  it("map to the simulator's handle prefixes", () => {
    expect(handleFor("succeed")).toBe("ok@bank");
    expect(handleFor("decline")?.startsWith("fail")).toBe(true);
    expect(handleFor("pending")?.startsWith("pending")).toBe(true);
    expect(handleFor("unknown")?.startsWith("uncertain")).toBe(true);
    expect(handleFor("nonsense")).toBeNull();
    expect(new Set(OUTCOMES.map((o) => o.handle)).size).toBe(OUTCOMES.length);
  });
});

describe("sandbox methods", () => {
  it("accept only the four the simulator supports", () => {
    expect(METHODS.map((m) => m.key)).toEqual(["upi", "card", "netbanking", "wallet"]);
    expect(methodFor("card")).toBe("card");
    expect(methodFor("bitcoin")).toBeNull();
    expect(methodFor("")).toBeNull();
  });
});
