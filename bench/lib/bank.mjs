// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

// Writers for synthetic bank statement CSVs in the layouts Vayora's importer names. All data is made up: account numbers
// are zeros, names are invented. Headings follow docs/zyvor/BANK_RECONCILIATION.md (and the fixtures used by the tests).
const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const p2 = (n) => String(n).padStart(2, "0");
const parts = (iso) => iso.split("-").map(Number); // [y, m, d]
const dmy = (iso, sep) => { const [y, m, d] = parts(iso); return `${p2(d)}${sep}${p2(m)}${sep}${y}`; };
const dmyShort = (iso) => { const [y, m, d] = parts(iso); return `${p2(d)}/${p2(m)}/${String(y).slice(2)}`; };
const dMonY = (iso, sep) => { const [y, m, d] = parts(iso); return `${p2(d)}${sep}${MON[m - 1]}${sep}${y}`; };
const iso = (s) => s;

/** Indian digit grouping (1,23,456.50) when `indian`, western (123,456.50) otherwise. */
export function money(minor, { indian = false, group = true } = {}) {
  const neg = minor < 0;
  const a = Math.abs(minor);
  const whole = String(Math.floor(a / 100));
  const frac = p2(a % 100);
  let g = whole;
  if (group && whole.length > 3) {
    if (indian) g = whole.slice(0, -3).replace(/\B(?=(\d{2})+(?!\d))/g, ",") + "," + whole.slice(-3);
    else g = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  }
  return (neg ? "-" : "") + g + "." + frac;
}
const q = (s) => (/[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
const row = (cells) => cells.map((c) => q(String(c ?? ""))).join(",");

// each row: { date: "YYYY-MM-DD", narration, reference, debit (minor|null), credit (minor|null) }
export const LAYOUTS = {
  hdfc: (rows) => [
    "Account No,0000000000000",
    `Statement of account for the period ${dmyShort(rows[0].date)} to ${dmyShort(rows[rows.length - 1].date)}`,
    "Date,Narration,Chq./Ref.No.,Value Dt,Withdrawal Amt.,Deposit Amt.,Closing Balance",
    ...rows.map((r) => row([dmyShort(r.date), r.narration, r.reference, dmyShort(r.date), r.debit ? money(r.debit, { indian: true }) : "", r.credit ? money(r.credit, { indian: true }) : "", ""])),
  ],
  icici: (rows) => [
    "Transactions List - Synthetic Account",
    "S No.,Value Date,Transaction Date,Cheque Number,Transaction Remarks,Withdrawal Amount (INR ),Deposit Amount (INR ),Balance (INR )",
    ...rows.map((r, i) => row([i + 1, dmy(r.date, "/"), dmy(r.date, "/"), r.reference, r.narration, r.debit ? money(r.debit, { indian: true }) : "0.00", r.credit ? money(r.credit, { indian: true }) : "0.00", ""])),
  ],
  sbi: (rows) => [
    "Account Name :,SYNTHETIC ACCOUNT",
    "Txn Date,Value Date,Description,Ref No./Cheque No.,Debit,Credit,Balance",
    ...rows.map((r) => row([dMonY(r.date, " "), dMonY(r.date, " "), r.narration, r.reference, r.debit ? money(r.debit, { indian: true }) : "", r.credit ? money(r.credit, { indian: true }) : "", ""])),
  ],
  axis: (rows) => [
    "SRL NO,Tran Date,CHQNO,PARTICULARS,DR,CR,BAL,SOL",
    ...rows.map((r, i) => row([i + 1, dmy(r.date, "-"), r.reference, r.narration, r.debit ? money(r.debit, { group: false }) : "", r.credit ? money(r.credit, { group: false }) : "", "", "000"])),
  ],
  kotak: (rows) => [
    "Sl. No.,Transaction Date,Value Date,Description,Chq / Ref No.,Debit,Credit,Balance,Dr / Cr",
    ...rows.map((r, i) => row([i + 1, dmy(r.date, "-"), dmy(r.date, "-"), r.narration, r.reference, r.debit ? money(r.debit, { indian: true }) : "", r.credit ? money(r.credit, { indian: true }) : "", "", "CR"])),
  ],
  generic_split: (rows) => [
    "Date,Narration,Reference,Debit,Credit",
    ...rows.map((r) => row([iso(r.date), r.narration, r.reference, r.debit ? money(r.debit, { group: false }) : "", r.credit ? money(r.credit, { group: false }) : ""])),
  ],
  generic_signed: (rows) => [
    "Date,Narration,Reference,Amount",
    ...rows.map((r) => row([dmy(r.date, "/"), r.narration, r.reference, r.debit ? money(-r.debit, { indian: true }) : money(r.credit, { indian: true })])),
  ],
  generic_drcr: (rows) => [
    "Date,Description,Amount,Dr/Cr",
    // this layout has no separate reference column: the reference only exists inside the narration
    ...rows.map((r) => row([dMonY(r.date, "-"), r.narration, money(r.debit ?? r.credit, { indian: true }), r.debit ? "DR" : "CR"])),
  ],
};
export const writeStatement = (layout, rows) => LAYOUTS[layout](rows).join("\n") + "\n";
