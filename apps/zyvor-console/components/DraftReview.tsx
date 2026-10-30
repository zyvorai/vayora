"use client";
import { useActionState } from "react";
import { approveDraft, rejectDraft } from "@/lib/actions";
import { Act } from "@/components/Act";

/** One pending reminder: edit the wording if you like, then approve (sends it on) or reject. */
export function DraftReview({ id, invoice, channel, message, source }: { id: string; invoice: string; channel: string; message: string; source: string }) {
  const [state, run, pending] = useActionState(approveDraft, undefined);
  return (
    <div className="card" style={{ display: "grid", gap: 10 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline", flexWrap: "wrap" }}>
        <strong>{invoice}</strong><span className="muted">via {channel}</span>
        {source === "ai" && <span className="muted">· worded by the AI assistant, please read before approving</span>}
      </div>
      <form action={run} style={{ display: "grid", gap: 10 }}>
        <input type="hidden" name="id" value={id} />
        <textarea name="message" defaultValue={message} rows={4} maxLength={1000} aria-label={`Reminder text for ${invoice}`} />
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button className="btn primary small" disabled={pending}>{pending ? "…" : "Approve and send"}</button>
          <Act action={rejectDraft} id={id} label="Reject" />
          {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        </div>
      </form>
    </div>
  );
}
