import Link from "next/link";
import { redirect } from "next/navigation";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { replayEvent, sendTestEvent } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "pending", "delivered", "dead"];

export default async function Webhooks({ searchParams }: { searchParams: Promise<{ status?: string }> }) {
  const { status } = await searchParams;
  if ((await currentSession())?.r !== "admin") redirect("/");
  const z = await api();
  const [page, settings] = await Promise.all([z.webhooks.outbox(status || undefined), z.settings.get()]);
  return (
    <>
      <div className="head">
        <div><h1>Webhooks</h1><p className="lede">{settings.outbound_webhook_url ? <>Sent to <code style={{ overflowWrap: "anywhere" }}>{settings.outbound_webhook_url}</code>. </> : <>No webhook address is set; add one in <Link href="/settings">Settings</Link>. </>}Events we send to your systems. Replaying is always safe: receivers de-duplicate on the event id.</p></div>
        {settings.outbound_webhook_url && <Act action={sendTestEvent} label="Send test event" primary />}
        <div className="seg" role="group" aria-label="Filter by status">
          {FILTERS.map((f) => <Link key={f} href={f ? `/webhooks?status=${f}` : "/webhooks"} aria-current={(status ?? "") === f}>{f || "All"}</Link>)}
        </div>
      </div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Event</th><th>Type</th><th>Status</th><th className="num">Attempts</th><th>Last error</th><th>Created (IST)</th><th>Payload</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={8} className="empty">No events.</td></tr>}
            {page.data.map((e) => {
              const r = e as Record<string, string | number | null>;
              return (
                <tr key={String(r.id)}>
                  <td className="mono">{String(r.id).slice(0, 14)}…</td><td>{String(r.type)}</td><td><Chip status={String(r.status)} /></td>
                  <td className="num">{r.attempts}</td><td className="muted">{r.last_error ?? ""}</td><td className="muted">{istDateTime(String(r.created_at))}</td>
                  <td><details><summary className="muted" style={{ cursor: "pointer" }}>View</summary><pre className="code" tabIndex={0}>{JSON.stringify(r.payload, null, 2)}</pre></details></td>
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
