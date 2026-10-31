export const LANGS = ["curl", "node", "python", "go"] as const;
export type Lang = (typeof LANGS)[number];
export const LANG_LABEL: Record<Lang, string> = { curl: "curl", node: "Node.js", python: "Python", go: "Go" };

/** A first call that works as soon as a key is in the environment: create a ₹1,250.00 payment. */
export function createPayment(lang: Lang, base: string): string {
  switch (lang) {
    case "curl":
      return `curl ${base}/v1/payment_intents \\
  -H "Authorization: Bearer $ZYVOR_API_KEY" \\
  -H "Content-Type: application/json" \\
  -H "Idempotency-Key: $(uuidgen)" \\
  -d '{"amount_minor": 125000, "currency": "INR", "external_ref": "ORDER-1001"}'`;
    case "node":
      return `import { Zyvor } from "@zyvor/node";

const zyvor = new Zyvor({ apiKey: process.env.ZYVOR_API_KEY!, baseUrl: "${base}" });
const intent = await zyvor.intents.create({ amountMinor: 125000, externalRef: "ORDER-1001" });
console.log(intent.id, intent.status);`;
    case "python":
      return `import os
from zyvor import Zyvor

z = Zyvor(os.environ["ZYVOR_API_KEY"], base_url="${base}")
intent = z.create_intent(125000, external_ref="ORDER-1001")
print(intent["id"], intent["status"])`;
    case "go":
      return `package main

import (
	"fmt"
	"os"

	zyvor "github.com/zyvorai/vayora/sdks/go"
)

func main() {
	c := zyvor.New(os.Getenv("ZYVOR_API_KEY"), "${base}")
	intent, err := c.CreateIntent(125000, "INR", "")
	if err != nil {
		panic(err)
	}
	fmt.Println(intent.ID, intent.Status)
}`;
  }
}

/** Checking that a webhook really came from Zyvor, against the raw request body. */
export const VERIFY_WEBHOOK = `import { verifyWebhook } from "@zyvor/node";

// rawBody is the exact bytes received, before any JSON parsing
const ok = verifyWebhook({
  secret: process.env.ZYVOR_WEBHOOK_SECRET!,
  header: req.headers["zyvor-signature"] as string,
  rawBody,
});
if (!ok) return res.status(400).end();
// Delivery is at least once: ignore an event id you have already handled.`;
