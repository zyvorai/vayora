# Open items: what was not finished, and how to do each

Written after the self-serve console work (PR #19) was merged. Each item says why it matters, how to do it,
where the code is, and how to check it. Order is by risk, not by effort. Anything already provided for (config
placeholders, scripts, docs) is marked **Provision**.

Rules of thumb used throughout: nothing here has run against a live provider; the simulator verifies nothing real;
never put a live key in a test environment (Zyvor refuses it, and so should you).

---

## 1. Before any real merchant (do these first)

### 1.1 A merchant can approve their own onboarding, so live opens with no Zyvor-side check — **DONE** (Zyvor-operated review: `docs/zyvor/PLATFORM_REVIEW.md`)
- **Why:** `open_live` (`crates/zyvor-api/src/signup.rs`) opens live for an organisation whose business is approved in
  either of its own tenants. A signed-up merchant can invite a second admin they control to approve the first (the
  admin scope includes `onboarding:review`), and with `ZYVOR_KYC_PROVIDER=simulator` almost any PAN is auto-approved.
- **How:** decide who approves. Two workable designs:
  1. *Zyvor-operated review.* Add a platform reviewer (a separate table, `platform_reviewers`, and a header or key
     that is never given to merchants). `onboarding::review` and `kyc` auto-approve then only set `ready_for_platform_review`;
     a platform reviewer sets `platform_approved_at`; `open_live` requires `platform_approved_at`.
  2. *A real KYC provider* (see 1.5) and keep auto-approve, with a human review for anything not clean.
  Either way, remove the merchant's ability to approve themselves: take `onboarding:review` out of what a merchant's
  `admin` session key can do for their own organisation.
- **Check:** a test in `crates/zyvor-api/tests/flow.rs` that signs up, invites a second admin, has them approve, and
  expects `switch_env` to live to still be refused.

### 1.2 Bank accounts are auto-verified after approval, and the holder name is not compared — **DONE** (a new account is only auto-verified before the business is submitted; the bank's name for the account is compared with the name given (`kyc::names_match`))
- **Why:** `add_bank_account` (`onboarding.rs`) marks an account verified (`verified_by = 'system:kyc'`) whenever the
  simulator's penny drop passes, at any time, and `kyc.rs::verify_bank` ignores the holder name.
- **How:** only auto-verify while the profile is `draft`/`rejected`; for an approved merchant a new account goes to a
  second person (`verify_bank_account`). In `verify_bank`, compare the name the bank returns with `holder_name` and
  return `Inconclusive` on a mismatch. A real provider returns the registered name; the simulator should return the
  `holder` it was given unless the handle says otherwise.
- **Check:** extend `onboarding_auto_approves_a_clean_case_and_routes_the_rest_to_a_person`.

### 1.3 Session keys can be renewed forever, and are not revoked at sign-out — **DONE** (a session ends 12 hours after its sign-in whatever swaps happen; a switch revokes the key it replaced; `POST /v1/console/logout` revokes the key; the worker prunes session keys a day after they end)
- **Why:** `switch_env` mints a fresh 12-hour key from any session key with no password or second factor; the old key
  stays valid; each sign-in or switch adds an `api_keys` row named `console session`. Sign-out only deletes cookies.
- **How:** (a) add `session_started_at` to `api_keys` (new migration, number after the highest on `main`), copy it
  to the new key on switch, and refuse a switch when `now - session_started_at > 12h`; (b) add
  `POST /v1/console/logout` that sets `revoked_at` on the calling session key, and call it from the console's `logout`
  action (`apps/zyvor-console/lib/actions.ts`); (c) have the worker delete expired `console session` keys daily
  (`crates/zyvor-api/src/worker.rs`).
- **Check:** tests for the 12-hour cap, for logout making the key return 401, and for the prune job.

### 1.4 The console's server calls share one rate-limit bucket — **DONE** (the console signs the visitor's address with `ZYVOR_ACTOR_SECRET` and the API uses it; set `CONSOLE_TRUST_PROXY_HOPS` to the number of proxies in front of the console, default 0 sends nothing)
- **Why:** `apps/zyvor-console/lib/public.ts` calls the public endpoints (signup, login, reset, invitations) from the
  console server without the visitor's address, so the API sees one client and one visitor can lock everyone out.
- **How:** in `publicPost`, forward the visitor's address from `headers()` (`x-forwarded-for`, set by your proxy) as
  `X-Forwarded-For`, and set `ZYVOR_TRUST_PROXY_HOPS` on the API to the number of proxies in front of it so
  `ratelimit::client_ip` believes it (see `deploy/zyvor/k3s/zyvor.yaml`, which already sets it for the demo). Do not
  trust the header when the hop count is 0.
- **Check:** two simulated visitors with different `X-Forwarded-For` values get separate buckets.

### 1.5 Real identity and bank checks — **BUILT, not yet run against Cashfree** (`docs/zyvor/KYC_CASHFREE.md`; needs Secure ID credentials, then `zyvor-api kyc-selftest`)
- **Why:** `kyc.rs` only has a simulator. Zyvor does not do KYC (`docs/zyvor/REGULATORY.md`).
- **How:** the Cashfree account in use has **Secure ID** activated (PAN, GSTIN and bank-account verification). Add
  `Kyc::CashfreeSecureId` beside `Kyc::Simulator` and implement `verify_pan`, `verify_bank` (penny drop or name
  match) from Cashfree's Secure ID documentation; credentials belong to the *platform* (env vars read in
  `Kyc::from_name`), not to each merchant. Update `REGULATORY.md` first (who is the verifying party, what you may
  store; today only the outcome and a short reason are stored, never the PAN or account number). Have a person
  review `kyc::decide` before turning it on.
