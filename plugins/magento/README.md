# Magento 2 — Zyvor Payments (scaffold)

Thin payment method: creates a Zyvor payment link and redirects the shopper to Zyvor Checkout. Magento never sees PAN/CVV; money moves through Zyvor (orchestrate or acquire).

**Not production-ready.** No Marketplace listing, signing keys, or Magento version matrix.

## Install (dev)

1. Copy this folder to `app/code/Zyvor/Payments/` in a Magento 2 tree (or symlink).
2. Admin → Stores → Configuration → Sales → Payment Methods → Zyvor: set API URL, API key, Checkout public URL.
3. `bin/magento module:enable Zyvor_Payments && bin/magento setup:upgrade`.

## Flow

1. Place order with Zyvor selected.
2. Module `POST`s `/v1/payment_links` with `amount_minor`, `external_ref=magento-{increment_id}`.
3. Customer redirects to `{checkout_url}/?link={id}`.
4. On `payment.captured`, mark the Magento order paid (wire a webhook endpoint or poll — stub left in `Controller/Webhook/Index.php`).

See also [WooCommerce](../woocommerce/) and [Shopify](../shopify/).
