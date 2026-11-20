# WooCommerce ↔ Zyvor

## Install

1. Build a zip: `scripts/package-woo-plugin.sh` (writes `dist/zyvor-woocommerce-*.zip`).
2. WordPress → Plugins → Add New → Upload Plugin → activate **Zyvor Payments**.
3. WooCommerce → Settings → Payments → Zyvor: set API URL, API key, Checkout public URL, and webhook signing secret.
4. In Zyvor Settings, set the outbound webhook URL to `https://your-shop.example/wc-api/zyvor_webhook` and the same secret.

## Flow

1. At checkout, the gateway creates a Zyvor payment link for the order total (INR paise) with `external_ref=woo-{order_id}`.
2. The customer pays on Zyvor Checkout.
3. Zyvor sends a signed `payment.captured` (or `payment.failed`) webhook; the plugin marks the order paid or failed.

## Packaging

`zyvor-gateway.php` is the installable plugin file. Release by running `scripts/package-woo-plugin.sh` and attaching the zip — there is no WordPress.org listing yet.
