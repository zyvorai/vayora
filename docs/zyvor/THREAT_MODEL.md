# Zyvor Payments threat model (living document)

Scope: the API, worker, console and checkout as deployed from this repository. Rails are simulated; this
document says what the code defends against today and what it does not.

## Assets
Ledger and invoice records (integrity first), provider credentials and webhook secrets (confidentiality),
API keys, customer contact data, the audit trail.

## Trust boundaries
| Boundary | Authenticated by | Notes |
|---|---|---|
| Merchant system -> API | Bearer API key (SHA-256 hashed at rest, per-tenant, scoped) | optional expiry, address allow-list, rotation with overlap |
| Console user -> console | signed session cookie (Secure, HttpOnly) | single shared admin today; per-user accounts and MFA are planned |
| Console -> API | an API key held server-side in a cluster Secret | never sent to the browser |
| Payer -> checkout | unguessable intent URL plus a signed per-intent secret | rate limited per address |
| Provider -> API | HMAC signature on the raw body, tenant in the URL | replay window enforced |
| API -> merchant webhook | HMAC signature with timestamp, at-least-once | consumers must de-duplicate |
| Tenant <-> tenant | Postgres row-level security via `zyvor.tenant_id`, the app role is not a superuser | tested with cross-tenant negative tests |

## Threats and controls
- **Stolen or leaked API key**: scopes limit blast radius; keys can expire, be pinned to addresses, be rotated
  with an overlap and be revoked; `last_used_at` shows unexpected use; only hashes are stored.
- **Credential stuffing / floods**: token buckets per key and per address (429 with `Retry-After`). Behind
  reverse proxies the address comes from `X-Forwarded-For` counted from the right with
  `ZYVOR_TRUST_PROXY_HOPS`; client-supplied leftmost entries are never trusted.
- **Cross-tenant access**: row-level security is forced on every tenant table; the app connects as a
  non-superuser; the API never accepts a tenant id from the caller for authenticated routes.
- **Double charge / double refund / replay**: idempotency keys, unique constraints per attempt/period, state
  machine guards, unknown outcomes block new attempts until resolved.
- **Tampering with money records**: ledger is append-only and balanced per currency; corrections are reversals.
- **SSRF via merchant webhook URL**: https and public addresses only; credentials in URLs refused; the worker pins the
  connection to addresses vetted at delivery time and does not follow redirects.
- **Provider credential theft**: encrypted with AES-GCM under `ZYVOR_MASTER_KEY` (never in the database).
- **Network lateral movement in the cluster**: default-deny ingress NetworkPolicy; Postgres reachable only
  from the API, worker and migrate Job; API only from console and checkout; namespace enforces the
  `baseline` Pod Security level; containers drop all capabilities and use read-only root filesystems.
- **Supply chain**: cargo-deny (advisories, licences, sources), `pnpm audit`, gitleaks, CodeQL, Trivy on the
  image, weekly Dependabot, GitHub Actions pinned by commit SHA, CycloneDX SBOM per run.

## Known gaps (do not rely on these yet)
- Single shared console admin, no MFA, no per-user audit identity (planned).
- No master-key rotation tooling; webhook secret rotation without a dual-signature window.
- Provider `base_url` (sandbox field) is not covered by the SSRF guard; production accounts should not set it.
- Audit log is append-only by convention, not hash-chained.
- Rate limiter is per replica, in memory.
- Rails are simulated; this is not a licensed payment service (see REGULATORY.md).
