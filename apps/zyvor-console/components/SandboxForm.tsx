"use client";
import { useActionState, useState } from "react";
import { createTestPayment } from "@/lib/actions";
import { OUTCOMES } from "@/lib/sandbox";

/** One test payment that ends the way you choose. The key is made once per page, so a double click is one payment. */
export function SandboxForm() {
  const [state, run, pending] = useActionState(createTestPayment, undefined);
  const [key] = useState(() => crypto.randomUUID());
  const [outcome, setOutcome] = useState<string>(OUTCOMES[0].key);
  const chosen = OUTCOMES.find((o) => o.key === outcome)!;
  return (
    <form action={run} className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <input type="hidden" name="key" value={key} />
      <label className="field" style={{ flex: "0 1 140px" }}>Amount (₹)<input name="amount" inputMode="decimal" defaultValue="500.00" required /></label>
      <label className="field" style={{ flex: "1 1 200px" }}>The payment…
        <select name="outcome" value={outcome} onChange={(e) => setOutcome(e.target.value)}>{OUTCOMES.map((o) => <option key={o.key} value={o.key}>{o.label}</option>)}</select>
      </label>
      <label className="field" style={{ flex: "1 1 180px" }}>Your reference (optional)<input name="reference" maxLength={60} placeholder="ORDER-1001" /></label>
      <button className="btn primary" disabled={pending}>{pending ? "Creating…" : "Create test payment"}</button>
      <p className="muted" style={{ flexBasis: "100%", margin: 0 }}>Uses the handle <code>{chosen.handle}</code>. {chosen.note}</p>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
    </form>
  );
}
