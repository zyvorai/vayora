import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { resolveDispute } from "@/lib/actions";
import Link from "next/link";
import { inr, istDateTime } from "@/lib/format";
import { dueIn, words } from "@/lib/labels";
import { api } from "@/lib/zyvor";

export default async function Disputes() {
  const page = await (await api()).disputes.list();
  const active = page.data.filter((d) => d.status === "open" || d.status === "under_review");
  return (
    <>
      <div className="head"><div><h1>Disputes</h1><p className="lede">{active.length ? `${active.length} awaiting a decision. The disputed amount is held and cannot be refunded.` : "No active disputes."}</p></div></div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Opened</th><th>Payment</th><th>Reason</th><th className="num">Amount</th><th>Respond by</th><th className="num">Evidence</th><th>Status</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={8} className="empty">No disputes. One appears here when a payer's bank challenges a payment, and you answer it with evidence.</td></tr>}
            {page.data.map((d) => (
              <tr key={d.id}>
                <td className="muted" style={{ whiteSpace: "nowrap" }}>{istDateTime(d.created_at)}</td>
                <td><Link className="mono" href={`/payments/${d.intent_id}`}>{d.intent_id.slice(0, 13)}…</Link></td>
                <td>{words(d.reason)}</td><td className="num">{inr(d.amount_minor)}</td>
                <td>{d.respond_by ? <>{d.respond_by}{(d.status === "open" || d.status === "under_review") && (() => { const due = dueIn(d.respond_by); return <> <span className={`chip ${due.tone}`}>{due.text}</span></>; })()}</> : <span className="muted">—</span>}</td><td className="num">{d.evidence_count}</td><td><Chip status={d.status} /></td>
                <td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                  {(d.status === "open" || d.status === "under_review") && <><Act action={resolveDispute} id={d.id} label="Mark won" extra={{ outcome: "won" }} /><Act action={resolveDispute} id={d.id} label="Mark lost" extra={{ outcome: "lost" }} /></>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
