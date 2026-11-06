# Acquire-mode KYC / AML (PMLA)

**Not legal advice.** In `money_mode=acquire`, the operator (or sponsor PA) must perform merchant due diligence before live collection into escrow.

## Gate in product

- Organisation fields: `acquire_kyc_status` (`not_started` | `pending` | `approved` | `rejected` | `re_kyc_due`), `acquire_kyc_approved_at`
- API: `GET/PUT /v1/acquire_kyc`
- Live acquire payments are refused until status is `approved` (`money_mode::acquire_live_allowed`)

## Programme requirements (operator checklist)

1. Named AML / KYC programme owner
2. Customer due diligence levels (simplified / regular / enhanced) mapped to merchant risk
3. PAN, GSTIN, beneficial ownership, bank account verification (reuse Cashfree Secure ID or equivalent)
4. PEP / sanctions screening hooks before `approved`
5. Periodic re-KYC (`re_kyc_due`) and trigger events (ownership change, unusual volume)
6. Retention and access controls for KYC artefacts (DPDP + PMLA)
7. Escalation to FIU-IND where applicable — confirm with counsel

## Orchestrate mode

KYC remains vendor checks + platform live-access review; Zyvor is not the regulated KYC party for the merchant's own Razorpay/Cashfree account.
