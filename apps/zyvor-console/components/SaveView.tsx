"use client";
import { useActionState } from "react";
import type { FormState } from "@/lib/actions";

/** Name the filters currently applied and keep them. The filters ride along as hidden fields. */
export function SaveView({ action, filters }: { action: (s: FormState, f: FormData) => Promise<FormState>; filters: Record<string, string> }) {
  const [state, run, pending] = useActionState(action, undefined);
  return (
    <form action={run} style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      {Object.entries(filters).map(([k, v]) => <input key={k} type="hidden" name={k} value={v} />)}
      <input name="name" placeholder="Name this view" aria-label="View name" maxLength={60} style={{ maxWidth: 220 }} />
      <label className="muted" style={{ display: "inline-flex", gap: 6, alignItems: "center" }}><input type="checkbox" name="shared" /> Share with everyone here</label>
      <button className="btn small" disabled={pending}>{pending ? "Saving…" : "Save view"}</button>
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
      {state?.ok && <span className="muted" role="status">Saved.</span>}
    </form>
  );
}
