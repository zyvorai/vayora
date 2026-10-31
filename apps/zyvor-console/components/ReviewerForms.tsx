"use client";
import { useActionState, useState } from "react";
import { decideReview, reviewerSignIn } from "@/lib/reviewer-actions";

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
