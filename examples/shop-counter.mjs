// A shop with two tills: counter QR with a payer-typed amount, cash, an end-of-day closing, and the
// plain-words timeline of a payment. Run against the sandbox (scripts/zyvor-sandbox.sh prints a key):
//   ZYVOR_API_KEY=zk_test_... node examples/shop-counter.mjs
import { client, rupees, run, say } from "./_client.mjs";

const z = client();
const branch = await z.branches.create(`Kothrud ${run}`);
const till = await z.counters.create({ branchId: branch.id, name: "Till 1" });
say("Branch and counter", { branch: branch.name, counter: till.name });

// The QR printed at the till: the customer types what they owe, between ₹10 and ₹5,000.
const link = await z.paymentLinks.create({ description: "Pay at Till 1", openAmount: true, minAmountMinor: 1_000, maxAmountMinor: 500_000, counterId: till.id });
say("Print this as a QR", `${z.paymentLinks.qrUrl(link.id)}\n(needs ZYVOR_CHECKOUT_PUBLIC_URL on the API; the page itself is ${link.path})`);

// A payment taken at this branch directly, and the timeline that explains where it stands.
const intent = await z.intents.create({ amountMinor: 24_900, currency: "INR", branchId: branch.id, counterId: till.id, externalRef: `SALE-${run}`, idempotencyKey: `shop-${run}` });
const story = await z.intents.timeline(intent.id);
say("Where that payment stands", story.standing);

// Cash through the day, then count the till.
await z.cash.add({ branchId: branch.id, kind: "float", amountMinor: 200_000 });
await z.cash.add({ branchId: branch.id, kind: "sale", amountMinor: 85_050 });
await z.cash.add({ branchId: branch.id, kind: "refund", amountMinor: 5_000 });
const today = new Date(Date.now() + 330 * 60_000).toISOString().slice(0, 10);
const closing = await z.closings.create({ branchId: branch.id, day: today, countedCashMinor: 279_000 });
say("End of day", `expected ${rupees(closing.expected_cash_minor)}, counted ${rupees(closing.counted_cash_minor)}, difference ${rupees(closing.difference_minor)}; waiting for a second person to sign off (${closing.status})`);

const summary = await z.counters.branchSummary(branch.id);
say("Collected digitally in the last 30 days", summary.data.length ? summary.data : "nothing captured yet (the payment above is still waiting for the customer)");
