import Link from "next/link";
import { Chip } from "@/components/Chip";
import { istDateTime } from "@/lib/format";
import { retryEvent } from "@/lib/actions";
import { api } from "@/lib/vayora";

const FILTERS = [["", "All"], ["pending", "Pending"], ["delivered", "Delivered"], ["dead", "Dead"]] as const;

export default async function Webhooks({ searchParams }: { searchParams: Promise<{ status?: string; after?: string }> }) {
  const { status = "", after } = await searchParams;
  const safe = FILTERS.some(([k]) => k === status) ? status : "";
  const page = await api().events.list({ limit: 20, status: safe || undefined, startingAfter: after });
  const last = page.data.at(-1);
  const q = (extra: Record<string, string>) => {
    const s = new URLSearchParams({ ...(safe ? { status: safe } : {}), ...extra }).toString();
    return s ? `/webhooks?${s}` : "/webhooks";
  };

  return (
    <>
      <div className="head">
        <div><h1>Webhooks</h1><p className="lede">Signed events, delivered at least once. Failed deliveries retry with backoff, then park as dead.</p></div>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map(([k, label]) => (<Link key={k} href={k ? `/webhooks?status=${k}` : "/webhooks"} aria-current={safe === k}>{label}</Link>))}
        </div>
      </div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Event</th><th>Status</th><th className="num">Attempts</th><th>Created (IST)</th><th>Detail</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={6} className="empty">No events{safe ? ` with status “${safe}”` : " yet"}.</td></tr>}
            {page.data.map((e) => (
              <tr key={e.id}>
                <td><b>{e.type}</b><div className="mono muted">{e.id.slice(0, 14)}…</div></td>
                <td><Chip status={e.status} /></td>
                <td className="num">{e.attempts}</td>
                <td className="muted">{istDateTime(e.created_at)}</td>
                <td style={{ maxWidth: 360 }}>
                  {e.last_error && <div style={{ color: "var(--bad)", fontSize: ".8125rem" }}>{e.last_error}</div>}
                  <details><summary>Payload</summary><pre className="json mono">{JSON.stringify(e.payload, null, 2)}</pre></details>
                </td>
                <td className="num">
                  {e.status !== "delivered" && (
                    <form action={retryEvent}><input type="hidden" name="id" value={e.id} /><button className="btn small">Retry now</button></form>
                  )}
                </td>
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
