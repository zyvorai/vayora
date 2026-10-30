"use client";
import { startTransition, useActionState, type FormEvent } from "react";
import { createKey } from "@/lib/actions";

export function KeyForm({ branches }: { branches: { id: string; name: string }[] }) {
  const [state, run, pending] = useActionState(createKey, undefined);
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "1 1 200px" }}>Name<input name="name" placeholder="Kothrud till" required maxLength={60} /></label>
      <label className="field" style={{ flex: "0 1 220px" }}>Can
        <select name="preset" defaultValue="till"><option value="till">Take payments and look them up</option><option value="read">Only look payments up</option></select>
      </label>
      <label className="field" style={{ flex: "0 1 200px" }}>Limited to
        <select name="branch_id" defaultValue=""><option value="">The whole business</option>{branches.map((b) => <option key={b.id} value={b.id}>{b.name} only</option>)}</select>
      </label>
      <button className="btn primary" disabled={pending}>{pending ? "Creating…" : "Create key"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Copy it now; it is shown once: <span className="mono" style={{ overflowWrap: "anywhere" }}>{state.ok}</span></p>}
    </form>
  );
}
