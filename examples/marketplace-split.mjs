// A marketplace takes one payment, owes shares to two sellers, and pays them out in a batch.
import { client, rupees, run, say } from "./_client.mjs";

const z = client();

const sellers = [];
for (const [name, vpa] of [[`Artisan Weavers ${run}`, "weavers@okbank"], [`Safe Couriers ${run}`, "couriers@okbank"]]) {
  sellers.push(await z.payouts.createVendor({ name, vpa, externalRef: `${name}`.replace(/\s+/g, "-") }));
}
say("Sellers", sellers);

// ₹2,000 order: ₹1,400 to the weaver, ₹300 to the courier, ₹300 is the marketplace's commission.
const intent = await z.intents.create({
  amountMinor: 200_000,
  externalRef: `ORDER-${run}`,
  splits: [{ vendorId: sellers[0].id, amountMinor: 140_000 }, { vendorId: sellers[1].id, amountMinor: 30_000 }],
});
say("Order payment (shares accrue to sellers when it is captured, shrink if it is refunded)", { id: intent.id, amount: rupees(intent.amount_minor) });

for (const s of sellers) say(`Balance of ${s.name}`, await z.payouts.vendorBalance(s.id)); // awaiting_capture_minor until paid

// month-end payouts and safe to run twice (same references = no new payouts)
const batch = await z.payouts.createBulk(sellers.map((s, i) => ({ vendorId: s.id, amountMinor: 50_000 * (i + 1), purpose: "October settlement adjustment", externalRef: `OCT-${run}-${i}` })));
say("Bulk payouts waiting for a second person to approve", batch.data.map((p) => `${p.id}  ${rupees(p.amount_minor)}  ${p.status}`).join("\n"));
const again = await z.payouts.createBulk(sellers.map((s, i) => ({ vendorId: s.id, amountMinor: 50_000 * (i + 1), purpose: "October settlement adjustment", externalRef: `OCT-${run}-${i}` })));
say("Running it again", `${again.already_existed} already existed, ${again.new_total_minor} new`);
