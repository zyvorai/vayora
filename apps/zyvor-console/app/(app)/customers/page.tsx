import { SettingsForm } from "@/components/SettingsForm";
import { updateCustomer } from "@/lib/actions";
import { api } from "@/lib/zyvor";

export const dynamic = "force-dynamic";

export default async function Customers() {
  const { data } = await (await api()).customers.list();
  return (
    <>
      <div className="head"><div><h1>Customers</h1><p className="lede">Fix a name, GSTIN or address here. Invoices already issued keep the GSTIN they were issued with; printed invoices and draft e-invoices read the address from this record.</p></div></div>
      {data.length === 0 && <div className="card"><p className="muted" style={{ margin: 0 }}>No customers yet. Add one on the Billing page.</p></div>}
      {data.map((c) => (
        <details key={c.id} className="card" style={{ marginBottom: 12 }}>
          <summary><strong>{c.name}</strong> <span className="muted">· {c.gstin ?? "no GSTIN"} · {[c.address1, c.location, c.pincode].filter(Boolean).join(", ") || "no address"}</span></summary>
          <div style={{ marginTop: 12 }}>
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
