"use client";
import { useActionState, useState } from "react";
import type { FormState } from "@/lib/actions";

/** A button that asks first: for something that cannot be undone, such as cancelling a subscription. */
export function ConfirmAct({ action, id, label, question, yes }: { action: (s: FormState, f: FormData) => Promise<FormState>; id: string; label: string; question: string; yes: string }) {
  const [state, run, pending] = useActionState(action, undefined);
  const [asking, setAsking] = useState(false);
  if (!asking) return <button type="button" className="btn small" onClick={() => setAsking(true)}>{label}</button>;
  return (
    <form action={run} style={{ display: "inline-flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      <input type="hidden" name="id" value={id} />
      <span>{question}</span>
      <button className="btn small primary" disabled={pending}>{pending ? "…" : yes}</button>
      <button type="button" className="btn small" onClick={() => setAsking(false)} disabled={pending}>Keep it</button>
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
    </form>
  );
}
