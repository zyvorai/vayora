"use client";
import { useActionState } from "react";
import { refundPayment } from "@/lib/actions";

/** `idempotencyKey` is minted when the page renders, so a double-click replays instead of refunding twice. */
export function RefundForm({ paymentId, idempotencyKey, remaining }: { paymentId: string; idempotencyKey: string; remaining: string }) {
  const [state, action, pending] = useActionState(refundPayment, undefined);
  return (
    <form action={action} style={{ display: "grid", gap: 12 }}>
      <input type="hidden" name="payment_id" value={paymentId} />
      <input type="hidden" name="idempotency_key" value={idempotencyKey} />
      <div>
        <label className="f" htmlFor="amount">Amount in ₹ (leave empty to refund {remaining})</label>
        <input id="amount" name="amount" type="text" inputMode="decimal" autoComplete="off" placeholder={remaining.replace("₹", "")} />
      </div>
      {state?.error && <div className="note bad" role="alert">{state.error}</div>}
      {state?.ok && <div className="note ok" role="status">{state.ok}</div>}
      <button className="btn primary" disabled={pending}>{pending ? "Refunding…" : "Refund"}</button>
    </form>
  );
}
