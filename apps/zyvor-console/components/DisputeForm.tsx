"use client";
import { useActionState } from "react";
import { openDispute } from "@/lib/actions";
import { inr } from "@/lib/format";

/** Open a dispute that holds money on a captured payment so it cannot also be refunded. */
export function DisputeForm({ attemptId, intentId, maxMinor }: { attemptId: string; intentId: string; maxMinor: number }) {
  const [state, run, pending] = useActionState(openDispute, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginTop: 20 }}>
      <input type="hidden" name="attempt_id" value={attemptId} />
      <input type="hidden" name="intent_id" value={intentId} />
      <div style={{ flexBasis: "100%" }}><strong>Open a dispute</strong> <span className="muted">holds up to {inr(maxMinor)}; that amount cannot be refunded while the dispute is open</span></div>
      <label className="field" style={{ flex: "1 1 150px" }}>Amount (₹)<input name="amount" inputMode="decimal" placeholder={(maxMinor / 100).toFixed(2)} /></label>
      <label className="field" style={{ flex: "2 1 240px" }}>Reason<input name="reason" required maxLength={200} placeholder="Customer says they were charged twice" /></label>
      <label className="field" style={{ flex: "1 1 150px" }}>Respond by<input name="respond_by" type="date" /></label>
      <button className="btn" disabled={pending}>{pending ? "Opening…" : "Open dispute"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note ok" role="status" style={{ flexBasis: "100%", margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
