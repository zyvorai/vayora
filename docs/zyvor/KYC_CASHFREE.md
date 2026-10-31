# Identity and bank checks with Cashfree Secure ID

During onboarding Zyvor can check a business's PAN, GSTIN and bank account with **Cashfree Secure ID** (the
"Verification Suite" APIs) instead of the built-in simulator. A clean submission is approved for **test mode** at
once; live mode still needs a Zyvor reviewer (`PLATFORM_REVIEW.md`). Anything the provider cannot answer clearly
goes to a person.

## What it asks Cashfree

| Check | Call | Passes when |
|---|---|---|
| PAN | `POST /verification/pan` (PAN and legal name) | the PAN is valid and the name matches the record |
| Bank account | `POST /verification/bank-account/sync` (account, IFSC, holder name) | the account exists and its holder matches the name given |
| GSTIN | `POST /verification/gstin` (GSTIN and name), after a local check that it carries the PAN | active, and the name matches the registration |

Written from Cashfree's API reference. Cashfree's name match is a paid add-on; when its fields are missing Zyvor
compares the registered name itself (`kyc::names_match`: case, punctuation, honorifics and company suffixes ignored).

## What happens when something is not a clear yes or no

| What came back | Result |
|---|---|
| a clear "no" (invalid or deleted PAN, account not found, bad IFSC, blocked account, GSTIN missing or not active) | `fail`, shown to the reviewer |
| valid record, but the name does not clearly match | `inconclusive`: a person decides |
| credentials refused (401), our address not whitelisted (403), wrong environment (400), no balance (422), rate limited (429), provider error (5xx), timeout, no connection | `inconclusive`, **never a pass**; logged at error level for the operator |
| more than 12 paid checks for one business in 24 hours | `inconclusive` ("too many automatic checks today"), no call made |

An `inconclusive` check always sends the submission to a person and is never counted as verified, so a provider
outage slows onboarding but cannot approve anyone.

## Turning it on

You need, from Cashfree: Secure ID enabled on the account, **TEST and PROD modes** enabled (Cashfree enables them
on request), a balance (each call is charged), and for production your server's IP address whitelisted in the
dashboard (otherwise `403 ip_validation_failed`).

```
ZYVOR_KYC_PROVIDER=cashfree
ZYVOR_KYC_CASHFREE_ENV=sandbox            # or production; required, so a real account is never used by accident
ZYVOR_KYC_CASHFREE_CLIENT_ID=...          # the Secure ID app id for that environment
ZYVOR_KYC_CASHFREE_CLIENT_SECRET=...
```

These belong to the **platform** (the API), not to each merchant. Set them on the API only. **Provision:** the k3s
manifest and the Helm chart read them from an optional secret; see below.

1. **Prove the credentials** with Cashfree's documented sandbox test inputs (no database needed):
   `ZYVOR_KYC_PROVIDER=cashfree ZYVOR_KYC_CASHFREE_ENV=sandbox ZYVOR_KYC_CASHFREE_CLIENT_ID=... ZYVOR_KYC_CASHFREE_CLIENT_SECRET=... zyvor-api kyc-selftest`
   It runs a valid and an invalid PAN, account, IFSC and GSTIN and exits non-zero unless all seven behave. It refuses
   the production address. If every row says "a person will review this", read the reason after it (credentials,
   address, balance).
2. Run a real onboarding in a test environment against the sandbox and read the checks on the Onboarding page.
3. Switch to `production` with fresh production credentials. Watch the Cashfree balance and the number of
   `inconclusive` checks for the first days.

Cashfree rejects test credentials on production and the reverse (`Client secret belongs to test environment`); that
shows up as an `inconclusive` check with "our identity-check credentials are for the wrong environment".

## Cost, privacy and the law

- Each PAN, bank-account and GSTIN call is charged by Cashfree. The per-business daily cap (12) is a constant in
  `crates/zyvor-api/src/onboarding.rs` (`MAX_PAID_CHECKS_PER_DAY`); raise it deliberately. Counts are in `kyc_usage`.
- Zyvor sends Cashfree the PAN, name, account number, IFSC and GSTIN a merchant types, and stores **only the outcome and a short
  reason** (never the PAN, account number or name; the last four digits stay as before). The Onboarding page tells the
  merchant these are checked with a verification partner.
- Using a verification vendor for onboarding is a decision with consent, data-sharing and retention consequences.
  Have counsel read `REGULATORY.md` and your agreement with Cashfree before production.

## Not built

- Re-checking a business after it changes, or on a schedule.
- Cashfree's other products (Aadhaar and DigiLocker, face match, director checks, business-registration lookups).
- Caching a recent identical check to avoid paying twice.
- A per-merchant provider choice; one platform provider for everyone.
