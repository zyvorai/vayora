import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { resolveDispute } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Disputes() {
  const page = await api().disputes.list();
  const active = page.data.filter((d) => d.status === "open" || d.status === "under_review");
  return (
    <>
      <div className="head"><div><h1>Disputes</h1><p className="lede">{active.length ? `${active.length} awaiting a decision. The disputed amount is held and cannot be refunded.` : "No active disputes."}</p></div></div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th>Reason</th><th className="num">Amount</th><th>Respond by</th><th className="num">Evidence</th><th>Status</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={7} className="empty">No disputes.</td></tr>}
            {page.data.map((d) => (
              <tr key={d.id}>
                <td className="mono">{d.id.slice(0, 14)}…</td><td>{d.reason}</td><td className="num">{inr(d.amount_minor)}</td>
                <td>{d.respond_by ?? <span className="muted">—</span>}</td><td className="num">{d.evidence_count}</td><td><Chip status={d.status} /></td>
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
