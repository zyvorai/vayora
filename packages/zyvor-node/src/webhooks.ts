// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

import { createHmac, timingSafeEqual } from "node:crypto";

/** Why a webhook was refused. `signature_mismatch` is checked before the clock, so a forged request never learns the tolerance. */
export type WebhookFailure = "malformed_header" | "signature_mismatch" | "timestamp_outside_tolerance";
export type WebhookVerdict = { ok: true; timestamp: number } | { ok: false; reason: WebhookFailure };

/** A signature header longer than this is refused unread; real ones are about 80 bytes. */
export const MAX_SIGNATURE_HEADER_BYTES = 1024;
/** At most this many `v1` entries are tried (a rotation sends one per live secret). */
export const MAX_V1_ENTRIES = 8;

/**
 * Verify a `Zyvor-Signature: t=<unix>,v1=<hex hmac-sha256("{t}.{body}")>` header against the RAW request body, and say why it failed.
 *
 * Compared with {@link verifyWebhook} (which stays as it was): every `v1` entry is tried, so a header carrying two signatures
 * during a secret rotation verifies if either matches; unknown schemes (`v0`, ...) are ignored; the timestamp must be plain
 * digits (no `1e3`, no sign); a header over {@link MAX_SIGNATURE_HEADER_BYTES} is refused; and a tolerance that is not a
 * positive finite number throws `RangeError` instead of silently disabling the replay check.
 *
 * A valid signature does not stop a replay inside the tolerance window: also de-duplicate on the event id ({@link WebhookReplayGuard}).
 */
export function verifyWebhookDetailed(opts: { secret: string; header: string; rawBody: string | Buffer; toleranceSeconds?: number; nowSeconds?: number }): WebhookVerdict {
  const tolerance = opts.toleranceSeconds ?? 300;
  if (!Number.isFinite(tolerance) || tolerance <= 0) throw new RangeError("toleranceSeconds must be a positive number");
  if (Buffer.byteLength(opts.header) > MAX_SIGNATURE_HEADER_BYTES) return { ok: false, reason: "malformed_header" };
  let t: number | undefined;
  const candidates: Buffer[] = [];
  for (const part of opts.header.split(",")) {
    const eq = part.indexOf("=");
    if (eq < 0) continue;
    const key = part.slice(0, eq);
    const value = part.slice(eq + 1);
    if (key === "t") {
      if (t !== undefined || !/^\d{1,15}$/.test(value)) return { ok: false, reason: "malformed_header" };
      t = Number(value);
    } else if (key === "v1") {
      if (!/^(?:[0-9a-fA-F]{2})+$/.test(value) || candidates.length >= MAX_V1_ENTRIES) return { ok: false, reason: "malformed_header" };
      candidates.push(Buffer.from(value, "hex"));
    }
  }
  if (t === undefined || candidates.length === 0) return { ok: false, reason: "malformed_header" };
  const expected = createHmac("sha256", opts.secret).update(`${t}.`).update(opts.rawBody).digest();
  // no early exit: the work does not depend on which entry matched
  let match = false;
  for (const c of candidates) match = (c.length === expected.length && timingSafeEqual(c, expected)) || match;
  if (!match) return { ok: false, reason: "signature_mismatch" };
  const now = opts.nowSeconds ?? Math.floor(Date.now() / 1000);
  if (Math.abs(now - t) > tolerance) return { ok: false, reason: "timestamp_outside_tolerance" };
  return { ok: true, timestamp: t };
}

/**
 * Remember event ids for a while so a delivery that arrives twice (retries are at least once) or a captured request that is
 * replayed inside the timestamp tolerance is handled once. In memory and per process: with several instances, or to survive a
 * restart, record the id in your database under a unique constraint instead and treat the constraint violation as "seen".
 */
export class WebhookReplayGuard {
  private readonly seenUntil = new Map<string, number>();
  constructor(private readonly o: { ttlSeconds?: number; maxEntries?: number } = {}) {
    if (o.ttlSeconds !== undefined && !(o.ttlSeconds > 0)) throw new RangeError("ttlSeconds must be positive");
    if (o.maxEntries !== undefined && !(Number.isInteger(o.maxEntries) && o.maxEntries > 0)) throw new RangeError("maxEntries must be a positive integer");
  }
  /** True when `eventId` was already seen (skip it); false the first time (and it is now remembered). Call it only after the signature verified. */
  seen(eventId: string, nowSeconds: number = Math.floor(Date.now() / 1000)): boolean {
    for (const [id, until] of this.seenUntil) {
      if (until > nowSeconds) break; // insertion order is expiry order for a fixed ttl
      this.seenUntil.delete(id);
    }
    if (this.seenUntil.has(eventId)) return true;
    this.seenUntil.set(eventId, nowSeconds + (this.o.ttlSeconds ?? 24 * 3600));
    const max = this.o.maxEntries ?? 100_000;
    while (this.seenUntil.size > max) this.seenUntil.delete(this.seenUntil.keys().next().value as string);
    return false;
  }
  get size(): number {
    return this.seenUntil.size;
  }
}
