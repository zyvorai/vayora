// A factory bills a purchase order in milestones: mobilisation, delivery, commissioning.
import { client, rupees, run, say } from "./_client.mjs";

const z = client();

const buyer = await z.customers.create({
  name: "Acme Industrial Pvt Ltd",
  externalRef: `PO-BUYER-${run}`,
  gstin: "29AABCT1332L1ZB",
});
say("Buyer", buyer);

const m1 = await z.milestones.create({
  customerId: buyer.id,
  name: "Mobilisation advance",
  amountMinor: 5_00_000_00,
  dueDays: 7,
  externalRef: `PO-${run}-M1`,
});
const m2 = await z.milestones.create({
  customerId: buyer.id,
  name: "On delivery",
  amountMinor: 12_00_000_00,
  dueDays: 45,
  externalRef: `PO-${run}-M2`,
});
const m3 = await z.milestones.create({
  customerId: buyer.id,
  name: "Commissioning",
  amountMinor: 3_00_000_00,
  dueDays: 90,
  externalRef: `PO-${run}-M3`,
});
say("Milestones", [m1, m2, m3].map((m) => m.id).join(", "));

// Accept the first milestone → raises an invoice (idempotent if you call complete again).
const inv = await z.milestones.complete(m1.id);
say("Mobilisation invoice", inv);

const link = await z.paymentLinks.create({
  description: `PO ${run} mobilisation`,
  amountMinor: 5_00_000_00,
  externalRef: `PO-${run}-LINK-M1`,
});
say("Share this link for milestone 1", link.path ?? link.id);

say("Statement", await z.billing.statement(buyer.id));
