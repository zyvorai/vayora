"use client";
import { useActionState } from "react";
import { acceptInvitation, confirmReset, requestReset, resendVerification, type FormState } from "@/lib/actions";

function Messages({ state }: { state: FormState }) {
  return <>{state?.error && <div className="note bad" role="alert">{state.error}</div>}{state?.ok && <div className="note ok" role="status">{state.ok}</div>}</>;
}

const grid = { display: "grid", gap: 14, marginTop: 22 } as const;

export function ForgotForm() {
  const [state, run, pending] = useActionState(requestReset, undefined);
  return (
    <form action={run} style={grid}>
      <div><label className="f" htmlFor="email">Email</label><input id="email" name="email" type="email" autoComplete="email" autoCapitalize="none" required autoFocus /></div>
      <Messages state={state} />
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Sending…" : "Send reset link"}</button>
      <p className="fine" style={{ margin: 0, textAlign: "center" }}><a href="/login">Back to sign in</a></p>
    </form>
  );
}

export function PasswordForm({ token, mode }: { token: string; mode: "reset" | "accept" }) {
  const [state, run, pending] = useActionState(mode === "reset" ? confirmReset : acceptInvitation, undefined);
  return (
    <form action={run} style={grid}>
      <input type="hidden" name="token" value={token} />
      <div><label className="f" htmlFor="password">New password</label><input id="password" name="password" type="password" autoComplete="new-password" minLength={12} required autoFocus /><p className="fine" style={{ marginTop: 6 }}>At least 12 characters.</p></div>
      <div><label className="f" htmlFor="again">Type it again</label><input id="again" name="again" type="password" autoComplete="new-password" minLength={12} required /></div>
      <Messages state={state} />
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Saving…" : mode === "reset" ? "Set new password" : "Join and sign in"}</button>
    </form>
  );
}

export function ResendForm({ email }: { email: string }) {
  const [state, run, pending] = useActionState(resendVerification, undefined);
  return (
    <form action={run} style={grid}>
      <input type="hidden" name="email" value={email} />
      <Messages state={state} />
      <button className="btn" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Sending…" : "Send the link again"}</button>
    </form>
  );
}
