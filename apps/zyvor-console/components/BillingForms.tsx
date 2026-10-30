"use client";
import { startTransition, useActionState, useState, type FormEvent, type ReactNode } from "react";
import { createCustomer, createInstalments, createInvoice, type FormState } from "@/lib/actions";

type Customer = { id: string; name: string };

/** A compact inline form that keeps what was typed when the API refuses it. */
function Inline({ action, children, done }: { action: (s: FormState, f: FormData) => Promise<FormState>; children: ReactNode; done: string }) {
  const [state, run, pending] = useActionState(action, undefined);
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 16 }}>
      {children}
      <button className="btn primary" disabled={pending}>{pending ? "Saving…" : "Create"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%", margin: 0 }}>{done}</p>}
    </form>
  );
}

const customerSelect = (customers: Customer[], required = false) => (
  <label className="field" style={{ flex: "1 1 170px" }}>Customer
    <select name="customer" required={required} defaultValue="">{!required && <option value="">None</option>}{required && <option value="" disabled>Choose…</option>}{customers.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}</select>
  </label>
);

export function NewCustomer() {
  return (
    <Inline action={createCustomer} done="Customer added.">
      <label className="field" style={{ flex: "2 1 180px" }}>New customer<input name="name" required maxLength={120} /></label>
      <label className="field" style={{ flex: "1 1 160px" }}>Email<input name="email" type="email" /></label>
      <label className="field" style={{ flex: "1 1 130px" }}>Phone<input name="phone" /></label>
      <label className="field" style={{ flex: "1 1 150px" }}>GSTIN (optional)<input name="gstin" maxLength={15} autoCapitalize="characters" spellCheck={false} /></label>
    </Inline>
  );
}

export function NewInvoice({ customers }: { customers: Customer[] }) {
  const [gst, setGst] = useState(false);
  return (
    <Inline action={createInvoice} done="Invoice created.">
      <label className="field" style={{ flex: "1 1 120px" }}>Number<input name="number" required maxLength={40} /></label>
      {customerSelect(customers)}
      <label className="field" style={{ flex: "1 1 120px" }}>{gst ? "Price before tax (₹)" : "Amount (₹)"}<input name="amount" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "1 1 130px" }}>Due<input name="due" type="date" /></label>
      <label className="field checkbox" style={{ flex: "1 1 140px" }}><input type="checkbox" checked={gst} onChange={(e) => setGst(e.target.checked)} /> GST tax invoice</label>
      {gst && (
        <>
          <label className="field" style={{ flex: "1 1 110px" }}>HSN/SAC<input name="hsn" required inputMode="numeric" minLength={4} maxLength={8} /></label>
          <label className="field" style={{ flex: "1 1 100px" }}>GST rate
            <select name="rate" defaultValue="1800">{[0, 5, 12, 18, 28].map((r) => <option key={r} value={r * 100}>{r}%</option>)}</select>
          </label>
          <label className="field" style={{ flex: "1 1 130px" }}>Place of supply<input name="pos" placeholder="27" maxLength={2} inputMode="numeric" /></label>
          <label className="field" style={{ flex: "2 1 180px" }}>Description<input name="description" maxLength={120} /></label>
        </>
      )}
    </Inline>
  );
}

export function NewInstalments({ customers }: { customers: Customer[] }) {
  return (
    <Inline action={createInstalments} done="Instalment plan created. Its invoices are in the list above.">
      {customerSelect(customers, true)}
      <label className="field" style={{ flex: "2 1 170px" }}>Plan name<input name="name" required placeholder="Annual fee 2026-27" /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Invoice prefix<input name="prefix" required placeholder="FEE-26" maxLength={30} /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Total (₹)<input name="total" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "0 1 90px" }}>Instalments<input name="count" type="number" min={2} max={60} defaultValue={3} required /></label>
      <label className="field" style={{ flex: "1 1 140px" }}>First due<input name="first_due" type="date" required /></label>
    </Inline>
  );
}
