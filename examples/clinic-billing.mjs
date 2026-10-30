// A clinic bills a treatment with GST, takes a refundable deposit, and keeps its own risk limits.
import { client, rupees, run, say } from "./_client.mjs";

const z = client();

// 1. Register the clinic's GSTIN once (an admin key is needed). Use your own; this is a published sample.
await z.settings.update({ gst: { gstin: "27AAPFU0939F1ZV", legal_name: "Acme Clinic" } });

// 2. A patient, and a treatment invoice: healthcare services are SAC 999311, GST-exempt (0%) in most cases.
//    Check the rate that applies to each service with your accountant.
const patient = await z.customers.create({ name: "R. Menon", externalRef: `UHID-${run}` });
const bill = await z.invoices.create({
  number: `OPD-${run}`,
  customerId: patient.id,
  placeOfSupply: "27",
  lines: [{ description: "Consultation", hsnSac: "999311", quantity: 1, unitPriceMinor: 80_000, rateBp: 0 }, { description: "Dental scaling", hsnSac: "999312", quantity: 1, unitPriceMinor: 150_000, rateBp: 1800 }],
  dueDate: "2026-12-15",
});
say("Invoice", `${rupees(bill.amount_minor)} including ${rupees(bill.tax_minor)} GST`);
say("Tax breakdown", await z.invoices.lines(bill.id));

// 3. A refundable admission deposit is a payment with no invoice. It appears on the patient's statement when
//    captured, and a refund shows as a deposit refund. (Capturing needs a customer to pay: see docs/zyvor/ARCHITECTURE.md.)
const deposit = await z.intents.create({ amountMinor: 1_000_000, customerId: patient.id, externalRef: `DEP-${run}`, description: "Admission deposit" });
say("Deposit intent (give its client_secret to the checkout page)", { id: deposit.id, status: deposit.status });

// 4. Stop silly payments before they reach a provider.
await z.settings.update({ risk: { max_intent_amount_minor: 5_000_000, max_attempts_per_intent: 4 } });
say("Statement", await z.billing.statement(patient.id));
