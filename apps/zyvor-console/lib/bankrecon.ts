import { dayYear, inr } from "@/lib/format";

/** Shapes the bank-statement endpoints send back (docs/zyvor/BANK_RECONCILIATION.md). Amounts are whole paise; a debit is negative. */
export type Layout = { name: string; about: string; mapping: Record<string, string | null>; date_formats: string[] };
export type Problem = { line: number; column?: string | null; message: string };
export type Preview = {
  ok: boolean; layout: string; rows: number; header_line?: number; new_rows?: number; duplicate_rows?: number;
  credits?: { count: number; total_minor: number }; debits?: { count: number; total_minor: number };
  period_from?: string | null; period_to?: string | null; already_imported_as?: number | null;
  errors: Problem[]; warnings: Problem[]; sample?: { line: number; posted_on: string; amount_minor: number; narration: string; reference: string | null }[];
};
export type ImportDone = { import_id: number; rows: number; added: number; duplicates: number; settlements_rechecked: number; matched: { confirmed: number; suggested: number }; cases_opened: number; warnings?: Problem[] };
export type ImportRow = { id: number; source: string; layout: string; filename: string | null; rows: number; added: number; duplicates: number; credits_minor: number; debits_minor: number; period_from: string | null; period_to: string | null; created_by: string; created_at: string };
export type Suggestion = { id: number; subject_type: string; subject_id: string; rule: string; evidence: Record<string, unknown> };
export type BankLine = {
  id: number; posted_on: string; amount_minor: number; direction: "credit" | "debit"; narration: string; reference: string | null; import_id: number | null; line_no: number | null;
  status: "matched" | "suggested" | "unmatched";
  match: { id?: number; subject_type: string; subject_id: string; rule: string; key_matched?: string | null; confirmed_by?: string | null } | null;
  suggestions: Suggestion[];
};
export type AgeingKind = { buckets: Record<string, number>; open: number; unassigned: number; oldest_days: number; amount_minor: number };
export type Ageing = { bucket_days: string[]; open: number; unassigned: number; oldest_days: number; by_kind: Record<string, AgeingKind> };
export type BankCase = { id: string; kind: string; status: "open" | "assigned" | "resolved"; subject_type: string; subject_id: string; expected_minor: number | null; actual_minor: number | null; explanation: string; assignee: string | null; age_days: number };

/** The kinds of case that come from comparing the bank with Zyvor's records. */
export const BANK_CASE_KINDS = ["unmatched_bank_credit", "bank_amount_mismatch", "settlement_no_bank_credit", "payout_no_bank_debit"] as const;

type FormLike = { get(name: string): FormDataEntryValue | null };
const val = (f: FormLike, k: string) => String(f.get(k) ?? "").trim();

/** The columns a person can name themselves, with the plain label shown for each. */
export const MAPPING_FIELDS: { key: string; label: string; hint: string }[] = [
  { key: "date", label: "Date column", hint: "The day the transaction was posted" },
  { key: "narration", label: "Description column", hint: "The text the bank writes about the transaction" },
  { key: "reference", label: "Reference column", hint: "UTR, RRN or cheque number, if there is a column for it" },
  { key: "credit", label: "Money in column", hint: "Credit or deposit amounts" },
  { key: "debit", label: "Money out column", hint: "Debit or withdrawal amounts" },
  { key: "amount", label: "Single amount column", hint: "Use this when money in and out share one column" },
  { key: "drcr", label: "Dr/Cr column", hint: "Only if one amount column comes with a Dr or Cr column" },
  { key: "balance", label: "Balance column", hint: "Optional; used for a warning when balances do not add up" },
];
export const DATE_FORMATS: { value: string; label: string }[] = [
  { value: "", label: "Let Zyvor work it out (day first)" },
  { value: "%d/%m/%Y", label: "01/10/2026" },
  { value: "%d-%m-%Y", label: "01-10-2026" },
  { value: "%d.%m.%Y", label: "01.10.2026" },
  { value: "%d %b %Y", label: "01 Oct 2026" },
  { value: "%d-%b-%Y", label: "01-Oct-2026" },
  { value: "%Y-%m-%d", label: "2026-10-01" },
];
export const DELIMITERS: { value: string; label: string }[] = [
  { value: "", label: "Work it out" }, { value: ",", label: "Comma" }, { value: ";", label: "Semicolon" }, { value: "|", label: "Bar |" }, { value: "tab", label: "Tab" },
];
/** Largest file the console sends: the API's own limit is 20,000 transactions, and a file much past this will not get through. */
export const MAX_STATEMENT_BYTES = 5 * 1024 * 1024;

