# UPI AutoPay: taking mandates live

Simulator mandates are built. For **live** AutoPay:

1. **Orchestrate — Cashfree:** adapter already declares `mandates` (from docs). Verify end-to-end against Cashfree’s real sandbox: create → payer approve → notice window → debit → revoke. Confirm NPCI notice/limit assumptions in [REGULATORY.md](REGULATORY.md) with Cashfree and current circulars before production.
2. **Orchestrate — Razorpay:** recurring/token adapter declares `mandates` (from public docs, local mock only). Same sandbox verification before production; several shapes marked `UNCLEAR` in `crates/zyvor-providers/src/razorpay/recurring.rs`.
3. **Acquire — sponsor UPI:** `sponsor_upi` capabilities include mandates. Point it at the sponsor PSP’s AutoPay API (same HTTP contract as custom). Do not enable live acquire AutoPay until Phase 0 fund-flow freeze and TPAP path allow it.
4. Leave `ZYVOR_MANDATE_*` env defaults until circulars are confirmed; then pin production values.

Until verification is done, keep mandate routing on the simulator in test tenants only.
