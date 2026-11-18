import { describe, expect, it } from "vitest";
import { automationBody, automationSentence, classText, secondsText, shadowSentence, type AutomationState, type ShadowReport } from "./routeauto";

const form = (o: Record<string, string>) => ({ get: (k: string) => (k in o ? o[k] : null) }) as unknown as FormData;
const ok = { soft_retry_max_attempts: "2", soft_retry_window_seconds: "600", cost_success_floor_pct: "80", cost_premium_pct: "5", cost_min_fee_samples: "30" };
const settings = { paused: false, paused_reason: null, paused_by: null, paused_at: null, shadow_enabled: false, soft_retry_enabled: false, soft_retry_max_attempts: 2, soft_retry_window_seconds: 600, cost_aware_enabled: false, cost_success_floor_bps: 8000, cost_premium_bps: 500, cost_min_fee_samples: 30 };
const state = (over: Partial<AutomationState["settings"]>, eff: Partial<AutomationState["effective"]> = {}): AutomationState => ({ settings: { ...settings, ...over }, effective: { shadow: false, soft_retry: false, cost_aware: false, failover: true, circuit_breaker: true, note: "", ...eff } });

describe("the automation form", () => {
  it("turns whole percents into the basis points the API takes, and unticked boxes into false", () => {
    expect(automationBody(form({ ...ok, soft_retry_enabled: "on" }))).toEqual({ ok: true, body: { shadow_enabled: false, soft_retry_enabled: true, cost_aware_enabled: false, soft_retry_max_attempts: 2, soft_retry_window_seconds: 600, cost_success_floor_bps: 8000, cost_premium_bps: 500, cost_min_fee_samples: 30 } });
  });
  it("explains a mistake with the same limits as the API, before anything is sent", () => {
    for (const [k, v] of [["soft_retry_max_attempts", "1"], ["soft_retry_max_attempts", "6"], ["soft_retry_window_seconds", "5"], ["cost_success_floor_pct", "101"], ["cost_premium_pct", "-1"], ["cost_min_fee_samples", "0"], ["cost_min_fee_samples", "abc"]]) {
      const r = automationBody(form({ ...ok, [k]: v }));
      expect(r.ok, `${k}=${v}`).toBe(false);
    }
    expect(automationBody(form({ ...ok, soft_retry_max_attempts: "9" }))).toEqual({ ok: false, error: "Most attempts per payment: enter a whole number from 2 to 5." });
  });
});

describe("what is acting", () => {
  it("names the kill switch first, because it overrides everything", () => {
    expect(automationSentence(state({ paused: true, paused_by: "root", shadow_enabled: true }, { shadow: false }))).toBe("The kill switch is on (pulled by root): nothing below acts. New payments use your own routing rules and the lowest configured fee.");
  });
  it("lists only what really acts", () => {
    expect(automationSentence(state({}))).toMatch(/^No routing automation is switched on/);
    expect(automationSentence(state({ shadow_enabled: true, cost_aware_enabled: true }, { shadow: true, cost_aware: true }))).toBe("Acting now: shadow mode, cost-aware routing.");
  });
});

const rep = (over: Partial<ShadowReport>): ShadowReport => ({ window_days: 7, decisions: 0, agreed: 0, differed: 0, agreement_rate_bps: null, shadow: { enabled: true, recording_now: true }, estimated_fee: { comparable_decisions: 0, difference_minor: 0 }, differences: [], limits: "", ...over });
describe("shadow mode in words", () => {
  it("says so when nothing has been recorded, on or off", () => {
    expect(shadowSentence(rep({}))).toMatch(/has not recorded a payment yet/);
    expect(shadowSentence(rep({ shadow: { enabled: false, recording_now: false } }))).toMatch(/is off/);
  });
  it("gives the agreement and the estimated fee in whichever direction it points", () => {
    const base = { decisions: 6, agreed: 2, differed: 4, agreement_rate_bps: 3333 };
    expect(shadowSentence(rep({ ...base, estimated_fee: { comparable_decisions: 6, difference_minor: -2000 } }))).toBe("In the last 7 days shadow mode agreed with the routing applied on 2 of 6 payments (33.3%). The shadow choice would have cost about ₹20.00 more. Nothing was changed or sent by shadow mode.");
    expect(shadowSentence(rep({ ...base, estimated_fee: { comparable_decisions: 6, difference_minor: 150 } }))).toContain("would have cost about ₹1.50 less");
    expect(shadowSentence(rep({ ...base }))).not.toMatch(/estimated fees|would have cost/);
  });
});

describe("scorecard text", () => {
  it("shows a time a person can read and a failure mix, or why there is none", () => {
    expect(secondsText(null)).toBe("no data");
    expect(secondsText(42)).toBe("42 s");
    expect(secondsText(600)).toBe("10 min");
    expect(secondsText(7200)).toBe("2.0 h");
    expect(classText({ provider_fault: 2, payer_declined: 3 })).toBe("3 payer declined, 2 provider fault");
    expect(classText({})).toBe("none");
  });
});
