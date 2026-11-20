// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Redaction for everything the server sends back to a model. It mirrors the rules in
// crates/zyvor-api/src/redact.rs (key prefixes, bearer tokens, card-number-like runs, UPI ids) and adds the
// personal data a payments API returns (emails, phone numbers, PAN, Aadhaar-shaped numbers) plus a key-based pass
// for structured data. It is best effort: the first defence is that tools never ask for secrets or personal data.

const KEY_PREFIXES = ["zk_test_", "zk_live_", "rzp_test_", "rzp_live_", "sk_test_", "sk_live_", "whsec_"];
const isKeyChar = (c: string) => /[A-Za-z0-9_\-.+/=]/.test(c);

/** Replace the run of key characters after each case-insensitive prefix (same behaviour as the API's redactor). */
function redactPrefixed(s: string, prefixes: string[]): string {
  let out = "";
  let i = 0;
  while (i < s.length) {
    const hit = prefixes.find((p) => s.slice(i, i + p.length).toLowerCase() === p);
    if (hit) {
      out += s.slice(i, i + hit.length) + "[REDACTED]";
      i += hit.length;
      while (i < s.length && isKeyChar(s[i])) i++;
      continue;
    }
    out += s[i];
    i++;
  }
  return out;
}

/** Runs of 13 to 19 digits (spaces and dashes allowed between them) keep only the last four. A run inside a longer token (a hex link token, a reference) is left alone. */
function redactPans(s: string): string {
  return s.replace(/(?<![A-Za-z0-9])\d(?:[ -]?\d){12,18}(?![A-Za-z0-9])/g, (m) => {
    const d = m.replace(/\D/g, "");
    return d.length >= 13 && d.length <= 19 ? `****${d.slice(-4)}` : m;
  });
}

/** `name@okbank` becomes `[REDACTED]@okbank`. UPI handles are alphabetic bank codes, not email domains. */
function redactVpas(s: string): string {
  return s.replace(/([A-Za-z0-9._-]{2,})@([A-Za-z]{2,})(?![A-Za-z0-9.])/g, "[REDACTED]@$2");
}

function redactEmails(s: string): string {
  return s.replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+/g, "[email]");
}

/** Ten-digit Indian mobile numbers (optionally +91). A 12-digit bank reference is left alone. */
function redactPhones(s: string): string {
  return s.replace(/(?<![A-Za-z0-9])(?:\+91[ -]?)?[6-9]\d{9}(?![A-Za-z0-9])/g, (m) => `[phone ending ${m.slice(-4)}]`);
}

function redactIdNumbers(s: string): string {
  return s
    .replace(/\b[A-Z]{5}\d{4}[A-Z]\b/g, "[PAN]")
    .replace(/(?<![A-Za-z0-9])\d{4}[ -]\d{4}[ -]\d{4}(?![A-Za-z0-9])/g, "[ID number]");
}

/** Characters that hide or reorder text: controls, zero-width, bidi overrides. They are removed from free text. */
// eslint-disable-next-line no-control-regex
const INVISIBLE = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f​-‏‪-‮⁠-⁤⁦-⁩﻿]/g;

export const MAX_TEXT = 300;

/** Secrets, card-like numbers, UPI ids, emails, phones and ID numbers in a string. */
export function redactText(s: string): string {
  let t = redactPrefixed(s, KEY_PREFIXES);
  t = redactPrefixed(t, ["bearer "]);
  t = redactPans(t);
  t = redactEmails(t);
  t = redactVpas(t);
  t = redactPhones(t);
  return redactIdNumbers(t);
}

/**
 * Text a third party wrote (a payment description, a customer or bank narration) is data, never instructions. Besides
 * redaction this removes invisible and control characters, collapses line breaks (so text cannot pose as a new
 * paragraph or a new message) and caps the length.
 */
export function sanitizeText(s: string, max = MAX_TEXT): string {
  const t = redactText(s.replace(INVISIBLE, "").replace(/\s+/g, " ").trim());
  return t.length > max ? `${t.slice(0, max)}... [cut at ${max} characters]` : t;
}

/** Keys whose value is a secret: replaced whatever it holds. */
const SECRET_KEY = /secret|password|passwd|authorization|credential|api_?key|signature|token|private_key|(^|_)(otp|cvv|pin|key)($|_)/i;
/** Keys whose value is bulky or raw provider material: replaced with a short marker. */
const OMIT_KEY = /^(raw|raw_response|raw_payload|client_action|webhook|headers)$/i;
/** Keys that hold a person's contact or identity details. */
const PERSONAL_KEY = /^(email|phone|mobile|customer_name|payer_name|buyer_name|beneficiary_name|account_holder|holder_name|vpa|payer_vpa|upi_id|pan|aadhaar|dob|address\d?|address_line\d?)$/i;
const ACCOUNT_KEY = /^(account_number|account_no|beneficiary_account|card_number)$/i;
/** Free text written by someone outside the business (a payer, a bank, an upstream system). */
const FREE_TEXT_KEY = /^(description|narration|note|notes|reason|remarks|memo|purpose|resolution|label|failure_reason|statement_descriptor|title|message)$/i;

export interface RedactOptions {
  /** Show bank narrations (redacted) instead of withholding them. Off by default: narrations often hold names. */
  showNarrations?: boolean;
}

function maskTail(v: string): string {
  const d = v.replace(/\s/g, "");
  return d.length <= 4 ? "[REDACTED]" : `[REDACTED]${d.slice(-4)}`;
}

function walk(v: unknown, key: string | undefined, o: RedactOptions, depth: number): unknown {
  if (depth > 12) return "[too deep]";
  if (key !== undefined) {
    if (SECRET_KEY.test(key)) return v == null ? v : "[REDACTED]";
    if (OMIT_KEY.test(key)) return v == null ? v : "[omitted]";
    if (ACCOUNT_KEY.test(key)) return typeof v === "string" ? maskTail(v) : v == null ? v : "[REDACTED]";
    if (PERSONAL_KEY.test(key)) return v == null ? v : "[redacted: personal data]";
    if (/^narration$/i.test(key) && !o.showNarrations) return v == null ? v : "[withheld: bank narrations can hold names; the operator can enable them]";
  }
  if (typeof v === "string") return key !== undefined && FREE_TEXT_KEY.test(key) ? sanitizeText(v) : sanitizeText(v, 1000);
  if (Array.isArray(v)) return v.slice(0, 200).map((x) => walk(x, undefined, o, depth + 1));
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) out[k] = walk(x, k, o, depth + 1);
    return out;
  }
  return v;
}

/** Redact a parsed API response before it is shown to a model. Never mutates its input. */
export function redactData(v: unknown, o: RedactOptions = {}): unknown {
  return walk(v, undefined, o, 0);
}
