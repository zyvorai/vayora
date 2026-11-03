import { NewCustomer } from "@/components/BillingForms";
import { SettingsForm } from "@/components/SettingsForm";
import { updateCustomer } from "@/lib/actions";
import { Chip } from "@/components/Chip";
import { inr } from "@/lib/format";
import { api } from "@/lib/zyvor";

export const dynamic = "force-dynamic";

export default async function Customers() {
  const z = await api();
  const [{ data }, subs, plans] = await Promise.all([z.customers.list(), z.billing.subscriptions(), z.billing.plans()]);
  return (
    <>
      <div className="head"><div><h1>Customers</h1><p className="lede">Everyone you bill or collect from. Open one to fix a name, email, GSTIN or address. Invoices already issued keep the GSTIN they were issued with; printed invoices and draft e-invoices read the address from this record.</p></div></div>
      <details className="card" style={{ marginBottom: 16 }}>
        <summary style={{ cursor: "pointer", fontWeight: 600 }}>Add a customer</summary>
        <div style={{ marginTop: 14 }}><NewCustomer /></div>
      </details>
      {data.length === 0 && <div className="card"><p className="muted" style={{ margin: 0 }}>No customers yet. Add one above, then bill them under Billing or send them a payment link.</p></div>}
      {data.map((c) => (
        <details key={c.id} className="card" style={{ marginBottom: 12 }}>
          <summary><strong>{c.name}</strong> <span className="muted">· {[c.email, c.phone, c.gstin, [c.address1, c.location, c.pincode].filter(Boolean).join(", ")].filter(Boolean).join(" · ") || "no contact details yet"}</span></summary>
          <div style={{ marginTop: 12 }}>
            {subs.data.filter((s) => s.customer_id === c.id).map((s) => { const p = plans.data.find((x) => x.id === s.plan_id); return (
              <p key={String(s.id)} style={{ margin: "0 0 10px" }}>
                <strong>{p ? String(p.name) : String(s.plan_id)}</strong> × {s.quantity} <Chip status={String(s.status)} /> <span className="muted">next bill {String(s.next_bill_date)}{s.unit_amount_minor != null && `, kept at ${inr(Number(s.unit_amount_minor))}${p && Number(p.amount_minor) !== Number(s.unit_amount_minor) ? ` (plan is now ${inr(Number(p.amount_minor))})` : ""}`}</span>
              </p>); })}
            <SettingsForm title="Edit customer" hint="Empty a box to clear it." action={updateCustomer}>
              <input type="hidden" name="id" value={c.id} />
              <label className="field">Name<input name="name" defaultValue={c.name} required maxLength={120} /></label>
              <label className="field">Email<input name="email" type="email" defaultValue={c.email ?? ""} /></label>
              <label className="field">Phone<input name="phone" defaultValue={c.phone ?? ""} /></label>
              <label className="field">GSTIN<input name="gstin" defaultValue={c.gstin ?? ""} maxLength={15} autoCapitalize="characters" spellCheck={false} /></label>
              <label className="field">Address<input name="address1" defaultValue={c.address1 ?? ""} maxLength={100} /></label>
              <label className="field">City<input name="location" defaultValue={c.location ?? ""} maxLength={50} /></label>
              <label className="field">Pincode<input name="pincode" defaultValue={c.pincode ?? ""} inputMode="numeric" maxLength={6} /></label>
            </SettingsForm>
          </div>
        </details>
      ))}
    </>
  );
}
