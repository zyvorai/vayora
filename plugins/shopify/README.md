# Shopify ↔ Zyvor

## Flow

1. Merchant installs a custom app with Admin API access.
2. On checkout, app creates `POST /v1/payment_links` (fixed amount) with Zyvor secret key (`create-payment-link.js`).
3. Customer is redirected to Zyvor Checkout.
4. Zyvor webhook `payment.captured` → `webhook.js` marks the Shopify order paid (`external_ref=shopify-{order_id}`).

## Env

```
ZYVOR_API_URL=https://api.example.com
ZYVOR_API_KEY=zk_live_...
ZYVOR_CHECKOUT_PUBLIC_URL=https://pay.example.com
ZYVOR_WEBHOOK_SECRET=whsec_...
```

## Scaffold

- `create-payment-link.js` — server call for payment links  
- `webhook.js` — HMAC verify + `markPaid(orderId)` hook  

Publish as a Shopify app when you have Partner credentials.
