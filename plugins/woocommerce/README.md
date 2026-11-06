# WooCommerce ↔ Zyvor

## Flow

1. Merchant installs the plugin and enters API URL + key.
2. At checkout, plugin creates a Zyvor payment link for the order total (INR paise).
3. Customer pays on Zyvor Checkout; merchant marks the order paid on `payment.captured` webhook (or polls).

## Scaffold

`zyvor-gateway.php` is a stub gateway class — complete against WooCommerce payment API when packaging a release.
