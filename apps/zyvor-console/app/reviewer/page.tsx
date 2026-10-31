import { redirect } from "next/navigation";
import { Chip } from "@/components/Chip";
import { ReviewerSignIn } from "@/components/ReviewerForms";
import { platform, requireReviewerSurface, reviewerKey, type ReviewRow } from "@/lib/reviewer";

const FILTERS = ["pending", "approved", "rejected", "revoked"] as const;

const DONE: Record<string, string> = {
  approve: "Approved. The merchant can switch to Live and has been emailed.",
  reject: "Rejected. The merchant sees your note and has been emailed.",
  revoke: "Live mode taken away. New payments and payouts for that business have stopped, and the merchant has been emailed.",
  restore: "Live mode given back. The merchant has been emailed.",
};

export default async function Queue({ searchParams }: { searchParams: Promise<{ status?: string; done?: string }> }) {
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
  const r = await platform<{ data: ReviewRow[] }>(key, "GET", `/reviews?status=${status}`);
  if (!r.ok && r.status === 401) redirect("/reviewer?expired=1");
  const rows = r.ok ? r.body.data : [];
  return (
    <>
      <div className="head"><div><h1>Live access requests</h1><p className="lede">Oldest first. Open one to see what the business submitted and the checks that ran.</p></div></div>
      {q.done && <div className="note ok" role="status" style={{ marginBottom: 16 }}>{DONE[q.done] ?? "Done."}</div>}
      {!r.ok && <div className="note bad" role="alert" style={{ marginBottom: 16 }}>{r.message}</div>}
      <nav className="tabs" aria-label="Status" style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        {FILTERS.map((f) => <a key={f} className={`btn small${f === status ? " primary" : ""}`} href={`/reviewer?status=${f}`} aria-current={f === status ? "page" : undefined}>{f[0].toUpperCase() + f.slice(1)}</a>)}
      </nav>
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
                <td><Chip status={x.status} /></td>
              </tr>
            ))}
            {rows.length === 0 && <tr><td colSpan={5} className="fine" style={{ padding: 24, textAlign: "center" }}>{status === "pending" ? "Nobody is waiting. " : "Nothing here. "}</td></tr>}
          </tbody>
        </table>
      </div>
    </>
  );
}
