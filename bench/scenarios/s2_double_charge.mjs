// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Scenario 2: double-charge check under timeouts, retries, concurrent identical requests and idempotency-key replays.
// The adversary here is the CLIENT (and the network): it retries aggressively, in parallel, with new and repeated
// idempotency keys, and the provider replays webhooks. The payer approves every attempt the platform shows as pending,
// and late approvals arrive after timeouts. Money only "leaves" when the simulated provider records a successful payment.
// Pass criterion: 0 intents paid more than once, checked in the platform's database AND in the provider's own records.
import { env, stats, newTenant, setRoutingFees, outage, sim, rng, subseed, call, createIntent, startAttempt, payerApproves, refreshAttempt, deliver, uniq, auditDoubleCharges, sqlScalar } from "../lib/common.mjs";

const K = env.quick ? 6 : 20;
const counters = { requests: 0, attempt_posts: 0, created_201: 0, refused_409: 0, same_key_replays: 0, webhook_deliveries: 0, refresh_calls: 0 };
const failures = []; // any unexpected observation is listed here, never hidden
const expect = (cond, msg) => {
  if (!cond) failures.push(msg);
};

async function attempt(t, intent, key) {
  counters.requests++;
  counters.attempt_posts++;
  const r = await startAttempt(t, intent, key);
  if (r.status === 201) counters.created_201++;
  if (r.status === 409) counters.refused_409++;
  return r;
}
async function webhook(t, provider, hook, times = 1) {
  for (let i = 0; i < times; i++) {
    counters.webhook_deliveries++;
    await deliver(provider, t.id, hook);
  }
}
async function lateApproval(t, a, outcome = "SUCCESS", times = 2) {
  const r = await sim("cashfree/pay", { order_id: a.id, outcome });
  await webhook(t, "cashfree", r.webhook, times);
}
const refresh = async (t, id) => {
  counters.refresh_calls++;
  return refreshAttempt(t, id);
};

