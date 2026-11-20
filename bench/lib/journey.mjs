// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// One customer payment, end to end, with the same retry policy for every arm of a comparison:
// up to `maxAttempts` attempts; a retry only after a definite failure; an unknown outcome is asked about once.
import { call, createIntent, startAttempt, payerApproves, getAttempt, refreshAttempt, now, ok } from "./common.mjs";

export async function journey(t, { pinned, maxAttempts = 3, lookup = true }) {
  const t0 = now();
  const intent = await createIntent(t, { pinned });
  const log = { intent, attempts: [], end: null, via: null, ms: 0, unknownSeen: 0 };
  for (let n = 1; n <= maxAttempts; n++) {
    const r = await startAttempt(t, intent);
    if (r.status === 409) {
      log.attempts.push({ n, provider: null, status: "refused_" + (r.body?.error?.code ?? "409") });
      log.end = "unknown"; // the API refuses a new attempt because an earlier outcome is unresolved or live
      break;
    }
    if (r.status >= 300) {
      log.attempts.push({ n, provider: null, status: "http_" + r.status });
      log.end = "failed";
      break;
    }
    let a = r.body;
    const first = { n, provider: a.provider, status: a.status, outcome: a.route?.considered?.find((c) => c.provider === "razorpay")?.outcome };
    log.attempts.push(first);
    if (a.status === "unknown") {
      log.unknownSeen++;
      if (lookup) {
        const rr = await refreshAttempt(t, a.id);
        a = rr.body ?? a;
        first.afterLookup = a.status;
      }
      if (a.status === "unknown") {
        log.end = "unknown"; // unresolved: nothing more may be done for this payment (that is the rule)
        break;
      }
    }
    if (a.status === "pending" || a.status === "created") {
      await payerApproves(t, a);
      a = await getAttempt(t, a.id);
      first.afterPay = a.status;
    }
    if (a.status === "captured" || a.status === "authorized") {
      log.end = "captured";
      log.via = a.provider;
      break;
    }
    // failed: retry
    log.end = "failed";
  }
  log.ms = now() - t0;
  return log;
}
