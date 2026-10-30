"use client";
import { useActionState, useState } from "react";
import { login } from "@/lib/actions";

export function LoginForm({ next }: { next: string }) {
  const [state, action, pending] = useActionState(login, undefined);
  const [show, setShow] = useState(false);
  return (
    <form action={action} style={{ display: "grid", gap: 14, marginTop: 22 }}>
      <input type="hidden" name="next" value={next} />
      <div>
        <label className="f" htmlFor="username">Username</label>
        <input id="username" name="username" type="text" autoComplete="username" autoCapitalize="none" spellCheck={false} required autoFocus />
      </div>
      <div>
        <label className="f" htmlFor="password">Password</label>
        <div style={{ position: "relative" }}>
          <input id="password" name="password" type={show ? "text" : "password"} autoComplete="current-password" required style={{ paddingRight: 64 }} />
          <button type="button" className="linkbtn" onClick={() => setShow((v) => !v)} aria-pressed={show} style={{ position: "absolute", right: 12, top: 10 }}>{show ? "Hide" : "Show"}</button>
        </div>
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Signing in…" : "Sign in"}</button>
    </form>
  );
}
