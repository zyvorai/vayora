import Link from "next/link";
import { Chip } from "@/components/Chip";
import { inr, istDateTime } from "@/lib/format";
import { api } from "@/lib/vayora";

const FILTERS = [["", "All"], ["captured", "Captured"], ["failed", "Failed"]] as const;

export default async function Payments({ searchParams }: { searchParams: Promise<{ status?: string; after?: string }> }) {
  const { status = "", after } = await searchParams;
  const safeStatus = FILTERS.some(([k]) => k === status) ? status : "";
  const page = await api().payments.list({ limit: 20, status: safeStatus || undefined, startingAfter: after });
  const last = page.data.at(-1);
  const q = (extra: Record<string, string>) => {
    const p = new URLSearchParams({ ...(safeStatus ? { status: safeStatus } : {}), ...extra });
    const s = p.toString();
    return s ? `/payments?${s}` : "/payments";
  };

  return (
    <>
      <div className="head">
        <div><h1>Payments</h1><p className="lede">Every UPI attempt, newest first.</p></div>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map(([k, label]) => (
            <Link key={k} href={k ? `/payments?status=${k}` : "/payments"} aria-current={safeStatus === k}>{label}</Link>
          ))}
        </div>
      </div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th>Amount</th><th>Status</th><th>UPI ID</th><th>UPI ref</th><th>Created (IST)</th></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={6} className="empty">No payments{safeStatus ? ` with status “${safeStatus}”` : " yet"}.</td></tr>}
            {page.data.map((p) => (
              <tr key={p.id}>
                <td><Link className="row mono" href={`/payments/${p.id}`}>{p.id.slice(0, 12)}…</Link></td>
                <td>{inr(p.amount_paise)}{p.refunded_paise > 0 && <div className="muted" style={{ fontSize: ".75rem" }}>{inr(p.refunded_paise)} refunded</div>}</td>
                <td><Chip status={p.status} />{p.failure_reason && <div className="muted" style={{ fontSize: ".75rem" }}>{p.failure_reason.replace("_", " ")}</div>}</td>
                <td className="mono">{p.vpa}</td>
                <td className="mono muted">{p.upi_ref ?? "n/a"}</td>
                <td className="muted">{istDateTime(p.created_at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="pager">
        {after ? <Link className="btn small" href={q({})}>← Newest</Link> : <span />}
        {page.has_more && last ? <Link className="btn small" href={q({ after: last.id })}>Older →</Link> : <span />}
      </div>
    </>
  );
}
