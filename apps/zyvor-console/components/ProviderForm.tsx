"use client";
import { useActionState, useState } from "react";
import { connectProvider } from "@/lib/actions";

const PROVIDERS = [
  { id: "razorpay", label: "Razorpay" },
  { id: "cashfree", label: "Cashfree" },
  { id: "phonepe", label: "PhonePe" },
  { id: "sponsor_upi", label: "Sponsor UPI" },
] as const;

export function ProviderForm({ env }: { env: string }) {
  const [state, run, pending] = useActionState(connectProvider, undefined);
  const [provider, setProvider] = useState<string>("razorpay");
  const razor = provider === "razorpay";
  const cashfree = provider === "cashfree";
  const phonepe = provider === "phonepe";
  const sponsor = provider === "sponsor_upi";
  return (
    <form action={run} autoComplete="off" className="card" style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end", marginBottom: 24 }}>
      <label className="field" style={{ flex: "0 1 160px" }}>Provider
        <select name="provider" value={provider} onChange={(e) => setProvider(e.target.value)}>
          {PROVIDERS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
        </select>
      </label>
      <label className="field" style={{ flex: "1 1 160px" }}>Label (optional)<input name="label" maxLength={60} placeholder={`${PROVIDERS.find((p) => p.id === provider)?.label ?? "Provider"} ${env}`} /></label>
      {(razor || cashfree) && (
        <>
          <label className="field" style={{ flex: "1 1 200px" }}>{razor ? "Key id" : "App id (client id)"}<input name="id" autoComplete="off" spellCheck={false} placeholder={razor ? `rzp_${env}_…` : env === "test" ? "TEST…" : ""} required /></label>
          <label className="field" style={{ flex: "1 1 200px" }}>{razor ? "Key secret" : "Secret key"}<input name="secret" type="password" autoComplete="new-password" required /></label>
        </>
      )}
      {phonepe && (
        <>
          <label className="field" style={{ flex: "1 1 200px" }}>Merchant id<input name="merchant_id" autoComplete="off" spellCheck={false} required /></label>
          <label className="field" style={{ flex: "1 1 200px" }}>Salt key<input name="salt_key" type="password" autoComplete="new-password" required /></label>
          <label className="field" style={{ flex: "1 1 200px" }}>Base URL (optional)<input name="base_url" placeholder="https://api-preprod.phonepe.com/…" spellCheck={false} /></label>
        </>
      )}
      {sponsor && (
        <>
          <label className="field" style={{ flex: "1 1 200px" }}>API key<input name="api_key" type="password" autoComplete="new-password" required /></label>
          <label className="field" style={{ flex: "1 1 240px" }}>Base URL<input name="base_url" placeholder="https://sponsor.example.com" required spellCheck={false} /></label>
        </>
      )}
      {razor && <label className="field" style={{ flex: "1 1 200px" }}>Webhook secret<input name="webhook_secret" type="password" autoComplete="new-password" required /></label>}
      {(phonepe || sponsor) && <label className="field" style={{ flex: "1 1 200px" }}>Webhook secret<input name="webhook_secret" type="password" autoComplete="new-password" required /></label>}
      {(cashfree || razor) && <label className="field checkbox"><input type="checkbox" name="upi_intent" /> {cashfree ? "Cashfree" : "Razorpay"} has turned on UPI intent / QR for this account</label>}
      <button className="btn primary" disabled={pending}>{pending ? "Connecting…" : "Connect"}</button>
      <p className="muted" style={{ flexBasis: "100%", margin: 0 }}>
        {razor && <>Use the {env === "test" ? <strong>test / sandbox</strong> : <strong>live</strong>} keys from Razorpay. The webhook secret is one you choose here and then enter in Razorpay&apos;s webhook settings.</>}
        {cashfree && <>Cashfree signs its webhooks with this secret key, so there is no separate webhook secret to enter. Connecting again replaces what was saved.</>}
        {phonepe && <>PhonePe PG credentials for this environment. Point Base URL at a mock or PhonePe sandbox until live credentials are verified.</>}
        {sponsor && <>Sponsor UPI / PSP HTTP contract (same shape as custom). Required for acquire-mode collection under a licensed sponsor.</>}
      </p>
      {state?.error && <p className="note bad" role="alert" style={{ flexBasis: "100%", margin: 0 }}>{state.error}</p>}
      {state?.ok && <p className="note ok" role="status" style={{ flexBasis: "100%", margin: 0 }}>{state.ok}</p>}
    </form>
  );
}
