// The scripted "developer time to first payment" steps that run against an already started local stack.
// Usage: node firstpayment.mjs <out.json>   (BENCH_T_START / _CLONED / _BUILT / _STACK_UP are epoch ms stamps from devtime.sh)
import { newTenant, setRoutingFees, createIntent, startAttempt, payerApproves, getAttempt, writeJson } from "./common.mjs";

const T = (k) => (process.env[k] ? Number(process.env[k]) : null);
const t0 = T("BENCH_T_START"), tClone = T("BENCH_T_CLONED"), tBuilt = T("BENCH_T_BUILT"), tUp = T("BENCH_T_STACK_UP");
const mark = {};
const lap = (n) => (mark[n] = Date.now());

const t = await newTenant("first payment"); // creates the organisation, a test key, and connects both simulated providers
lap("tenant_and_providers");
await setRoutingFees(t);
const intent = await createIntent(t, { amount: 249_900 });
lap("intent");
const r = await startAttempt(t, intent);
if (r.status !== 201) throw new Error("attempt: " + r.text);
lap("attempt");
await payerApproves(t, r.body);
const a = await getAttempt(t, r.body.id);
lap("captured");
if (a.status !== "captured") throw new Error("not captured: " + a.status);

const secs = (x, y) => (x === null || y === null ? null : Math.round((x - y) / 100) / 10);
await writeJson(process.argv[2], {
  description: "Time from a clean checkout to the first captured SANDBOX payment (simulated providers, local machine)",
  stages_seconds: {
    clone: secs(tClone, t0),
    build: secs(tBuilt, tClone),
    database_migrations_and_services_start: secs(tUp, tBuilt),
    tenant_key_and_provider_setup: secs(mark.tenant_and_providers, tUp),
    intent_attempt_payer_approves_captured: secs(mark.captured, mark.tenant_and_providers),
  },
  total_seconds_to_first_captured_payment: secs(mark.captured, t0),
  total_seconds_excluding_compile: t0 === null ? null : secs(mark.captured, t0) - secs(tBuilt, tClone),
  api_calls_after_stack_up: 5,
});
console.log(`first payment captured: total ${secs(mark.captured, t0)} s`);
