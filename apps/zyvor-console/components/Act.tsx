"use client";
import { useActionState } from "react";
import type { FormState } from "@/lib/actions";

/** A one-button form bound to a server action; shows the API's refusal inline. */
export function Act({ action, id, label, extra, primary }: { action: (s: FormState, f: FormData) => Promise<FormState>; id?: string; label: string; extra?: Record<string, string>; primary?: boolean }) {
  const [state, run, pending] = useActionState(action, undefined);
  return (
    <form action={run} style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
      {id && <input type="hidden" name="id" value={id} />}
      {extra && Object.entries(extra).map(([k, v]) => <input key={k} type="hidden" name={k} value={v} />)}
      <button className={`btn small${primary ? " primary" : ""}`} disabled={pending}>{pending ? "…" : label}</button>
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
    </form>
  );
}
