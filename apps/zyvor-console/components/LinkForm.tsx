"use client";
import { startTransition, useActionState, useState, type FormEvent } from "react";
import { createCoupon, createLink } from "@/lib/actions";

/** Make a link and show its URL right away, ready to paste into WhatsApp or a fee notice. */
export function LinkForm({ base, counters = [], customers = [] }: { base: string; counters?: { id: string; name: string; branch: string }[]; customers?: { id: string; name: string; email: string }[] }) {
  const [state, run, pending] = useActionState(createLink, undefined);
  const [open, setOpen] = useState(false);
  // Submitted by hand so a refusal keeps what was typed (a form `action` resets its fields afterwards).
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    data.set("open", open ? "on" : "");
    startTransition(() => run(data));
  };
  return (
    <form onSubmit={submit} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "2 1 220px" }}>What is it for<input name="description" placeholder="Term 2 fee" maxLength={120} /></label>
      {open ? (
        <>
          <label className="field" style={{ flex: "1 1 120px" }}>Smallest (₹)<input name="min" inputMode="decimal" placeholder="10" /></label>
          <label className="field" style={{ flex: "1 1 120px" }}>Largest (₹)<input name="max" inputMode="decimal" placeholder="50000" /></label>
        </>
      ) : (
        <label className="field" style={{ flex: "1 1 140px" }}>Amount (₹)<input name="amount" inputMode="decimal" placeholder="1250.00" required /></label>
      )}
      <button className="btn primary" disabled={pending}>{pending ? "Creating…" : "Create link"}</button>
      <details style={{ flexBasis: "100%" }}>
        <summary style={{ cursor: "pointer", fontWeight: 500 }}>More options: limit uses, add a fee, tie it to a customer, set a last day</summary>
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginTop: 12 }}>
          {!open && (
            <>
              <label className="field" style={{ flex: "1 1 120px" }}>Stop after this many payers<input name="max_uses" inputMode="numeric" placeholder="No limit" /></label>
              <label className="field" style={{ flex: "1 1 140px" }}>Fee the payer adds on top (%)<input name="fee" inputMode="decimal" placeholder="0" /></label>
            </>
          )}
          {counters.length > 0 && (
            <label className="field" style={{ flex: "1 1 160px" }}>Counter
              <select name="counter_id" defaultValue=""><option value="">None</option>{counters.map((c) => <option key={c.id} value={c.id}>{c.branch} · {c.name}</option>)}</select>
            </label>
          )}
          {!open && customers.length > 0 && (
            <>
              <label className="field" style={{ flex: "1 1 200px" }}>For a customer
                <select name="customer_id" defaultValue=""><option value="">Anyone with the link</option>{customers.map((c) => <option key={c.id} value={c.id}>{c.name} · {c.email}</option>)}</select>
              </label>
              <label className="field" style={{ flex: "1 1 150px" }}>Last day to pay<input name="expires_on" type="date" /></label>
              <label style={{ flexBasis: "100%", display: "flex", gap: 8, alignItems: "center" }}>
                <input type="checkbox" name="remind" /> Email that customer once, in the last day before it stops working (needs a customer and a last day)
              </label>
            </>
          )}
          <label style={{ flexBasis: "100%", display: "flex", gap: 8, alignItems: "center" }}>
            <input type="checkbox" checked={open} onChange={(e) => setOpen(e.target.checked)} /> The payer types the amount (for a printed counter QR; no coupon, split or fee)
          </label>
        </div>
      </details>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Created: <a className="mono" style={{ overflowWrap: "anywhere" }} href={base + state.ok} target="_blank" rel="noreferrer">{base + state.ok}</a></p>}
    </form>
  );
}

export function CouponForm() {
  const [state, run, pending] = useActionState(createCoupon, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "1 1 140px" }}>Coupon code<input name="code" required minLength={3} maxLength={24} autoCapitalize="characters" spellCheck={false} placeholder="SAVE10" /></label>
      <label className="field" style={{ flex: "0 1 130px" }}>Takes off
        <select name="kind" defaultValue="percent"><option value="percent">percent (%)</option><option value="flat">rupees (₹)</option></select>
      </label>
      <label className="field" style={{ flex: "1 1 100px" }}>Amount<input name="value" inputMode="decimal" required placeholder="10" /></label>
      <label className="field" style={{ flex: "1 1 110px" }}>Max uses<input name="max_uses" inputMode="numeric" placeholder="Unlimited" /></label>
      <button className="btn" disabled={pending}>{pending ? "Adding…" : "Add coupon"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%", margin: 0 }}>Coupon added.</p>}
    </form>
  );
}
