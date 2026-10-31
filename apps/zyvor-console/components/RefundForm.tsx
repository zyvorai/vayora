"use client";
import { useActionState, useEffect, useState } from "react";
import { refundPayment } from "@/lib/actions";
import { inr } from "@/lib/format";

/** Refund part or all of what is left on a payment. Leaving the amount empty refunds the remainder. */
export function RefundForm({ intentId, refundableMinor }: { intentId: string; refundableMinor: number }) {
  const [state, run, pending] = useActionState(refundPayment, undefined);
  // a fresh key per request: a double click repeats the same one and refunds once
  const [key, setKey] = useState(() => crypto.randomUUID());
  useEffect(() => { if (state?.ok) setKey(crypto.randomUUID()); }, [state]);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginTop: 20 }}>
      <input type="hidden" name="id" value={intentId} /><input type="hidden" name="key" value={key} />
      <div style={{ flexBasis: "100%" }}><strong>Refund</strong> <span className="muted">up to {inr(refundableMinor)} can still be refunded</span></div>
      <label className="field" style={{ flex: "1 1 150px" }}>Amount (₹)<input name="amount" inputMode="decimal" placeholder={(refundableMinor / 100).toFixed(2)} /></label>
      <label className="field" style={{ flex: "2 1 240px" }}>Reason (optional)<input name="reason" maxLength={200} /></label>
      <button className="btn" disabled={pending}>{pending ? "Requesting…" : "Refund"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note ok" role="status" style={{ flexBasis: "100%", margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
