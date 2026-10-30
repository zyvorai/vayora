import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { CashForm, ClosingForm } from "@/components/StoreForms";
import { signOffClosing } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Closing() {
  const [me, z] = [await currentSession(), api()];
  const [branches, closings] = await Promise.all([z.branches.list(), z.closings.list()]);
  const name = new Map(branches.data.map((b) => [b.id, b.name]));
  const today = new Date(Date.now() + 330 * 60_000).toISOString().slice(0, 10);
  return (
    <>
      <div className="head"><div><h1>Daily closing</h1><p className="lede">Record cash as it comes and goes, then count the till at the end of the day. Zyvor shows what it expected and the difference; a second person signs it off. This is a record, not a transfer of money.</p></div></div>
      {branches.data.length === 0 ? <div className="card empty">Add a branch under Branches first.</div> : (
        <><CashForm branches={branches.data} /><ClosingForm branches={branches.data} today={today} /></>
      )}
      <div className="tablewrap"><table>
        <thead><tr><th>Day</th><th>Branch</th><th className="num">Expected cash</th><th className="num">Counted</th><th className="num">Difference</th><th className="num">Digital (net)</th><th>Status</th><th /></tr></thead>
        <tbody>
          {closings.data.length === 0 && <tr><td colSpan={8} className="empty">No days closed yet.</td></tr>}
          {closings.data.map((c) => (
            <tr key={c.id}>
              <td>{c.day}</td><td>{name.get(c.branch_id) ?? c.branch_id}</td>
              <td className="num">{inr(c.expected_cash_minor)}</td><td className="num">{inr(c.counted_cash_minor)}</td>
              <td className="num" style={{ color: c.difference_minor === 0 ? undefined : "var(--bad, #c0392b)" }}>{c.difference_minor === 0 ? "None" : (c.difference_minor > 0 ? "+" : "") + inr(c.difference_minor)}</td>
              <td className="num">{inr(c.digital.reduce((n, d) => n + d.net_minor, 0))}</td>
              <td><Chip status={c.status} /></td>
              <td style={{ whiteSpace: "nowrap" }}>
                {c.status === "pending_signoff" && me?.r === "admin" && c.counted_by !== `user:${me.u}` && <Act action={signOffClosing} id={c.id} label="Sign off" />}
                {c.status === "pending_signoff" && c.counted_by === `user:${me?.u}` && <span className="muted">Waiting for someone else</span>}
              </td>
            </tr>
          ))}
        </tbody>
      </table></div>
    </>
  );
}
