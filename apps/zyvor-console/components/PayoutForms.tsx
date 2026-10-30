"use client";
import { useActionState } from "react";
import { addVendor, markPayoutPaid, requestPayout } from "@/lib/actions";

export function RequestPayout({ vendors }: { vendors: { id: string; name: string }[] }) {
  const [state, run, pending] = useActionState(requestPayout, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 16 }}>
      <label className="field" style={{ flex: "1 1 180px" }}>Vendor
        <select name="vendor" required>{vendors.map((v) => <option key={v.id} value={v.id}>{v.name}</option>)}</select>
      </label>
      <label className="field" style={{ flex: "1 1 120px" }}>Amount (₹)<input name="amount" inputMode="decimal" required /></label>
      <label className="field" style={{ flex: "2 1 200px" }}>Purpose<input name="purpose" required maxLength={200} /></label>
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
