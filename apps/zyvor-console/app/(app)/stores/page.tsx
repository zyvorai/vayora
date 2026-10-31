import { Act } from "@/components/Act";
import { BranchForm, CounterForm } from "@/components/StoreForms";
import { deactivateCounter } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Stores() {
  const [me, z] = [await currentSession(), await api()];
  const [branches, counters] = await Promise.all([z.branches.list(), z.counters.list()]);
  const admin = me?.r === "admin";
  const summaries = await Promise.all(branches.data.map(async (b) => [b.id, (await z.counters.branchSummary(b.id)).data] as const));
  const sum = new Map(summaries);
  return (
    <>
      <div className="head"><div><h1>Branches and counters</h1><p className="lede">A counter’s payment-link QR ties every payment made through it to that counter and branch. Switching a counter off also cancels its links.</p></div></div>
      {admin && <><BranchForm />{branches.data.length > 0 && <CounterForm branches={branches.data} />}</>}
      {branches.data.length === 0 && <div className="card empty">No branches yet.</div>}
      {branches.data.map((b) => {
        const rows = sum.get(b.id) ?? [];
        const net = rows.reduce((n, r) => n + r.net_minor, 0);
        const mine = counters.data.filter((c) => c.branch_id === b.id);
        return (
          <section key={b.id} style={{ marginTop: 24 }}>
            <h2>{b.name} <span className="muted" style={{ fontWeight: 400 }}>· {inr(net)} collected in the last 30 days, after refunds</span></h2>
            <div className="tablewrap"><table>
              <thead><tr><th>Counter</th><th>Status</th><th className="num">Collected (30 days)</th><th /></tr></thead>
              <tbody>
                {mine.length === 0 && <tr><td colSpan={4} className="empty">No counters yet.</td></tr>}
                {mine.map((c) => (
                  <tr key={c.id}>
                    <td>{c.name}</td><td>{c.active ? "On" : "Off"}</td>
                    <td className="num">{inr(rows.filter((r) => r.counter_id === c.id).reduce((n, r) => n + r.net_minor, 0))}</td>
                    <td>{c.active && admin && <Act action={deactivateCounter} id={c.id} label="Switch off" />}</td>
                  </tr>
                ))}
              </tbody>
            </table></div>
          </section>
        );
      })}
    </>
  );
}
