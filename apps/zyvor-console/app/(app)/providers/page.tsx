import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { SettingsForm } from "@/components/SettingsForm";
import { connectProvider, toggleProvider } from "@/lib/actions";
import { api } from "@/lib/zyvor";

export const dynamic = "force-dynamic";

// Where each provider's callbacks must reach us. Set API_PUBLIC_URL to the public address of the API's
// /v1/webhooks path; without it only the path is shown.
const hookBase = () => (process.env.API_PUBLIC_URL ?? "").replace(/\/$/, "");

export default async function Providers() {
  const { data } = await api().providers.list();
  return (
    <>
      <div className="head"><div><h1>Payment providers</h1><p className="lede">Connect your own Cashfree or Razorpay account. Keys you type here are stored encrypted and are never shown again. With no address override, a test environment talks to the provider&apos;s sandbox and a live one to production.</p></div></div>
      <div className="tablewrap" style={{ marginBottom: 24 }}>
        <table>
          <thead><tr><th>Provider</th><th>Connected to</th><th>Status</th><th>Webhook address</th><th /></tr></thead>
          <tbody>
            {data.length === 0 && <tr><td colSpan={5} className="empty">Nothing connected yet.</td></tr>}
            {data.map((p) => (
              <tr key={p.id}>
                <td><strong>{p.provider}</strong>{p.label ? ` · ${p.label}` : ""}</td>
                <td>{p.base_url ? <span title={p.base_url}>Custom address: {p.base_url}</span> : "Provider's own address"}{p.base_url?.includes("provider-mock") ? " (simulator)" : ""}</td>
                <td><Chip status={p.enabled ? "active" : "paused"} /></td>
                <td><code>{hookBase()}{p.webhook_url}</code></td>
                <td><Act action={toggleProvider} label={p.enabled ? "Turn off" : "Turn on"} extra={{ provider: p.provider, enabled: p.enabled ? "0" : "1" }} /></td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <SettingsForm title="Connect or replace an account" hint="Replaces that provider's current account (including the simulator) and turns it on. Paste the webhook address above into the provider's dashboard, with the same webhook secret." action={connectProvider}>
        <label className="field">Provider<select name="provider" defaultValue="cashfree"><option value="cashfree">Cashfree</option><option value="razorpay">Razorpay</option></select></label>
        <label className="field">Label<input name="label" placeholder="Cashfree test" maxLength={60} /></label>
        <label className="field">Client ID / Key ID<input name="key_id" autoComplete="off" spellCheck={false} required /></label>
        <label className="field">Client secret / Key secret<input name="key_secret" type="password" autoComplete="new-password" required /></label>
        <label className="field">Webhook secret<input name="webhook_secret" type="password" autoComplete="new-password" required /></label>
        <label className="field" style={{ display: "flex", gap: 8, alignItems: "center" }}><input name="upi_intent" type="checkbox" />Cashfree has enabled UPI intent / QR on this account</label>
      </SettingsForm>
    </>
  );
}
