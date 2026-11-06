# Shopify ↔ Zyvor

## Flow

1. Merchant installs a custom app with Admin API access.
2. On checkout, app creates `POST /v1/payment_links` (fixed amount) with Zyvor secret key.
3. Customer is redirected to Zyvor Checkout; Zyvor webhooks update order payment status.

## Env

```
ZYVOR_API_URL=https://api.example.com
ZYVOR_API_KEY=zk_live_...
ZYVOR_CHECKOUT_PUBLIC_URL=https://pay.example.com
```

## Scaffold

`create-payment-link.js` shows the minimal Admin Action / checkout UI extension server call. Publish as a Shopify app when you have Partner credentials.
