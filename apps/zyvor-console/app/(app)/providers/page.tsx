import { Chip } from "@/components/Chip";
import { ProviderControls } from "@/components/ProviderControls";
import { ProviderForm } from "@/components/ProviderForm";
import { requireRole } from "@/lib/auth";
import { api } from "@/lib/zyvor";

const LABEL: Record<string, string> = { razorpay: "Razorpay", cashfree: "Cashfree", sim: "Simulator", custom: "Your own gateway" };

export default async function Providers() {
  await requireRole("admin");
  const z = await api();
  const [s, accounts] = await Promise.all([z.settings.get(), z.providers.list()]);
  const publicBase = (process.env.ZYVOR_PUBLIC_API_URL ?? process.env.API_PUBLIC_URL ?? "").replace(/\/$/, "");
  const onLocalhost = !publicBase || /localhost|127\.0\.0\.1|\.svc|provider-mock/.test(publicBase);
  return (
    <>
      <div className="head"><div><h1>Providers</h1><p className="lede">Where payments are really taken. In {s.env} mode use the provider&apos;s own {s.env === "test" ? "test (sandbox)" : "live"} keys: Zyvor refuses a live Razorpay key here in test, and the other way round.</p></div></div>
      <ProviderForm env={s.env} />
      <div className="tablewrap" style={{ marginBottom: 24 }}><table>
        <thead><tr><th>Provider</th><th>Methods</th><th>Status</th><th>Address</th><th /></tr></thead>
        <tbody>
          {accounts.data.length === 0 && <tr><td colSpan={5} className="empty">No provider connected yet.</td></tr>}
          {accounts.data.map((a) => (
            <tr key={a.id}>
              <td>{LABEL[a.provider] ?? a.provider}</td>
              <td className="muted">{(a.capabilities?.methods ?? []).join(", ") || "—"}</td>
              <td><Chip status={a.enabled ? "active" : "paused"} /></td>
              <td className="mono muted" style={{ overflowWrap: "anywhere" }}>{a.base_url ?? "provider default"}</td>
              <td>{a.provider === "sim" ? <span className="muted">built in</span> : <ProviderControls provider={a.provider} enabled={a.enabled} />}</td>
            </tr>
          ))}
        </tbody>
      </table></div>
      <h2>Tell the provider where to send events</h2>
      {accounts.data.filter((a) => a.provider !== "sim").length === 0 ? <p className="muted">Connect a provider first, then its webhook address appears here.</p> : (
        <div className="card" style={{ display: "grid", gap: 10 }}>
          {onLocalhost && <p className="note bad" role="alert" style={{ margin: 0 }}>The providers cannot reach <code>localhost</code> or an internal address. Set <code>ZYVOR_PUBLIC_API_URL</code> on the console to the public address of the API, or use a tunnel while testing, then paste the address below into the provider&apos;s dashboard.</p>}
          {accounts.data.filter((a) => a.provider !== "sim").map((a) => (
            <div key={a.id}><strong>{LABEL[a.provider] ?? a.provider}</strong><div className="mono" style={{ overflowWrap: "anywhere" }}>{(publicBase || "https://<your-api-address>") + a.webhook_url}</div></div>
          ))}
          <p className="muted" style={{ margin: 0 }}>Enter the same webhook secret you saved above. Without webhooks a payment can still be found by polling, but refunds and settlement updates arrive late.</p>
        </div>
      )}
    </>
  );
}
