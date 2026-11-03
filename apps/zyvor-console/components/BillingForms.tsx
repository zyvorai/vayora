"use client";
import { inr } from "@/lib/format";
import { startTransition, useActionState, useState, type FormEvent, type ReactNode } from "react";
import { createCustomer, createInstalments, createInvoice, changeSubscriptionPlan, changeTrial, createMilestone, createPlan, createSubscription, recordUsage, updatePlan, type FormState } from "@/lib/actions";

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
      <label className="field" style={{ flex: "2 1 200px" }}>Address (for e-invoices)<input name="address1" maxLength={100} /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>City<input name="location" maxLength={50} /></label>
      <label className="field" style={{ flex: "0 1 100px" }}>Pincode<input name="pincode" inputMode="numeric" maxLength={6} /></label>
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

export function NewPlan() {
  return (
    <Inline action={createPlan} done="Plan created. Subscribe a customer to it on the Subscriptions tab.">
      <label className="field" style={{ flex: "2 1 180px" }}>Plan name<input name="name" required maxLength={80} placeholder="Monthly tuition" /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Price (₹)<input name="price" inputMode="decimal" required placeholder="4500.00" /></label>
      <label className="field" style={{ flex: "0 1 90px" }}>Every<input name="every" type="number" min={1} max={365} defaultValue={1} required /></label>
      <label className="field" style={{ flex: "0 1 120px" }}>Period
        <select name="unit" defaultValue="month"><option value="day">day(s)</option><option value="week">week(s)</option><option value="month">month(s)</option><option value="year">year(s)</option></select>
      </label>
      <label className="field" style={{ flex: "1 1 150px" }}>Per unit used (₹, optional)<input name="per_unit" inputMode="decimal" placeholder="2.50" /></label>
      <label className="field" style={{ flex: "0 1 130px" }}>Free units per period<input name="included" inputMode="numeric" placeholder="0" /></label>
      <label style={{ flexBasis: "100%", display: "flex", gap: 8, alignItems: "flex-start" }}><input type="checkbox" name="per_seat" style={{ marginTop: 3 }} /><span>Free units are per seat: the allowance is multiplied by the subscription's quantity.</span></label>
      <label className="field" style={{ flex: "0 1 130px" }}>Free trial (days)<input name="trial" type="number" min={0} max={365} placeholder="0" /></label>
    </Inline>
  );
}

export function NewSubscription({ customers, plans }: { customers: Customer[]; plans: { id: string; name: string }[] }) {
  if (plans.length === 0) return <p className="muted">Create a plan first (Plans tab), then subscribe a customer to it.</p>;
  return (
    <Inline action={createSubscription} done="Subscribed. Invoices are made automatically as each bill date arrives; the first appears once the next billing run picks it up.">
      {customerSelect(customers, true)}
      <label className="field" style={{ flex: "1 1 170px" }}>Plan
        <select name="plan" required defaultValue=""><option value="" disabled>Choose…</option>{plans.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
      </label>
      <label className="field" style={{ flex: "0 1 90px" }}>Quantity<input name="quantity" type="number" min={1} defaultValue={1} required /></label>
      <label className="field" style={{ flex: "1 1 140px" }}>Starts (optional)<input name="start" type="date" /></label>
    </Inline>
  );
}

export function NewMilestone({ customers }: { customers: Customer[] }) {
  return (
    <Inline action={createMilestone} done="Milestone added. Accept it when it is reached and it becomes an invoice.">
      {customerSelect(customers, true)}
      <label className="field" style={{ flex: "2 1 170px" }}>Milestone<input name="name" required maxLength={80} placeholder="Admission deposit" /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Amount (₹)<input name="amount" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "0 1 130px" }}>Due after (days)<input name="due_days" type="number" min={0} placeholder="e.g. 7" /></label>
    </Inline>
  );
}

