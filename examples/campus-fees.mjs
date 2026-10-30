// A school collects an annual fee in three instalments, chases late payers, and gives parents one link.
import { client, rupees, run, say } from "./_client.mjs";

const z = client();

// the student is a customer, found later by your own reference (the admission number)
const student = await z.customers.create({ name: `Aarav Sharma (Class 4)`, externalRef: `ADM-${run}`, phone: "+919800000000" });
say("Student", student);

// ₹45,000 across Term 1-3, due on the 1st of each month from December. The remainder, if any, lands on the last.
const plan = await z.installments.create({
  customerId: student.id,
  name: "Annual fee 2026-27",
  totalMinor: 4_500_000,
  instalments: 3,
  firstDueDate: "2026-12-01",
  numberPrefix: `FEE-${run}`,
  externalRef: `FEEPLAN-ADM-${run}`, // repeating this script's request returns the same plan, never a second one
});
say("Instalments", plan.invoices.map((i) => `${i.number}  ${rupees(i.amount_minor)}  due ${i.due_date}`).join("\n"));

// one reusable link for the whole class, capped, paying into the first term's amount
const link = await z.paymentLinks.create({ description: "Class 4 trip fee", amountMinor: 120_000, maxUses: 60, convenienceFeeBp: 0 });
say("Payment link (share it, or print its QR with GET /v1/payment_links/{id}/qr.svg)", link.path);

// a parent who promised to pay by Friday is recorded, and the dunning run skips them until then
const first = plan.invoices[0];
const promise = await z.collections.promise(first.invoice_id, { promisedMinor: first.amount_minor, promisedDate: "2026-12-05", note: "Father will pay Friday" });
say("Promise to pay", promise);

// a reminder is drafted for a person to read before anything is sent
const draft = await z.collections.draft(plan.invoices[1].invoice_id, { channel: "whatsapp" }).catch((e) => ({ note: e.message }));
say("Reminder draft (a person approves it in the console)", draft);

say("What the student owes (the statement)", await z.billing.statement(student.id));
