import Link from "next/link";
import { notFound } from "next/navigation";
import { Chip } from "@/components/Chip";
import { istDateTime } from "@/lib/format";
import { api } from "@/lib/zyvor";
import { ZyvorError } from "@zyvor/node";

const LABEL: Record<string, string> = { intent: "Created", attempt: "Attempt", webhook: "Provider message", ledger: "Ledger", refund: "Refund", dispute: "Dispute", settlement: "Settlement" };

export default async function Payment({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  let t;
  try {
    t = await (await api()).intents.timeline(id);
  } catch (e) {
    if (e instanceof ZyvorError && e.status === 404) notFound();
    throw e;
  }
  return (
    <>
      <div className="head"><div><h1>Payment</h1><p className="lede mono">{t.payment_id}</p></div><Chip status={t.status} /></div>
      <div className="card" style={{ marginBottom: 24 }}><strong>Where it stands</strong><p style={{ margin: "6px 0 0" }}>{t.standing}</p></div>
      <ol style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 14 }}>
        {t.events.map((e, i) => (
          <li key={i} style={{ display: "grid", gridTemplateColumns: "150px 110px 1fr", gap: 12, alignItems: "baseline" }}>
            <span className="muted">{istDateTime(e.at)}</span>
            <span className="muted">{LABEL[e.kind] ?? e.kind}</span>
            <span>{e.title}</span>
          </li>
        ))}
      </ol>
      <p style={{ marginTop: 24 }}><Link className="btn small" href="/payments">← All payments</Link></p>
    </>
  );
}
