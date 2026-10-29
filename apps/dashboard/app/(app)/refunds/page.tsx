import Link from "next/link";
import { Chip } from "@/components/Chip";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/vayora";

export default async function Refunds({ searchParams }: { searchParams: Promise<{ after?: string }> }) {
  const { after } = await searchParams;
  const page = await api().refunds.list({ limit: 20, startingAfter: after });
  const last = page.data.at(-1);
  return (
    <>
      <div className="head"><div><h1>Refunds</h1><p className="lede">Money returned to payers, newest first.</p></div></div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th>Payment</th><th>Amount</th><th>Status</th><th>UPI ref</th><th>Created (IST)</th></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={6} className="empty">No refunds yet.</td></tr>}
            {page.data.map((r) => (
              <tr key={r.id}>
                <td className="mono">{r.id.slice(0, 13)}…</td>
                <td><Link className="row mono" href={`/payments/${r.payment_id}`}>{r.payment_id.slice(0, 12)}…</Link></td>
                <td>{inr(r.amount_paise)}</td><td><Chip status={r.status} /></td>
                <td className="mono muted">{r.upi_ref}</td><td className="muted">{istDateTime(r.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="pager">
        {after ? <Link className="btn small" href="/refunds">← Newest</Link> : <span />}
        {page.has_more && last ? <Link className="btn small" href={`/refunds?after=${last.id}`}>Older →</Link> : <span />}
      </div>
    </>
  );
}
