import Link from "next/link";
import { redirect } from "next/navigation";
import { Act } from "@/components/Act";
import { Chip } from "@/components/Chip";
import { sendTestEvent } from "@/lib/actions";
import { currentSession } from "@/lib/auth";
import { LANGS, LANG_LABEL, VERIFY_WEBHOOK, createPayment, type Lang } from "@/lib/snippets";
import { api } from "@/lib/zyvor";

export default async function Developers({ searchParams }: { searchParams: Promise<{ lang?: string }> }) {
  const [me, q] = await Promise.all([currentSession(), searchParams]);
  if (me?.r !== "admin") redirect("/");
  const lang: Lang = LANGS.find((l) => l === q.lang) ?? "curl";
  const z = await api();
  const [s, keys, recent] = await Promise.all([z.settings.get(), z.keys.list(), z.webhooks.outbox()]);
  const base = (process.env.ZYVOR_PUBLIC_API_URL ?? process.env.ZYVOR_API_URL ?? "http://localhost:8080").replace(/\/$/, "");
  const activeKeys = keys.data.filter((k) => !k.revoked_at && (!k.expires_at || new Date(k.expires_at) > new Date()) && !k.name.startsWith("console session"));
  const last = recent.data.slice(0, 5) as Record<string, string | number | null>[];
  return (
    <>
      <div className="head"><div><h1>Developers</h1><p className="lede">Make your first API call, then point webhooks at your own system. You are in <strong>{s.env}</strong> mode: keys and events here never touch {s.env === "test" ? "live money" : "test data"}.</p></div></div>

      <ol className="steps">
        <li className="card">
          <h2>1. Get an API key</h2>
          <p className="muted">{activeKeys.length === 0 ? "You have no API keys yet." : `You have ${activeKeys.length} active key${activeKeys.length === 1 ? "" : "s"}.`} A key is shown once, when you create it. Keep it in an environment variable, never in code.</p>
          <Link className="btn small primary" href="/keys">{activeKeys.length === 0 ? "Create a key" : "Manage keys"}</Link>
        </li>
        <li className="card">
          <h2>2. Make your first call</h2>
          <p className="muted">Creates a payment of ₹1,250.00. Set <code>ZYVOR_API_KEY</code> to the key from step 1. The SDKs live in this repository (<code>packages/zyvor-node</code>, <code>sdks/python</code>, <code>sdks/go</code>); they are not published to a package registry.</p>
          <nav className="tabs" aria-label="Language"><div className="seg">{LANGS.map((l) => <Link key={l} href={`/developers?lang=${l}`} aria-current={l === lang}>{LANG_LABEL[l]}</Link>)}</div></nav>
          <pre className="code" tabIndex={0}>{createPayment(lang, base)}</pre>
        </li>
        <li className="card">
          <h2>3. Receive webhooks</h2>
          {s.outbound_webhook_url
            ? <p className="muted">Events go to <code style={{ overflowWrap: "anywhere" }}>{s.outbound_webhook_url}</code>. Send a test to check delivery and your signature check.</p>
            : <p className="muted">No webhook address is set. Add one under <Link href="/settings">Settings</Link>, then send a test event.</p>}
          <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
            {s.outbound_webhook_url && <Act action={sendTestEvent} label="Send test event" primary />}
            <Link className="btn small" href="/webhooks">All deliveries</Link>
          </div>
          {last.length > 0 && (
            <div className="tablewrap" style={{ marginTop: 14 }}><table>
              <thead><tr><th>Event</th><th>Status</th><th className="num">Attempts</th><th>Last error</th></tr></thead>
              <tbody>{last.map((e) => <tr key={String(e.id)}><td>{String(e.type)}</td><td><Chip status={String(e.status)} /></td><td className="num">{e.attempts}</td><td className="muted">{e.last_error ?? ""}</td></tr>)}</tbody>
            </table></div>
          )}
          <h3 style={{ margin: "18px 0 8px", fontSize: ".9375rem" }}>Check the signature</h3>
          <pre className="code" tabIndex={0}>{VERIFY_WEBHOOK}</pre>
        </li>
      </ol>
    </>
  );
}
