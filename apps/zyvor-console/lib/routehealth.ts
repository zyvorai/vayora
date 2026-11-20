// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { istDateTime } from "@/lib/format";
import { methodLabel, providerLabel } from "@/lib/labels";

/** Shapes of `GET /v1/routing/health` (docs/zyvor/ROUTING.md). */
export type HealthConfig = { breaker_enabled: boolean; failover_enabled: boolean; window_attempts: number; window_minutes: number; min_samples: number; degrade_failure_pct: number; recover_failure_pct: number; cooldown_seconds: number };
export type Signal = { samples: number; successes: number; failures: number; unresolved: number; success_rate_bps: number | null; min_samples: number; breaker?: string };
export type HealthRow = { method: string; signal: Signal; in_flight: number; state: "healthy" | "degraded" | "probe_due"; stored_state: "healthy" | "degraded"; reason: string | null; degraded_since: string | null; last_probe_at: string | null; pending_transition: { to: "healthy" | "degraded"; reason: string } | null };
export type HealthProvider = { provider: string; enabled: boolean; methods: HealthRow[] };
export type HealthEvent = { event: "provider.degraded" | "provider.recovered" | string; provider: string | null; detail: { method?: string; reason?: string } | null; at: string };
export type Health = { config: HealthConfig; providers: HealthProvider[]; recent_changes: HealthEvent[] };

/** The success rate, rounded down to a tenth of a percent so 99.97% is never shown as 100%. */
export function ratePercent(bps: number): string {
  const tenths = Math.floor(bps / 10);
  return `${Math.floor(tenths / 10)}${tenths % 10 ? `.${tenths % 10}` : ""}%`;
}

/** The rate, or why there is none. A rate under the minimum sample is never shown: it would mislead. */
export function rateText(s: Signal): { headline: string; detail: string } {
  if (s.success_rate_bps === null || s.success_rate_bps === undefined) {
    return { headline: "Not enough data", detail: `${s.samples} recent attempt${s.samples === 1 ? "" : "s"}; at least ${s.min_samples} are needed before a rate is shown.` };
  }
  return { headline: ratePercent(s.success_rate_bps), detail: `${s.successes} of ${s.samples} recent attempts succeeded${s.failures ? `, ${s.failures} failed` : ""}${s.unresolved ? `, ${s.unresolved} still unresolved` : ""}.` };
}

export type StateView = { label: string; tone: "ok" | "bad" | "warn"; plain: string; differs: string | null };

/** The breaker state in plain words. `state` is what the next payment will see; `stored_state` is what was last written.
 *  They differ only because states are written when a payment is routed, not in the background. */
export function stateView(r: Pick<HealthRow, "state" | "stored_state" | "pending_transition">, breakerOn = true): StateView {
  const base: StateView =
    r.state === "healthy" ? { label: "Healthy", tone: "ok", plain: "New payments can be sent to this provider.", differs: null }
    : r.state === "probe_due" ? { label: "Trying again", tone: "warn", plain: "It was set aside after failures. It has been quiet long enough that the next payment may be sent to it as a test.", differs: null }
    : { label: "Degraded", tone: "bad", plain: "Too many recent payments failed. New payments skip this provider while another one can take them.", differs: null };
  if (!breakerOn) base.plain += " The circuit breaker is switched off, so nothing is skipped.";
  const now = r.state === "healthy" ? "healthy" : "degraded";
  if (now !== r.stored_state) {
    const to = r.pending_transition?.to === "healthy" ? "recovered" : "degraded";
    base.differs = `The saved state still says ${r.stored_state}, but the latest figures say ${now}. The saved state changes when the next payment for this method is routed${r.pending_transition ? ` (it will be marked ${to}: ${r.pending_transition.reason})` : ""}.`;
  } else if (r.state === "probe_due") {
    base.differs = "The saved state is degraded. A test payment is allowed now, but nothing changes until one is routed.";
  }
  return base;
}

/** "saved: degraded" when what is saved differs from what the latest figures say; nothing when they agree. */
export function savedNote(r: Pick<HealthRow, "state" | "stored_state">): string | null {
  const agree = r.state === "probe_due" ? r.stored_state === "degraded" : r.state === r.stored_state;
  return r.state === "probe_due" || !agree ? `saved: ${r.stored_state}` : null;
}

/** The thresholds as one sentence a person can check against what they see. */
export function thresholdSentence(c: HealthConfig): string {
  const mins = c.window_minutes === 1 ? "1 minute" : `${c.window_minutes} minutes`;
  return `A method is marked degraded when at least ${c.degrade_failure_pct}% of its last ${c.window_attempts} attempts (no older than ${mins}) failed or are unresolved, once there are at least ${c.min_samples} attempts to judge by. It is marked healthy again when a test payment succeeds, or the failure rate is at or below ${c.recover_failure_pct}%. A set-aside provider may get one test payment after ${c.cooldown_seconds} seconds.`;
}

export const eventLabel = (e: string) => (e === "provider.degraded" ? "Set aside" : e === "provider.recovered" ? "Back in use" : e);

export function eventText(e: HealthEvent): string {
  const who = `${providerLabel(e.provider ?? "provider")}${e.detail?.method ? ` · ${methodLabel(e.detail.method)}` : ""}`;
  return `${istDateTime(e.at)}: ${who} ${e.event === "provider.degraded" ? "was set aside" : e.event === "provider.recovered" ? "is back in use" : e.event}${e.detail?.reason ? `. ${e.detail.reason}` : ""}`;
}

type FormLike = { get(name: string): FormDataEntryValue | null };
const RANGES: { key: keyof HealthConfig; label: string; min: number; max: number }[] = [
  { key: "window_attempts", label: "Attempts to look at", min: 5, max: 1000 },
  { key: "window_minutes", label: "How far back, in minutes", min: 1, max: 1440 },
  { key: "min_samples", label: "Fewest attempts before judging", min: 3, max: 1000 },
  { key: "degrade_failure_pct", label: "Set aside at this failure percent", min: 1, max: 100 },
  { key: "recover_failure_pct", label: "Back in use at or below this failure percent", min: 0, max: 99 },
  { key: "cooldown_seconds", label: "Seconds before a test payment", min: 10, max: 86400 },
];
export const HEALTH_FIELDS = RANGES;

/** The thresholds form as the body the API takes, with the same limits, so a mistake is explained here before it is sent. */
export function healthSettingsBody(f: FormLike): { ok: true; body: HealthConfig } | { ok: false; error: string } {
  const body: Partial<HealthConfig> = { breaker_enabled: f.get("breaker_enabled") === "on", failover_enabled: f.get("failover_enabled") === "on" };
  for (const r of RANGES) {
    const raw = String(f.get(r.key) ?? "").trim();
    if (!/^\d{1,6}$/.test(raw)) return { ok: false, error: `${r.label}: enter a whole number from ${r.min} to ${r.max}.` };
    const n = Number(raw);
    if (n < r.min || n > r.max) return { ok: false, error: `${r.label}: enter a whole number from ${r.min} to ${r.max}.` };
    (body as Record<string, number>)[r.key] = n;
  }
  if ((body.recover_failure_pct as number) >= (body.degrade_failure_pct as number)) return { ok: false, error: "The percent for coming back must be lower than the percent for setting a provider aside." };
  return { ok: true, body: body as HealthConfig };
}
