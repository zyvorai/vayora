import { notFound, redirect } from "next/navigation";
import { Chip } from "@/components/Chip";
import { DecisionForm } from "@/components/ReviewerForms";
import { outcomeTone, platform, requireReviewerSurface, reviewerKey, type ReviewDetail } from "@/lib/reviewer";

const KIND: Record<string, string> = { pan: "PAN", bank_account: "Bank account", gstin: "GSTIN" };

export default async function Review({ params }: { params: Promise<{ org: string }> }) {
  requireReviewerSurface();
  const key = await reviewerKey();
  if (!key) redirect("/reviewer");
  const { org } = await params;
  const r = await platform<ReviewDetail>(key, "GET", `/reviews/${encodeURIComponent(org)}`);
  if (!r.ok) {
    if (r.status === 401) redirect("/reviewer");
    if (r.status === 404) notFound();
    return <div className="note bad" role="alert">{r.message}</div>;
  }
  const d = r.body;
  const p = d.profile;
  const unclear = d.checks.filter((c) => c.outcome !== "pass");
  const pending = d.status === "pending";
  return (
    <>
      <p className="fine"><a href="/reviewer">← All requests</a></p>
      <div className="head"><div><h1>{p?.legal_name ?? d.organisation}</h1><p className="lede">{d.organisation} · {d.organisation_id} · asked by {d.requested_by} on {new Date(d.requested_at).toLocaleString("en-IN", { dateStyle: "medium", timeStyle: "short" })} · round {d.round}</p></div><Chip status={d.status} /></div>

      {!pending && (
        <div className={`note ${d.status === "approved" ? "ok" : "bad"}`} role="status" style={{ marginBottom: 16 }}>
          {d.status === "approved" ? "Approved" : "Rejected"} by {d.reviewed_by ?? "a reviewer"}.{d.note ? ` Note: ${d.note}` : ""}
        </div>
      )}
      {pending && d.checks.length === 0 && (
        <div className="note warn" role="status" style={{ marginBottom: 16 }}>
          <strong>No automatic check ran for this business.</strong> Nothing here verifies the details, so check them yourself before approving.
        </div>
      )}
      {pending && unclear.length > 0 && (
        <div className="note warn" role="status" style={{ marginBottom: 16 }}>
          <strong>{unclear.length} check{unclear.length === 1 ? " did" : "s did"} not pass.</strong> Read the reasons below before you decide.
        </div>
      )}

      <div className="grid" style={{ gridTemplateColumns: "repeat(auto-fit, minmax(300px, 1fr))" }}>
        <section className="card">
          <h2>Business</h2>
          {p ? (
            <dl className="kv">
              <dt>Legal name</dt><dd>{p.legal_name}</dd>
              <dt>Type</dt><dd>{p.business_type.replace(/_/g, " ")}</dd>
              <dt>PAN</dt><dd>{p.pan}</dd>
              <dt>GSTIN</dt><dd>{p.gstin ?? "none"}</dd>
              <dt>Address</dt><dd>{p.address}</dd>
              <dt>Details approved by</dt><dd>{p.reviewed_by ?? "no one yet"}</dd>
              {p.review_reasons && p.review_reasons.length > 0 && (<><dt>Sent to a person because</dt><dd><ul style={{ margin: 0, paddingLeft: 18 }}>{p.review_reasons.map((x) => <li key={x}>{x}</li>)}</ul></dd></>)}
            </dl>
          ) : <p className="fine">No business details on file.</p>}
        </section>

        <section className="card">
          <h2>Checks that ran</h2>
          {d.checks.length === 0 ? <p className="fine">No automatic checks ran, so nothing here verifies the details. Check them yourself.</p> : (
            <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 10 }}>
              {d.checks.map((c, i) => (
                <li key={i}><span className={`chip ${outcomeTone(c.outcome)}`}>{c.outcome}</span> <strong>{KIND[c.kind] ?? c.kind}</strong><div className="fine">{c.detail}</div></li>
              ))}
            </ul>
          )}
        </section>

        <section className="card">
          <h2>Bank accounts</h2>
          {d.bank_accounts.length === 0 ? <p className="fine">None added.</p> : (
            <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 10 }}>
              {d.bank_accounts.map((b, i) => <li key={i}><strong>{b.holder_name}</strong><div className="fine">{b.ifsc} · ••••{b.account_last4} · {b.verified ? "verified" : "not verified"}</div></li>)}
            </ul>
          )}
        </section>

        <section className="card">
          <h2>Documents</h2>
          <p className="fine">Zyvor keeps references, not files. Open each one where it is kept.</p>
          {d.documents.length === 0 ? <p className="fine">None added.</p> : (
            <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 10 }}>
              {d.documents.map((x, i) => <li key={i}><strong>{x.kind.replace(/_/g, " ")}</strong> <Chip status={x.status} /><div className="fine">{x.reference}</div></li>)}
            </ul>
          )}
        </section>
      </div>

      {pending && (
        <section className="card" style={{ marginTop: 16 }}>
          <h2>Your decision</h2>
          <p className="fine">Check that the legal name, PAN holder and bank account holder are the same party, that the documents exist and match, and that the business is one you will put on live rails.</p>
          <DecisionForm org={d.organisation_id} />
        </section>
      )}
    </>
  );
}
