# Regulatory position (not legal advice)

**As built, Zyvor Payments orchestrates a merchant's own payment-provider accounts. The provider collects and settles; Zyvor never receives, holds or pays out customer funds.** That is a design choice, and it is the reason the platform can ship without being a payment aggregator.

It is **not** a conclusion. Whether a particular deployment (hosted by you for many merchants, with routing or any influence over fund flows) is a regulated activity must be decided by qualified counsel against the money-flow model of that deployment. The RBI's 2025 payment-aggregator directions cover authorisation, merchant due diligence and escrow operations, and "we integrate a licensed provider" does not by itself settle classification. Get the actual flow reviewed **before launch**.

Things this repository deliberately does not do: hold or move money, operate an escrow, store card numbers or CVV, or onboard merchants (KYC) on behalf of providers. Things you still need regardless: PCI-DSS scoping for any page that embeds provider fields, data-localisation review for where Postgres runs, DPDP Act obligations for customer data, CERT-In incident reporting, and the providers' own terms.
