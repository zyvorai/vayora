// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

/** Paise -> "₹1,23,456.78" with Indian digit grouping. */
export function inr(paise: number): string {
  const sign = paise < 0 ? "-" : "";
  const abs = Math.abs(paise);
  const rupees = Math.floor(abs / 100);
  const frac = String(abs % 100).padStart(2, "0");
  return `${sign}₹${rupees.toLocaleString("en-IN")}.${frac}`;
}

/** Compact rupee label for chart axes: ₹1.2L, ₹45K, ₹900. */
export function inrCompact(paise: number): string {
  const r = paise / 100;
  if (r >= 1e7) return `₹${trim(r / 1e7)}Cr`;
  if (r >= 1e5) return `₹${trim(r / 1e5)}L`;
  if (r >= 1e3) return `₹${trim(r / 1e3)}K`;
  return `₹${trim(r)}`;
}
const trim = (n: number) => String(Math.round(n * 10) / 10);

/**
 * Parse a user-typed rupee amount ("499", "499.5", "1,200.00") into integer paise.
 * Returns null for anything else, so a typo can never become a different amount.
 */
export function parseRupees(input: string): number | null {
  const m = /^(\d{1,9})(?:\.(\d{1,2}))?$/.exec(input.trim().replace(/,/g, ""));
  if (!m) return null;
  const paise = Number(m[1]) * 100 + Number((m[2] ?? "").padEnd(2, "0") || 0);
  return paise > 0 ? paise : null;
}

const IST = "Asia/Kolkata";
export function istDateTime(iso: string): string {
  return new Intl.DateTimeFormat("en-IN", { timeZone: IST, day: "numeric", month: "short", hour: "numeric", minute: "2-digit", hour12: true }).format(new Date(iso));
}
export function istDay(isoDate: string): string {
  return new Intl.DateTimeFormat("en-IN", { timeZone: "UTC", day: "numeric", month: "short" }).format(new Date(isoDate + "T00:00:00Z"));
}

export function pct(rate: number | null): string {
  return rate === null ? "n/a" : `${(rate * 100).toFixed(rate === 1 || rate === 0 ? 0 : 1)}%`;
}

/** "sk_test_abcdef123456" -> "sk_test_••••3456" */
export function maskKey(key: string): string {
  return key.length <= 12 ? "••••" : `${key.slice(0, key.lastIndexOf("_") + 1)}••••${key.slice(-4)}`;
}

/** "2026-10-01" as "1 Oct 2026". A day with no time, so no time zone shifts it. */
export function dayYear(isoDate: string): string {
  const d = /^\d{4}-\d{2}-\d{2}/.test(isoDate) ? new Date(`${isoDate.slice(0, 10)}T00:00:00Z`) : null;
  return d && !Number.isNaN(d.getTime()) ? new Intl.DateTimeFormat("en-IN", { timeZone: "UTC", day: "numeric", month: "short", year: "numeric" }).format(d) : isoDate;
}

/** Basis points as a percentage in whole hundredths, with no float arithmetic: 150 is "1.5%", 175 "1.75%", 200 "2%". */
export function bpsPercent(bps: number): string {
  const whole = Math.trunc(bps / 100);
  const rest = Math.abs(bps % 100);
  const frac = rest === 0 ? "" : `.${String(rest).padStart(2, "0").replace(/0$/, "")}`;
  return `${whole}${frac}%`;
}

/** What a person typed as a percentage ("1.5") as basis points (150). Up to two decimals; null for anything else. */
export function parsePercentBps(input: string, max = 100): number | null {
  const m = /^(\d{1,3})(?:\.(\d{1,2}))?$/.exec(input.trim());
  if (!m) return null;
  const bps = Number(m[1]) * 100 + Number((m[2] ?? "").padEnd(2, "0") || 0);
  return bps <= max * 100 ? bps : null;
}

/** Like `parseRupees` but zero is allowed (a flat fee of nothing, a tolerance of nothing). */
export function parseRupeesOrZero(input: string): number | null {
  const t = input.trim().replace(/,/g, "");
  if (/^0+(\.0{1,2})?$/.test(t)) return 0;
  return parseRupees(t);
}
