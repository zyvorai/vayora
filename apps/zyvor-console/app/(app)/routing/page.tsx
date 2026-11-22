// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { SettingsForm } from "@/components/SettingsForm";
import { currentSession } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { methodLabel, providerLabel } from "@/lib/labels";
import { saveRoutingAutomation, saveRoutingSettings, setRoutingKillSwitch } from "@/lib/recon-actions";
import { rest } from "@/lib/rest";
import { automationSentence, classText, secondsText, shadowSentence, type AutomationState, type ScoreRow, type ShadowReport } from "@/lib/routeauto";
import { HEALTH_FIELDS, ratePercent, eventLabel, eventText, rateText, savedNote, stateView, thresholdSentence, type Health } from "@/lib/routehealth";

const toneClass = { ok: "chip ok", warn: "chip warn", bad: "chip bad" } as const;

export default async function RoutingHealth() {
  const me = await currentSession();
  const isAdmin = me?.r === "admin";
  const h = await (await rest()).get<Health>("/v1/routing/health");
  const head = (
    <div className="head"><div><h1>Routing health</h1><p className="lede">How often each provider has recently taken payments successfully, and whether Zyvor is still sending new payments to it. This only changes where a <em>new</em> payment goes. A payment that already exists is never moved.</p></div></div>
  );
  if (!h.ok) return <>{head}<p className="note bad" role="alert">{h.message}</p></>;
  const { config, providers, recent_changes } = h.body;
  const r = await rest();
  const [auto, shadow, score] = await Promise.all([r.get<AutomationState>("/v1/routing/automation"), r.get<ShadowReport>("/v1/routing/shadow/report"), r.get<{ scorecard: ScoreRow[] }>("/v1/routing/scorecard")]);
  const rows = providers.flatMap((p) => p.methods.map((m) => ({ p, m })));
  return (
    <>
      {head}
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>Tested against simulated providers only; no real provider has been used. The limits below are starting points, not tuned values. A decline that was the customer\u2019s doing (for example, not enough balance) is shown but not counted against a provider; a decline whose cause is not recognised still is.</p>
      {!config.breaker_enabled && <p className="note warn" role="status">The circuit breaker is switched off, so no provider is being set aside, whatever its recent results.</p>}

      <h2>Providers now</h2>
      <div className="tablewrap" style={{ marginBottom: 12 }}>
        <table>
          <caption className="sr">Recent success rate and state, by provider and payment method</caption>
          <thead><tr><th scope="col">Provider and method</th><th scope="col">Recent success rate</th><th scope="col">State</th><th scope="col">What it means</th></tr></thead>
          <tbody>
            {rows.length === 0 && <tr><td colSpan={4} className="empty">{providers.length === 0 ? "No provider is connected yet." : `No attempts in the last ${config.window_minutes} minutes, and no saved state.`}</td></tr>}
            {rows.map(({ p, m }) => {
              const rate = rateText(m.signal);
              const st = stateView(m, config.breaker_enabled);
              return (
                <tr key={`${p.provider}/${m.method}`}>
                  <th scope="row" style={{ fontWeight: 500, color: "var(--fg)", textTransform: "none", fontSize: "inherit" }}>{providerLabel(p.provider)} · {methodLabel(m.method)}{!p.enabled && <div className="muted">switched off in Providers</div>}</th>
                  <td><strong style={m.signal.success_rate_bps === null ? { fontWeight: 500, color: "var(--muted)" } : undefined}>{rate.headline}</strong><div className="muted" style={{ maxWidth: 300 }}>{rate.detail}{m.in_flight > 0 ? ` ${m.in_flight} still in progress.` : ""}</div></td>
                  <td><span className={toneClass[st.tone]}>{st.label}</span>{savedNote(m) && <div className="muted">{savedNote(m)}</div>}</td>
                  <td style={{ maxWidth: 420 }}>
                    {st.plain}
                    {m.reason && m.stored_state === "degraded" && <div className="muted">Why: {m.reason}</div>}
                    {m.degraded_since && <div className="muted">Set aside since {istDateTime(m.degraded_since)}{m.last_probe_at ? `; last test payment ${istDateTime(m.last_probe_at)}` : ""}.</div>}
                    {st.differs && <div className="note warn" style={{ marginTop: 6 }}>{st.differs}</div>}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <details style={{ marginBottom: 28, maxWidth: "70ch" }}>
        <summary>Saved state and current state</summary>
        <p className="muted">Zyvor saves a provider&apos;s state when a payment is routed and also checks in the background. So there are two things on this page: the <strong>saved</strong> state (what was last written) and the <strong>current</strong> state (what the latest figures say, which is what the next payment will see). Usually they agree. When they do not, the row says so, and the saved state catches up at the next check or payment.</p>
      </details>

      <h2>Recent changes</h2>
      {recent_changes.length === 0 ? <p className="muted">No provider has been set aside or brought back yet.</p> : (
        <ul style={{ listStyle: "none", margin: "0 0 28px", padding: 0, display: "grid", gap: 8 }}>
          {recent_changes.map((e, i) => (
            <li key={i} className="card" style={{ padding: "12px 16px", display: "flex", gap: 12, alignItems: "baseline", flexWrap: "wrap" }}>
              <span className={e.event === "provider.degraded" ? "chip bad" : "chip ok"}>{eventLabel(e.event)}</span>
              <span>{eventText(e)}</span>
            </li>
          ))}
        </ul>
      )}

      <h2 id="automation" style={{ scrollMarginTop: 72 }}>Automation</h2>
      {!auto.ok ? <p className="note bad" role="alert">{auto.message}</p> : (
        <>
          <p className={auto.body.settings.paused ? "note warn" : "muted"} role="status" style={{ marginTop: 0, maxWidth: "70ch" }}>{automationSentence(auto.body)} Every automation is off until you turn it on. Tested against simulated providers only.</p>
          {isAdmin ? (
            <>
              <SettingsForm title="Kill switch" hint={auto.body.settings.paused ? "Routing automation is paused. Releasing it lets each switch below act again." : "Stops shadow mode, retry on another provider, cost-aware routing, failover and the circuit breaker at once. New payments then use your own rules and the lowest configured fee. Recorded in the audit trail."} action={setRoutingKillSwitch} button={auto.body.settings.paused ? "Release the kill switch" : "Pause all routing automation"} done="Done.">
                <input type="hidden" name="action" value={auto.body.settings.paused ? "resume" : "pause"} />
                {!auto.body.settings.paused && <label className="field">Reason (optional)<input name="reason" maxLength={500} /></label>}
              </SettingsForm>
              <SettingsForm title="Routing automation" hint="Applies to new payments straight away. Each change is recorded in the audit trail." action={saveRoutingAutomation}>
                <label className="field checkbox"><input type="checkbox" name="shadow_enabled" defaultChecked={auto.body.settings.shadow_enabled} /> Shadow mode: record what a different policy would choose, change nothing</label>
                <label className="field checkbox"><input type="checkbox" name="soft_retry_enabled" defaultChecked={auto.body.settings.soft_retry_enabled} /> After a definite provider failure, retry once on another provider</label>
                <label className="field checkbox"><input type="checkbox" name="cost_aware_enabled" defaultChecked={auto.body.settings.cost_aware_enabled} /> Cost-aware routing (never picks a provider below the success floor just for price)</label>
                <label className="field">Most attempts per payment<input name="soft_retry_max_attempts" type="number" min={2} max={5} step={1} required defaultValue={auto.body.settings.soft_retry_max_attempts} /><small className="muted" style={{ fontWeight: 400 }}>2 to 5, the first included</small></label>
                <label className="field">Retry window in seconds<input name="soft_retry_window_seconds" type="number" min={30} max={86400} step={1} required defaultValue={auto.body.settings.soft_retry_window_seconds} /><small className="muted" style={{ fontWeight: 400 }}>from the first attempt</small></label>
                <label className="field">Success-rate floor, percent<input name="cost_success_floor_pct" type="number" min={0} max={100} step={1} required defaultValue={auto.body.settings.cost_success_floor_bps / 100} /><small className="muted" style={{ fontWeight: 400 }}>below this, price cannot win</small></label>
                <label className="field">Extra cost allowed for reliability, percent<input name="cost_premium_pct" type="number" min={0} max={100} step={1} required defaultValue={auto.body.settings.cost_premium_bps / 100} /></label>
                <label className="field">Fewest observed fees before they are used<input name="cost_min_fee_samples" type="number" min={1} max={100000} step={1} required defaultValue={auto.body.settings.cost_min_fee_samples} /></label>
              </SettingsForm>
            </>
          ) : <p className="muted">Changing automation needs an admin.</p>}
        </>
      )}

      <h2>Shadow mode: what it would have chosen</h2>
      {!shadow.ok ? <p className="note bad" role="alert">{shadow.message}</p> : (
        <>
          <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>{shadowSentence(shadow.body)}</p>
          {shadow.body.differences.length > 0 && (
            <div className="tablewrap" style={{ marginBottom: 12 }}>
              <table>
                <caption className="sr">Where shadow mode would have chosen a different provider</caption>
                <thead><tr><th scope="col">Method</th><th scope="col">Applied</th><th scope="col">Shadow would choose</th><th scope="col">Payments</th></tr></thead>
                <tbody>{shadow.body.differences.map((d, i) => <tr key={i}><td>{methodLabel(d.method)}</td><td>{providerLabel(d.applied_provider)}</td><td>{d.shadow_provider ? providerLabel(d.shadow_provider) : "no route"}</td><td>{d.decisions}</td></tr>)}</tbody>
              </table>
            </div>
          )}
          <p className="muted" style={{ maxWidth: "70ch" }}>{shadow.body.limits}</p>
        </>
      )}

      <h2>Provider scorecard</h2>
      {!score.ok ? <p className="note bad" role="alert">{score.message}</p> : (
        <div className="tablewrap" style={{ marginBottom: 28 }}>
          <table>
            <caption className="sr">Last 7 days by provider and method; payer declines are not counted against a provider</caption>
            <thead><tr><th scope="col">Provider and method</th><th scope="col">Success rate</th><th scope="col">Failures by class</th><th scope="col">Median time to resolve</th><th scope="col">Unresolved now</th><th scope="col">Observed fee</th></tr></thead>
            <tbody>
              {score.body.scorecard.length === 0 && <tr><td colSpan={6} className="empty">No attempts in the last 7 days.</td></tr>}
              {score.body.scorecard.map((s) => (
                <tr key={`${s.provider}/${s.method}`}>
                  <th scope="row" style={{ fontWeight: 500, color: "var(--fg)", textTransform: "none", fontSize: "inherit" }}>{providerLabel(s.provider)} · {methodLabel(s.method)}</th>
                  <td>{s.success_rate_bps === null ? <span className="muted">Not enough data ({s.samples} of {s.min_samples})</span> : <strong>{ratePercent(s.success_rate_bps)}</strong>}<div className="muted">{s.payer_declines} payer decline{s.payer_declines === 1 ? "" : "s"} not counted</div></td>
                  <td>{classText(s.failure_classes)}</td>
                  <td>{secondsText(s.median_seconds_to_resolve)}</td>
                  <td>{s.unresolved_now}</td>
                  <td>{s.observed_fee?.effective_rate_bps == null ? <span className="muted">none yet</span> : `${(s.observed_fee.effective_rate_bps / 100).toFixed(2)}% on ${s.observed_fee.payments} payments`}{s.last_degraded_at && <div className="muted">Last set aside {istDateTime(s.last_degraded_at)}{s.last_recovered_at ? `; back ${istDateTime(s.last_recovered_at)}` : ""}</div>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <h2 id="limits" style={{ scrollMarginTop: 72 }}>The limits</h2>
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>{thresholdSentence(config)} {config.failover_enabled ? "After a provider definitely fails a payment, the next attempt on that payment goes to another provider when one is available." : "Failover is off: a retry goes to the same provider."} A payment whose result is unknown is never retried until it is resolved.</p>
      {isAdmin ? (
        <SettingsForm title="Change the limits" hint="Only an admin can change these. They apply to new payments straight away and are recorded in the audit trail." action={saveRoutingSettings}>
          <label className="field checkbox"><input type="checkbox" name="breaker_enabled" defaultChecked={config.breaker_enabled} /> Set aside a provider that is failing (circuit breaker)</label>
          <label className="field checkbox"><input type="checkbox" name="failover_enabled" defaultChecked={config.failover_enabled} /> Try another provider after a definite failure</label>
          {HEALTH_FIELDS.map((f) => (
            <label key={f.key} className="field">{f.label}<input name={f.key} type="number" inputMode="numeric" min={f.min} max={f.max} step={1} required defaultValue={config[f.key] as number} /><small className="muted" style={{ fontWeight: 400 }}>{f.min} to {f.max}</small></label>
          ))}
        </SettingsForm>
      ) : <p className="muted">Changing these limits needs an admin.</p>}
    </>
  );
}
