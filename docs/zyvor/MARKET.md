# Where Zyvor stands against Indian payment gateways

Source for the gateway side: GoKwik's published comparison of eleven Indian gateways (PayU, Easebuzz, Razorpay,
Cashfree, CCAvenue, Paytm, PhonePe, Instamojo, Stripe India, BillDesk, NTT DATA). It is a vendor's marketing
article, so treat its claims as unverified and check pricing with each provider. Zyvor is **not** a gateway or an
acquirer: it sits above them (orchestration, ledger, reconciliation, billing, collections). That decides which
features make sense to build and which can only come from the underlying provider.

## What gateways compete on, and Zyvor's answer

| Gateway feature | Zyvor |
|---|---|
| Smart routing / success-rate optimisation (PayU, Easebuzz, Razorpay, CCAvenue) | **Built.** Capability-matrix router with recorded reasons, cost-based choice, and routing suggestions from measured effective cost. Using several gateways at once is the point of Zyvor: no single point of failure, no lock-in. |
| Payment links (Razorpay, Instamojo) | **Built.** Fixed amount or an invoice's balance, caps, expiry, per-payer checkout, console page. |
| QR codes (Razorpay, Paytm) | **Built** for payment links (`GET /v1/payment_links/{id}/qr.svg`). Not a UPI-QR-for-in-store product. |
| Split payments for marketplaces (Easebuzz Slices, Stripe Connect) | **Built.** Shares per vendor on a payment or link; accrue on capture, reverse in proportion on refunds, tracked as a vendor balance in the ledger, payable from that balance. Zyvor tracks who is owed what; it does not move money to vendors. Dispute losses are not yet apportioned to vendors. |
| Bulk payouts / disbursals (PayU salaries, Cashfree bulk payouts) | **Built as control and bookkeeping**: `POST /v1/payouts/bulk` (200 at once, all or none, safe to repeat), maker-checker approval, bank reference, ledger. **Not** an actual transfer: there is no payout-rail adapter, you pay from your bank and record the reference. |
| Instant / fast settlement (Cashfree) | Provider feature; Zyvor reconciles whatever settles and turns every difference into an assigned task. |
| Automated reconciliation (Cashfree) | **Built**, and deeper: settlement and bank-statement matching, case queue, CSV import. |
| Instant refunds, refund APIs | **Built**, with reservation-based over-refund protection and second-person approval above a threshold. |
| Invoicing, subscriptions, recurring billing (Stripe, Razorpay, Easebuzz EasyCollect) | **Built**: plans, subscriptions, usage, proration, milestones, instalment plans, credit notes, GST tax invoices. Recurring *charging* via UPI AutoPay or e-mandate needs provider mandate support and is not built. |
| Webhooks, SDKs, APIs (Stripe) | **Built**: signed at-least-once webhooks with an event catalogue and subscriptions, Node/Python/Go SDKs, OpenAPI. |
| Fraud / risk engine (CCAvenue F.R.I.S.K.) | **Partial.** Rate limits per key and address, plus merchant-set risk limits (payment ceiling, tries per payment, failed tries per customer per hour, blocked UPI ids and handles), each stop audited. No scoring or network intelligence: card-network fraud detection belongs to the acquirer. |
| Multi-currency, 27+ currencies (CCAvenue, Stripe) | **Partial.** INR and USD only, with reporting rates you enter; no revaluation. International acceptance depends on the provider. |
| Multilingual checkout (CCAvenue, 18 languages) | **Not built** (English only). |
| BNPL, EMI, wallets, cards, netbanking breadth | Depends on what each connected provider supports; the router already filters by a provider's declared capabilities. Zyvor adds no methods of its own. |
| COD handling, RTO reduction, COD-to-prepaid (GoKwik's own focus) | **Not built**; e-commerce checkout concern, not a payments-orchestration one. |
| Coupons / convenience-fee pass-through (Easebuzz, Instamojo) | **Built** for fixed-amount payment links (percent or flat coupons with caps and expiry; a payer-covered fee). Not for invoice links, which a credit note handles instead. |
| Saved cards / tokenisation, one-click checkout (PayU, PhonePe) | **Not built, deliberately.** Card data stays with the provider so Zyvor stays out of PCI scope. |
| Plugins for Shopify, WooCommerce, Magento (Razorpay, Paytm) | **Not built**; the API and payment links cover simple storefronts. |
| Onboarding speed, KYC, transparent pricing | Not applicable until Zyvor is a licensed operator (see REGULATORY.md). |

## Where Zyvor is genuinely different
Gateways sell you one provider's rails. Zyvor keeps the books above all of them: a double-entry ledger that
balances per currency, exactly-once money movement, tamper-evident audit, tenant isolation enforced in the
database, and reconciliation that explains every rupee. Those are what finance teams, schools, clinics and
marketplaces lack when they stitch two gateways together.

## What this means for the roadmap
Next candidates that fit the layer: coupons and convenience fees on links, checkout
translations, and UPI AutoPay/e-mandate once a real provider sandbox is connected. Anything that moves money
for real (payout rails, instant settlement, mandates) needs live provider credentials and the regulatory
standing described in REGULATORY.md; none of that can be verified against simulators alone.
