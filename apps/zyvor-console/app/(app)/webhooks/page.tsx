import Link from "next/link";
import { redirect } from "next/navigation";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { replayEvent, sendTestEvent } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";

const FILTERS = ["", "pending", "delivered", "dead"];

export default async function Webhooks({ searchParams }: { searchParams: Promise<{ status?: string; event?: string }> }) {
  const { status, event } = await searchParams;
  if ((await currentSession())?.r !== "admin") redirect("/");
  const z = await api();
  const [page, settings, deliveries, inbox] = await Promise.all([
    z.webhooks.outbox(status || undefined),
    z.settings.get(),
    event ? z.webhooks.deliveries(event).catch(() => ({ data: [] as Record<string, unknown>[] })) : Promise.resolve(null),
    z.webhooks.inbox(30).catch(() => ({ data: [] as Record<string, unknown>[] })),
  ]);
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
            {page.data.length === 0 && <tr><td colSpan={8} className="empty">No events yet. They appear here when payments, refunds and invoices change.</td></tr>}
            {page.data.map((e) => {
              const r = e as Record<string, string | number | null>;
              const id = String(r.id);
              return (
                <tr key={id}>
                  <td className="mono"><Link href={`/webhooks?event=${encodeURIComponent(id)}${status ? `&status=${status}` : ""}`}>{id.slice(0, 14)}…</Link></td>
                  <td>{String(r.type)}</td><td><Chip status={String(r.status)} /></td>
                  <td className="num">{r.attempts}</td><td className="muted">{r.last_error ?? ""}</td><td className="muted">{istDateTime(String(r.created_at))}</td>
                  <td><details><summary className="muted" style={{ cursor: "pointer" }}>View</summary><pre className="code" tabIndex={0}>{JSON.stringify(r.payload, null, 2)}</pre></details></td>
                  <td><Act action={replayEvent} id={id} label="Replay" /></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {event && (
        <section style={{ marginTop: 24 }}>
          <h2 style={{ fontSize: "1.1rem" }}>Delivery attempts for <code className="mono">{event}</code></h2>
          <p className="muted"><Link href={status ? `/webhooks?status=${status}` : "/webhooks"}>Clear</Link></p>
          <div className="tablewrap">
            <table>
              <thead><tr><th className="num">#</th><th>Result</th><th className="num">HTTP</th><th className="num">ms</th><th>Error</th><th>When (IST)</th></tr></thead>
              <tbody>
                {(deliveries?.data?.length ?? 0) === 0 && <tr><td colSpan={6} className="empty">No delivery attempts recorded yet for this event.</td></tr>}
                {(deliveries?.data ?? []).map((d) => {
                  const r = d as Record<string, string | number | boolean | null>;
                  return (
                    <tr key={String(r.id)}>
                      <td className="num">{r.attempt_no}</td>
                      <td><Chip status={r.ok ? "delivered" : "dead"} /></td>
                      <td className="num">{r.http_status ?? "—"}</td>
                      <td className="num">{r.latency_ms ?? "—"}</td>
                      <td className="muted">{r.error ?? ""}</td>
                      <td className="muted">{istDateTime(String(r.created_at))}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </section>
      )}
      <section style={{ marginTop: 32 }}>
        <h2 style={{ fontSize: "1.1rem" }}>From providers</h2>
        <p className="muted">Webhooks Zyvor received from connected payment providers (inbound inspector).</p>
        <div className="tablewrap">
          <table>
            <thead><tr><th>Provider</th><th>Kind</th><th>Outcome</th><th>Event</th><th>Received (IST)</th></tr></thead>
            <tbody>
              {inbox.data.length === 0 && <tr><td colSpan={5} className="empty">No provider webhooks received yet.</td></tr>}
              {inbox.data.map((e) => {
                const r = e as Record<string, string | number | null>;
                return (
                  <tr key={String(r.id)}>
                    <td>{String(r.provider)}</td>
                    <td>{String(r.kind)}</td>
                    <td><Chip status={String(r.outcome)} /></td>
                    <td className="mono muted">{String(r.event_id).slice(0, 18)}…</td>
                    <td className="muted">{istDateTime(String(r.received_at))}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}
