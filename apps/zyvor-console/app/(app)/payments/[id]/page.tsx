import Link from "next/link";
import { notFound } from "next/navigation";
import { Chip } from "@/components/Chip";
import { RefundForm } from "@/components/RefundForm";
import { currentSession } from "@/lib/auth";
import { inr, istDateTime } from "@/lib/format";
import { allows } from "@/lib/session";
import { api } from "@/lib/zyvor";
import { ZyvorError } from "@zyvor/node";

const LABEL: Record<string, string> = { intent: "Created", attempt: "Attempt", webhook: "Provider message", ledger: "Ledger", refund: "Refund", dispute: "Dispute", settlement: "Settlement" };

export default async function Payment({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const z = await api();
  let t, p;
  try {
    [t, p] = await Promise.all([z.intents.timeline(id), z.intents.retrieve(id)]);
  } catch (e) {
    if (e instanceof ZyvorError && e.status === 404) notFound();
    throw e;
  }
  const me = await currentSession();
  const paid = p.attempts.find((a) => a.status === "captured");
  // what is left to refund: what was captured, less refunds done and refunds already asked for
  const refundable = paid ? paid.amount_minor - paid.refunded_minor - paid.reserved_refund_minor : 0;
  return (
    <>
      <div className="head"><div><h1>Payment</h1><p className="lede mono" style={{ overflowWrap: "anywhere" }}>{t.payment_id}</p></div><Chip status={t.status} /></div>
      <div className="facts">
        <div className="card"><div className="k">Amount</div><div className="v">{inr(p.intent.amount_minor)}</div></div>
        <div className="card"><div className="k">Reference</div><div className="v">{p.intent.external_ref ?? "—"}</div></div>
        <div className="card"><div className="k">Paid with</div><div className="v">{paid ? `${paid.method.toUpperCase()} · ${paid.provider}` : "—"}</div></div>
        <div className="card"><div className="k">Refunded</div><div className="v">{paid ? inr(paid.refunded_minor) : "—"}</div></div>
      </div>
      <div className="card" style={{ marginBottom: 24 }}><strong>Where it stands</strong><p style={{ margin: "6px 0 0" }}>{t.standing}</p></div>
      {p.attempts.length > 0 && (<>
        <h2>Attempts</h2>
        <div className="tablewrap" style={{ marginBottom: 24 }}><table>
          <thead><tr><th>Provider</th><th>Method</th><th className="num">Amount</th><th>Status</th><th>Why it failed</th></tr></thead>
          <tbody>{p.attempts.map((a) => <tr key={a.id}><td>{a.provider}</td><td>{a.method}</td><td className="num">{inr(a.amount_minor)}</td><td><Chip status={a.status} /></td><td className="muted">{a.failure_reason ?? "—"}</td></tr>)}</tbody>
        </table></div>
      </>)}
      <h2>Timeline</h2>
      <ol style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 14 }}>
        {t.events.map((e, i) => (
          <li key={i} style={{ display: "grid", gridTemplateColumns: "150px 110px 1fr", gap: 12, alignItems: "baseline" }}>
            <span className="muted">{istDateTime(e.at)}</span>
            <span className="muted">{LABEL[e.kind] ?? e.kind}</span>
            <span>{e.title}</span>
          </li>
        ))}
      </ol>
      {me && allows(me.r, "finance") && refundable > 0 && <RefundForm intentId={id} refundableMinor={refundable} />}
      <p style={{ marginTop: 24 }}><Link className="btn small" href="/payments">← All payments</Link></p>
    </>
  );
}
