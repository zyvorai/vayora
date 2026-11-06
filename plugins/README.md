# Store plugins (Shopify / WooCommerce / Magento)

Thin plugins that create Zyvor payment intents / payment links — they do **not** move money themselves.

| Plugin | Path | Status |
|---|---|---|
| Shopify | [shopify/](shopify/) | Scaffold — payment link helper + webhook HMAC → mark paid |
| WooCommerce | [woocommerce/](woocommerce/) | Scaffold — payment link + `/wc-api/zyvor_webhook` mark paid |
| Magento 2 | [magento/](magento/) | Scaffold — `Zyvor_Payments` payment link redirect + `/zyvor/webhook` |

All call Zyvor REST (`POST /v1/payment_intents` or `POST /v1/payment_links`) with the merchant API key and redirect the payer to Zyvor Checkout (`ZYVOR_CHECKOUT_PUBLIC_URL`). Webhooks verify `Zyvor-Signature` and match `external_ref` (`shopify-` / `woo-` / `magento-`).
