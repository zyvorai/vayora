// A pharmacy sells with GST, records a return as a credit note, and keeps risk limits tight.
import { client, rupees, run, say } from "./_client.mjs";

const z = client();

await z.settings.update({
  gst: { gstin: "27AAPFU0939F1ZV", legal_name: "Neighbourhood Pharmacy" },
  risk: { max_intent_amount_minor: 50_000_00, max_attempts_per_intent: 3 },
});

const patient = await z.customers.create({
  name: "Walk-in counter",
  externalRef: `RX-${run}`,
});

const sale = await z.invoices.create({
  number: `RXINV-${run}`,
  customerId: patient.id,
  placeOfSupply: "27",
  lines: [
    { description: "Paracetamol 500mg (strip)", hsnSac: "300490", quantity: 2, unitPriceMinor: 3_500, rateBp: 1200 },
    { description: "ORS sachets", hsnSac: "210690", quantity: 5, unitPriceMinor: 1_200, rateBp: 1200 },
  ],
  dueDate: "2026-11-07",
});
say("Sale", `${rupees(sale.amount_minor)} incl. ${rupees(sale.tax_minor)} GST`);

const cn = await z.invoices.creditNote(sale.id, {
  amountMinor: Math.round(sale.amount_minor * 0.25),
  reason: "Customer returned unopened strip",
});
say("Return / credit note", cn);

const link = await z.paymentLinks.create({
  description: `RX ${run}`,
  amountMinor: sale.amount_minor - Math.round(sale.amount_minor * 0.25),
  externalRef: `RXPAY-${run}`,
});
say("Counter QR / link", link.path ?? link.id);
say("Statement", await z.billing.statement(patient.id));