/** The body for preview and import, from the upload form. Only what the person set is sent. */
export function statementBody(f: FormLike): { ok: true; body: Record<string, unknown> } | { ok: false; error: string } {
  const csv = String(f.get("csv") ?? "");
  if (!csv.trim()) return { ok: false, error: "Choose a statement file, or paste its text." };
  if (new TextEncoder().encode(csv).length > MAX_STATEMENT_BYTES) return { ok: false, error: "That file is too large to send in one go. Split it by month and upload each month." };
  const body: Record<string, unknown> = { csv };
  const filename = val(f, "filename");
  if (filename) body.filename = filename.slice(0, 200);
  const layout = val(f, "layout");
  body.layout = layout || "auto";
  const mapping: Record<string, string> = {};
  for (const { key } of MAPPING_FIELDS) {
    const v = val(f, `map_${key}`);
    if (v) mapping[key] = v;
  }
  if (Object.keys(mapping).length) body.mapping = mapping;
  const df = val(f, "date_format");
  if (df) {
    if (!DATE_FORMATS.some((d) => d.value === df)) return { ok: false, error: "Choose one of the listed date styles." };
    body.date_format = df;
  }
  const dl = val(f, "delimiter");
  if (dl) {
    if (!DELIMITERS.some((d) => d.value === dl)) return { ok: false, error: "Choose one of the listed separators." };
    body.delimiter = dl;
  }
  return { ok: true, body };
}

/** One problem as a sentence: "Line 12 (Date): ..." or just the message when it is about the file as a whole. */
export function problemText(p: Problem): string {
  const where = p.line > 0 ? `Line ${p.line}${p.column ? ` (${p.column})` : ""}: ` : p.column ? `${p.column}: ` : "";
  return `${where}${p.message}`;
}

/** The errors the API sends with a refused import (`error.errors`), when there are any. */
export function problemsOf(body: unknown, key: "errors" | "warnings" = "errors"): Problem[] {
  const list = (body as { error?: Record<string, unknown> } | null)?.error?.[key];
  return Array.isArray(list) ? list.filter((p): p is Problem => !!p && typeof (p as Problem).message === "string").map((p) => ({ line: Number(p.line) || 0, column: p.column ?? null, message: p.message })) : [];
}

const RULES: Record<string, string> = {
  reference_column: "The reference on the bank line is the one we hold",
  reference_in_narration: "The reference we hold is in the bank line's description",
  amount_date_window: "Same amount within a few days. No reference matched",
  reference_amount_differs: "The reference matches, but the amount is different",
  reference_ambiguous: "The reference is on more than one item",
  recorded_before_evidence: "Matched before Zyvor kept the evidence",
};
export const ruleLabel = (rule: string) => RULES[rule] ?? rule.replace(/_/g, " ");

/** A suggestion is only ever a guess: it never becomes final until a person confirms it. */
export const SUGGESTED_LABEL = "Suggested, needs a person to confirm";

const SUBJECTS: Record<string, string> = { settlement: "Settlement", payout: "Payout", refund: "Refund", payment: "Payment" };
export const subjectLabel = (type: string, id: string) => `${SUBJECTS[type] ?? type} ${id}`;

const asNum = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
const rec = (v: unknown) => (v && typeof v === "object" ? (v as Record<string, unknown>) : null);

