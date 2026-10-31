import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { approveRefund, rejectRefund, retryRefund } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Refunds() {
  const page = await (await api()).refunds.list({ limit: 50 });
  const needs = page.data.filter((r) => r.status === "pending_approval");
  return (
    <>
      <div className="head"><div><h1>Refunds</h1><p className="lede">{needs.length ? `${needs.length} waiting for a second person to approve.` : "Nothing waiting for approval."}</p></div></div>
      <div className="tablewrap">
        <table>
          <thead><tr><th>ID</th><th className="num">Amount</th><th>Status</th><th>Requested by</th><th>Return ref</th><th /></tr></thead>
          <tbody>
            {page.data.length === 0 && <tr><td colSpan={6} className="empty">No refunds yet.</td></tr>}
            {page.data.map((r) => (
              <tr key={r.id}>
                <td className="mono">{r.id.slice(0, 16)}…</td>
                <td className="num">{inr(r.amount_minor)}</td><td><Chip status={r.status} /></td>
                <td>{r.requested_by}</td><td>{r.external_ref ?? <span className="muted">—</span>}</td>
                <td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
                  {r.status === "pending_approval" && <><Act action={approveRefund} id={r.id} label="Approve" primary /><Act action={rejectRefund} id={r.id} label="Reject" /></>}
                  {r.status === "failed" && <Act action={retryRefund} id={r.id} label="Retry" />}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </>
  );
}