- **Check:** keep the simulator for tests; add one test per outcome with the provider's documented test inputs.

---

## 2. Take a payment through Razorpay's and Cashfree's own sandboxes

Everything is built; the blocker is only the **test keys**, which must be entered by a person (or pasted by the user
into the chat of a session that is allowed to enter them). Runbook and checklist: `docs/zyvor/REAL_SANDBOX.md`.

1. **Connect.** Console → Providers (test mode) → Cashfree → test App ID and secret → Connect → Check connection.
   For Cashfree the webhook secret *is* the secret key (Cashfree signs merchant webhooks with it, per
   <https://www.cashfree.com/docs/payments/online/webhooks/signature-verification>); the form handles it.
   Connecting again replaces what was saved.
2. **Public address for webhooks.** Providers can only call a public HTTPS address.
   - *Temporary:* a Cloudflare quick tunnel: `cloudflared tunnel --url http://127.0.0.1:8091` (prints a
     `https://….trycloudflare.com` address; it changes on every restart and exposes the whole API, so stop it when
     done). Set `ZYVOR_PUBLIC_API_URL` (or `API_PUBLIC_URL`) on the console to that address.
   - *Stable:* expose only `/v1/webhooks/*` through Caddy as described under "Letting a real provider call back" in
     `deploy/zyvor/k3s/README.md` (the demo server already has an `sslip.io` name for this), or create a named
     Cloudflare tunnel on a domain you own.
3. **Tell the provider.** Cashfree: Developers → Webhooks (Test Environment) → add the address shown on the
   Providers page. Razorpay: Settings → Webhooks, same address plus the webhook secret you chose.
4. **Run the checklist from the command line.** **Provision:** `scripts/real-sandbox-check.sh`
   (`accounts`, `check`, `pay <provider>`, `status <id>`, `refund <id> <amount>`). It refuses a live key and never
   prints the key. Tick off the table at the end of `REAL_SANDBOX.md`.
5. **Known uncertainty:** one Cashfree doc page shows a `.` between timestamp and body in the signed string; the
   adapter (and the other pages) concatenate directly. The first real webhook settles it; if signatures fail, change
   `crates/zyvor-providers/src/cashfree.rs::verify_webhook`.

Not done: Razorpay (no test keys yet); card, wallet and netbanking against a real provider; settlement sync and
reconciliation against real data.

---

## 3. Email

Built: a database mail queue drained by the worker (`crates/zyvor-api/src/mail.rs`), verification, invitations, password
reset. Verified only against a plain local SMTP server.

- **Turn it on.** **Provision:** the k3s manifest and the Helm chart now carry optional mail settings.
  - k3s: `kubectl -n zyvor create secret generic zyvor-mail --from-literal=smtp-url='smtp://USER:PASS@HOST:587?tls=required' --from-literal=from='Zyvor <no-reply@your-domain>' --from-literal=console-url='https://console.your-domain'` then restart the `zyvor-api` and `zyvor-worker` deployments. Without the secret nothing changes.
  - Helm: set `mail.enabled=true`, `mail.existingSecret`, `mail.consoleUrl` (secret keys `smtp-url`, `from`).
  - Both the API (which checks "is mail delivered?") and the **worker** (which sends) need the SMTP settings.
- **Verify for real:** with TLS and a username and password, send to a mailbox you can read; check it arrives and is
  not in spam. Needs SPF, DKIM and DMARC on the sending domain (done at your DNS and mail provider, not in this repo).
- **Watch it:** **Done:** `/metrics` now carries `zyvor_mail_pending` and `zyvor_mail_dead`, and `deploy/zyvor/alerts.yml`
  has alert rules for them (and the other gauges). The rules are valid YAML but have **not** been run through `promtool`.
- **Later:** HTML templates, a per-organisation sender, payment receipts and reminder emails (the events exist; add
  consumers that enqueue mail), unsubscribe handling for anything that is not transactional.

---

## 4. Sandbox and testing

Built: every new test tenant gets the simulator; a Sandbox page creates payments with a chosen outcome.

- ~~Card, wallet and netbanking test payments in the console.~~ **Done:** the Sandbox form has a "Paid by" choice. The
  simulator picks the outcome from the handle whatever the method and no card number or bank is collected, so this
  exercises routing, the ledger and refunds, not a card form (a test covers all three methods).
- ~~Settlement simulation.~~ **Done:** `POST /v1/settlements/sync` with provider `sim` builds one settlement per day from the
  organisation's own captured simulated payments and processed refunds, with an invented 2% fee (button on Reconciliation in
  test mode). `POST /v1/settlements/{id}/simulate_bank` adds the matching bank credit (optionally short, which opens a
  case); buttons on the same page. `POST /v1/payment_attempts/{id}/simulate_event` (test mode) settles a pending simulated
  payment as captured or failed through the normal update path, standing in for a provider's late update (API, Node, Python and Go;
  a console button on the payment page for pending simulated attempts, and no signed webhook is actually sent to Zyvor's receiver).
