import Link from "next/link";
import { Chip } from "@/components/Chip";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "succeeded", "processing", "failed", "requires_payment_method"];

export default async function Payments({ searchParams }: { searchParams: Promise<{ status?: string; after?: string }> }) {
  const { status, after } = await searchParams;
  const page = await api().intents.list({ limit: 25, status: status || undefined, startingAfter: after });
  const last = page.data.at(-1);
  return (
    <>
      <div className="head">
        <div><h1>Payments</h1><p className="lede">Every payment intent, with the reference of the thing it paid for.</p></div>
        <a className="btn small" href={`/export/payments${status ? `?status=${status}` : ""}`}>Export CSV</a>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map((f) => <Link key={f} href={f ? `/payments?status=${f}` : "/payments"} aria-current={(status ?? "") === f}>{f ? f.replace(/_/g, " ") : "All"}</Link>)}
        </div>
      </div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th>Reference</th><th className="num">Amount</th><th>Status</th><th>Created (IST)</th></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={5} className="empty">No payments match.</td></tr>}
            {page.data.map((i) => (
              <tr key={i.id}>
                <td className="mono">{i.id.slice(0, 16)}…</td>
                <td>{i.external_ref ?? <span className="muted">—</span>}</td>
                <td className="num">{inr(i.amount_minor)}</td><td><Chip status={i.status} /></td>
                <td className="muted">{istDateTime((i as unknown as { created_at: string }).created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="pager">
        {after ? <Link className="btn small" href="/payments">← Newest</Link> : <span />}
        {page.has_more && last ? <Link className="btn small" href={`/payments?${status ? `status=${status}&` : ""}after=${last.id}`}>Older →</Link> : <span />}
      </div>
    </>
  );
}
