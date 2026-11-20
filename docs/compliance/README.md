# Compliance map (not legal advice; verify current rules)

| Area | Requirement | Where it lands |
|---|---|---|
| RBI PA | Authorisation, net-worth thresholds, escrow, T+1 settlement | `zyvor-settle`, sponsor escrow first, then [OWN_LICENCES.md](OWN_LICENCES.md) |
| UPI | Sponsor PSP bank, NPCI onboarding, TPAP caps | `sponsor_upi` in `zyvor-providers`; Vayora Pay off `sim_pay` only after TPAP |
| Cards | PCI-DSS, RBI tokenisation (no raw PAN storage) | `zyvor-cards`, network tokens only; [PCI_SCOPE.md](PCI_SCOPE.md) |
| Data | India-only storage, DPDP Act 2023 | deploy config, retention policy |
| Incidents | CERT-In 6-hour reporting | [CERT_IN_RUNBOOK.md](CERT_IN_RUNBOOK.md) |
| AML/KYC | PMLA obligations (acquire mode) | [KYC_AML_ACQUIRE.md](KYC_AML_ACQUIRE.md), merchant onboarding |
| Money modes | orchestrate vs acquire fund flows | [REGULATORY.md](../zyvor/REGULATORY.md), ADR 0004 |

## Fund-flow freeze checklist (acquire go-live)

- [ ] Counsel memo on the signed fund-flow diagram
- [ ] Sponsor LOI (PA / bank PSP + escrow banks)
- [ ] Named PMLA KYC/AML program owner
- [ ] Capital / net-worth plan for eventual own PA
- [ ] CERT-In reporting contacts tested
- [ ] No live `money_mode=acquire` until the above are done
