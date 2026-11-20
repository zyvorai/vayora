// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { inr } from "@/lib/format";
import { ratePercent } from "@/lib/routehealth";

/** Shapes of `GET /v1/routing/automation`, `/shadow/report` and `/scorecard` (docs/zyvor/ROUTING.md). */
export type Automation = {
  paused: boolean; paused_reason: string | null; paused_by: string | null; paused_at: string | null;
  shadow_enabled: boolean; soft_retry_enabled: boolean; soft_retry_max_attempts: number; soft_retry_window_seconds: number;
  cost_aware_enabled: boolean; cost_success_floor_bps: number; cost_premium_bps: number; cost_min_fee_samples: number;
};
export type AutomationState = { settings: Automation; effective: { shadow: boolean; soft_retry: boolean; cost_aware: boolean; failover: boolean; circuit_breaker: boolean; note: string } };
export type ShadowReport = {
  window_days: number; decisions: number; agreed: number; differed: number; agreement_rate_bps: number | null;
  shadow: { enabled: boolean; recording_now: boolean };
  estimated_fee: { comparable_decisions: number; difference_minor: number };
  differences: { method: string; applied_provider: string; shadow_provider: string | null; decisions: number; estimated_fee_difference_minor: number }[];
  limits: string;
};
export type ScoreRow = {
  provider: string; method: string; success_rate_bps: number | null; samples: number; min_samples: number; payer_declines: number;
  provider_failures: number; unresolved_now: number; failure_classes: Record<string, number>; median_seconds_to_resolve: number | null;
  observed_fee: { effective_rate_bps: number | null; payments: number } | null; last_degraded_at: string | null; last_recovered_at: string | null;
};

type FormLike = { get(name: string): FormDataEntryValue | null };
const whole = (f: FormLike, k: string, label: string, min: number, max: number): number | string => {
  const raw = String(f.get(k) ?? "").trim();
  if (!/^\d{1,6}$/.test(raw) || Number(raw) < min || Number(raw) > max) return `${label}: enter a whole number from ${min} to ${max}.`;
  return Number(raw);
};

/** The automation form as the body the API takes. Percent fields are whole percents here and basis points there. */
export function automationBody(f: FormLike): { ok: true; body: Record<string, unknown> } | { ok: false; error: string } {
  const body: Record<string, unknown> = { shadow_enabled: f.get("shadow_enabled") === "on", soft_retry_enabled: f.get("soft_retry_enabled") === "on", cost_aware_enabled: f.get("cost_aware_enabled") === "on" };
  for (const [key, label, min, max, mult] of [
    ["soft_retry_max_attempts", "Most attempts per payment", 2, 5, 1],
    ["soft_retry_window_seconds", "Retry window in seconds", 30, 86400, 1],
    ["cost_success_floor_pct", "Success-rate floor, percent", 0, 100, 100],
    ["cost_premium_pct", "Extra cost allowed for reliability, percent", 0, 100, 100],
    ["cost_min_fee_samples", "Fewest observed fees before they are used", 1, 100000, 1],
  ] as const) {
    const v = whole(f, key, label, min, max);
    if (typeof v === "string") return { ok: false, error: v };
    body[key.replace("_pct", "_bps")] = v * mult;
  }
  return { ok: true, body };
}

/** What the switches mean right now, in one sentence. The kill switch is named first because it overrides the rest. */
export function automationSentence(a: AutomationState): string {
  if (a.settings.paused) return `The kill switch is on${a.settings.paused_by ? ` (pulled by ${a.settings.paused_by})` : ""}: nothing below acts. New payments use your own routing rules and the lowest configured fee.`;
  const on = [a.effective.shadow && "shadow mode", a.effective.soft_retry && "retry on another provider", a.effective.cost_aware && "cost-aware routing"].filter(Boolean);
  return on.length ? `Acting now: ${on.join(", ")}.` : "No routing automation is switched on. Routing uses your rules and the lowest configured fee.";
}

/** Shadow versus applied, in words. */
export function shadowSentence(r: ShadowReport): string {
  if (r.decisions === 0) return r.shadow.enabled ? "Shadow mode is on but has not recorded a payment yet." : "Shadow mode is off. Turn it on to see, before changing anything, what a different policy would have chosen.";
  const rate = r.agreement_rate_bps === null ? "" : ` (${ratePercent(r.agreement_rate_bps)})`;
  const fee = r.estimated_fee.comparable_decisions === 0 ? "" : r.estimated_fee.difference_minor === 0 ? " The estimated fees are the same." : r.estimated_fee.difference_minor > 0 ? ` The shadow choice would have cost about ${inr(r.estimated_fee.difference_minor)} less.` : ` The shadow choice would have cost about ${inr(-r.estimated_fee.difference_minor)} more.`;
  return `In the last ${r.window_days} days shadow mode agreed with the routing applied on ${r.agreed} of ${r.decisions} payments${rate}.${fee} Nothing was changed or sent by shadow mode.`;
}

export const secondsText = (s: number | null): string => (s === null ? "no data" : s < 90 ? `${Math.round(s)} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${(s / 3600).toFixed(1)} h`);

export const classText = (c: Record<string, number>): string => {
  const parts = Object.entries(c).sort((a, b) => b[1] - a[1]).map(([k, n]) => `${n} ${k.replace(/_/g, " ")}`);
  return parts.length ? parts.join(", ") : "none";
};
