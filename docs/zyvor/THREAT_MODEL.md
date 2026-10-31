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
| Console user -> console | signed session cookie (Secure, HttpOnly) carrying user and role | named users with Argon2id passwords, optional TOTP + recovery codes, lockout; deployment admin is break-glass only until the first user exists |
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
- **Over-debit, early debit or debit after revoke (AutoPay mandates)**: a debit above the mandate's cap, outside its
  dates, in another currency, or inside the notice window is refused when scheduled and again when sent; two debits
  never share a day (a unique constraint) or a frequency period; a paused, revoked or expired mandate never sends;
  a payer's revoke is learned on refresh and cancels debits not yet sent; the provider is called outside any
  transaction after the attempt is saved, and a lost answer stays `unknown` and is resolved by asking.
- **Double charge / double refund / replay**: idempotency keys, unique constraints per attempt/period, state
  machine guards, unknown outcomes block new attempts until resolved.
- **Tampering with the audit trail**: events are append-only (trigger), and the worker seals them into a hash chain
  (`/v1/audit_events/verify` recomputes it and names the first broken seal), so even a superuser edit shows up.
- **Overflow and absurd amounts**: every endpoint that takes a money amount refuses anything above ₹10 billion
  (`MAX_AMOUNT_MINOR`), so sums in the database and ledger cannot overflow a 64-bit integer; dynamic SQL interpolates
  only constants, never request data, and every value is a bound parameter.
- **Tampering with money records**: ledger is append-only and balanced per currency; corrections are reversals.
- **SSRF via merchant webhook URL**: https and public addresses only; credentials in URLs refused; the worker pins the
  connection to addresses vetted at delivery time and does not follow redirects.
- **AI assistant misuse (prompt injection, data leakage, harmful wording)**: off by default; a person approves every
  AI draft; the model sees only non-personal invoice facts (no customer names, contacts or free text); output with links,
  emails, threats or missing invoice number is discarded; per-tenant monthly cap; every request is audited.
- **Provider credential theft**: encrypted with AES-GCM under `ZYVOR_MASTER_KEY` (never in the database). Rotate by
  setting the new key, moving the old to `ZYVOR_MASTER_KEY_PREVIOUS`, restarting, and running `zyvor-api rekey`
  (`--dry-run` first); secrets no key can open are reported and never overwritten.
- **Network lateral movement in the cluster**: default-deny ingress NetworkPolicy; Postgres reachable only
  from the API, worker and migrate Job; API only from console and checkout; namespace enforces the
  `baseline` Pod Security level; containers drop all capabilities and use read-only root filesystems.
- **Supply chain**: cargo-deny (advisories, licences, sources), `pnpm audit`, gitleaks, CodeQL, Trivy on the
  image, weekly Dependabot, GitHub Actions pinned by commit SHA, CycloneDX SBOM per run.

## Known gaps (do not rely on these yet)
- MFA is optional unless `CONSOLE_REQUIRE_MFA=1`; sessions are not individually revocable before their 12 h expiry; no SSO.
- Webhook secret rotation has no dual-signature window yet. Master-key rotation exists (below) but is operator-run.
- Provider `base_url` (sandbox field) is not covered by the SSRF guard; production accounts should not set it.
- Audit seals detect tampering after the fact; they do not prevent it, and the chain head is not yet anchored outside the database (publish `head` somewhere the database owner cannot edit to close this). Events newer than the seal delay (120 s) are not yet covered.
- Rate limiter is per replica, in memory.
- **Second-person rules trust the `Zyvor-Actor` header unless `ZYVOR_ACTOR_SECRET` is set.** Refund, payout, onboarding and closing approvals compare the person who asked with the person who approved, and the API takes that name from a header. With no actor secret, any holder of one key that has both the write and the approve scope can name two different people and approve their own request. With `ZYVOR_ACTOR_SECRET` set on the API (and the same value on the console, which then signs each name with a timestamp), the API believes a name only when it carries a valid, fresh signature; anything else is recorded as the key itself, so an integration key cannot pose as a person. The k3s deploy script creates this secret and wires both sides. Still keep approve scopes (`refunds:approve`, `payouts:approve`, `closings:approve`, `onboarding:review`) on the console's key only: whoever holds the actor secret can name anyone, and a signature stays valid for five minutes.
- A branch-limited key is closed to every endpoint that does not enforce its branch, but the scopes it holds still apply on the ones that do; give it only what a till needs.
- **AutoPay mandates are simulator-only.** No real provider supports them yet. A debit already in flight when the payer revokes is left to the provider to decide. Zyvor emits the pre-debit notice but does not deliver it; a merchant who ignores `mandate.debit_notified` debits without telling the payer. The notice window and ceiling are defaults, not verified against NPCI's circular.
- Rails are simulated; this is not a licensed payment service (see REGULATORY.md).
