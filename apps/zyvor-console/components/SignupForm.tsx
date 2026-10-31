"use client";
import { useActionState } from "react";
import { signup } from "@/lib/actions";

export function SignupForm() {
  const [state, action, pending] = useActionState(signup, undefined);
  return (
    <form action={action} style={{ display: "grid", gap: 14, marginTop: 22 }}>
      <div>
        <label className="f" htmlFor="organisation">Business name</label>
        <input id="organisation" name="organisation" type="text" autoComplete="organization" required autoFocus minLength={2} maxLength={120} />
      </div>
      <div>
        <label className="f" htmlFor="email">Work email</label>
        <input id="email" name="email" type="email" autoComplete="email" autoCapitalize="none" spellCheck={false} required />
      </div>
      <div>
        <label className="f" htmlFor="password">Password</label>
        <input id="password" name="password" type="password" autoComplete="new-password" minLength={12} required />
        <p className="fine" style={{ marginTop: 6 }}>At least 12 characters.</p>
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Creating your account…" : "Create account"}</button>
      <p className="fine" style={{ margin: 0, textAlign: "center" }}>Already have an account? <a href="/login">Sign in</a></p>
    </form>
  );
}
