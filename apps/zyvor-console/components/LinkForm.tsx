"use client";
import { useActionState } from "react";
import { createCoupon, createLink } from "@/lib/actions";

/** Make a link and show its URL right away, ready to paste into WhatsApp or a fee notice. */
export function LinkForm({ base }: { base: string }) {
  const [state, run, pending] = useActionState(createLink, undefined);
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "2 1 220px" }}>Description<input name="description" placeholder="Term 2 fee" maxLength={120} /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Amount (₹)<input name="amount" inputMode="decimal" placeholder="1250.00" required /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Max uses<input name="max_uses" inputMode="numeric" placeholder="Unlimited" /></label>
      <label className="field" style={{ flex: "1 1 120px" }}>Fee payer covers (%)<input name="fee" inputMode="decimal" placeholder="0" /></label>
      <button className="btn primary" disabled={pending}>{pending ? "Creating…" : "Create link"}</button>
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
