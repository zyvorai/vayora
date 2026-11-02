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

### 1.4a Uploaded documents are read and compared — **BUILT, checked on real documents** (`docs/zyvor/ONBOARDING_DOCUMENTS.md`)
- A business uploads its GST certificate, certificate of incorporation, PAN card or cancelled cheque; Zyvor stores the file encrypted, reads it (the PDF's text layer, or OCR for a scan) and compares the GSTIN, PAN ending, CIN, IFSC and legal name with what was entered. Checked with the company's real GST certificate (text layer) and a scanned certificate of incorporation (OCR): every comparison matched, and changing the profile to another business made the same documents flag. A Zyvor reviewer can open the file (audited in the business's own trail; tested through the API, the console button was not driven in a browser). Only the reviewer who holds the case can open its files. What is **not** done: a PAN card image and a cancelled cheque photo are only as good as OCR on a photo; no check that a certificate is genuine (a forged but well-formed one reads the same; only the provider check or a person can catch that); the reading runs inside the API container (see THREAT_MODEL); Aadhaar documents are refused.

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
- **Done:** payment receipts. `payment_receipts` in settings (off by default; a Receipts box on the Settings page) queues a plain-text
  receipt in the same transaction as the capture, to the intent's customer when that customer has an email address, once per attempt
  (`mail_queue.dedupe_key`). Checked in the local sandbox: the Settings box saves and persists, and a simulated captured payment for a customer with an email queued one receipt that the worker handed to the log-only mailer (no SMTP configured, so nothing was delivered). Delivery uses the mailer above, so it is as verified as that is (plain local SMTP only).
- **Done:** refund receipts. The same `payment_receipts` switch also emails the payer when a refund is processed (once per refund, dedupe key `refund_receipt:{id}`; the merchant's private refund reason is never included). Covered by the receipts API test; not yet looked at in the browser, and delivery is only as verified as the mailer above.
- **Done:** emailing a payment link. `POST /v1/payment_links/{id}/send` (`payments:write`; Node `paymentLinks.send(id)`, Python `send_payment_link`, Go `SendPaymentLink`; an Email button on the console Payment links page, shown only when the link's customer has an address) queues the link to that customer. It is a person's action, not the Receipts setting: it needs `ZYVOR_CHECKOUT_PUBLIC_URL`, an active link and a customer with an email, and goes once per link per day (409 `already_sent_today`). Audited as `payment_link.sent`. Covered by an API test; not looked at in a browser, and delivery is only as verified as the mailer above.
- **Done:** one automatic reminder before a link expires. A link made with `remind_before_expiry: true` (needs `customer_id` and `expires_at`; API, Node `remindBeforeExpiry`, Python `remind_before_expiry=True` and Go `CreatePaymentLinkFor(..., PaymentLinkOptions{...})`; off by default) makes the worker email its customer once in the last 24 hours, if the link is not canceled or used up, the invoice (if any) still owes money, the customer has an address and `ZYVOR_CHECKOUT_PUBLIC_URL` is set. Once per link (`expiry_reminded_at`, mail key `link_expiry:{id}`), audited as `payment_link.expiry_reminded`, skipped for a business whose live mode is frozen. Covered by a worker test; the worker runs about once per cycle, so the email goes out within a cycle of the 24-hour mark, not at an exact time. Not looked at in a browser or with real mail.
- **Done:** the console's new-link form has an optional customer (only customers with an email), a last day to pay (end of that day, India time) and a "remind once" box (refused with a message if either is missing). Checked in the local sandbox in a browser: the refusal, creating a link (stored with the reminder on and the expiry at 23:59:59 +05:30), the Email button appearing, and its refusal when no public checkout address is set. The reminder email itself was only checked by the worker test.
- **Done:** a reply-to address per business (a smaller step than a per-organisation sender). `reply_to_email` in settings (one plain address, empty clears; a Replies go to field on the console Receipts card) is stamped on every email queued for that business (receipts, refund receipts, payment-link emails and reminders) and sent as a `Reply-To` header, so the "reply to the business" line in those emails works. The From address is still Zyvor's. Mail from Zyvor itself (sign-in links, review notices) has none. Covered by an API test that reads the header at a stand-in SMTP server; not looked at in a browser, and not tried against a real mail server.
- **Done:** HTML versions of the payer-facing emails (receipts, refund receipts, payment links and reminders). They go as plain text plus an HTML alternative (`multipart/alternative`), made from the plain text by `mail::html_from_text` so the wording lives in one place: paragraphs, a "Pay now" button for the `Pay here:` line (https/http addresses only), small print last, everything escaped, no images, scripts or tracking. Stored in `mail_queue.html` and cleared once sent. Zyvor's own mail (sign-in links, review notices) stays plain text. Checked: unit test for escaping and the button rule, the SMTP stand-in sees both parts, and a sample rendered in a browser (layout only). **Not** looked at in a real mail client (Gmail, Outlook and phones differ), no dark-mode styling, no business logo or colours.
- **Done:** payment reminder emails, opt-in. `reminder_emails` in settings (off by default; a checkbox on the console Settings card, which is now "Receipts, reminders and replies") makes a reminder whose channel is `email` also go to the invoice's customer when they have an address: a reminder logged by hand (the wording the person gave), a draft a person approved (the approved wording), and the unattended dunning run (the standard wording). Once per reminder (mail key `reminder:{id}`), audited as `reminder.emailed`, carries the business's reply-to and the HTML version. It is off by default because the existing contract is that Zyvor logs the reminder and emits `invoice.reminder` for the business's own messaging; turning this on while that system also sends would email the customer twice. Covered by an API test (off, other channels, no customer, manual, approved draft with edited wording, the dunning run); not looked at in a browser or a real mail client, and no unsubscribe link (these are bills, not marketing).
- **Later:** per-business logo and colours in the HTML, a per-organisation From address (needs domain verification: SPF/DKIM), unsubscribe handling for anything that is not transactional.

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
  organisation can see the next period invoiced or the reminders due on a later day (Node, Python and Go). Nothing else
  moves with it: the worker, mandates, due dates on new invoices and the console still use today. Still open: a real clock the
  whole test organisation follows; and ~~a "reset test data" button~~ **Done:** `POST /v1/test_data/reset` (admin, test environments only, body `{"confirm":"reset test data"}`; Node `testData.reset()`, Python `reset_test_data()`, Go `ResetTestData()`; a Test data card on the console Settings page) clears payments, invoices, refunds, customers, settlements, mandates, payouts, cash entries and the ledger for that tenant and keeps settings, provider accounts, keys, people, branches, counters, vendors, plans and coupons (coupon use counts go back to 0). The audit trail is never cleared and records the reset with row counts. Checked in the local sandbox in a browser: a wrong confirmation is refused with the form kept, the right one clears the two payments, the invoice, the customer and the ledger, Payments then shows its empty state, and the provider accounts, keys and settings are still there.
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

**First-run Overview** (done, looked at in a browser with the sandbox's sample payment cleared): the setup checklist now starts with the quick win, "Take a test payment" (test mode only), puts the slow review steps last and marks the next step; an empty dashboard says "No payments yet" with where to go instead of "0 successful of your last 0"; and the row of five zero "Due to come in" cards becomes one line when nothing is due. The first test payment's page, the Providers page, the light theme on Overview and Payments, and the sign-up screen were then looked at as a new user: internal ids became words (Simulator (test), UPI; a payment waiting for the payer reads "awaiting payment" instead of "requires payment method"), and "Take a test payment" now counts only a payment that succeeded. Not reviewed: the other pages in the light theme, connecting a real provider, (the payment timeline's sentences now name the provider and method in words too: "Attempt started on Razorpay (UPI)", "Razorpay sent ..."). Mobile was checked on three pages only, then Payments, Payment links and Settings were checked at 390 px in a browser (a frame of that width, since the browser window would not go narrower): no sideways page scroll; fixed the Payments search box being squeezed to a sliver by the date pickers, the status filter being clipped (it now scrolls), unlabeled unstyled date boxes (now From/To) and unstyled dropdowns in forms. Then every other page in the app group was walked the same way (21 in all): none scrolls sideways at 390 px once the Developers page's code sample was made to scroll inside its card. Only widths were measured on those; most were not looked at one by one, and dark mode and a real phone are unchecked. Walk every page at 390 px (`apps/zyvor-console/app/globals.css` has the phone
rules at the end), and add: saved views on Payments (**done**: search `q=`, date range `created_from`/`created_to`, and named views: `GET/POST /v1/saved_views`, `DELETE /v1/saved_views/{id}`; personal by default, shareable with the environment by a person with `payments:write`; 20 per person; filters status, q, from, to are validated on save; Node `savedViews`, Python `*_saved_view`, Go `*SavedView`; chips with a save form on the Payments page and the CSV export keeps the same filters; other lists are not covered). Checked in the local sandbox in a browser: filters, saving a shared view, applying it, deleting it, and the date range. That check found and fixed a bug: the page listed views as a different person from the one who saved them, so a personal view would not have shown,
payment-link reminders (**done** in the API: a link can be emailed to its customer by hand, and can ask for one reminder before it expires (the console form has the field)), an audit of empty states in a browser (**done** for the lists a new business sees first: the bare one-liners on Refunds, Disputes, Webhooks, Collections, Reconciliation and Payments now say what the list is for and how the first row appears, with links, and Reconciliation no longer says "Everything reconciles" when there is nothing at all; read in a browser against an empty sandbox; the other pages were not reworded) (earlier: a code scan found every list page has empty-state text except Keys, which cannot be empty because the viewing key is listed; `error.tsx`, `loading.tsx` and `not-found.tsx` exist for the app group), a keyboard and screen-reader pass (**part done**: every page was scanned in a browser for missing labels, nameless buttons and links, heading order, duplicate ids, positive tabindex, tables without headers and unlabeled navs, and all were clean; the scan found no skip link and no visible focus ring on the search and theme buttons, both now fixed and checked with the keyboard. The command menu was then made a proper modal combobox (Tab stays inside, the list is a valid listbox with the highlighted row announced, the number of matches is announced, Esc returns focus to what opened it) and checked with the keyboard in a browser; that check caught and fixed a bug of my own, where Enter on a command reopened the menu. The share sheet is a native dialog and already trapped focus and returned it on Esc (checked). Colour contrast was then measured on every console page in the light and dark themes with a script (text against its real background, 4.5:1, or 3:1 for large text): it found the status colours used as text (green 2.9:1, amber 3.6:1, red 4.0:1 on their tinted chips and notes), grey text on tinted backgrounds (4.3:1), white on the primary blue (4.3:1) and the new skip link (3.5:1 light, 2.9:1 dark); text variants and darker blues were added and a re-scan is clean apart from disabled buttons, which are exempt. Only what the sandbox data shows was measured; warning and error states that did not appear were checked by calculation only. The payer-facing checkout app (the pay page and the payment-link page, which refuse to be framed so they were scanned directly) passed the structural scan (labels, names, heading, language tag that follows the chosen language, an alert region for errors) and the same contrast scan, in both themes (dark by applying its dark colour variables, since the browser's colour-scheme setting could not be switched): it needed darker text colours (the selected method tile 4.1:1, status and notice text, links), black text on the dark-mode pay button (white was 3.0:1) and a placeholder colour (the browser default was 4.1:1; the console had the same and got the same rule). The error and "paid" states beyond the empty-UPI-ID error, and the payer-facing screens after a real payment, were not scanned. The payer screens after a real payment were then driven in a browser with the simulator (paid, failed with its reason, processing, an invalid link and an invalid payment, plus the empty-ID error): all pass the contrast scan, each has a live region (alert for failures, status for progress), and keyboard focus used to drop to the top of the page when the Pay button went away, so the checkout now moves focus to the new screen's heading (or the failure notice, or the method list on Try again) without stealing it on first load; the invalid-link screens also gained a real top-level heading. The console's signed-out screens (login, signup, forgot, reset, accept, check-email) had no main landmark and faint small text (2.5:1): both fixed (the card's gradient background only tints it a few percent, so it was scanned as a normal card), the password toggle is now named "Show password"/"Hide password", and a wrong sign-in was checked with the keyboard (an alert is announced, the typed username and focus stay). "Screen reader" here means the browser's accessibility tree (what assistive technology is handed: landmarks, names, roles, live regions) and keyboard behaviour. Not done: listening to VoiceOver, NVDA or TalkBack themselves, the card, netbanking and wallet screens of a real provider, and the UPI QR and app-link screens (they need the Cashfree or Razorpay sandbox)),
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
| `ZYVOR_REVIEW_TIMEZONE` | API, Worker | The review desk's IANA time zone for the reviewer rota, for example `Europe/London` (follows daylight saving; wins over the fixed offset; an unknown name is ignored with a warning) |
| `ZYVOR_REVIEW_UTC_OFFSET_MINUTES` | API, Worker | The desk's fixed offset from UTC in minutes when no zone is set (default 330 = India; never changes with the seasons; -720 to 840) |
| `ZYVOR_REVIEW_AUTO_ASSIGN` | Worker | `1` gives each waiting request and change to the on-duty reviewer holding the fewest (never the first approver); off by default; needs hours set on the rota (docs/zyvor/PLATFORM_REVIEW.md) |
