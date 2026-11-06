/**
 * Express-style handler: verify Zyvor-Signature and mark a Shopify order paid.
 * Match external_ref `shopify-{order_id}` from create-payment-link.js.
 *
 *   app.post("/webhooks/zyvor", express.raw({ type: "*/*" }), zyvorWebhook({ secret, shopifyAdmin }))
 */
import crypto from "node:crypto";

export function verifyZyvorSignature({ secret, header, rawBody, toleranceSeconds = 300, nowSeconds = Math.floor(Date.now() / 1000) }) {
  const parts = Object.fromEntries(
    String(header || "")
      .split(",")
      .map((p) => p.trim().split("="))
      .filter((kv) => kv.length === 2)
  );
  if (!parts.t || !parts.v1) return false;
  const t = Number(parts.t);
  if (!Number.isFinite(t) || Math.abs(nowSeconds - t) > toleranceSeconds) return false;
  const expected = crypto.createHmac("sha256", secret).update(`${t}.${rawBody}`).digest("hex");
  try {
    return crypto.timingSafeEqual(Buffer.from(expected, "hex"), Buffer.from(parts.v1, "hex"));
  } catch {
    return false;
  }
}

/**
 * @param {{ secret: string, markPaid: (orderId: string, payload: object) => Promise<void> }} opts
 */
export function zyvorWebhook(opts) {
  return async (req, res) => {
    const raw = Buffer.isBuffer(req.body) ? req.body.toString("utf8") : String(req.body || "");
    const header = req.get("Zyvor-Signature") || "";
    if (!verifyZyvorSignature({ secret: opts.secret, header, rawBody: raw })) {
      res.status(401).send("invalid signature");
      return;
    }
    let payload;
    try {
      payload = JSON.parse(raw);
    } catch {
      res.status(400).send("bad json");
      return;
    }
    const type = payload.type || payload.event || "";
    if (type !== "payment.captured") {
      res.status(204).end();
      return;
    }
    const data = payload.data || payload;
    const external = String(data.external_ref || "");
    if (!external.startsWith("shopify-")) {
      res.status(204).end();
      return;
    }
    const orderId = external.slice("shopify-".length);
    await opts.markPaid(orderId, payload);
    res.status(204).end();
  };
}
