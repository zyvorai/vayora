"use client";
import { useActionState } from "react";
import { inviteUser } from "@/lib/actions";

export function InviteForm({ branches }: { branches: { id: string; name: string }[] }) {
  const [state, run, pending] = useActionState(inviteUser, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 16 }}>
      <label className="field" style={{ flex: "2 1 220px" }}>Invite by email<input name="email" type="email" autoComplete="off" autoCapitalize="none" placeholder="name@company.com" required /></label>
      <label className="field" style={{ flex: "0 1 150px" }}>Role
        <select name="role" defaultValue="viewer"><option value="viewer">Viewer</option><option value="finance">Finance</option><option value="admin">Admin</option><option value="cashier">Cashier (till only)</option></select>
      </label>
      <label className="field" style={{ flex: "0 1 170px" }}>Cashier&apos;s branch
        <select name="branch_id" defaultValue=""><option value="">Any (not tied)</option>{branches.map((b) => <option key={b.id} value={b.id}>{b.name}</option>)}</select>
      </label>
      <button className="btn primary" disabled={pending}>{pending ? "Sending…" : "Send invitation"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note ok" role="status" style={{ flexBasis: "100%", margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
