# Regulatory position (not legal advice)

**As built, Zyvor Payments orchestrates a merchant's own payment-provider accounts. The provider collects and settles; Zyvor never receives, holds or pays out customer funds.** That is a design choice, and it is the reason the platform can ship without being a payment aggregator.

It is **not** a conclusion. Whether a particular deployment (hosted by you for many merchants, with routing or any influence over fund flows) is a regulated activity must be decided by qualified counsel against the money-flow model of that deployment. The RBI's 2025 payment-aggregator directions cover authorisation, merchant due diligence and escrow operations, and "we integrate a licensed provider" does not by itself settle classification. Get the actual flow reviewed **before launch**.

Things this repository deliberately does not do: hold or move money, operate an escrow, store card numbers or CVV, or onboard merchants (KYC) on behalf of providers. Things you still need regardless: PCI-DSS scoping for any page that embeds provider fields, data-localisation review for where Postgres runs, DPDP Act obligations for customer data, CERT-In incident reporting, and the providers' own terms.

## Automated onboarding checks (simulator only)

`ZYVOR_KYC_PROVIDER=simulator` turns on identity and bank checks during merchant onboarding and lets a clean submission be approved without a person. **The simulator verifies nothing about a real business**: it answers from fixed patterns (for example a PAN ending in `X` is "not found") so the flow can be built and tested. It logs a warning at startup. Never enable it for live merchants. With it off, which is the default, nothing is checked and a second person approves every submission.

Zyvor still does not perform KYC. Before any real provider is added: choose a licensed verification vendor, agree what the contract and your own regulator require you to keep, update this document, and have a person review the auto-approve rules in `crates/zyvor-api/src/kyc.rs` (`decide`). The checks store only an outcome and a short reason; the full PAN and account number are not kept.

## Cashfree Secure ID as the verification partner

`ZYVOR_KYC_PROVIDER=cashfree` sends a merchant's PAN, name, bank account number, IFSC and GSTIN to Cashfree Secure ID to
check them (`docs/zyvor/KYC_CASHFREE.md`). Zyvor keeps only each check's outcome and a short reason. Zyvor is still not
the party that verifies identity for a regulator: it uses a vendor, and a Zyvor reviewer makes the live-mode decision.
Before turning it on for real merchants: agree the vendor terms with Cashfree, decide how consent to the check is
captured and shown to the merchant (the Onboarding page says the details are checked with a verification partner),
and set a retention rule for the check records. The simulator is for test environments only.

## UPI AutoPay mandates: assumptions to verify before any live use
Zyvor's mandate code (simulator only today) enforces rules that came from provider documents and articles, **not from NPCI's circulars**, and those sources disagree on details. Treat each as an assumption to confirm with your provider and the current NPCI and RBI circulars.

| Assumption | What the sources say | How the code treats it |
|---|---|---|
| The payer is told before each debit | Vendor documents describe a notice at least 24 hours ahead (some providers send it earlier); auto-replenishment for FASTag and RuPay NCMC is reported as exempt | `ZYVOR_MANDATE_NOTICE_HOURS`, default 24. Zyvor emits `mandate.debit_notified` and refuses a debit whose notice is younger than the window. **Zyvor does not message the payer**; the merchant or the provider must |
| An authentication step at registration | One source says registration needs extra authentication; NPCI's RuPay AutoPay circular covers RuPay, not all UPI | The provider performs it; Zyvor records the mandate only once the provider reports it active |
| A per-debit ceiling | Cashfree's documents give ₹15,000 up to ₹1,00,000 depending on authentication; other articles give ₹15,000 or ₹1,00,000 for some categories | `ZYVOR_MANDATE_MAX_MINOR`, default ₹15,000 (1,500,000 paise); a mandate above it is refused at creation |
| The payer can revoke at any time | Stated consistently | A payer's revoke is learned on refresh and stops every debit not yet sent; a debit already in flight is left to the provider |
| Frequency limits | Not confirmed by the sources | Zyvor allows at most one debit per period of the mandate's frequency and never two on one day |

Zyvor still never holds the money: the provider debits and settles. Consent records, the notice itself and any licence the merchant needs remain the merchant's and the provider's responsibility. Sources read: Cashfree Subscriptions documentation, Razorpay recurring-payments documentation, vendor explainers on UPI AutoPay.

