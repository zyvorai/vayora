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
