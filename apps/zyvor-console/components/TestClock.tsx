"use client";
import { useActionState } from "react";
import { runAsOf } from "@/lib/actions";

/** Test mode: run billing or reminders as of another day. Nothing else in the organisation follows that day. */
export function TestClock() {
  const [state, run, pending] = useActionState(runAsOf, undefined);
  return (
    <form action={run} style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", marginBottom: 16 }}>
      <span className="muted">Test mode: run as of</span>
      <input type="date" name="day" required aria-label="Day to run as of" />
      <button className="btn small" name="what" value="billing" disabled={pending}>{pending ? "…" : "Billing"}</button>
      <button className="btn small" name="what" value="reminders" disabled={pending}>{pending ? "…" : "Reminders"}</button>
      {state?.ok && <span className="muted" role="status">{state.ok}</span>}
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
    </form>
  );
}
