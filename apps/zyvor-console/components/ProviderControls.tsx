"use client";
import { useState, useTransition } from "react";
import { checkProvider, setProviderEnabled } from "@/lib/actions";

/** Check the connection and switch a provider on or off. The check only reads from the provider. */
export function ProviderControls({ provider, enabled }: { provider: string; enabled: boolean }) {
  const [pending, start] = useTransition();
  const [result, setResult] = useState<{ ok: boolean; detail: string } | null>(null);
  return (
    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", justifyContent: "flex-end" }}>
      <button className="btn small" disabled={pending} onClick={() => start(async () => setResult(await checkProvider(provider)))}>{pending ? "Checking…" : "Check connection"}</button>
      <button className="btn small" disabled={pending} onClick={() => start(async () => { const r = await setProviderEnabled(provider, !enabled); if (r?.error) setResult({ ok: false, detail: r.error }); })}>{enabled ? "Turn off" : "Turn on"}</button>
      {result && <span className={`note ${result.ok ? "ok" : "bad"}`} role="status" style={{ flexBasis: "100%", textAlign: "left" }}>{result.ok ? "Connected: " : "Not connected: "}{result.detail}</span>}
    </div>
  );
}
