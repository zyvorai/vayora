"use client";
import { useActionState, useState } from "react";
import { changeLiveAccess, decideReview, reviewerSignIn } from "@/lib/reviewer-actions";

export function ReviewerSignIn() {
  const [state, action, pending] = useActionState(reviewerSignIn, undefined);
  return (
    <form action={action} style={{ display: "grid", gap: 14, marginTop: 22 }}>
      <div>
        <label className="f" htmlFor="key">Reviewer key</label>
        <input id="key" name="key" type="password" autoComplete="off" autoCapitalize="none" spellCheck={false} placeholder="zp_…" required autoFocus />
        <p className="fine" style={{ marginTop: 6 }}>Your own key from <code>zyvor-api platform-reviewer add</code>. A merchant sign-in does not work here.</p>
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <button className="btn primary" style={{ minHeight: 44 }} disabled={pending}>{pending ? "Checking…" : "Sign in"}</button>
    </form>
  );
}

/** Reject needs a note, because the merchant is shown it; approve may carry one. */
export function DecisionForm({ org }: { org: string }) {
  const [state, action, pending] = useActionState(decideReview, undefined);
  const [note, setNote] = useState("");
  return (
    <form action={action} style={{ display: "grid", gap: 12 }}>
      <input type="hidden" name="org" value={org} />
      <div>
        <label className="f" htmlFor="note">Note to the merchant</label>
        <textarea id="note" name="note" rows={3} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Required to reject: say exactly what would change your mind. Optional to approve." />
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
        <button className="btn primary" name="decision" value="approve" disabled={pending}>{pending ? "…" : "Approve and open live mode"}</button>
        <button className="btn" name="decision" value="reject" disabled={pending}>Reject with note</button>
      </div>
    </form>
  );
}

/** Revoking stops a live business at once, so it asks for the reason and a tick; restoring needs neither. */
export function LiveAccessForm({ org, mode }: { org: string; mode: "revoke" | "restore" }) {
  const [state, action, pending] = useActionState(changeLiveAccess, undefined);
  // controlled, so a refusal does not wipe what was typed
  const [note, setNote] = useState("");
  const [sure, setSure] = useState(false);
  const revoke = mode === "revoke";
  return (
    <form action={action} style={{ display: "grid", gap: 12 }}>
      <input type="hidden" name="org" value={org} />
      <input type="hidden" name="action" value={mode} />
      <div>
        <label className="f" htmlFor="live-note">{revoke ? "Why (the merchant is shown this)" : "Note to the merchant (optional)"}</label>
        <textarea id="live-note" name="note" rows={3} value={note} onChange={(e) => setNote(e.target.value)} placeholder={revoke ? "For example: chargeback rate is above our limit; send the dispute report to ask for a review." : "What changed."} />
      </div>
      {revoke && (
        <label style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
          <input type="checkbox" name="confirm" value="yes" checked={sure} onChange={(e) => setSure(e.target.checked)} style={{ marginTop: 3 }} />
          <span>I understand new payments, payment links and payouts for this business stop immediately. Reading and refunds keep working.</span>
        </label>
      )}
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <div><button className={revoke ? "btn" : "btn primary"} disabled={pending}>{pending ? "…" : revoke ? "Take live mode away" : "Give live mode back"}</button></div>
    </form>
  );
}
