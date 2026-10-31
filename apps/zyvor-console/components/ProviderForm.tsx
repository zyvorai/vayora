"use client";
import { useActionState, useState } from "react";
import { connectProvider } from "@/lib/actions";

export function ProviderForm({ env }: { env: string }) {
  const [state, run, pending] = useActionState(connectProvider, undefined);
  const [provider, setProvider] = useState("razorpay");
  const razor = provider === "razorpay";
  return (
    <form action={run} autoComplete="off" className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "0 1 150px" }}>Provider
        <select name="provider" value={provider} onChange={(e) => setProvider(e.target.value)}><option value="razorpay">Razorpay</option><option value="cashfree">Cashfree</option></select>
      </label>
      <label className="field" style={{ flex: "1 1 200px" }}>{razor ? "Key id" : "App id (client id)"}<input name="id" autoComplete="off" spellCheck={false} placeholder={razor ? `rzp_${env}_…` : env === "test" ? "TEST…" : ""} required /></label>
      <label className="field" style={{ flex: "1 1 200px" }}>{razor ? "Key secret" : "Secret key"}<input name="secret" type="password" autoComplete="new-password" required /></label>
      {razor && <label className="field" style={{ flex: "1 1 200px" }}>Webhook secret<input name="webhook_secret" type="password" autoComplete="new-password" required /></label>}
      {!razor && <label className="field checkbox"><input type="checkbox" name="upi_intent" /> Cashfree has turned on UPI intent / QR for this account</label>}
      <button className="btn primary" disabled={pending}>{pending ? "Connecting…" : "Connect"}</button>
      <p className="muted" style={{ flexBasis: "100%", margin: 0 }}>Use the {env === "test" ? <strong>test / sandbox</strong> : <strong>live</strong>} keys from the provider&apos;s dashboard. {razor ? "The webhook secret is one you choose here and then enter in Razorpay's webhook settings." : "Cashfree signs its webhooks with this secret key, so there is no separate webhook secret to enter. Connecting again replaces what was saved."}</p>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note ok" role="status" style={{ flexBasis: "100%", margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
