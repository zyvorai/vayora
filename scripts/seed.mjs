#!/usr/bin/env node
// Dev-only demo data. Drives the real API through @vayora/node, then (if DATABASE_URL and psql
// are available) spreads timestamps over 14 days and varies webhook states so the dashboard has
// something to show. Never run against a database that holds real data.
//
//   VAYORA_API_KEY=sk_test_x DATABASE_URL=postgres://... node scripts/seed.mjs [count]
import { spawnSync } from "node:child_process";
import { Vayora } from "../packages/node/dist/index.js";

const count = Number(process.argv[2] ?? 60);
const v = new Vayora({ apiKey: process.env.VAYORA_API_KEY ?? "sk_test_123", baseUrl: process.env.VAYORA_API_URL ?? "http://localhost:8080" });
const run = Date.now().toString(36);
const amounts = [7900, 12900, 19900, 29900, 49900, 99900, 149900, 249900, 399900];
const names = ["asha", "ravi.k", "meera", "arjun", "priya.s", "karan", "divya", "sanjay"];
const psps = ["okvayora", "ybl", "paytm", "axl"];

let captured = 0, failed = 0, refunded = 0;
for (let i = 0; i < count; i++) {
  const amountPaise = amounts[(i * 7) % amounts.length];
  const handle = i % 7 === 3 ? "fail" : i % 11 === 5 ? "timeout" : names[i % names.length];
  const vpa = `${handle}${i % 5 === 0 ? i : ""}@${psps[i % psps.length]}`;
  const order = await v.orders.create({ amountPaise, idempotencyKey: `seed-${run}-o${i}` });
  const pay = await v.payments.create({ orderId: order.id, vpa, idempotencyKey: `seed-${run}-p${i}` });
  if (pay.status === "failed") {
    failed++;
    if (i % 3 === 0) { // some customers retry successfully
      await v.payments.create({ orderId: order.id, vpa: "asha@okvayora", idempotencyKey: `seed-${run}-p${i}b` });
      captured++;
    }
    continue;
  }
  captured++;
  if (i % 5 === 1) { await v.refunds.create({ paymentId: pay.id, amountPaise: Math.floor(amountPaise / 4 / 100) * 100, idempotencyKey: `seed-${run}-r${i}` }); refunded++; }
  else if (i % 13 === 2) { await v.refunds.create({ paymentId: pay.id, idempotencyKey: `seed-${run}-r${i}` }); refunded++; }
}
console.log(`seeded via API: ${captured} captured, ${failed} failed attempts, ${refunded} refunds`);

const db = process.env.DATABASE_URL;
if (!db) { console.log("DATABASE_URL not set: skipped timestamp spreading and webhook states."); process.exit(0); }
const sql = `
UPDATE orders SET created_at = now() - make_interval(hours => abs(hashtext(id)) % 312);
UPDATE payments p SET created_at = o.created_at + interval '12 seconds' FROM orders o WHERE o.id = p.order_id;
UPDATE refunds r SET created_at = LEAST(now(), p.created_at + interval '3 hours') FROM payments p WHERE p.id = r.payment_id;
UPDATE webhook_events SET status = 'delivered', delivered_at = created_at + interval '1 second', attempts = 1 WHERE abs(hashtext(id)) % 10 < 8;
UPDATE webhook_events SET status = 'dead', attempts = 10, last_error = 'error sending request: connection refused' WHERE abs(hashtext(id)) % 3 = 0 AND status = 'pending';
`;
const r = spawnSync("psql", [db, "-v", "ON_ERROR_STOP=1", "-q", "-c", sql], { encoding: "utf8" });
if (r.status !== 0) { console.error(r.stderr || r.error); process.exit(1); }
console.log("spread timestamps over 14 days and varied webhook states (dev only).");
