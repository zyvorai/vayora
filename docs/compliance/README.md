# Compliance map (not legal advice; verify current rules)

| Area | Requirement | Where it lands |
|---|---|---|
| RBI PA | Authorisation, net-worth thresholds, escrow, T+1 settlement | `vayora-settle`, operator licensing |
| UPI | Sponsor PSP bank, NPCI onboarding, TPAP caps | `vayora-upi` (simulator until onboarded) |
| Cards | PCI-DSS, RBI tokenisation (no raw PAN storage) | `vayora-cards`, network tokens only |
| Data | India-only storage, DPDP Act 2023 | deploy config, retention policy |
| Incidents | CERT-In 6-hour reporting | runbook (todo) |
| AML/KYC | PMLA obligations | merchant onboarding (todo) |
