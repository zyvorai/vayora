import { describe, expect, it } from "vitest";
import { LANGS, VERIFY_WEBHOOK, createPayment } from "./snippets";

describe("quickstart snippets", () => {
  it("point at the given API address and never contain a key", () => {
    for (const lang of LANGS) {
      const s = createPayment(lang, "https://api.example.com");
      expect(s).toContain("https://api.example.com");
      expect(s).toContain("ZYVOR_API_KEY");
      expect(s).not.toMatch(/zk_(test|live)_/);
      expect(s).toContain("125000");
    }
    expect(VERIFY_WEBHOOK).not.toMatch(/zk_|whsec_/);
  });
});
