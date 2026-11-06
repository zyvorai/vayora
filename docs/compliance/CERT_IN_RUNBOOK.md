# CERT-In incident reporting runbook

**Not legal advice.** Indian organisations must report certain cybersecurity incidents to CERT-In within **six hours** of noticing them. Confirm current directions before relying on this checklist.

## When to open an incident

Treat as reportable until counsel says otherwise:

- Unauthorised access to production systems or merchant/payer personal data
- Ransomware, destructive malware, or confirmed data exfiltration
- Compromise of API keys, master encryption keys, or escrow/bank credentials
- Targeted DoS that takes payment acceptance offline for a material period

## Six-hour clock

1. **T+0** — On-call acknowledges and pages compliance owner.
2. **T+30m** — Contain: revoke keys, isolate hosts, freeze suspect tenants if needed.
3. **T+2h** — Draft facts: what, when noticed, systems affected, data classes, containment so far.
4. **Before T+6h** — Submit to CERT-In via the current portal / email channel your counsel confirms. Keep the submission id.
5. **T+24h / ongoing** — Follow-up with root cause, customer notices (DPDP), provider/sponsor notice.

## Contacts (fill for your deployment)

| Role | Name | Phone | Email |
|---|---|---|---|
| On-call engineering | | | |
| Compliance / DPO | | | |
| Counsel | | | |
| Sponsor bank / PA security | | | |

## Evidence to preserve

- Audit chain (`audit_events` / seals), outbox, provider webhook raw bodies
- Access logs, deploy versions, key rotation timestamps
- Do **not** copy PAN/CVV (Zyvor must never have them); redact secrets in tickets

## Related

- [REGULATORY.md](../zyvor/REGULATORY.md)
- [THREAT_MODEL.md](../zyvor/THREAT_MODEL.md)
