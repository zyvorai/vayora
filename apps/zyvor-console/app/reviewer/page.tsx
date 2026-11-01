import { redirect } from "next/navigation";
import { Chip } from "@/components/Chip";
import { ChangeForm, ReviewerSignIn } from "@/components/ReviewerForms";
import { dueLabel } from "@/lib/due";
import { platform, requireReviewerSurface, reviewerKey, type ChangeRow, type ReviewRow } from "@/lib/reviewer";

const FILTERS = ["pending", "approved", "rejected", "revoked"] as const;

const DONE: Record<string, string> = {
  approve: "Approved. The merchant can switch to Live and has been emailed.",
  reject: "Rejected. The merchant sees your note and has been emailed.",
  revoke: "Live mode taken away. New payments and payouts for that business have stopped, and the merchant has been emailed.",
  restore: "Live mode given back. The merchant has been emailed.",
  first_approval: "First approval recorded. A different reviewer must approve it too before live mode opens.",
  change_first_approval: "First approval recorded. A different reviewer must approve this change too; the business's live payouts stay held until then.",
  change_approve: "Change approved. Live payouts for that business are no longer held for it.",
  change_reject: "Change rejected. The account was removed and the merchant has been emailed your reason.",
};

export default async function Queue({ searchParams }: { searchParams: Promise<{ status?: string; done?: string; assigned?: string }> }) {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) {
    return (
      <div className="login" style={{ minHeight: "70dvh" }}>
        <div className="card">
          <h1>Zyvor review</h1>
          <p className="lede">Businesses asking for live mode wait here for a person at Zyvor.</p>
          <ReviewerSignIn />
        </div>
      </div>
    );
  }
  const q = await searchParams;
  const status = (FILTERS as readonly string[]).includes(q.status ?? "") ? (q.status as (typeof FILTERS)[number]) : "pending";
  const assigned = q.assigned === "me" || q.assigned === "none" ? q.assigned : "";
  const r = await platform<{ data: ReviewRow[] }>(key, "GET", `/reviews?status=${status}${assigned ? `&assigned=${assigned}` : ""}`);
  if (!r.ok && r.status === 401) redirect("/reviewer?expired=1");
  const rows = r.ok ? r.body.data : [];
  const ch = status === "pending" ? await platform<{ data: ChangeRow[] }>(key, "GET", "/changes?status=pending") : null;
  const changes = ch?.ok ? ch.body.data : [];
  return (
    <>
      <div className="head"><div><h1>Live access requests</h1><p className="lede">Oldest first. Open one to see what the business submitted and the checks that ran.</p></div></div>
      {q.done && <div className="note ok" role="status" style={{ marginBottom: 16 }}>{DONE[q.done] ?? "Done."}</div>}
      {!r.ok && <div className="note bad" role="alert" style={{ marginBottom: 16 }}>{r.message}</div>}
      {changes.length > 0 && (
        <section className="card" style={{ marginBottom: 16 }}>
          <h2>Changes waiting ({changes.length})</h2>
          <p className="fine">Businesses already live added something after you approved them. Their live payouts are held until you decide.</p>
          <div style={{ display: "grid", gap: 14 }}>
            {changes.map((c) => (
              <div key={c.id} style={{ display: "grid", gap: 6, gridTemplateColumns: "minmax(0, 1fr) minmax(0, 1fr)", alignItems: "start" }}>
                <div><a href={`/reviewer/review/${encodeURIComponent(c.organisation_id)}`}>{c.organisation}</a>{c.kind === "profile" ? ": change to legal details" : ": new bank account"}<div className="fine">{c.summary}</div><div className="fine">added by {c.requested_by.replace(/^user:/, "")} on {new Date(c.requested_at).toLocaleString("en-IN", { dateStyle: "medium", timeStyle: "short" })}</div>{c.stage === "awaiting_second" ? <div className="fine">{c.first_approved_by} approved; needs a different reviewer</div> : c.needs_second_because.length > 0 && <div className="fine">needs two reviewers: {c.needs_second_because.join("; ")}</div>}<div className="fine" style={dueLabel(c.due_at).late ? { color: "var(--bad)", fontWeight: 600 } : undefined}>{dueLabel(c.due_at).text}</div></div>
                <ChangeForm id={c.id} kind={c.kind} firstApprovalOnly={c.stage !== "awaiting_second" && c.needs_second_because.length > 0} />
              </div>
            ))}
          </div>
        </section>
      )}
      <nav className="tabs" aria-label="Status" style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        {FILTERS.map((f) => <a key={f} className={`btn small${f === status ? " primary" : ""}`} href={`/reviewer?status=${f}`} aria-current={f === status ? "page" : undefined}>{f[0].toUpperCase() + f.slice(1)}</a>)}
      </nav>
      {status === "pending" && (
        <nav aria-label="Who has it" style={{ display: "flex", gap: 8, marginBottom: 16 }}>
          {([["", "Everyone's"], ["me", "Mine"], ["none", "Nobody's"]] as const).map(([v, label]) => <a key={v} className={`btn small${v === assigned ? " primary" : ""}`} href={`/reviewer?status=pending${v ? `&assigned=${v}` : ""}`} aria-current={v === assigned ? "page" : undefined}>{label}</a>)}
        </nav>
      )}
      <div className="card" style={{ padding: 0, overflowX: "auto" }}>
        <table>
          <thead><tr><th>Business</th><th>Asked</th><th>Round</th><th>Checks not passed</th><th>Status</th></tr></thead>
          <tbody>
            {rows.map((x) => (
              <tr key={x.organisation_id}>
                <td><a href={`/reviewer/review/${encodeURIComponent(x.organisation_id)}`}>{x.legal_name || x.organisation}</a><div className="fine">{x.organisation_id}</div></td>
                <td>{new Date(x.requested_at).toLocaleString("en-IN", { dateStyle: "medium", timeStyle: "short" })}</td>
                <td>{x.round}</td>
                <td>{x.checks_failed === 0 ? "None" : <span className="chip warn">{x.checks_failed} to look at</span>}</td>
                <td>
                  <Chip status={x.status} />
                  {x.status === "pending" && <div className={`fine${dueLabel(x.due_at).late ? " late" : ""}`} style={dueLabel(x.due_at).late ? { color: "var(--bad)", fontWeight: 600 } : undefined}>{dueLabel(x.due_at).text}</div>}
                  {x.status === "pending" && <div className="fine">{x.assigned_to_me ? "yours" : x.assigned_to_name ? `held by ${x.assigned_to_name}` : "nobody has it"}</div>}
                  {x.stage === "awaiting_second" && <div className="fine">{x.first_approved_by} approved; needs a different reviewer</div>}
                  {x.status === "pending" && x.stage !== "awaiting_second" && x.needs_second_because.length > 0 && <div className="fine">needs two reviewers</div>}
                </td>
              </tr>
            ))}
            {rows.length === 0 && <tr><td colSpan={5} className="fine" style={{ padding: 24, textAlign: "center" }}>{status === "pending" ? "Nobody is waiting. " : "Nothing here. "}</td></tr>}
          </tbody>
        </table>
      </div>
    </>
  );
}
