import Link from "next/link";
import { Act } from "@/components/Act";
import { SandboxForm } from "@/components/SandboxForm";
import { enableSandbox } from "@/lib/actions";
import { requireRole } from "@/lib/auth";
import { OUTCOMES } from "@/lib/sandbox";
import { api } from "@/lib/zyvor";

export default async function Sandbox() {
  const me = await requireRole("finance");
  const z = await api();
  const [s, providers] = await Promise.all([z.settings.get(), z.providers.list()]);
  const ready = providers.data.some((p) => p.provider === "sim" && p.enabled);
  return (
    <>
      <div className="head"><div><h1>Sandbox</h1><p className="lede">Try payments, refunds, reconciliation and webhooks with no real money. A test payment ends the way you choose, so you can see every state your own system has to handle.</p></div></div>
      {s.env !== "test" ? (
        <div className="card"><strong>The sandbox lives in test mode.</strong><p className="muted">Switch to Test with the control at the top, or <Link href="/">go back to the overview</Link>.</p></div>
      ) : !ready ? (
        <div className="card"><strong>The test provider is not turned on yet.</strong>
          {me.r === "admin" ? <><p className="muted">It only exists in test mode; nothing here can reach a real provider.</p><Act action={enableSandbox} label="Turn on the test provider" primary /></> : <p className="muted">Ask an admin to turn it on from this page.</p>}
        </div>
      ) : (<>
        <SandboxForm />
        <h2>How a test payment ends</h2>
        <div className="tablewrap" style={{ marginBottom: 24 }}><table>
          <thead><tr><th>The payment…</th><th>UPI handle</th><th>What you will see</th></tr></thead>
          <tbody>{OUTCOMES.map((o) => <tr key={o.key}><td>{o.label}</td><td><code>{o.handle}</code></td><td className="muted">{o.note}</td></tr>)}</tbody>
        </table></div>
        <p className="muted">The same handles work from your own code in test mode: create a payment, then start a UPI attempt with the handle (see <Link href="/developers">Developers</Link>). Any other handle succeeds. Test payments are real records in your test data, with ledger entries, so you can <Link href="/payments">refund them</Link> and reconcile them. Cards, wallets and netbanking use the same simulator through the API but have no console form yet.</p>
      </>)}
    </>
  );
}
