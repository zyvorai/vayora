"use client";
import { startTransition, useActionState, useEffect, useRef, type FormEvent } from "react";
import { addVendor, bulkPayouts, markPayoutPaid, requestPayout } from "@/lib/actions";

export function RequestPayout({ vendors }: { vendors: { id: string; name: string }[] }) {
  const [state, run, pending] = useActionState(requestPayout, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 16 }}>
      <label className="field" style={{ flex: "1 1 180px" }}>Vendor
        <select name="vendor" required>{vendors.map((v) => <option key={v.id} value={v.id}>{v.name}</option>)}</select>
      </label>
      <label className="field" style={{ flex: "1 1 120px" }}>Amount (₹)<input name="amount" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "2 1 200px" }}>Purpose<input name="purpose" required maxLength={200} /></label>
      <label className="field checkbox" style={{ flex: "1 1 100%" }}><input type="checkbox" name="from_balance" /> Pay from what this vendor is owed on split payments (not your own funds)</label>
      <button className="btn primary" disabled={pending || vendors.length === 0}>{pending ? "Requesting…" : "Request payout"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ flexBasis: "100%" }}>Requested. A second person approves it before it can be paid.</p>}
    </form>
  );
}

export function AddVendor() {
  const [state, run, pending] = useActionState(addVendor, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "2 1 180px" }}>New vendor<input name="name" required maxLength={120} /></label>
      <label className="field" style={{ flex: "1 1 150px" }}>Account number<input name="account" inputMode="numeric" autoComplete="off" /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>IFSC<input name="ifsc" autoComplete="off" maxLength={11} /></label>
      <label className="field" style={{ flex: "1 1 150px" }}>or UPI id<input name="vpa" autoComplete="off" /></label>
      <button className="btn" disabled={pending}>{pending ? "Adding…" : "Add vendor"}</button>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%" }}>{state.error}</p>}
    </form>
  );
}

/** After the transfer is made from your bank: record its reference so the books and the bank statement agree. */
export function MarkPaid({ id }: { id: string }) {
  const [state, run, pending] = useActionState(markPayoutPaid, undefined);
  return (
    <form action={run} style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
      <input type="hidden" name="id" value={id} />
      <input name="bank_ref" placeholder="Bank reference (UTR)" aria-label="Bank reference" required minLength={6} maxLength={40} style={{ width: 170 }} />
      <button className="btn small primary" disabled={pending}>{pending ? "…" : "Mark paid"}</button>
      {state?.error && <span className="note bad" role="alert">{state.error}</span>}
    </form>
  );
}

export function BulkPayouts() {
  const [state, run, pending] = useActionState(bulkPayouts, undefined);
  const form = useRef<HTMLFormElement>(null);
  // submitted by hand so a refusal keeps the pasted text; it is cleared only once everything went through
  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    startTransition(() => run(data));
  };
  useEffect(() => {
    if (state?.ok) form.current?.reset();
  }, [state]);
  return (
    <form ref={form} onSubmit={submit} className="card" style={{ display: "grid", gap: 12, marginBottom: 16 }}>
      <label className="field">Many at once: one per line as <span className="mono">vendor, amount, purpose, reference</span>
        <textarea name="csv" rows={4} spellCheck={false} placeholder={"Sharma Stationers, 12500, Term 2 supplies, RUN-1\nCity Couriers, 3400, October deliveries, RUN-2"} />
      </label>
      <label className="field checkbox"><input type="checkbox" name="from_balance" /> Pay from each vendor's split-payment balance</label>
      <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
        <button className="btn" disabled={pending}>{pending ? "Creating…" : "Create all"}</button>
        <span className="muted">All or none. Repeating the same references creates nothing new.</span>
      </div>
      {state?.error && <p className="note bad" role="alert" style={{ margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note" role="status" style={{ margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
