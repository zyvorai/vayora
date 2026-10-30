"use client";
import { useActionState } from "react";
import { createUser } from "@/lib/actions";

export function UserForm() {
  const [state, run, pending] = useActionState(createUser, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "1 1 160px" }}>Username<input name="username" autoComplete="off" autoCapitalize="none" required /></label>
      <label className="field" style={{ flex: "1 1 200px" }}>Password (12+ characters)<input name="password" type="password" autoComplete="new-password" minLength={12} required /></label>
      <label className="field" style={{ flex: "0 1 140px" }}>Role
        <select name="role" defaultValue="viewer"><option value="viewer">Viewer</option><option value="finance">Finance</option><option value="admin">Admin</option></select>
      </label>
      <button className="btn primary" disabled={pending}>{pending ? "Adding…" : "Add person"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Added. Ask them to turn on a second factor under Account.</p>}
    </form>
  );
}
