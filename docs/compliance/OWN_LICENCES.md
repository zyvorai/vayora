# Own licences track (RBI PA + NPCI TPAP)

**Not legal advice.** Production acquire traffic runs under a **sponsor** until these applications succeed.

## RBI Payment Aggregator

**Evidence to gather while on sponsor escrow (`zyvor-settle`):**

- Signed fund-flow diagram and counsel memo
- Escrow bank statements vs daily ledger proof (`GET /v1/escrow/proof`)
- T+0 / T+1 settlement batch history
- Merchant KYC/AML files (acquire programme)
- Incident / CERT-In drills
- Net-worth and capital plan meeting current PA directions

**Application package:** counsel-led; attach the above; migrate escrow banks to own authorisation when granted.

## NPCI TPAP / UPI

**Evidence while on sponsor PSP (`sponsor_upi` + Vayora Pay):**

- Sponsor PSP agreement and VPA namespace allocation
- Production collect / intent / QR success metrics
- Mandate (AutoPay) rules verified against NPCI circulars
- Volume caps and grievance handling

**Application package:** with sponsor bank; graduate Vayora Pay off `sim_pay` only after TPAP approval for the consumer handle.

## Status log

| Milestone | Target | Status |
|---|---|---|
| Fund-flow freeze + sponsor LOI | Phase 0 | Open — see REGULATORY.md checklist |
| Pilot acquire merchant on sponsor | After Phase 2 | Not started |
| RBI PA filing | After pilot evidence | Not started |
| NPCI TPAP filing | After UPI production metrics | Not started |