/** The evidence for a match as label/value pairs. Only what the API recorded: nothing is worked out here. */
export function evidenceFacts(evidence: unknown): { label: string; value: string }[] {
  const e = rec(evidence);
  if (!e) return [];
  const out: { label: string; value: string }[] = [];
  const line = rec(e.line);
  if (line) {
    const a = asNum(line.amount_minor);
    out.push({ label: "Bank line", value: `${a === null ? "" : `${inr(Math.abs(a))} `}${typeof line.posted_on === "string" ? `on ${dayYear(line.posted_on)}` : ""}`.trim() });
    if (typeof line.reference === "string" && line.reference) out.push({ label: "Reference on the line", value: line.reference });
  }
  const st = rec(e.settlement);
  if (st) {
    const n = asNum(st.net_minor);
    out.push({ label: "Settlement", value: `${n === null ? "" : `${inr(n)} `}${typeof st.settled_on === "string" ? `settled ${dayYear(st.settled_on)}` : ""}`.trim() });
    if (typeof st.utr === "string" && st.utr) out.push({ label: "Settlement UTR", value: st.utr });
  }
  const sub = rec(e.subject);
  if (sub) {
    const n = asNum(sub.amount_minor);
    out.push({ label: "We recorded", value: `${n === null ? "" : inr(n)}${typeof sub.reference === "string" && sub.reference ? ` (reference ${sub.reference})` : ""}`.trim() });
  }
  if (typeof e.key === "string" && e.key) out.push({ label: "Reference matched", value: e.key });
  if (typeof e.amount_equal === "boolean") out.push({ label: "Amount", value: e.amount_equal ? "Same" : "Different" });
  const days = asNum(e.days_apart);
  if (days !== null) out.push({ label: "Days apart", value: String(Math.abs(days)) });
  const window = asNum(e.window_days);
  if (window !== null) out.push({ label: "Looked within", value: `${window} days` });
  const cands = asNum(e.competing_candidates) ?? asNum(e.candidates);
  if (cands !== null) out.push({ label: "Possible matches found", value: String(cands) });
  if (typeof e.note === "string" && e.note) out.push({ label: "Note", value: e.note });
  if (typeof e.confirmed_by_person === "string") out.push({ label: "Confirmed by", value: e.confirmed_by_person });
  return out;
}

/** Ageing as table rows, kinds in the order the API listed buckets. */
export function ageingRows(a: Ageing): { kind: string; open: number; unassigned: number; oldest_days: number; amount_minor: number; buckets: number[] }[] {
  return Object.entries(a.by_kind ?? {}).map(([kind, k]) => ({ kind, open: k.open, unassigned: k.unassigned, oldest_days: k.oldest_days, amount_minor: k.amount_minor, buckets: (a.bucket_days ?? []).map((b) => k.buckets?.[b] ?? 0) }));
}

/** "Waiting 3 days" for a case that is still open. */
export const waitingText = (days: number) => (days <= 0 ? "Opened today" : `Waiting ${days} day${days === 1 ? "" : "s"}`);

/** What the import said happened, in one sentence. */
export function importSentence(r: ImportDone): string {
  const parts = [`${r.added} new line${r.added === 1 ? "" : "s"} added`];
  if (r.duplicates) parts.push(`${r.duplicates} already held and skipped`);
  parts.push(`${r.matched.confirmed} matched by reference`, `${r.matched.suggested} suggested for a person to check`);
  if (r.cases_opened) parts.push(`${r.cases_opened} case${r.cases_opened === 1 ? "" : "s"} opened`);
  return `${parts.join(", ")}.`;
}

const CASE_KINDS: Record<string, string> = {
  unmatched_bank_credit: "Money arrived and nothing explains it",
  bank_amount_mismatch: "The bank amount differs from the settlement",
  settlement_no_bank_credit: "A settlement has no bank credit",
  payout_no_bank_debit: "A payout has no bank debit",
};
export const caseKindLabel = (kind: string) => CASE_KINDS[kind] ?? kind.replace(/_/g, " ");

/** A bank line's amount for display: money in or out, never a minus sign a reader might miss. */
export function lineAmount(l: Pick<BankLine, "amount_minor">): { text: string; direction: "in" | "out" } {
  return { text: inr(Math.abs(l.amount_minor)), direction: l.amount_minor >= 0 ? "in" : "out" };
}
