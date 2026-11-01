"use client";
import { useActionState, useState } from "react";
import { addDayOff, assignReview, changeLiveAccess, decideChange, decideReview, escalateChange, escalateReview, rebalance, removeDayOff, reviewerSignIn, saveMyEmail, saveRota } from "@/lib/reviewer-actions";
import { DAYS, type RotaShift } from "@/lib/rota";

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
export function DecisionForm({ org, approveLabel = "Approve and open live mode", iApprovedFirst = false }: { org: string; approveLabel?: string; iApprovedFirst?: boolean }) {
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
        {iApprovedFirst ? <span className="fine">You gave the first approval; a different reviewer gives the second. You can still reject.</span> : <button className="btn primary" name="decision" value="approve" disabled={pending}>{pending ? "…" : approveLabel}</button>}
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

/** One waiting change: approve, or reject with the reason the merchant is shown. */
export function ChangeForm({ id, kind, firstApprovalOnly, iApprovedFirst = false }: { id: string; kind: "bank_account" | "profile"; firstApprovalOnly: boolean; iApprovedFirst?: boolean }) {
  const [state, action, pending] = useActionState(decideChange, undefined);
  const [note, setNote] = useState("");
  return (
    <form action={action} style={{ display: "grid", gap: 8 }}>
      <input type="hidden" name="id" value={id} />
      <input name="note" value={note} onChange={(e) => setNote(e.target.value)} placeholder="Reason (required to reject)" aria-label="Reason" type="text" />
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      <div style={{ display: "flex", gap: 8 }}>
        {iApprovedFirst ? <span className="fine">You gave the first approval; a different reviewer completes it.</span> : <button className="btn small primary" name="decision" value="approve" disabled={pending}>{firstApprovalOnly ? "Give the first approval" : "Approve"}</button>}
        <button className="btn small" name="decision" value="reject" disabled={pending}>{kind === "profile" ? "Reject" : "Reject and remove"}</button>
      </div>
    </form>
  );
}

/** For a review no rule flagged that still deserves a second pair of eyes. */
export function EscalateForm({ org }: { org: string }) {
  const [state, action, pending] = useActionState(escalateReview, undefined);
  const [note, setNote] = useState("");
  return (
    <form action={action} style={{ display: "grid", gap: 10 }}>
      <input type="hidden" name="org" value={org} />
      <textarea name="note" rows={2} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Why a second reviewer should look (they will read this)" aria-label="Why a second reviewer should look" />
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <div><button className="btn small" disabled={pending}>{pending ? "…" : "Ask for a second reviewer"}</button></div>
    </form>
  );
}

/** Who has this review. Holding it keeps two people from reading the same file; a held review can be taken over on purpose. */
export function AssignForm({ org, change, holder, mine }: { org?: string; change?: string; holder: string | null; mine: boolean }) {
  const [state, action, pending] = useActionState(assignReview, undefined);
  return (
    <form action={action} style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
      {org && <input type="hidden" name="org" value={org} />}
      {change && <input type="hidden" name="change" value={change} />}
      <span className="fine">{mine ? `You have this ${change ? "change" : "review"}.` : holder ? `${holder} has this ${change ? "change" : "review"}.` : `Nobody has this ${change ? "change" : "review"}.`}</span>
      {!holder && <button className="btn small" name="how" value="claim" disabled={pending}>{pending ? "…" : `Take this ${change ? "change" : "review"}`}</button>}
      {mine && <button className="btn small" name="how" value="release" disabled={pending}>{pending ? "…" : "Hand it back"}</button>}
      {holder && !mine && <button className="btn small" name="how" value="take_over" disabled={pending}>{pending ? "…" : `Take it from ${holder}`}</button>}
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
      {state?.ok && <span className="note ok" role="status">{state.ok}</span>}
    </form>
  );
}

/** For a change no rule flagged that still deserves a second pair of eyes. */
export function EscalateChangeForm({ id }: { id: string }) {
  const [state, action, pending] = useActionState(escalateChange, undefined);
  const [note, setNote] = useState("");
  return (
    <form action={action} style={{ display: "grid", gap: 8 }}>
      <input type="hidden" name="id" value={id} />
      <input name="note" value={note} onChange={(e) => setNote(e.target.value)} placeholder="Why a second reviewer should look" aria-label="Why a second reviewer should look" type="text" />
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <div><button className="btn small" disabled={pending}>{pending ? "…" : "Ask for a second reviewer"}</button></div>
    </form>
  );
}

/** A reviewer's weekly hours: one shift per day, in the review desk's local time. `reviewer` is set by a lead for someone else. */
export function RotaForm({ shifts, reviewer }: { shifts: RotaShift[]; reviewer?: string }) {
  const [state, action, pending] = useActionState(saveRota, undefined);
  const by = (d: string) => shifts.find((s) => s.weekday === d);
  const [on, setOn] = useState<Record<string, boolean>>(() => Object.fromEntries(DAYS.map((d) => [d, !!by(d)])));
  const [hours, setHours] = useState<Record<string, { start: string; end: string }>>(() => Object.fromEntries(DAYS.map((d) => [d, { start: by(d)?.start ?? "09:00", end: by(d)?.end ?? "18:00" }])));
  return (
    <form action={action} style={{ display: "grid", gap: 10 }}>
      {reviewer && <input type="hidden" name="reviewer" value={reviewer} />}
      {DAYS.map((d) => (
        <div key={d} style={{ display: "flex", gap: 12, alignItems: "center" }}>
          <label style={{ width: 70, textTransform: "capitalize" }}><input type="checkbox" name={`${d}_on`} checked={on[d]} onChange={(e) => setOn({ ...on, [d]: e.target.checked })} /> {d}</label>
          <input type="time" name={`${d}_start`} value={hours[d].start} disabled={!on[d]} onChange={(e) => setHours({ ...hours, [d]: { ...hours[d], start: e.target.value } })} aria-label={`${d} from`} />
          <span className="fine">to</span>
          <input type="time" name={`${d}_end`} value={hours[d].end} disabled={!on[d]} onChange={(e) => setHours({ ...hours, [d]: { ...hours[d], end: e.target.value } })} aria-label={`${d} until`} />
        </div>
      ))}
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <div><button className="btn small primary" disabled={pending}>{pending ? "…" : reviewer ? "Save their hours" : "Save my hours"}</button></div>
    </form>
  );
}

/** Leave for yourself or, ticked, a holiday for the whole desk. Controlled, so a refusal keeps what was typed. */
export function DayOffForm({ today, others }: { today: string; others?: { id: string; name: string }[] }) {
  const [state, action, pending] = useActionState(addDayOff, undefined);
  const [v, setV] = useState({ from: today, to: "", note: "", desk: false, reviewer: "" });
  return (
    <form action={action} style={{ display: "grid", gap: 10, maxWidth: 420 }}>
      <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
        <div><label className="f" htmlFor="off-from">From</label><input id="off-from" type="date" name="from" min={today} value={v.from} onChange={(e) => setV({ ...v, from: e.target.value })} required /></div>
        <div><label className="f" htmlFor="off-to">Until (optional)</label><input id="off-to" type="date" name="to" min={v.from || today} value={v.to} onChange={(e) => setV({ ...v, to: e.target.value })} /></div>
      </div>
      <div><label className="f" htmlFor="off-note">Note (optional)</label><input id="off-note" name="note" maxLength={200} value={v.note} onChange={(e) => setV({ ...v, note: e.target.value })} placeholder="Diwali, or why you are away" /></div>
      {others && others.length > 0 && !v.desk && (
        <div><label className="f" htmlFor="off-who">For</label>
          <select id="off-who" name="reviewer" value={v.reviewer} onChange={(e) => setV({ ...v, reviewer: e.target.value })}>
            <option value="">Me</option>
            {others.map((o) => <option key={o.id} value={o.id}>{o.name}</option>)}
          </select></div>
      )}
      <label style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
        <input type="checkbox" name="desk" checked={v.desk} onChange={(e) => setV({ ...v, desk: e.target.checked })} style={{ marginTop: 3 }} />
        <span>This is a holiday for the whole desk, not only my leave.</span>
      </label>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <div><button className="btn small primary" disabled={pending}>{pending ? "…" : v.desk ? "Add holiday" : "Book leave"}</button></div>
    </form>
  );
}

export function RemoveDayOff({ id }: { id: string }) {
  const [state, action, pending] = useActionState(removeDayOff, undefined);
  return (
    <form action={action} style={{ display: "inline" }}>
      <input type="hidden" name="id" value={id} />
      <button className="btn small" disabled={pending}>{pending ? "…" : "Remove"}</button>
      {state?.error && <span className="fine" role="alert"> {state.error}</span>}
    </form>
  );
}

/** Where to be told that something was given to you; empty stops the emails. Controlled, so a refusal keeps what was typed. */
export function EmailForm({ email }: { email: string | null }) {
  const [state, action, pending] = useActionState(saveMyEmail, undefined);
  const [v, setV] = useState(email ?? "");
  return (
    <form action={action} style={{ display: "grid", gap: 10, maxWidth: 420 }}>
      <div><label className="f" htmlFor="my-email">Your email</label><input id="my-email" type="email" name="email" autoComplete="email" value={v} onChange={(e) => setV(e.target.value)} placeholder="you@example.com" /></div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <div><button className="btn small primary" disabled={pending}>{pending ? "…" : "Save"}</button></div>
    </form>
  );
}

export function RebalanceForm() {
  const [state, action, pending] = useActionState(rebalance, undefined);
  return (
    <form action={action} style={{ display: "grid", gap: 10 }}>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <label style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
        <input type="checkbox" name="even" style={{ marginTop: 3 }} />
        <span>Also even out the load between the people on duty. This can move something a reviewer has already started.</span>
      </label>
      <div><button className="btn small" disabled={pending}>{pending ? "…" : "Re-balance"}</button></div>
    </form>
  );
}
