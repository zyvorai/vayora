/**
 * Minimal helper: create a Zyvor payment link for a Shopify order total (paise).
 * Run from a Shopify app backend — never expose ZYVOR_API_KEY to the storefront.
 */
export async function createZyvorPaymentLink({
  apiUrl,
  apiKey,
  amountMinor,
  externalRef,
  description,
}) {
  const res = await fetch(`${apiUrl.replace(/\/$/, "")}/v1/payment_links`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      amount_minor: amountMinor,
      currency: "INR",
      external_ref: externalRef,
      description: description || "Shopify order",
    }),
  });
  if (!res.ok) {
    throw new Error(`Zyvor payment link failed: ${res.status} ${await res.text()}`);
  }
  return res.json();
}
