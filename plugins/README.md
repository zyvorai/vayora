# Store plugins (Shopify / WooCommerce)

Thin plugins that create Zyvor payment intents / payment links — they do **not** move money themselves.

| Plugin | Path | Status |
|---|---|---|
| Shopify | [shopify/](shopify/) | Scaffold — draft checkout extension notes |
| WooCommerce | [woocommerce/](woocommerce/) | Scaffold — WP plugin stub |

Both call Zyvor REST (`POST /v1/payment_intents` or `POST /v1/payment_links`) with the merchant API key and redirect the payer to Zyvor Checkout (`ZYVOR_CHECKOUT_PUBLIC_URL`).
