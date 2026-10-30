"use client";
import { useActionState } from "react";
import { createLink } from "@/lib/actions";

/** Make a link and show its URL right away, ready to paste into WhatsApp or a fee notice. */
export function LinkForm({ base }: { base: string }) {
  const [state, run, pending] = useActionState(createLink, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "2 1 220px" }}>Description<input name="description" placeholder="Term 2 fee" maxLength={120} /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Amount (₹)<input name="amount" inputMode="decimal" placeholder="1250.00" required /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Max uses<input name="max_uses" inputMode="numeric" placeholder="Unlimited" /></label>
      <button className="btn primary" disabled={pending}>{pending ? "Creating…" : "Create link"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Created: <a className="mono" style={{ overflowWrap: "anywhere" }} href={base + state.ok} target="_blank" rel="noreferrer">{base + state.ok}</a></p>}
    </form>
  );
}
