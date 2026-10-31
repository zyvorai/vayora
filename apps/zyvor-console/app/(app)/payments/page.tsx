import Link from "next/link";
import { Chip } from "@/components/Chip";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "succeeded", "processing", "failed", "requires_payment_method"];

export default async function Payments({ searchParams }: { searchParams: Promise<{ status?: string; after?: string; q?: string }> }) {
  const { status, after, q } = await searchParams;
  const term = q?.trim() || undefined;
  const keep = (extra = "") => { const p = new URLSearchParams(); if (status) p.set("status", status); if (term) p.set("q", term); const e = new URLSearchParams(extra); e.forEach((v, k) => p.set(k, v)); const t = p.toString(); return t ? `?${t}` : ""; };
  const page = await (await api()).intents.list({ limit: 25, status: status || undefined, startingAfter: after, q: term });
  const last = page.data.at(-1);
  return (
    <>
      <div className="head">
        <div><h1>Payments</h1><p className="lede">Every payment intent, with the reference of the thing it paid for.</p></div>
        <a className="btn small" href={`/export/payments${keep()}`}>Export CSV</a>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map((f) => <Link key={f} href={`/payments${(() => { const p = new URLSearchParams(); if (f) p.set("status", f); if (term) p.set("q", term); const t = p.toString(); return t ? `?${t}` : ""; })()}`} aria-current={(status ?? "") === f}>{f ? f.replace(/_/g, " ") : "All"}</Link>)}
        </div>
      </div>
      <form method="get" action="/payments" role="search" style={{ marginBottom: 12, display: "flex", gap: 8 }}>
        {status && <input type="hidden" name="status" value={status} />}
        <input name="q" defaultValue={term ?? ""} placeholder="Search payment ID, reference or description" aria-label="Search payments" maxLength={100} style={{ flex: 1 }} />
        <button className="btn small" type="submit">Search</button>
        {term && <Link className="btn small" href={`/payments${status ? `?status=${status}` : ""}`}>Clear</Link>}
      </form>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th>Reference</th><th className="num">Amount</th><th>Status</th><th>Created (IST)</th></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={5} className="empty">No payments match.</td></tr>}
            {page.data.map((i) => (
              <tr key={i.id}>
                <td className="mono"><Link href={`/payments/${i.id}`}>{i.id.slice(0, 16)}…</Link></td>
                <td>{i.external_ref ?? <span className="muted">—</span>}</td>
                <td className="num">{inr(i.amount_minor)}</td><td><Chip status={i.status} /></td>
                <td className="muted">{istDateTime((i as unknown as { created_at: string }).created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="pager">
        {after ? <Link className="btn small" href={`/payments${keep()}`}>← Newest</Link> : <span />}
        {page.has_more && last ? <Link className="btn small" href={`/payments${keep(`after=${last.id}`)}`}>Older →</Link> : <span />}
      </div>
    </>
  );
}
