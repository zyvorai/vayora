import { apiKey, apiUrl } from "@/lib/vayora";
import { maskKey } from "@/lib/format";

export default function Developers() {
  const base = apiUrl();
  return (
    <>
      <div className="head"><div><h1>Developers</h1><p className="lede">Everything needed to call the API and verify webhooks.</p></div></div>
      <div className="grid">
        <section className="card" aria-labelledby="creds">
          <h2 id="creds">Credentials</h2>
          <dl className="kv">
            <dt>API base URL</dt><dd className="mono">{base}</dd>
            <dt>Secret key</dt><dd className="mono">{maskKey(apiKey())}</dd>
            <dt>Webhook secret</dt><dd className="mono">Set via <code>VAYORA_WEBHOOK_SECRET</code> on the API server</dd>
          </dl>
          <p className="muted" style={{ marginBottom: 0 }}>Keys are configured through environment variables. Rotation and multiple keys are not built yet, and the full key is never shown here.</p>
        </section>
        <section className="card" aria-labelledby="try">
          <h2 id="try">Try it</h2>
          <pre className="codeblock mono">{`curl ${base}/v1/orders \\
  -H "Authorization: Bearer $VAYORA_API_KEY" \\
  -H "Idempotency-Key: order-1001" \\
  -H "Content-Type: application/json" \\
  -d '{"amount_paise": 49900}'`}</pre>
        </section>
        <section className="card" aria-labelledby="verify">
          <h2 id="verify">Verify a webhook</h2>
          <pre className="codeblock mono">{`import { verifyWebhook } from "@vayora/node";

const ok = verifyWebhook({
  secret: process.env.VAYORA_WEBHOOK_SECRET!,
  header: req.get("Vayora-Signature")!,
  rawBody: req.body, // the raw bytes, not re-serialised JSON
});`}</pre>
        </section>
      </div>
    </>
  );
}