- **Test clock, in part:** `POST /v1/billing/run` and `POST /v1/collections/dunning/run` accept `as_of` (a date) in test mode, so a test
  organisation can see the next period invoiced or the reminders due on a later day (Node and Python; not Go). Nothing else
  moves with it: the worker, mandates, due dates on new invoices and the console still use today. Still open: a real clock the
  whole test organisation follows; and a "reset test data" button (delete everything for a test tenant, never live).
- ~~A payment's timeline can show "Payment captured" after a later refund.~~ **Done:** a capture is dated by its ledger
  posting (`crates/zyvor-api/src/timeline.rs`); the test fails without the fix.

---

## 5. People and roles

- People live in the test tenant, so there are no live-only roles and cashier sessions stay in test (use a
  branch-limited API key at a live till).
- ~~The People page listed branches with the caller's own key while `invite` checked them in the test tenant.~~
  **Done:** `GET /v1/console/branches` reads the home tenant and the page uses it (`z.branches.assignable()`).
- ~~A revoked or expired session shows the error page instead of the sign-in page.~~ **Done:** `api()`
  (`apps/zyvor-console/lib/zyvor.ts`) turns a 401 for a person's own key into a redirect to `/login`.

---

## 6. Console polish

Mobile was checked on three pages only. Walk every page at 390 px (`apps/zyvor-console/app/globals.css` has the phone
rules at the end), and add: saved views on Payments (search is **done**: `q=` on `GET /v1/payment_intents` and a box on the page),
payment-link reminders (backend feature), an audit of empty states in a browser (a code scan found every list page has empty-state text except Keys, which cannot be empty because the viewing key is listed; `error.tsx`, `loading.tsx` and `not-found.tsx` exist for the app group), keyboard and screen-reader pass,
translations for the new pages (`apps/zyvor-checkout` shows the approach).

