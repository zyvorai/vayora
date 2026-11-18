// Shared helpers for the benchmark scenarios. Node 20+ only, no dependencies.
// Everything here talks to a LOCAL Zyvor API and the SIMULATED provider mock. No real provider is involved.
import { execFileSync } from "node:child_process";

export const env = {
  api: process.env.BENCH_API ?? "http://127.0.0.1:18080",
  mock: process.env.BENCH_MOCK ?? "http://127.0.0.1:19090",
  ownerUrl: process.env.BENCH_OWNER_URL, // superuser connection, used only to READ ground truth with psql
  appUrl: process.env.BENCH_APP_URL, // restricted role the API itself uses (needed by `bootstrap`)
  bin: process.env.BENCH_BIN_DIR,
  seed: Number(process.env.BENCH_SEED ?? 20261118),
  quick: process.env.BENCH_QUICK === "1",
};

// ---- seeded random numbers (mulberry32): the same seed gives the same numbers on every machine ----
export function rng(seed) {
  let a = seed >>> 0;
  const next = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  next.int = (lo, hi) => lo + Math.floor(next() * (hi - lo + 1));
  next.pick = (xs) => xs[Math.floor(next() * xs.length)];
  next.shuffle = (xs) => {
    const a2 = xs.slice();
    for (let i = a2.length - 1; i > 0; i--) {
      const j = Math.floor(next() * (i + 1));
      [a2[i], a2[j]] = [a2[j], a2[i]];
    }
    return a2;
  };
  return next;
}
/** A seed for one named part of a scenario, derived from the run seed. */
export function subseed(name) {
  let h = env.seed >>> 0;
  for (const ch of name) h = (Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0);
  return h;
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const now = () => performance.now();

export function pct(xs, p) {
  if (!xs.length) return null;
  const s = xs.slice().sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
}
export const round = (x, d = 1) => (x === null || x === undefined ? x : Math.round(x * 10 ** d) / 10 ** d);
export const rate = (n, d) => (d === 0 ? null : round((100 * n) / d, 1));

// ---- HTTP against the Zyvor API ----
// The API limits requests per client address (300/s sustained for authenticated calls, no environment switch for it).
// Everything here comes from 127.0.0.1, so the load generator paces itself below that limit instead of changing the
// product: a token bucket of 250 requests/s (burst 600), and a wait-and-retry if the API still answers 429.
export const stats = { rate_limited_retries: 0 };
const bucket = { tokens: 600, last: performance.now() };
async function pace() {
  for (;;) {
    const t = performance.now();
    bucket.tokens = Math.min(600, bucket.tokens + ((t - bucket.last) / 1000) * 250);
    bucket.last = t;
    if (bucket.tokens >= 1) { bucket.tokens -= 1; return; }
    await sleep(4);
  }
}
export async function call(key, method, path, body, opts = {}) {
  for (let i = 0; i < 8; i++) {
    await pace();
    const r = await callOnce(key, method, path, body, opts);
    if (r.status !== 429) return r;
    stats.rate_limited_retries++;
    await sleep(1100);
  }
  throw new Error(`${method} ${path}: still rate limited after retries`);
}
async function callOnce(key, method, path, body, { idem, timeoutMs = 60000 } = {}) {
  const headers = { "content-type": "application/json" };
  if (key) headers.authorization = `Bearer ${key}`;
  if (idem) headers["idempotency-key"] = idem;
  const res = await fetch(env.api + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {
    /* not json */
  }
  return { status: res.status, body: json, text };
}
export async function ok(key, method, path, body, opts) {
  const r = await call(key, method, path, body, opts);
  if (r.status >= 300) throw new Error(`${method} ${path} -> ${r.status} ${r.text.slice(0, 300)}`);
  return r.body;
}

// ---- the simulated provider's control endpoints ----
export async function sim(path, body) {
  const res = await fetch(`${env.mock}/_sim/${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return res.json();
}
export const outage = (provider, mode, failPct, seed) =>
  sim("outage", mode === "off" ? { provider, mode } : { provider, mode, fail_pct: failPct, seed });

/** Deliver a provider webhook (as produced by the mock) to the platform, as the provider would. */
export async function deliver(provider, tenantId, hook) {
  const res = await fetch(`${env.api}/v1/webhooks/${provider}/${tenantId}`, {
    method: "POST",
    headers: hook.headers,
    body: hook.body,
  });
  return res.status;
}

// ---- Postgres (read-only ground truth, through psql) ----
export function sql(query) {
  return execFileSync("psql", [env.ownerUrl, "-X", "-q", "-At", "-F", "\t", "-c", query], { encoding: "utf8" })
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((l) => l.split("\t"));
}
export const sqlScalar = (q) => Number(sql(q)[0]?.[0] ?? 0);

// ---- tenants ----
let tenantCounter = 0;
/** A fresh organisation with a test tenant, an API key and both simulated providers connected. */
export async function newTenant(label) {
  const out = execFileSync(`${env.bin}/zyvor-api`, ["bootstrap", `bench ${label} ${++tenantCounter}`], {
    encoding: "utf8",
    env: { ...process.env, DATABASE_URL: env.appUrl, ZYVOR_MASTER_KEY: process.env.ZYVOR_MASTER_KEY },
    stdio: ["ignore", "pipe", "ignore"],
  });
  const tenant = out.match(/test tenant (\S+)/)[1];
  const key = out.match(/test tenant \S+\s+key: (\S+)/)[1];
  await ok(key, "POST", "/v1/provider_accounts", {
    provider: "razorpay",
    credentials: { key_id: "rzp_bench", key_secret: "rzs_bench" },
    webhook_secret: "rz_hook",
    base_url: env.mock,
  });
  await ok(key, "POST", "/v1/provider_accounts", {
    provider: "cashfree",
    credentials: { client_id: "cf_bench", client_secret: "cfs_bench" },
    webhook_secret: "cf_hook",
    base_url: env.mock,
  });
  return { id: tenant, key };
}

/** Make Razorpay the cheaper provider for UPI so it is the primary and Cashfree the alternate. (Rates are invented.) */
export async function setRoutingFees(t) {
  await ok(t.key, "POST", "/v1/settings", {
    routing: {
      rules: [],
      fees: {
        razorpay: { upi: { bps: 100, fixed_minor: 0 } },
        cashfree: { upi: { bps: 200, fixed_minor: 0 } },
      },
    },
  });
}

// ---- one payment, from the customer's side ----
let seq = 0;
export const uniq = (p) => `${p}-${process.pid}-${Date.now().toString(36)}-${++seq}`;

export async function createIntent(t, { amount = 50_000, pinned = null, idem } = {}) {
  const body = { amount_minor: amount, currency: "INR", allowed_providers: pinned };
  return (await ok(t.key, "POST", "/v1/payment_intents", body, { idem: idem ?? uniq("pi") })).id;
}

/** Start an attempt on an intent. Returns the raw response so callers can look at the status code and error. */
export function startAttempt(t, intentId, idem) {
  return call(t.key, "POST", `/v1/payment_intents/${intentId}/attempts`, { method: "upi", vpa: "bench@ybl" }, { idem: idem ?? uniq("at") });
}

/** The payer approves a pending attempt at the (simulated) provider, and the provider's webhook reaches the platform. */
export async function payerApproves(t, attempt) {
  if (attempt.provider === "razorpay") {
    const r = await sim("razorpay/pay", { order_id: attempt.provider_order_ref, outcome: "captured" });
    return deliver("razorpay", t.id, r.webhook);
  }
  const r = await sim("cashfree/pay", { order_id: attempt.id, outcome: "SUCCESS" });
  return deliver("cashfree", t.id, r.webhook);
}

export const getAttempt = (t, id) => ok(t.key, "GET", `/v1/payment_attempts/${id}`);
export const refreshAttempt = (t, id) => call(t.key, "POST", `/v1/payment_attempts/${id}/refresh`);

export async function writeJson(path, obj) {
  const { writeFileSync, mkdirSync } = await import("node:fs");
  const { dirname } = await import("node:path");
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, JSON.stringify(obj, null, 2) + "\n");
}

/**
 * Double-charge audit, from two independent sources.
 *  - the platform's own records: intents with more than one captured attempt;
 *  - the SIMULATED provider's records: intents whose orders were successfully paid more than once at the provider
 *    (this is the money that would actually have left the payer's account).
 * Also reports money the provider took that the platform has no captured attempt for.
 */
export async function auditDoubleCharges(tenantIds) {
  const only = tenantIds?.length ? ` WHERE tenant_id IN (${tenantIds.map((t) => `'${t}'`).join(",")})` : "";
  const stats = await sim("stats");
  const rz = stats.razorpay_captured_payments_by_order ?? {};
  const cf = stats.cashfree_successful_payments_by_order ?? {};
  const rows = sql("SELECT intent_id, id, provider, COALESCE(provider_order_ref,''), status FROM payment_attempts" + only);
  const byIntent = new Map();
  for (const [intent, id, provider, orderRef, status] of rows) {
    const e = byIntent.get(intent) ?? { providerPaid: 0, captured: 0, attempts: 0 };
    e.attempts++;
    if (status === "captured") e.captured++;
    e.providerPaid += provider === "razorpay" ? (rz[orderRef] ?? 0) : (cf[id] ?? 0) + (orderRef && orderRef !== id ? (cf[orderRef] ?? 0) : 0);
    byIntent.set(intent, e);
  }
  let multiCapturedDb = 0, multiPaidProvider = 0, paidNotRecorded = 0, paidIntents = 0;
  for (const e of byIntent.values()) {
    if (e.captured > 1) multiCapturedDb++;
    if (e.providerPaid > 1) multiPaidProvider++;
    if (e.providerPaid > 0) paidIntents++;
    if (e.providerPaid > 0 && e.captured === 0) paidNotRecorded++;
  }
  return {
    intents_with_attempts: byIntent.size,
    intents_paid_at_provider: paidIntents,
    intents_with_more_than_one_captured_attempt_in_platform: multiCapturedDb,
    intents_paid_more_than_once_at_provider: multiPaidProvider,
    intents_paid_at_provider_but_no_captured_attempt_in_platform: paidNotRecorded,
  };
}
