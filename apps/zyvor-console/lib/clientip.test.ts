import { describe, expect, it } from "vitest";
import { visitorIp } from "./clientip";

describe("visitorIp", () => {
  it("takes the address the nearest trusted proxy saw, not what the caller wrote", () => {
    expect(visitorIp("1.2.3.4", 1)).toBe("1.2.3.4");
    expect(visitorIp("9.9.9.9, 1.2.3.4", 1)).toBe("1.2.3.4"); // the caller's own claim comes first and is ignored
    expect(visitorIp("9.9.9.9, 1.2.3.4, 10.0.0.1", 2)).toBe("1.2.3.4");
  });
  it("trusts nothing without proxies, and rejects what is not an address", () => {
    expect(visitorIp("1.2.3.4", 0)).toBeNull();
    expect(visitorIp(null, 1)).toBeNull();
    expect(visitorIp("1.2.3.4", 3)).toBeNull();
    expect(visitorIp("not an ip", 1)).toBeNull();
    expect(visitorIp("2001:db8::1", 1)).toBe("2001:db8::1");
  });
});