/** Move a subscription to another plan. An upgrade is charged now for the unused part of the period; a downgrade applies from the next invoice. */
export function RecordUsage({ id, unit, waiting }: { id: string; unit: string; waiting: number }) {
  const [state, run, pending] = useActionState(recordUsage, undefined);
  // one reference per form load, so pressing Save twice records once
  const [ref] = useState(() => crypto.randomUUID());
  return (
    <details>
      <summary style={{ cursor: "pointer" }}>Record usage{waiting > 0 && <span className="muted"> · {waiting.toLocaleString("en-IN")} unit{waiting === 1 ? "" : "s"} so far</span>}</summary>
      <form action={run} style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", marginTop: 8 }}>
        <input type="hidden" name="id" value={id} />
        <input type="hidden" name="ref" value={ref} />
        <input name="quantity" type="number" min={1} required placeholder="Units" aria-label="Units used" style={{ width: 90 }} />
        <input name="note" maxLength={120} placeholder="Note (optional)" aria-label="Note" style={{ flex: "1 1 120px" }} />
        <button className="btn small" disabled={pending}>{pending ? "…" : "Record"}</button>
        <span className="muted" style={{ flexBasis: "100%" }}>Billed at {unit} per unit on the next invoice.</span>
        {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        {state?.ok && <span className="note" role="status">{state.ok}</span>}
      </form>
    </details>
  );
}

export function EditPlan({ id, name, priceMinor, trialDays = 0, includedUnits = 0, perSeat = false, metered = false }: { id: string; name: string; priceMinor: number; trialDays?: number; includedUnits?: number; perSeat?: boolean; metered?: boolean }) {
  const [state, run, pending] = useActionState(updatePlan, undefined);
  return (
    <details>
      <summary style={{ cursor: "pointer" }}>Edit</summary>
      <form action={run} style={{ display: "flex", gap: 8, alignItems: "flex-end", flexWrap: "wrap", marginTop: 8 }}>
        <input type="hidden" name="id" value={id} />
        <label className="field" style={{ flex: "2 1 160px" }}>Name<input name="name" required maxLength={80} defaultValue={name} /></label>
        <label className="field" style={{ flex: "1 1 110px" }}>Price (₹)<input name="price" inputMode="decimal" required defaultValue={(priceMinor / 100).toFixed(2)} /></label>
        <label className="field" style={{ flex: "0 1 120px" }}>Free trial (days)<input name="trial" type="number" min={0} max={365} defaultValue={trialDays} /></label>
        {metered ? <label className="field" style={{ flex: "0 1 140px" }}>Free units per period<input name="included" inputMode="numeric" defaultValue={includedUnits} /></label> : <input type="hidden" name="included" value="0" />}
        {metered && <label style={{ flexBasis: "100%", display: "flex", gap: 8, alignItems: "flex-start" }}><input type="checkbox" name="per_seat" defaultChecked={perSeat} style={{ marginTop: 3 }} /><span>Free units are per seat (multiplied by each subscription's quantity).</span></label>}
        <button className="btn small primary" disabled={pending}>{pending ? "…" : "Save"}</button>
        <label style={{ flexBasis: "100%", display: "flex", gap: 8, alignItems: "flex-start" }}>
          <input type="checkbox" name="keep" style={{ marginTop: 3 }} />
          <span>Keep current subscribers on the old price ({inr(priceMinor)}). Only new subscribers pay the new price. Leave this off and everyone pays the new price from their next invoice.</span>
        </label>
        <span className="muted" style={{ flexBasis: "100%" }}>Invoices already raised keep their amount either way. A new trial length applies to subscriptions made from now on.</span>
        {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        {state?.ok && <span className="note" role="status">{state.ok}</span>}
      </form>
    </details>
  );
}

export function ChangeTrial({ id, endsOn, start = false }: { id: string; endsOn: string; start?: boolean }) {
  const [state, run, pending] = useActionState(changeTrial, undefined);
  return (
    <details>
      <summary style={{ cursor: "pointer" }}>{start ? "Start a free trial" : "Change trial"}</summary>
      <form action={run} style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", marginTop: 8 }}>
        <input type="hidden" name="id" value={id} />
        <input name="ends_on" type="date" defaultValue={endsOn} aria-label="Trial ends on" />
        <button className="btn small" disabled={pending}>{pending ? "…" : start ? "Start trial" : "Move end"}</button>
        {!start && <button className="btn small" name="now" value="1" formNoValidate disabled={pending}>Start billing now</button>}
        <span className="muted" style={{ flexBasis: "100%" }}>{start ? "Nothing has been billed for this subscription yet. Its first bill moves to the day the trial ends." : "The first bill falls on the day the trial ends. Starting now makes it due at the next billing run."}</span>
        {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        {state?.ok && <span className="note" role="status">{state.ok}</span>}
      </form>
    </details>
  );
}

export function ChangePlan({ id, plans, planId, quantity }: { id: string; plans: { id: string; name: string }[]; planId: string; quantity: number }) {
  const [state, run, pending] = useActionState(changeSubscriptionPlan, undefined);
  return (
    <details>
      <summary style={{ cursor: "pointer" }}>Change plan or quantity</summary>
      <form action={run} style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", marginTop: 8 }}>
        <input type="hidden" name="id" value={id} />
        <select name="plan" required defaultValue={planId} aria-label="Plan">{plans.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
        <input name="quantity" type="number" min={1} defaultValue={quantity} required aria-label="Quantity" style={{ width: 80 }} />
        <button className="btn small" disabled={pending}>{pending ? "…" : "Save"}</button>
        <span className="muted" style={{ flexBasis: "100%" }}>More value (a dearer plan or more seats) is charged now for the rest of the period; less applies from the next invoice.</span>
        {state?.error && <span className="note bad" role="alert">{state.error}</span>}
        {state?.ok && <span className="note" role="status">{state.ok}</span>}
      </form>
    </details>
  );
}
