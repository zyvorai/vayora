import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { NewCustomer, NewInstalments, NewInvoice } from "@/components/BillingForms";
import { completeMilestone, runBilling, voidInvoice } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

type Row = Record<string, string | number | null>;

export default async function Billing() {
  const z = await api();
  const [plans, subs, milestones, invoices, customers] = await Promise.all([z.billing.plans(), z.billing.subscriptions(), z.milestones.list(), z.invoices.list(), z.customers.list()]);
  return (
    <>
      <div className="head">
        <div><h1>Billing</h1><p className="lede">Plans, subscriptions, milestones and the invoices they produce.</p></div>
        <a className="btn small" href="/export/invoices">Export invoices CSV</a>
        <Act action={runBilling} label="Run billing now" primary />
      </div>
      <h2>New</h2>
      <NewCustomer />
      <NewInvoice customers={customers.data} />
      <NewInstalments customers={customers.data} />
      <h2>Invoices</h2>
      <div className="tablewrap" style={{ marginBottom: 28 }}>
        <table>
          <thead><tr><th>Number</th><th>Due</th><th className="num">Amount</th><th className="num">Outstanding</th><th>Status</th><th /></tr></thead>
          <tbody>
            {invoices.data.length === 0 && <tr><td colSpan={6} className="empty">No invoices yet.</td></tr>}
            {invoices.data.map((i) => <tr key={i.id}><td>{i.number}</td><td>{i.due_date ?? "—"}</td><td className="num">{inr(i.amount_minor)}</td><td className="num">{inr(i.outstanding_minor)}</td><td><Chip status={i.status} /></td><td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}><a className="btn small" href={`/einvoice/${i.id}`} title="Draft e-invoice JSON for a GST invoice to a registered buyer; not submitted">E-invoice draft</a>{i.status === "open" && i.outstanding_minor === i.amount_minor && <Act action={voidInvoice} id={i.id} label="Void" />}</td></tr>)}
          </tbody>
        </table>
      </div>
      <h2>Subscriptions</h2>
      <div className="tablewrap" style={{ marginBottom: 28 }}>
        <table>
          <thead><tr><th>ID</th><th>Plan</th><th className="num">Qty</th><th>Next bill</th><th>Status</th></tr></thead>
          <tbody>
            {subs.data.length === 0 && <tr><td colSpan={5} className="empty">No subscriptions.</td></tr>}
            {subs.data.map((s) => { const r = s as Row; const p = plans.data.find((x) => (x as Row).id === r.plan_id) as Row | undefined; return <tr key={String(r.id)}><td className="mono">{String(r.id).slice(0, 14)}…</td><td>{p ? String(p.name) : String(r.plan_id)}</td><td className="num">{r.quantity}</td><td>{String(r.next_bill_date)}</td><td><Chip status={String(r.status)} /></td></tr>; })}
          </tbody>
        </table>
      </div>
      <h2>Milestones</h2>
      <div className="tablewrap" style={{ marginBottom: 28 }}>
        <table>
          <thead><tr><th>Milestone</th><th className="num">Amount</th><th>Status</th><th /></tr></thead>
          <tbody>
            {milestones.data.length === 0 && <tr><td colSpan={4} className="empty">No milestones.</td></tr>}
            {milestones.data.map((m) => { const r = m as Row; return <tr key={String(r.id)}><td>{String(r.name)}</td><td className="num">{inr(Number(r.amount_minor))}</td><td><Chip status={String(r.status)} /></td><td>{r.status === "pending" && <Act action={completeMilestone} id={String(r.id)} label="Accept & invoice" />}</td></tr>; })}
          </tbody>
        </table>
      </div>
      <h2>Plans</h2>
      <div className="tablewrap">
        <table>
          <thead><tr><th>Name</th><th className="num">Price</th><th>Every</th></tr></thead>
          <tbody>
            {plans.data.length === 0 && <tr><td colSpan={3} className="empty">No plans.</td></tr>}
            {plans.data.map((p) => { const r = p as Row; return <tr key={String(r.id)}><td>{String(r.name)}</td><td className="num">{inr(Number(r.amount_minor))}</td><td>{r.interval_count} {String(r.interval_unit)}</td></tr>; })}
          </tbody>
        </table>
      </div>
    </>
  );
}
