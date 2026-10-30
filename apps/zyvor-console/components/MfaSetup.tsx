"use client";
import { useActionState, useState } from "react";
import { beginMfa, confirmMfa, type FormState } from "@/lib/actions";

export function MfaSetup({ enrolled }: { enrolled: boolean }) {
  const [start, setStart] = useState<FormState>();
  const [state, confirm, pending] = useActionState(confirmMfa, undefined);
  if (state?.codes)
    return (
      <div role="status">
        <p><strong>Second factor is on.</strong> Save these recovery codes somewhere safe. Each works once, and they are not shown again.</p>
        <pre className="mono" style={{ columns: 2, padding: 12, borderRadius: 10, background: "var(--bg2, rgba(127,127,127,.12))" }}>{state.codes.join("\n")}</pre>
        <a className="btn primary" href="/">Continue</a>
      </div>
    );
  // Confirming re-issues the session, so the page re-renders as "enrolled"; keep this component mounted and
  // still showing the codes above (they are never shown again).
  if (enrolled)
    return <p role="status">On. You will be asked for a code from your authenticator app each time you sign in. If you lose the device, an admin can reset it.</p>;
  if (!start?.secret)
    return (
      <>
        <button className="btn primary" onClick={async () => setStart(await beginMfa())}>Set up authenticator</button>
        {start?.error && <p className="note bad" role="alert">{start.error}</p>}
      </>
    );
  return (
    <form action={confirm} style={{ display: "grid", gap: 12 }}>
      <p>In your authenticator app choose <em>enter a setup key</em>, then type this key (account: your username, time-based):</p>
      <code className="mono" style={{ fontSize: 17, letterSpacing: 1, wordBreak: "break-all" }}>{start.secret}</code>
      <p className="fine">Or open <a href={start.uri}>this link</a> on the device that has the app.</p>
      <label className="field">6-digit code from the app<input name="otp" inputMode="numeric" autoComplete="one-time-code" pattern="[0-9]{6}" maxLength={6} required autoFocus /></label>
      <button className="btn primary" disabled={pending}>{pending ? "Checking…" : "Turn on"}</button>
      {state?.error && <p className="note bad" role="alert">{state.error}</p>}
    </form>
  );
}
