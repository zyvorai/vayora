import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { assignCase, simulateBankCredit, simulateSettlement } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export default async function Reconciliation() {
  const z = await api();
  const [cases, settlements, settings] = await Promise.all([z.cases.list({}), z.settlements.list(), z.settings.get()]);
  const open = cases.data.filter((c) => c.status !== "resolved");
  // a likeliest explanation for the first few open cases; failures just leave the row without a hint
  const hints = new Map(
    (await Promise.all(open.slice(0, 15).map(async (c) => [c.id, await z.cases.suggestions(c.id).catch(() => null)] as const))).filter(([, h]) => h !== null),
  );
  return (
    <>
      <div className="head"><div><h1>Reconciliation</h1><p className="lede">Every difference between what we recorded and what arrived becomes a task with an owner.</p></div></div>
      <h2>Cases ({open.length} open)</h2>
      <div className="tablewrap" style={{ marginBottom: 28 }}>
        <table>
          <thead><tr><th>Kind</th><th>What happened</th><th className="num">Difference</th><th>Status</th><th>Owner</th><th /></tr></thead>
          <tbody>
            {cases.data.length === 0 && <tr><td colSpan={6} className="empty">Everything reconciles.</td></tr>}
            {cases.data.map((c) => (
              <tr key={c.id}>
                <td className="mono">{c.kind}</td>
                <td style={{ maxWidth: 420 }}>
                  {c.explanation}
                  {hints.get(c.id) && <div className="muted" style={{ marginTop: 4 }}>Likely: {hints.get(c.id)!.summary}{hints.get(c.id)!.next_steps[0] ? ` Next: ${hints.get(c.id)!.next_steps[0]}` : ""}</div>}
                </td>
                <td className="num">{c.actual_minor !== null && c.expected_minor !== null ? inr(c.actual_minor - c.expected_minor) : "—"}</td>
                <td><Chip status={c.status} /></td><td>{c.assignee ?? <span className="muted">unassigned</span>}</td>
                <td>{c.status === "open" && <Act action={assignCase} id={c.id} label="Take it" />}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <h2>Settlements</h2>
      {settings.env === "test" && <p className="muted" style={{ marginTop: 0 }}>Test mode: <Act action={simulateSettlement} label="Settle today's simulated payments" showOk /> Builds one settlement from today's simulated payments with an invented 2% fee, so you can try matching it to a bank statement.</p>}
      <div className="tablewrap">
        <table>
          <thead><tr><th>Provider</th><th>Settled on</th><th className="num">Gross</th><th className="num">Refunds</th><th className="num">Fees</th><th className="num">Net</th><th>Bank</th><th /></tr></thead>
          <tbody>
            {settlements.data.length === 0 && <tr><td colSpan={8} className="empty">No settlements imported yet.</td></tr>}
            {settlements.data.map((s) => {
              const r = s as Record<string, number | string | null>;
              return (
                <tr key={String(r.id)}>
                  <td>{String(r.provider)}</td><td>{String(r.settled_on)}</td>
                  <td className="num">{inr(Number(r.gross_minor))}</td><td className="num">{inr(Number(r.refunds_minor))}</td>
                  <td className="num">{inr(Number(r.fee_minor))}</td><td className="num">{inr(Number(r.net_minor))}</td>
                  <td><Chip status={String(r.status)} /></td>
                  <td>{settings.env === "test" && r.provider === "sim" && r.status !== "matched" && <span style={{ display: "inline-flex", gap: 6 }}><Act action={simulateBankCredit} id={String(r.id)} label="Add bank credit" showOk /><Act action={simulateBankCredit} id={String(r.id)} extra={{ short: "50" }} label="…₹50 short" showOk /></span>}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}
