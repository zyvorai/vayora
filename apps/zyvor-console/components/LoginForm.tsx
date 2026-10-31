"use client";
import { useActionState, useState } from "react";
import { login } from "@/lib/actions";

export function LoginForm({ next, reset }: { next: string; reset?: boolean }) {
  const [state, action, pending] = useActionState(login, undefined);
  const [show, setShow] = useState(false);
  // Controlled, so the first two answers survive the extra step for the authenticator code.
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const needOtp = state?.otp === true;
  return (
    <form action={action} style={{ display: "grid", gap: 14, marginTop: 22 }}>
      <input type="hidden" name="next" value={next} />
      {reset && <div className="note ok" role="status">Password changed. Sign in with the new one.</div>}
      <div>
        <label className="f" htmlFor="username">Email or username</label>
        <input id="username" name="username" type="text" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoCapitalize="none" spellCheck={false} required autoFocus={!needOtp} readOnly={needOtp} />
      </div>
      <div>
        <label className="f" htmlFor="password">Password</label>
        <div style={{ position: "relative" }}>
          <input id="password" name="password" type={show ? "text" : "password"} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required style={{ paddingRight: 64 }} readOnly={needOtp} />
          <button type="button" className="linkbtn" onClick={() => setShow((v) => !v)} aria-pressed={show} style={{ position: "absolute", right: 12, top: 10 }}>{show ? "Hide" : "Show"}</button>
        </div>
      </div>
      {needOtp && (
        <div>
          <label className="f" htmlFor="otp">Authenticator code</label>
          <input id="otp" name="otp" inputMode="text" autoComplete="one-time-code" autoCapitalize="none" spellCheck={false} placeholder="6 digits, or a recovery code" required autoFocus />
          <p className="fine" style={{ marginTop: 6 }}>Open your authenticator app and enter the current code.</p>
        </div>
      )}
      {state?.error && <div className="note bad" role="alert">{state.error}{state.unverified && <> <a href={`/check-email?email=${encodeURIComponent(username)}`}>Send it again</a></>}</div>}
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Signing in…" : needOtp ? "Verify and sign in" : "Sign in"}</button>
      <p className="fine" style={{ margin: 0, textAlign: "center" }}><a href="/forgot">Forgot your password?</a></p>
      <p className="fine" style={{ margin: 0, textAlign: "center" }}>New to Zyvor? <a href="/signup">Create an account</a></p>
    </form>
  );
}
