import Link from "next/link";
import { DailyChart } from "@/components/DailyChart";
import { Chip } from "@/components/Chip";
import { inr, istDateTime, pct } from "@/lib/format";
import { api } from "@/lib/vayora";

export default async function Overview() {
  const v = api();
  const [s, recent] = await Promise.all([v.stats(), v.payments.list({ limit: 6 })]);
  const hooks = s.webhooks;

  return (
    <>
      <div className="banner">Simulated mode: payments here use the built-in UPI simulator. No real money moves.</div>
      <div className="head">
        <div><h1>Overview</h1><p className="lede">Net volume is captured payments minus refunds.</p></div>
      </div>

      <div className="grid stats">
        <div className="card stat"><div className="k">Net volume</div><div className="v">{inr(s.net_paise)}</div><div className="n">{inr(s.gross_paise)} gross · {inr(s.refunded_paise)} refunded</div></div>
        <div className="card stat"><div className="k">Success rate</div><div className="v">{pct(s.success_rate)}</div><div className="n">{s.captured_count} captured · {s.failed_count} failed</div></div>
        <div className="card stat"><div className="k">Owed to you</div><div className="v">{inr(s.merchant_payable_paise)}</div><div className="n">Ledger balance. Settlements are not built yet.</div></div>
        <div className="card stat"><div className="k">Webhooks</div><div className="v">{hooks.dead > 0 ? `${hooks.dead} dead` : hooks.pending > 0 ? `${hooks.pending} pending` : "Healthy"}</div><div className="n">{hooks.delivered} delivered · <Link href="/webhooks">view log</Link></div></div>
      </div>

      <div className="grid two">
        <section className="card" aria-label="Daily volume"><DailyChart days={s.daily} /></section>
        <section className="card" aria-labelledby="recent">
          <h2 id="recent">Recent payments</h2>
          {recent.data.length === 0 ? <div className="empty">No payments yet.</div> : (
            <table>
              <tbody>
                {recent.data.map((p) => (
                  <tr key={p.id}>
                    <td style={{ paddingLeft: 0 }}><Link className="row" href={`/payments/${p.id}`}>{inr(p.amount_paise)}</Link><div className="muted" style={{ fontSize: ".75rem" }}>{istDateTime(p.created_at)}</div></td>
                    <td className="num" style={{ paddingRight: 0 }}><Chip status={p.status} /></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <p style={{ margin: "12px 0 0" }}><Link href="/payments">All payments</Link></p>
        </section>
      </div>
    </>
  );
}
