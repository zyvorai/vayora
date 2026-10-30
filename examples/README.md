# Examples

Runnable scripts for the use cases Zyvor was built around. They talk to the **sandbox** (simulated providers, no
real money). Each creates its own records, so you can run it more than once.

```sh
OWNER_URL=postgres://you@localhost:5432/zyvor_sandbox ./scripts/zyvor-sandbox.sh   # prints an API key
pnpm --filter @zyvor/node build
export ZYVOR_API_KEY=zk_test_...            # the key it printed
node examples/campus-fees.mjs               # instalments, a class payment link, promises, reminder drafts
node examples/clinic-billing.mjs            # GST invoice by SAC code, refundable deposit, risk limits
node examples/marketplace-split.mjs         # split payment, vendor balances, bulk payouts
node examples/shop-counter.mjs              # branch and counter, payer-typed-amount QR, cash, end-of-day closing, payment timeline
```

They show the API, not a finished integration: capturing a payment needs a payer (or the simulator's approve call,
see the sandbox script's output), and rates, HSN/SAC codes and GSTINs here are placeholders to replace with your own.
