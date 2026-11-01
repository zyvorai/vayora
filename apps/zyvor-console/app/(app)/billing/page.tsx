import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { NewCustomer, NewInstalments, NewInvoice } from "@/components/BillingForms";
import { Tabs } from "@/components/Tabs";
import { TestClock } from "@/components/TestClock";
import { completeMilestone, runBilling, voidInvoice } from "@/lib/actions";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

const TABS = ["invoices", "subscriptions", "milestones", "plans"] as const;

export default async function Billing({ searchParams }: { searchParams: Promise<{ tab?: string }> }) {
  const q = await searchParams;
  const tab = TABS.find((t) => t === q.tab) ?? "invoices";
  const z = await api();
  const testMode = (await z.settings.get()).env === "test";
  const [plans, subs, milestones, invoices, customers] = await Promise.all([z.billing.plans(), z.billing.subscriptions(), z.milestones.list(), z.invoices.list(), z.customers.list()]);
  const today = new Date().toLocaleDateString("en-CA", { timeZone: "Asia/Kolkata" });
  const unpaid = invoices.data.filter((i) => i.status === "open" && i.outstanding_minor > 0);
  const overdue = unpaid.filter((i) => i.due_date && i.due_date < today);
  const sum = (xs: { outstanding_minor: number }[]) => xs.reduce((a, i) => a + i.outstanding_minor, 0);
  const pendingMilestones = milestones.data.filter((m) => m.status === "pending").length;
  return (
    <>
      <div className="head">
        <div><h1>Billing</h1><p className="lede">Plans, subscriptions, milestones and the invoices they produce.</p></div>
        <a className="btn small" href="/export/invoices">Export invoices CSV</a>
        <Act action={runBilling} label="Run billing now" primary />
      </div>
      {testMode && <TestClock />}
      <div className="facts">
        <div className="card"><div className="k">Outstanding</div><div className="v">{inr(sum(unpaid))}</div><div className="n muted">{unpaid.length} open invoice{unpaid.length === 1 ? "" : "s"}</div></div>
        <div className="card"><div className="k">Overdue</div><div className="v" style={overdue.length ? { color: "var(--bad)" } : undefined}>{inr(sum(overdue))}</div><div className="n muted">{overdue.length ? `${overdue.length} past their due date` : "Nothing is overdue"}</div></div>
        <div className="card"><div className="k">Subscriptions</div><div className="v">{subs.data.length}</div><div className="n muted">{plans.data.length} plan{plans.data.length === 1 ? "" : "s"}</div></div>
        <div className="card"><div className="k">Milestones waiting</div><div className="v">{pendingMilestones}</div><div className="n muted">to accept and invoice</div></div>
      </div>
      <details className="card" style={{ marginBottom: 20 }}>
        <summary style={{ cursor: "pointer", fontWeight: 600 }}>New customer, invoice or instalment plan</summary>
        <div style={{ marginTop: 14 }}><NewCustomer /><NewInvoice customers={customers.data} /><NewInstalments customers={customers.data} /></div>
      </details>
      <Tabs base="/billing" current={tab} tabs={[{ key: "invoices", label: "Invoices", count: invoices.data.length }, { key: "subscriptions", label: "Subscriptions", count: subs.data.length }, { key: "milestones", label: "Milestones", count: milestones.data.length }, { key: "plans", label: "Plans", count: plans.data.length }]} />
      {tab === "invoices" && (
        <div className="tablewrap">
          <table>
            <thead><tr><th>Number</th><th>Due</th><th className="num">Amount</th><th className="num">Outstanding</th><th>Status</th><th /></tr></thead>
            <tbody>
              {invoices.data.length === 0 && <tr><td colSpan={6} className="empty">No invoices yet.</td></tr>}
              {invoices.data.map((i) => <tr key={i.id}><td>{i.number}</td><td>{i.due_date ?? "—"}{i.status === "open" && i.outstanding_minor > 0 && i.due_date && i.due_date < today && <> <span className="chip bad">overdue</span></>}</td><td className="num">{inr(i.amount_minor)}</td><td className="num">{inr(i.outstanding_minor)}</td><td><Chip status={i.status} /></td><td style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}><a className="btn small" href={`/invoice/${i.id}`} target="_blank" rel="noreferrer">Print</a><a className="btn small" href={`/einvoice/${i.id}`} title="Draft e-invoice JSON for a GST invoice to a registered buyer; not submitted">E-invoice draft</a>{i.status === "open" && i.outstanding_minor === i.amount_minor && <Act action={voidInvoice} id={i.id} label="Void" />}</td></tr>)}
            </tbody>
          </table>
        </div>
      )}
      {tab === "subscriptions" && (
        <div className="tablewrap">
          <table>
            <thead><tr><th>ID</th><th>Plan</th><th className="num">Qty</th><th>Next bill</th><th>Status</th></tr></thead>
            <tbody>
              {subs.data.length === 0 && <tr><td colSpan={5} className="empty">No subscriptions.</td></tr>}
              {subs.data.map((s) => { const r = s; const p = plans.data.find((x) => x.id === r.plan_id); return <tr key={String(r.id)}><td className="mono">{String(r.id).slice(0, 14)}…</td><td>{p ? String(p.name) : String(r.plan_id)}</td><td className="num">{r.quantity}</td><td>{String(r.next_bill_date)}</td><td><Chip status={String(r.status)} /></td></tr>; })}
            </tbody>
          </table>
        </div>
      )}
      {tab === "milestones" && (
        <div className="tablewrap">
          <table>
            <thead><tr><th>Milestone</th><th className="num">Amount</th><th>Status</th><th /></tr></thead>
            <tbody>
              {milestones.data.length === 0 && <tr><td colSpan={4} className="empty">No milestones.</td></tr>}
              {milestones.data.map((m) => { const r = m; return <tr key={String(r.id)}><td>{String(r.name)}</td><td className="num">{inr(Number(r.amount_minor))}</td><td><Chip status={String(r.status)} /></td><td>{r.status === "pending" && <Act action={completeMilestone} id={String(r.id)} label="Accept & invoice" />}</td></tr>; })}
            </tbody>
          </table>
        </div>
      )}
      {tab === "plans" && (
        <div className="tablewrap">
          <table>
            <thead><tr><th>Name</th><th className="num">Price</th><th>Every</th></tr></thead>
            <tbody>
              {plans.data.length === 0 && <tr><td colSpan={3} className="empty">No plans.</td></tr>}
              {plans.data.map((p) => { const r = p; return <tr key={String(r.id)}><td>{String(r.name)}</td><td className="num">{inr(Number(r.amount_minor))}</td><td>{r.interval_count} {String(r.interval_unit)}</td></tr>; })}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
