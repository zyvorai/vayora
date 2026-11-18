import { SettingsForm } from "@/components/SettingsForm";
import { currentSession } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { methodLabel, providerLabel } from "@/lib/labels";
import { saveRoutingSettings } from "@/lib/recon-actions";
import { rest } from "@/lib/rest";
import { HEALTH_FIELDS, eventLabel, eventText, rateText, savedNote, stateView, thresholdSentence, type Health } from "@/lib/routehealth";

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
  const rows = providers.flatMap((p) => p.methods.map((m) => ({ p, m })));
  return (
    <>
      {head}
      <p className="muted" style={{ marginTop: 0, maxWidth: "70ch" }}>Tested against simulated providers only; no real provider has been used. The limits below are starting points, not tuned values. A customer declining a payment (for example, not enough balance) counts as a failure here, because Zyvor cannot tell whose fault a decline was.</p>
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
        <p className="muted">Zyvor saves a provider&apos;s state when a payment is routed; there is no background check. So there are two things on this page: the <strong>saved</strong> state (what was last written) and the <strong>current</strong> state (what the latest figures say, which is what the next payment will see). Usually they agree. When they do not, the row says so, and the saved state catches up on the next payment for that method. A quiet method can therefore show an old saved state for a while.</p>
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
