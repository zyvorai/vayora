"use client";
import { useActionState } from "react";
import { login } from "@/lib/actions";

export function LoginForm({ next }: { next: string }) {
  const [state, action, pending] = useActionState(login, undefined);
  return (
    <form action={action} style={{ display: "grid", gap: 14, marginTop: 18 }}>
      <input type="hidden" name="next" value={next} />
      <div>
        <label className="f" htmlFor="password">Password</label>
        <input id="password" name="password" type="password" autoComplete="current-password" required autoFocus />
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <button className="btn primary" disabled={pending}>{pending ? "Signing in…" : "Sign in"}</button>
    </form>
  );
}