---

## 7. Operations and process

- **History.** PR #19 was squash-merged, so the 16 individual commits are not on `main`. They remain on the
  `ux/stripe-console` branch; do not delete it until you are sure you will not want them.
- **Demo cluster.** The k3s demo runs whatever image was last deployed
  (`CONSOLE_PASSWORD=… deploy/zyvor/k3s/deploy.sh sus@80.79.5.173`; see `deploy/zyvor/k3s/README.md`). It uses the mock
  provider and has no public ingress except what the Caddy notes describe.
- **Dependency bumps.** `main` was briefly red when a Dependabot bump of `aes-gcm` (0.10 → 0.11) was merged before
  its code change; it was fixed in `36c67a8`. Merge Dependabot PRs only with a green `rust` job, and give majors their
  own PR.
- **Secret scanner.** `.gitleaksignore` lists four false positives in `apps/ios/.../SetupPayloadTests.swift`. If those
  fixtures change, regenerate with `gitleaks detect --redact --verbose`.
- **Local leftovers from the work session** (safe to remove): the API, console and Cloudflare tunnel started for the
  real-sandbox attempt; the throwaway database `zyvor_ux` and Postgres on port 54555; `brew install cloudflared`
  (`brew uninstall cloudflared`); the git worktree `../Vayora-ux` (`git worktree remove ../Vayora-ux`).

---

## 8. Settings added this round (reference)

| Variable | Where | Meaning |
|---|---|---|
| `ZYVOR_SMTP_URL`, `ZYVOR_MAIL_FROM` | API and worker | Send mail; unset = log only |
| `ZYVOR_CONSOLE_PUBLIC_URL` | API | Where links in emails point (default `http://localhost:3000`) |
| `ZYVOR_KYC_PROVIDER=simulator` | API | Auto-approve with the simulator. **Never for real merchants** |
| `ZYVOR_PUBLIC_API_URL` / `API_PUBLIC_URL` | Console | Public API address shown on the Providers page |
| `ZYVOR_TRUST_PROXY_HOPS` | API | How many proxies vouch for `X-Forwarded-For` |
| `CONSOLE_TRUST_PROXY_HOPS` | Console | How many proxies sit in front of the console; its visitors' addresses are only forwarded (signed) when this is 1 or more. Needs `ZYVOR_ACTOR_SECRET` on both sides |
| `ZYVOR_SIM_PAYER=1` | API | Lets the iPhone payer app approve simulated QR payments and mandates (test only) |
| `ZYVOR_MANDATE_MAX_MINOR` | API | The highest per-debit limit a UPI AutoPay mandate may be created with, in paise (default 1500000 = ₹15,000). Verify against the current NPCI circular before live use |
| `ZYVOR_MANDATE_NOTICE_HOURS` | API, Worker | How long before a debit the payer must be told (default 24). The worker also uses it, so set it on both |
| `ZYVOR_REVIEW_TWO_PERSON` | API | `never`, `flagged` (default) or `always`: when live access needs two different Zyvor reviewers (docs/zyvor/PLATFORM_REVIEW.md). Needs at least two active reviewers unless `never` |
| `ZYVOR_REVIEW_SLA_HOURS` | API, Worker | Target for answering a live-access request (default 24). A target, not a promise |
| `ZYVOR_CHANGE_SLA_HOURS` | API, Worker | Target for a live business's new bank account, whose payouts are held meanwhile (default 4) |
| `ZYVOR_REVIEW_ALERT_EMAIL` | Worker | Emailed once for each request or change that passes its target; nothing is sent when unset |
| `ZYVOR_REVIEW_UTC_OFFSET_MINUTES` | API | The review desk's offset from UTC in minutes for the reviewer rota (default 330 = India; no daylight saving; -720 to 840) |
