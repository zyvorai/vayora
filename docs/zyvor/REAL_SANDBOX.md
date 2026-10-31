# Running against the providers' own sandboxes

The Sandbox page in the console uses Zyvor's built-in simulator. This runbook is for the other thing: taking a payment through **Razorpay's test mode** and **Cashfree's sandbox**, with their real APIs and real webhooks, still with no real money.

Nothing in this repository has been verified against a live Razorpay test account. The Cashfree adapter's UPI calls were checked against Cashfree's sandbox once (see the comment in `crates/zyvor-providers/src/cashfree.rs`); everything else is only checked against the local mock. Use the checklist at the end to record what you actually saw.

## You need

1. **Test credentials** from each provider's own dashboard (Zyvor cannot create these for you):
   - Razorpay: a test-mode key id and key secret. A test key id starts with `rzp_test_`.
   - Cashfree: a sandbox app id and secret key.
2. **A webhook secret per provider**, which you choose. You enter it in Zyvor and in the provider's webhook settings.
3. **A public HTTPS address for the API.** Providers send webhooks from the internet, so `localhost` and in-cluster addresses do not work. Either a tunnel while testing (any tunnel that gives the API's port 8080 a public `https://` address) or an ingress on a server. The k3s sandbox in `deploy/zyvor/k3s` exposes only cluster-internal services, so it needs an ingress or a tunnel before a provider can reach it.
4. For the payer's side of a payment, the checkout app (`apps/zyvor-checkout`) reachable by the person paying, and `ZYVOR_CHECKOUT_PUBLIC_URL` set on the API.

## Steps

1. Start the API and worker with a database (see `docs/zyvor` and `scripts/zyvor-sandbox.sh` for the pieces). Set on the console `ZYVOR_PUBLIC_API_URL=https://<your public address>` so the Providers page shows the right webhook address.
2. Sign up or sign in as an admin and make sure you are in **Test** mode (top bar).
3. **Providers** → connect Razorpay and/or Cashfree with the test keys and your webhook secret. Zyvor refuses a `rzp_live_` key in a test environment, and a Cashfree production address in test.
4. Press **Check connection**. It makes one read-only call to the provider and reports whether the credentials were accepted. A "not connected" answer includes the provider's HTTP status.
5. Copy the webhook address from the page into the provider's dashboard, with the same secret, and subscribe to the payment and refund events.
6. Take a payment: create a payment link or a payment intent and open the checkout, then pay with a **test instrument from the provider's own documentation** (the numbers and UPI handles are theirs, not Zyvor's).
7. Watch it settle in Zyvor: Payments → the payment's page shows the attempt and the timeline; the webhook should move it to captured without any polling.
8. Refund part of it from the payment page and watch the refund finish.
9. Reconcile: sync the provider's settlement for the day (`POST /v1/settlements/sync`) and look at Reconciliation.

## Checklist (fill in as you go)

| Step | Expected | Razorpay | Cashfree |
|---|---|---|---|
| Connect with test keys | saved, no `wrong_mode_credentials` | | |
| Check connection | "accepted the credentials" | | |
| Wrong secret | "did not accept" (not a crash) | | |
| Pay: UPI | captured via webhook | | |
| Pay: card | captured via webhook | | |
| Pay: declined instrument | failed, with a reason | | |
| Webhook signature | wrong secret is refused | | |
| Refund (partial) | processed, balance updated | | |
| Settlement sync | settlement and lines imported | | |
| Reconciliation | matched, or a case with a reason | | |

If a row fails, keep the request id from the API log and the provider's own error text; those two are what is needed to fix an adapter.

## What this does not cover

Real money, a provider's production account, Razorpay's or Cashfree's own dispute and settlement timing in production, and anything that needs the provider to enable a feature for your account.
