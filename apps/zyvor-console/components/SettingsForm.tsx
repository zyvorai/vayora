"use client";
import { startTransition, useActionState, type FormEvent, type ReactNode } from "react";
import type { FormState } from "@/lib/actions";

/** One settings card: its own form and Save button, with the API's refusal shown inline. */
export function SettingsForm({ title, hint, action, children }: { title: string; hint: string; action: (s: FormState, f: FormData) => Promise<FormState>; children: ReactNode }) {
  const [state, run, pending] = useActionState(action, undefined);
  // Submitted by hand so a refusal keeps what was typed (a form `action` resets its fields afterwards).
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "grid", gap: 14, marginBottom: 16, maxWidth: 720 }}>
      <div><h2 style={{ margin: 0 }}>{title}</h2><p className="muted" style={{ margin: "4px 0 0" }}>{hint}</p></div>
      <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))" }}>{children}</div>
      <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
        <button className="btn primary small" disabled={pending}>{pending ? "Saving…" : "Save"}</button>
        {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        {state?.ok && <span className="note" role="status">Saved.</span>}
      </div>
    </form>
  );
}
