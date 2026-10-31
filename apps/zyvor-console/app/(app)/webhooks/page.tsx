import Link from "next/link";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { replayEvent } from "@/lib/actions";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "pending", "delivered", "dead"];

export default async function Webhooks({ searchParams }: { searchParams: Promise<{ status?: string }> }) {
  const { status } = await searchParams;
  const page = await (await api()).webhooks.outbox(status || undefined);
  return (
    <>
      <div className="head">
        <div><h1>Webhooks</h1><p className="lede">Events we send to your systems. Replaying is always safe: receivers de-duplicate on the event id.</p></div>
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map((f) => <Link key={f} href={f ? `/webhooks?status=${f}` : "/webhooks"} aria-current={(status ?? "") === f}>{f || "All"}</Link>)}
        </div>
      </div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Event</th><th>Type</th><th>Status</th><th className="num">Attempts</th><th>Last error</th><th>Created (IST)</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={7} className="empty">No events.</td></tr>}
            {page.data.map((e) => {
              const r = e as Record<string, string | number | null>;
              return (
                <tr key={String(r.id)}>
                  <td className="mono">{String(r.id).slice(0, 14)}…</td><td>{String(r.type)}</td><td><Chip status={String(r.status)} /></td>
                  <td className="num">{r.attempts}</td><td className="muted">{r.last_error ?? ""}</td><td className="muted">{istDateTime(String(r.created_at))}</td>
                  <td><Act action={replayEvent} id={String(r.id)} label="Replay" /></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
