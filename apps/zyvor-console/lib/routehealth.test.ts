// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { describe, expect, it } from "vitest";
import { eventText, healthSettingsBody, ratePercent, rateText, savedNote, stateView, thresholdSentence, type HealthConfig } from "./routehealth";

const form = (o: Record<string, string>) => ({ get: (k: string) => (k in o ? o[k] : null) }) as unknown as FormData;
const cfg: HealthConfig = { breaker_enabled: true, failover_enabled: true, window_attempts: 100, window_minutes: 30, min_samples: 10, degrade_failure_pct: 50, recover_failure_pct: 20, cooldown_seconds: 120 };

describe("rates", () => {
  it("rounds down, so almost-perfect is never shown as perfect", () => {
    expect(ratePercent(10000)).toBe("100%");
    expect(ratePercent(9997)).toBe("99.9%");
    expect(ratePercent(9234)).toBe("92.3%");
    expect(ratePercent(5000)).toBe("50%");
    expect(ratePercent(0)).toBe("0%");
  });
  it("says there is not enough data instead of a rate below the minimum", () => {
    const r = rateText({ samples: 6, successes: 6, failures: 0, unresolved: 0, success_rate_bps: null, min_samples: 10 });
    expect(r.headline).toBe("Not enough data");
    expect(r.detail).toBe("6 recent attempts; at least 10 are needed before a rate is shown.");
    expect(rateText({ samples: 1, successes: 0, failures: 1, unresolved: 0, success_rate_bps: null, min_samples: 3 }).detail).toMatch(/^1 recent attempt;/);
  });
  it("gives the rate with the counts behind it", () => {
    expect(rateText({ samples: 50, successes: 46, failures: 3, unresolved: 1, success_rate_bps: 9200, min_samples: 10 })).toEqual({ headline: "92%", detail: "46 of 50 recent attempts succeeded, 3 failed, 1 still unresolved." });
  });
});

describe("stateView", () => {
  it("healthy and saved healthy is the plain case", () => {
    expect(stateView({ state: "healthy", stored_state: "healthy", pending_transition: null })).toMatchObject({ label: "Healthy", tone: "ok", differs: null });
  });
  it("degraded in both places", () => {
    expect(stateView({ state: "degraded", stored_state: "degraded", pending_transition: null })).toMatchObject({ label: "Degraded", tone: "bad", differs: null });
  });
  it("saved degraded but the figures have recovered: explains when it catches up", () => {
    const v = stateView({ state: "healthy", stored_state: "degraded", pending_transition: { to: "healthy", reason: "the recent failure rate fell to 10%" } });
    expect(v.label).toBe("Healthy");
    expect(v.differs).toContain("saved state still says degraded");
    expect(v.differs).toContain("next payment");
    expect(v.differs).toContain("recovered");
  });
  it("saved healthy but the figures say degraded", () => {
    const v = stateView({ state: "degraded", stored_state: "healthy", pending_transition: { to: "degraded", reason: "5 of the last 10 failed" } });
    expect(v.differs).toContain("saved state still says healthy");
    expect(v.differs).toContain("5 of the last 10 failed");
  });
  it("a test payment is due: saved state is degraded and that is not a mismatch of figures", () => {
    const v = stateView({ state: "probe_due", stored_state: "degraded", pending_transition: null });
    expect(v.label).toBe("Trying again");
    expect(v.differs).toContain("test payment is allowed");
  });
  it("says nothing is skipped when the breaker is off", () => {
    expect(stateView({ state: "healthy", stored_state: "healthy", pending_transition: null }, false).plain).toContain("switched off");
  });
  it("shows the saved state only when it matters", () => {
    expect(savedNote({ state: "healthy", stored_state: "healthy" })).toBeNull();
    expect(savedNote({ state: "degraded", stored_state: "degraded" })).toBeNull();
    expect(savedNote({ state: "healthy", stored_state: "degraded" })).toBe("saved: degraded");
    expect(savedNote({ state: "probe_due", stored_state: "degraded" })).toBe("saved: degraded");
  });
});

describe("words", () => {
  it("writes the limits as a sentence from the numbers it was given", () => {
    const t = thresholdSentence(cfg);
    for (const n of ["50%", "100 attempts", "30 minutes", "at least 10 attempts", "20%", "120 seconds"]) expect(t).toContain(n);
    expect(thresholdSentence({ ...cfg, window_minutes: 1 })).toContain("1 minute)");
  });
  it("describes a recent change", () => {
    const t = eventText({ event: "provider.degraded", provider: "razorpay", detail: { method: "upi", reason: "5 of the last 10 failed" }, at: "2026-10-01T06:30:00Z" });
    expect(t).toContain("Razorpay · UPI was set aside. 5 of the last 10 failed");
    expect(eventText({ event: "provider.recovered", provider: "cashfree", detail: null, at: "2026-10-01T06:30:00Z" })).toContain("Cashfree is back in use");
  });
});

describe("healthSettingsBody", () => {
  const ok = { breaker_enabled: "on", failover_enabled: "on", window_attempts: "100", window_minutes: "30", min_samples: "10", degrade_failure_pct: "50", recover_failure_pct: "20", cooldown_seconds: "120" };
  it("reads every field and the two switches", () => {
    expect(healthSettingsBody(form(ok))).toEqual({ ok: true, body: cfg });
    const gone = { ...ok } as Record<string, string>;
    delete gone.breaker_enabled;
    const r = healthSettingsBody(form(gone));
    expect(r.ok && r.body.breaker_enabled).toBe(false);
  });
  it("applies the API's own limits and says which field is wrong", () => {
    for (const [k, v] of [["window_attempts", "4"], ["window_attempts", "1001"], ["window_minutes", "0"], ["min_samples", "2"], ["degrade_failure_pct", "101"], ["recover_failure_pct", "100"], ["cooldown_seconds", "9"], ["cooldown_seconds", "86401"], ["min_samples", "1.5"], ["min_samples", ""], ["min_samples", "ten"]]) {
      const r = healthSettingsBody(form({ ...ok, [k]: v }));
      expect(r.ok, `${k}=${v}`).toBe(false);
    }
    const r = healthSettingsBody(form({ ...ok, min_samples: "2" }));
    expect(!r.ok && r.error).toContain("Fewest attempts before judging");
  });
  it("needs the coming-back percent to be below the setting-aside percent", () => {
    expect(healthSettingsBody(form({ ...ok, recover_failure_pct: "50" })).ok).toBe(false);
    expect(healthSettingsBody(form({ ...ok, degrade_failure_pct: "21" })).ok).toBe(true);
  });
});
