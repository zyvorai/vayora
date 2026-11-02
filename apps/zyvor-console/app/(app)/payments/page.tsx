import Link from "next/link";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { SaveView } from "@/components/SaveView";
import { deleteView, saveView } from "@/lib/actions";
import { inr, istDateTime } from "@/lib/format";
import { currentSession } from "@/lib/auth";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "succeeded", "processing", "failed", "requires_payment_method"];

const DAY = /^\d{4}-\d{2}-\d{2}$/;
const queryOf = (f: Record<string, string | undefined>) => { const p = new URLSearchParams(); for (const [k, v] of Object.entries(f)) if (v) p.set(k, v); const t = p.toString(); return t ? `?${t}` : ""; };

export default async function Payments({ searchParams }: { searchParams: Promise<{ status?: string; after?: string; q?: string; from?: string; to?: string }> }) {
  const sp = await searchParams;
  const { status, after } = sp;
  const term = sp.q?.trim() || undefined;
  const from = sp.from && DAY.test(sp.from) ? sp.from : undefined;
  const to = sp.to && DAY.test(sp.to) ? sp.to : undefined;
  const filters = { status: status || undefined, q: term, from, to };
  const keep = (extra = "") => { const p = new URLSearchParams(queryOf(filters)); const e = new URLSearchParams(extra); e.forEach((v, k) => p.set(k, v)); const t = p.toString(); return t ? `?${t}` : ""; };
  // views belong to the person, so ask as them (the same name the save action uses)
  const z = await api((await currentSession())?.u);
  const [page, views] = await Promise.all([
    z.intents.list({ limit: 25, status: status || undefined, startingAfter: after, q: term, createdFrom: from, createdTo: to }),
    // a branch-limited key cannot use views; the page just carries on without them
    z.savedViews.list().then((r) => r.data).catch(() => null),
  ]);
  const active = Object.fromEntries(Object.entries(filters).filter(([, v]) => v)) as Record<string, string>;
  const isActive = (q: Record<string, string>) => { const a = Object.keys(active), b = Object.keys(q); return a.length === b.length && a.every((k) => q[k] === active[k]); };
  const matching = views?.find((v) => isActive(v.query as Record<string, string>));
  const last = page.data.at(-1);
  return (
    <>
      <div className="head">
        <div><h1>Payments</h1><p className="lede">Every payment intent, with the reference of the thing it paid for.</p></div>
        <a className="btn small" href={`/export/payments${keep()}`}>Export CSV</a>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map((f) => <Link key={f} href={`/payments${queryOf({ status: f || undefined, q: term, from, to })}`} aria-current={(status ?? "") === f}>{f ? f.replace(/_/g, " ") : "All"}</Link>)}
        </div>
      </div>
      <form method="get" action="/payments" role="search" style={{ marginBottom: 12, display: "flex", gap: 8 }}>
        {status && <input type="hidden" name="status" value={status} />}
        <input name="q" defaultValue={term ?? ""} placeholder="Search payment ID, reference or description" aria-label="Search payments" maxLength={100} style={{ flex: 1 }} />
        <button className="btn small" type="submit">Search</button>
        <input type="date" name="from" defaultValue={from ?? ""} aria-label="Created from" style={{ width: 150 }} />
        <input type="date" name="to" defaultValue={to ?? ""} aria-label="Created to" style={{ width: 150 }} />
        {(term || from || to) && <Link className="btn small" href={`/payments${queryOf({ status })}`}>Clear</Link>}
      </form>
      {views && (
        <div style={{ marginBottom: 14, display: "grid", gap: 8 }}>
          {views.length > 0 && (
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
              <span className="muted">Views</span>
              {views.map((v) => (
                <span key={v.id} style={{ display: "inline-flex", gap: 4, alignItems: "center" }}>
                  <Link className="btn small" href={`/payments${queryOf(v.query as Record<string, string>)}`} aria-current={isActive(v.query as Record<string, string>)} title={v.shared && !v.mine ? `Shared by ${v.owner.replace(/^user:/, "")}` : v.shared ? "Shared with everyone here" : "Only you"}>{v.name}{v.shared ? " ·" : ""}</Link>
                  {v.mine && <Act action={deleteView} id={v.id} label="×" />}
                </span>
              ))}
            </div>
          )}
          {Object.keys(active).length > 0 && !matching && <SaveView action={saveView} filters={active} />}
        </div>
      )}
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