export async function run() {
  const t = await newTenant("s2");
  await setRoutingFees(t);
  const rand = rng(subseed("s2"));
  const phases = {};

  // ---- P1: the provider accepted the payment but the answer never came back (timeout), then retries pile up ----
  {
    let n = 0;
    for (let i = 0; i < K; i++) {
      const intent = await createIntent(t, { pinned: ["cashfree"] });
      n++;
      await sim("behaviour", { op: "cashfree.pay", behaviour: "hang_after_accept" });
      const first = await attempt(t, intent);
      expect(first.body?.status === "unknown", `P1 first attempt expected unknown, got ${first.body?.status}`);
      const burst = await Promise.all([1, 2, 3, 4, 5].map(() => attempt(t, intent)));
      expect(burst.every((r) => r.status === 409), "P1 retries during unknown must all be refused");
      if (rand() < 0.5) {
        await lateApproval(t, first.body, "SUCCESS", 2); // the money moved; the webhook arrives twice
      } else {
        await lateApproval(t, first.body, "FAILED", 1); // definitively failed; only now may another attempt start
        const retry = await attempt(t, intent);
        if (retry.status === 201 && retry.body.status === "pending") await payerApproves(t, retry.body);
      }
      await refresh(t, first.body.id);
      const more = await attempt(t, intent); // whatever the state now, this must not create a second payment
      expect(more.status !== 201 || more.body.status !== "captured", "P1 no second capture");
    }
    phases.timeout_after_accept_then_parallel_retries = n;
  }

  // ---- P2: provider outage with unknown answers, client retries, lookup, failover, late approval ----
  {
    let n = 0;
    for (let i = 0; i < K; i++) {
      await outage("razorpay", rand.pick(["hang", "unavailable"]), 100, 1);
      const intent = await createIntent(t, { pinned: null });
      n++;
      const a1 = await attempt(t, intent);
      const burst = await Promise.all([1, 2, 3].map(() => attempt(t, intent)));
      expect(burst.every((r) => r.status === 409), "P2 no new attempt while unknown");
      await outage("razorpay", "off");
      const asked = await refresh(t, a1.body.id);
      if (asked.body?.status === "failed") {
        const a2 = await attempt(t, intent);
        if (a2.status === 201 && a2.body.status === "pending") await payerApproves(t, a2.body);
      }
      await refresh(t, a1.body.id);
      await attempt(t, intent);
    }
    phases.outage_503_or_hang_parallel_retries_lookup_failover_late_approval = n;
  }

  // ---- P3: concurrent identical requests ----
  {
    let n = 0;
    for (let i = 0; i < K; i++) {
      const intent = await createIntent(t, { pinned: null });
      n++;
      const sameKey = uniq("same");
      const same = await Promise.all(Array.from({ length: 8 }, () => attempt(t, intent, sameKey)));
      counters.same_key_replays += 7;
      const ids = new Set(same.filter((r) => r.status === 201).map((r) => r.body.id));
      expect(ids.size === 1, `P3 same key must give one attempt, got ${ids.size}`);
      const distinct = await Promise.all(Array.from({ length: 8 }, () => attempt(t, intent)));
      expect(distinct.every((r) => r.status === 409), "P3 distinct keys on a live attempt are refused");
      const a = same.find((r) => r.status === 201)?.body;
      if (a?.status === "pending") {
        const r = a.provider === "razorpay" ? await sim("razorpay/pay", { order_id: a.provider_order_ref, outcome: "captured" }) : await sim("cashfree/pay", { order_id: a.id, outcome: "SUCCESS" });
        await Promise.all([1, 2, 3].map(() => {
          counters.webhook_deliveries++;
          return deliver(a.provider, t.id, r.webhook);
        }));
      }
      const again = await attempt(t, intent, sameKey);
      expect(again.body?.id === a?.id, "P3 replay after capture returns the same attempt");
      counters.same_key_replays++;
      const after = await attempt(t, intent);
      expect(after.status === 409, "P3 new attempt on a paid intent refused");
    }
    phases.eight_parallel_same_key_and_eight_distinct_keys_repeated_webhooks = n;
  }

  // ---- P4: idempotency-key replays on intent creation ----
  {
    let n = 0;
    for (let i = 0; i < K; i++) {
      const key = uniq("pi");
      const body = { amount_minor: 10_000 + i, currency: "INR", allowed_providers: null };
      const rs = await Promise.all(
        Array.from({ length: 5 }, () => {
          counters.requests++;
          return call(t.key, "POST", "/v1/payment_intents", body, { idem: key });
        }),
      );
      counters.same_key_replays += 4;
      const ids = new Set(rs.filter((r) => r.status < 300).map((r) => r.body.id));
      expect(ids.size === 1, `P4 intent replays must return one intent, got ${ids.size}`);
      n++;
      counters.requests++;
      const different = await call(t.key, "POST", "/v1/payment_intents", { ...body, amount_minor: body.amount_minor + 1 }, { idem: key });
      expect(different.status >= 400, "P4 same key with a different body must be refused");
      const intent = [...ids][0];
      const a = await attempt(t, intent);
      if (a.body?.status === "pending") await payerApproves(t, a.body);
    }
    phases.intent_creation_replayed_five_times_per_key_and_with_a_different_body = n;
  }

  // ---- P5: seeded chaos: random faults on either provider with random client behaviour ----
  {
    let n = 0;
    for (let i = 0; i < K * 2; i++) {
      const prov = rand.pick(["razorpay", "cashfree"]);
      await outage(prov, rand.pick(["reject", "unavailable", "hang", "off", "off"]), rand.pick([30, 60, 100]), rand.int(1, 1e6));
      const intent = await createIntent(t, { pinned: rand() < 0.3 ? [rand.pick(["razorpay", "cashfree"])] : null });
      n++;
      for (let step = 0; step < 4; step++) {
        const par = rand.int(1, 4);
        const rs = await Promise.all(Array.from({ length: par }, () => attempt(t, intent, rand() < 0.3 ? `chaos-${intent}` : undefined)));
        for (const r of rs) {
          if (r.status === 201 && r.body.status === "pending" && rand() < 0.8) await payerApproves(t, r.body);
          if (r.status === 201 && r.body.status === "unknown") await refresh(t, r.body.id);
        }
      }
      await outage("razorpay", "off");
      await outage("cashfree", "off");
    }
    phases.seeded_chaos_random_faults_parallelism_and_keys = n;
  }

  await outage("razorpay", "off");
  await outage("cashfree", "off");
  const audit = await auditDoubleCharges([t.id]);
  const mockStats = await sim("stats");
  return {
    description: "Double-charge check under timeouts, retries, concurrency and replays (simulator)",
    intents_per_phase: phases,
    requests: counters,
    attempts_recorded: sqlScalar(`SELECT count(*) FROM payment_attempts WHERE tenant_id = '${t.id}'`),
    requests_that_were_rate_limited_and_retried: stats.rate_limited_retries,
    provider_orders_created: { razorpay: mockStats.razorpay_orders, cashfree: mockStats.cashfree_orders },
    audit,
    unexpected_observations: failures,
    pass: audit.intents_with_more_than_one_captured_attempt_in_platform === 0 && audit.intents_paid_more_than_once_at_provider === 0 && failures.length === 0,
  };
}
