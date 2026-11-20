// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Bank statement CSV import. Banks disagree on column names, date formats and whether debits and credits share a
//! column, so the parser finds the header row, maps columns by name (a named layout or your own column mapping) and
//! refuses anything it cannot read rather than guessing: a wrong sign on a credit is a wrong reconciliation.
//!
//! A file is all or nothing. Every problem is reported with its line number and nothing is stored while any remain.
//! Each line gets a stable fingerprint, so importing the same file again, or a file that overlaps an earlier one,
//! adds no second copy of a transaction. `POST /v1/bank_statements/preview` runs the same checks without storing.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::{
    ApiError, AppState,
    auth::Auth,
    recon::{BankLine, store_statement},
    tenant,
};

/// A file with more transactions than this is refused; split it by month.
pub const MAX_ROWS: usize = 20_000;
/// Errors listed in one response; the count of the rest is still given.
const MAX_REPORTED: usize = 50;
/// Anything above this (in paise) is treated as a misread column, not a transaction.
const MAX_AMOUNT_MINOR: i64 = 100_000_000_000_000;

// ---------- layouts ----------

/// Column names for each field. A value is a header (compared ignoring case, spaces and punctuation) or `#3` for
/// the third column (counting from 1). A field left out is found from the usual names.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ColumnMap {
    pub date: Option<String>,
    pub value_date: Option<String>,
    pub narration: Option<String>,
    /// A dedicated UTR / RRN / cheque or reference column.
    pub reference: Option<String>,
    pub debit: Option<String>,
    pub credit: Option<String>,
    /// One signed amount column (negative is a debit), or one unsigned amount together with `drcr`.
    pub amount: Option<String>,
    pub drcr: Option<String>,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    /// One of the names from `GET /v1/bank_statements/layouts`. Default `auto`.
    pub layout: Option<String>,
    /// Overrides single columns of the chosen layout.
    pub mapping: Option<ColumnMap>,
    /// A strftime pattern such as `%d/%m/%Y`, for a bank whose dates the usual patterns do not read.
    pub date_format: Option<String>,
    /// `,` (default, detected), `;`, `|` or `tab`.
    pub delimiter: Option<String>,
}

struct Preset {
    name: &'static str,
    about: &'static str,
    map: fn() -> ColumnMap,
    date_formats: &'static [&'static str],
}

fn cm(
    date: &str,
    value_date: Option<&str>,
    narration: &str,
    reference: Option<&str>,
    debit: Option<&str>,
    credit: Option<&str>,
    balance: Option<&str>,
) -> ColumnMap {
    let s = |x: Option<&str>| x.map(str::to_string);
    ColumnMap {
        date: Some(date.into()),
        value_date: s(value_date),
        narration: Some(narration.into()),
        reference: s(reference),
        debit: s(debit),
        credit: s(credit),
        balance: s(balance),
        ..Default::default()
    }
}

const DAY_FIRST: &[&str] = &[
    "%d/%m/%Y",
    "%d-%m-%Y",
    "%d/%m/%y",
    "%d-%m-%y",
    "%Y-%m-%d",
    "%d-%b-%Y",
    "%d %b %Y",
    "%d-%b-%y",
    "%d.%m.%Y",
    "%Y/%m/%d",
    "%Y-%m-%d %H:%M:%S",
    "%d/%m/%Y %H:%M:%S",
];

/// Layouts modelled on the column headings these banks' statement downloads are commonly shown with. They have only
/// been exercised against synthetic files written from those headings: check a preview before trusting one.
const PRESETS: &[Preset] = &[
    Preset {
        name: "auto",
        about: "Find the header row and the columns from the usual names (date, narration/description/particulars, UTR/ref, debit/withdrawal, credit/deposit, or amount with Dr/Cr).",
        map: ColumnMap::default,
        date_formats: DAY_FIRST,
    },
    Preset {
        name: "hdfc",
        about: "Date, Narration, Chq./Ref.No., Value Dt, Withdrawal Amt., Deposit Amt., Closing Balance",
        map: || {
            cm(
                "Date",
                Some("Value Dt"),
                "Narration",
                Some("Chq./Ref.No."),
                Some("Withdrawal Amt."),
                Some("Deposit Amt."),
                Some("Closing Balance"),
            )
        },
        date_formats: &["%d/%m/%y", "%d/%m/%Y"],
    },
    Preset {
        name: "icici",
        about: "Transaction Date, Value Date, Cheque Number, Transaction Remarks, Withdrawal Amount (INR ), Deposit Amount (INR ), Balance (INR )",
        map: || {
            cm(
                "Transaction Date",
                Some("Value Date"),
                "Transaction Remarks",
                Some("Cheque Number"),
                Some("Withdrawal Amount (INR )"),
                Some("Deposit Amount (INR )"),
                Some("Balance (INR )"),
            )
        },
        date_formats: &["%d/%m/%Y", "%d-%m-%Y"],
    },
    Preset {
        name: "sbi",
        about: "Txn Date, Value Date, Description, Ref No./Cheque No., Debit, Credit, Balance",
        map: || {
            cm(
                "Txn Date",
                Some("Value Date"),
                "Description",
                Some("Ref No./Cheque No."),
                Some("Debit"),
                Some("Credit"),
                Some("Balance"),
            )
        },
        date_formats: &["%d %b %Y", "%d-%b-%Y", "%d/%m/%Y"],
    },
    Preset {
        name: "axis",
        about: "Tran Date, CHQNO, PARTICULARS, DR, CR, BAL",
        map: || {
            cm(
                "Tran Date",
                None,
                "PARTICULARS",
                Some("CHQNO"),
                Some("DR"),
                Some("CR"),
                Some("BAL"),
            )
        },
        date_formats: &["%d-%m-%Y", "%d/%m/%Y"],
    },
    Preset {
        name: "kotak",
        about: "Transaction Date, Value Date, Description, Chq / Ref No., Debit, Credit, Balance",
        map: || {
            cm(
                "Transaction Date",
                Some("Value Date"),
                "Description",
                Some("Chq / Ref No."),
                Some("Debit"),
                Some("Credit"),
                Some("Balance"),
            )
        },
        date_formats: &["%d-%m-%Y", "%d/%m/%Y", "%d %b %Y"],
    },
    Preset {
        name: "generic_split",
        about: "Date, Narration, Reference, Debit, Credit (ISO or day-first dates)",
        map: || {
            cm(
                "Date",
                None,
                "Narration",
                Some("Reference"),
                Some("Debit"),
                Some("Credit"),
                None,
            )
        },
        date_formats: DAY_FIRST,
    },
    Preset {
        name: "generic_signed",
        about: "Date, Narration, Reference, Amount (a minus sign marks a debit)",
        map: || ColumnMap {
            date: Some("Date".into()),
            narration: Some("Narration".into()),
            reference: Some("Reference".into()),
            amount: Some("Amount".into()),
            ..Default::default()
        },
        date_formats: DAY_FIRST,
    },
    Preset {
        name: "generic_drcr",
        about: "Date, Description, Amount, Dr/Cr (an unsigned amount with a Dr or Cr column)",
        map: || ColumnMap {
            date: Some("Date".into()),
            narration: Some("Description".into()),
            amount: Some("Amount".into()),
            drcr: Some("Dr/Cr".into()),
            ..Default::default()
        },
        date_formats: DAY_FIRST,
    },
];

/// What `GET /v1/bank_statements/layouts` returns.
pub fn layouts_json() -> Value {
    json!({
        "data": PRESETS.iter().map(|p| json!({
            "name": p.name, "about": p.about, "mapping": (p.map)(),
            "date_formats": p.date_formats,
        })).collect::<Vec<_>>(),
        "notes": [
            "Dates are read day first (01/10/2026), never month first. ISO (2026-10-01) and 01-Oct-2026 are read too.",
            "Amounts may carry Indian grouping (1,23,456.50), a currency sign, a minus sign, brackets, or a trailing Dr/Cr. More than two decimals, or commas in the wrong places, are refused.",
            "Send `mapping` to name your own columns, `date_format` for an unusual date pattern and `delimiter` for ; | or tab files.",
            "The named layouts follow the headings the banks' downloads are commonly shown with and have been tested only on synthetic files."
        ],
        "limits": { "max_rows": MAX_ROWS },
    })
}

// ---------- reading the file ----------

#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    /// 1-based line of the file (0 when the problem is about the file as a whole).
    pub line: usize,
    pub column: Option<String>,
    pub message: String,
}

pub(crate) fn problem(line: usize, column: Option<&str>, message: impl Into<String>) -> Problem {
    Problem {
        line,
        column: column.map(str::to_string),
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub line_no: usize,
    pub posted_on: NaiveDate,
    pub value_date: Option<NaiveDate>,
    pub amount_minor: i64,
    pub narration: String,
    pub reference: Option<String>,
    pub balance_minor: Option<i64>,
    pub fingerprint: String,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub layout: String,
    pub header_line: usize,
    pub lines: Vec<ParsedLine>,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
}

impl Parsed {
    pub fn ok(&self) -> bool {
        self.errors.is_empty() && !self.lines.is_empty()
    }
    pub fn into_bank_lines(self) -> Vec<BankLine> {
        self.lines
            .into_iter()
            .map(BankLine::from_statement)
            .collect()
    }
    pub fn credits_debits(&self) -> ((i64, i64), (i64, i64)) {
        let (mut cn, mut ct, mut dn, mut dt) = (0, 0, 0, 0);
        for l in &self.lines {
            if l.amount_minor > 0 {
                cn += 1;
                ct += l.amount_minor;
            } else {
                dn += 1;
                dt += -l.amount_minor;
            }
        }
        ((cn, ct), (dn, dt))
    }
    pub fn period(&self) -> Option<(NaiveDate, NaiveDate)> {
        let min = self.lines.iter().map(|l| l.posted_on).min()?;
        let max = self.lines.iter().map(|l| l.posted_on).max()?;
        Some((min, max))
    }
}

/// Split a file into records, honouring quotes (a quoted field may hold commas, `""` and line breaks).
pub(crate) fn read_records(text: &str, delim: char) -> Result<Vec<(usize, Vec<String>)>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut out, mut rec, mut cur) = (Vec::new(), Vec::new(), String::new());
    let (mut q, mut line, mut start, mut quote_line) = (false, 1usize, 1usize, 1usize);
    let mut it = text.chars().peekable();
    let finish = |rec: &mut Vec<String>,
                  cur: &mut String,
                  out: &mut Vec<(usize, Vec<String>)>,
                  start: usize| {
        rec.push(std::mem::take(cur).trim().to_string());
        let r = std::mem::take(rec);
        if r.iter().any(|c| !c.is_empty()) {
            out.push((start, r));
        }
    };
    while let Some(c) = it.next() {
        if c == '"' {
            if q {
                if it.peek() == Some(&'"') {
                    cur.push('"');
                    it.next();
                } else {
                    q = false;
                }
            } else if cur.trim().is_empty() {
                q = true;
                quote_line = line;
                cur.clear();
            } else {
                cur.push('"');
            }
        } else if c == delim && !q {
            rec.push(std::mem::take(&mut cur).trim().to_string());
        } else if c == '\n' {
            if q {
                cur.push(' ');
                line += 1;
            } else {
                finish(&mut rec, &mut cur, &mut out, start);
                line += 1;
                start = line;
            }
        } else if c == '\r' {
            if q {
                cur.push(' ');
            }
        } else {
            cur.push(c);
        }
    }
    if q {
        return Err(format!(
            "a quote opened on line {quote_line} is never closed"
        ));
    }
    if !cur.is_empty() || !rec.is_empty() {
        finish(&mut rec, &mut cur, &mut out, start);
    }
    Ok(out)
}

pub(crate) fn detect_delimiter(text: &str) -> char {
    let sample: String = text.lines().take(40).collect::<Vec<_>>().join("\n");
    [',', ';', '\t', '|']
        .into_iter()
        .max_by_key(|d| (sample.matches(*d).count(), *d == ','))
        .unwrap_or(',')
}

pub(crate) fn norm_header(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Usual names per field, tried in order. A trailing `$` means the whole heading, otherwise its start.
const ALIASES: &[(&str, &[&str])] = &[
    (
        "date",
        &[
            "transactiondate",
            "txndate",
            "trandate",
            "postingdate",
            "bookingdate",
            "date",
        ],
    ),
    ("value_date", &["valuedate", "valuedt", "valdate"]),
    (
        "narration",
        &[
            "narration",
            "transactionremarks",
            "transactiondescription",
            "transactiondetails",
            "description",
            "particulars",
            "remarks",
            "details",
        ],
    ),
    (
        "reference",
        &[
            "utr",
            "chqrefno",
            "chqref",
            "refnochequeno",
            "refnochq",
            "referenceno",
            "reference",
            "refno",
            "ref$",
            "chequenumber",
            "chequeno",
            "chqno",
        ],
    ),
    (
        "debit",
        &["withdrawal", "debit", "dr$", "paidout", "moneyout"],
    ),
    (
        "credit",
        &["deposit", "credit", "cr$", "paidin", "moneyin", "receipts"],
    ),
    (
        "amount",
        &["transactionamount", "txnamount", "amount", "amt$"],
    ),
    (
        "drcr",
        &[
            "drcr$",
            "crdr$",
            "debitcredit$",
            "creditdebit$",
            "type$",
            "txntype$",
            "transactiontype$",
        ],
    ),
    ("balance", &["closingbalance", "balance", "bal$"]),
];

#[derive(Debug, Default, Clone, Copy)]
struct Cols {
    date: usize,
    value_date: Option<usize>,
    narration: Option<usize>,
    reference: Option<usize>,
    debit: Option<usize>,
    credit: Option<usize>,
    amount: Option<usize>,
    drcr: Option<usize>,
    balance: Option<usize>,
}

fn field<'a>(m: &'a ColumnMap, name: &str) -> &'a Option<String> {
    match name {
        "date" => &m.date,
        "value_date" => &m.value_date,
        "narration" => &m.narration,
        "reference" => &m.reference,
        "debit" => &m.debit,
        "credit" => &m.credit,
        "amount" => &m.amount,
        "drcr" => &m.drcr,
        _ => &m.balance,
    }
}

/// Try to read `row` as the header. Err lists the named columns it lacks (only when something was named).
fn resolve_header(row: &[String], map: &ColumnMap) -> Result<Cols, Vec<String>> {
    let h: Vec<String> = row.iter().map(|c| norm_header(c)).collect();
    let mut taken: Vec<usize> = Vec::new();
    let mut missing = Vec::new();
    let mut found: HashMap<&str, usize> = HashMap::new();
    for (fname, aliases) in ALIASES {
        let want = field(map, fname);
        let mut at = None;
        if let Some(w) = want {
            if let Some(n) = w.strip_prefix('#') {
                at = n
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 1 && *n <= row.len())
                    .map(|n| n - 1);
            } else {
                let w = norm_header(w);
                at = h.iter().position(|x| *x == w);
            }
            if at.is_none() {
                missing.push(format!("{fname}: `{}`", want.as_deref().unwrap_or("")));
            }
        } else {
            // the usual names; debit/credit/amount only when the layout did not name another way to read amounts
            for a in aliases.iter() {
                let (a, exact) = match a.strip_suffix('$') {
                    Some(a) => (a, true),
                    None => (*a, false),
                };
                at = h.iter().enumerate().position(|(i, x)| {
                    !taken.contains(&i) && if exact { x == a } else { x.starts_with(a) }
                });
                if at.is_some() {
                    break;
                }
            }
        }
        if let Some(i) = at {
            taken.push(i);
            found.insert(fname, i);
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    let split = found.contains_key("debit") || found.contains_key("credit");
    // with debit/credit columns an "amount" heading is something else (a running amount, a charge)
    let amount = if split {
        None
    } else {
        found.get("amount").copied()
    };
    let Some(date) = found.get("date").copied() else {
        return Err(vec![]);
    };
    if !found.contains_key("credit") && amount.is_none() {
        return Err(vec![]);
    }
    Ok(Cols {
        date,
        value_date: found.get("value_date").copied(),
        narration: found.get("narration").copied(),
        reference: found.get("reference").copied(),
        debit: found.get("debit").copied(),
        credit: found.get("credit").copied(),
        amount,
        drcr: if amount.is_some() {
            found.get("drcr").copied()
        } else {
            None
        },
        balance: found.get("balance").copied(),
    })
}

fn parse_date(s: &str, formats: &[&str]) -> Option<NaiveDate> {
    use chrono::Datelike;
    let s = s.trim();
    // `%Y` also accepts "26", which would read 01/10/26 as the year 26: only believable years count
    formats.iter().find_map(|f| {
        NaiveDate::parse_from_str(s, f)
            .ok()
            .filter(|d| (1990..=2100).contains(&d.year()))
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Amt {
    /// Magnitude in paise.
    minor: i64,
    negative: bool,
    /// `Some(true)` for a trailing Cr, `Some(false)` for Dr.
    credit: Option<bool>,
}

/// Indian (1,23,456) or western (123,456) grouping, nothing else.
fn grouping_ok(int_part: &str) -> bool {
    if !int_part.contains(',') {
        return true;
    }
    let g: Vec<&str> = int_part.split(',').collect();
    if g.iter()
        .any(|x| x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let (first, rest) = (g[0], &g[1..]);
    let (last, mid) = rest.split_last().expect("contains a comma");
    if last.len() != 3 || first.len() > 3 {
        return false;
    }
    let western = mid.iter().all(|x| x.len() == 3);
    let indian = mid.iter().all(|x| x.len() == 2) && first.len() <= 2;
    western || indian
}

/// An amount in rupees as people type it ("1,250.50", "₹ 499"), as whole paise. No sign, no Dr/Cr, at most two
/// decimals, grouping only the Indian or western way. For files that are not bank statements.
pub(crate) fn parse_rupees(s: &str) -> Result<i64, String> {
    match parse_amount(s)? {
        Some(a) if !a.negative && a.credit.is_none() && a.minor > 0 => Ok(a.minor),
        Some(_) => Err(format!("`{}` must be a positive amount", s.trim())),
        None => Err("the amount is empty".into()),
    }
}

fn parse_amount(s: &str) -> Result<Option<Amt>, String> {
    let mut t = s.trim().to_string();
    if t.is_empty() || t == "-" {
        return Ok(None);
    }
    let mut negative = false;
    let mut credit = None;
    let lower = t.to_lowercase();
    for (suffix, c) in [("dr.", false), ("cr.", true), ("dr", false), ("cr", true)] {
        if lower.ends_with(suffix) && lower.len() > suffix.len() {
            t.truncate(t.len() - suffix.len());
            credit = Some(c);
            break;
        }
    }
    let mut t = t.trim().to_string();
    if t.starts_with('(') && t.ends_with(')') {
        negative = true;
        t = t[1..t.len() - 1].trim().to_string();
    }
    for p in ["₹", "INR", "inr", "Rs.", "RS.", "Rs", "RS"] {
        if let Some(r) = t.strip_prefix(p) {
            t = r.trim().to_string();
            break;
        }
    }
    if let Some(r) = t.strip_prefix('-') {
        negative = true;
        t = r.trim().to_string();
    } else if let Some(r) = t.strip_prefix('+') {
        t = r.trim().to_string();
    }
    let bad = || format!("unreadable amount `{}`", s.trim());
    if t.is_empty()
        || !t
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b'.')
    {
        return Err(bad());
    }
    let (int_part, frac) = t.split_once('.').unwrap_or((&t, ""));
    if t.matches('.').count() > 1 || frac.contains(',') {
        return Err(bad());
    }
    if !grouping_ok(int_part) {
        return Err(format!(
            "`{}` has commas in unusual places (is the comma a decimal point?)",
            s.trim()
        ));
    }
    if frac.len() > 2 {
        return Err(format!("`{}` has more than two decimal places", s.trim()));
    }
    let digits = format!(
        "{}{}{}",
        if int_part.is_empty() { "0" } else { "" },
        int_part.replace(',', ""),
        if frac.is_empty() {
            String::new()
        } else {
            format!(".{frac}")
        }
    );
    let minor = zyvor_core::decimal_to_minor(&digits).ok_or_else(bad)?;
    if minor > MAX_AMOUNT_MINOR {
        return Err(format!("`{}` is implausibly large", s.trim()));
    }
    Ok(Some(Amt {
        minor,
        negative,
        credit,
    }))
}

/// A reference column a spreadsheet has turned into 4.04223E+11 has lost its digits: refuse it.
fn spreadsheet_damaged(r: &str) -> bool {
    let up = r.to_uppercase();
    match up.split_once("E+") {
        Some((m, e)) => {
            !m.is_empty()
                && m.chars().all(|c| c.is_ascii_digit() || c == '.')
                && !e.is_empty()
                && e.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn dir_word(s: &str) -> Option<bool> {
    match s.trim().trim_end_matches('.').to_lowercase().as_str() {
        "cr" | "c" | "credit" | "deposit" => Some(true),
        "dr" | "d" | "debit" | "withdrawal" => Some(false),
        _ => None,
    }
}

fn fingerprint(date: NaiveDate, amount: i64, narration: &str, reference: &str, nth: u32) -> String {
    let key = format!(
        "v1|{date}|{amount}|{}|{}|{nth}",
        narration.to_uppercase(),
        reference.to_uppercase()
    );
    hex::encode(Sha256::digest(key.as_bytes()))
}

pub fn parse_statement(text: &str, opts: &Options) -> Parsed {
    let mut out = Parsed::default();
    let layout = opts.layout.as_deref().unwrap_or("auto").to_lowercase();
    let Some(preset) = PRESETS.iter().find(|p| p.name == layout) else {
        out.errors.push(problem(
            0,
            None,
            format!(
                "unknown layout `{layout}` (choose from: {})",
                PRESETS
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
        return out;
    };
    out.layout = preset.name.to_string();
    // the layout's columns, with anything the caller named on top
    let mut map = (preset.map)();
    if let Some(over) = &opts.mapping {
        for name in [
            "date",
            "value_date",
            "narration",
            "reference",
            "debit",
            "credit",
            "amount",
            "drcr",
            "balance",
        ] {
            if let Some(v) = field(over, name).clone().filter(|v| !v.trim().is_empty()) {
                match name {
                    "date" => map.date = Some(v),
                    "value_date" => map.value_date = Some(v),
                    "narration" => map.narration = Some(v),
                    "reference" => map.reference = Some(v),
                    "debit" => map.debit = Some(v),
                    "credit" => map.credit = Some(v),
                    "amount" => map.amount = Some(v),
                    "drcr" => map.drcr = Some(v),
                    _ => map.balance = Some(v),
                }
            }
        }
        // naming an amount column replaces the layout's split columns, and the other way round
        if over.amount.is_some() && over.debit.is_none() && over.credit.is_none() {
            map.debit = None;
            map.credit = None;
        }
        if (over.debit.is_some() || over.credit.is_some()) && over.amount.is_none() {
            map.amount = None;
            map.drcr = None;
        }
    }
    let custom_fmt = opts
        .date_format
        .as_deref()
        .filter(|f| !f.trim().is_empty())
        .map(|f| vec![f.trim()]);
    let formats: Vec<&str> = custom_fmt.unwrap_or_else(|| preset.date_formats.to_vec());

    if text.trim().is_empty() {
        out.errors.push(problem(0, None, "the file is empty"));
        return out;
    }
    let delim = match opts.delimiter.as_deref().map(str::trim) {
        None | Some("") => detect_delimiter(text),
        Some(",") => ',',
        Some(";") => ';',
        Some("|") => '|',
        Some("tab") | Some("\\t") | Some("\t") => '\t',
        Some(o) => {
            out.errors.push(problem(
                0,
                None,
                format!("unsupported delimiter `{o}` (use `,`, `;`, `|` or `tab`)"),
            ));
            return out;
        }
    };
    let records = match read_records(text, delim) {
        Ok(r) => r,
        Err(e) => {
            out.errors.push(problem(0, None, e));
            return out;
        }
    };
    // statements often start with account details; the header is the first row that names the needed columns
    let mut header = None;
    let mut last_missing: Vec<String> = Vec::new();
    for (i, (_, r)) in records.iter().enumerate().take(60) {
        match resolve_header(r, &map) {
            Ok(c) => {
                header = Some((i, c));
                break;
            }
            Err(m) if !m.is_empty() && m.len() < 9 => {
                // a row that has the date column but lacks other named ones is probably the header
                if last_missing.is_empty() || m.len() < last_missing.len() {
                    last_missing = m;
                }
            }
            Err(_) => {}
        }
    }
    let Some((hi, cols)) = header else {
        let why = if last_missing.is_empty() {
            "no header row found: expected a date column and a credit/deposit or amount column"
                .to_string()
        } else {
            format!(
                "no header row found for layout `{}`: columns not found: {}",
                out.layout,
                last_missing.join(", ")
            )
        };
        out.errors.push(problem(0, None, why));
        return out;
    };
    out.header_line = records[hi].0;

    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut skipped_notes = 0usize;
    'rows: for (line_no, r) in records.iter().skip(hi + 1).map(|(n, r)| (*n, r)) {
        let cell = |i: Option<usize>| i.and_then(|i| r.get(i)).map(String::as_str).unwrap_or("");
        let raw_date = cell(Some(cols.date));
        if out.errors.len() > 1_000 {
            break;
        }
        let Some(posted_on) = parse_date(raw_date, &formats) else {
            // footers such as "Closing balance" are noted; a half-parsed data row is an error
            if raw_date.chars().any(|c| c.is_ascii_digit())
                && raw_date.chars().any(|c| matches!(c, '/' | '-' | '.' | ' '))
            {
                out.errors.push(problem(
                    line_no,
                    Some("date"),
                    format!("unreadable date `{raw_date}`"),
                ));
            } else if skipped_notes < 20 {
                skipped_notes += 1;
                out.warnings.push(problem(
                    line_no,
                    Some("date"),
                    format!(
                        "row skipped: no date (`{}`)",
                        squash(&r.join(" ")).chars().take(60).collect::<String>()
                    ),
                ));
            }
            continue;
        };
        let bad = |out: &mut Parsed, col: &str, msg: String| {
            out.errors.push(problem(line_no, Some(col), msg));
        };
        let value_date = match cell(cols.value_date) {
            "" => None,
            v => match parse_date(v, &formats) {
                Some(d) => Some(d),
                None => {
                    bad(&mut out, "value_date", format!("unreadable date `{v}`"));
                    continue 'rows;
                }
            },
        };
        let balance_minor = match parse_amount(cell(cols.balance)) {
            Ok(b) => b.map(|b| {
                if b.negative || b.credit == Some(false) {
                    -b.minor
                } else {
                    b.minor
                }
            }),
            Err(e) => {
                bad(&mut out, "balance", e);
                continue 'rows;
            }
        };
        // signed paise: credits positive
        let signed: Option<i64> = if let Some(a) = cols.amount {
            match parse_amount(cell(Some(a))) {
                Err(e) => {
                    bad(&mut out, "amount", e);
                    continue 'rows;
                }
                Ok(None) => None,
                Ok(Some(v)) => {
                    let word = cell(cols.drcr);
                    let from_col = if word.is_empty() {
                        None
                    } else {
                        match dir_word(word) {
                            Some(c) => Some(c),
                            None => {
                                bad(
                                    &mut out,
                                    "drcr",
                                    format!("`{word}` is neither debit nor credit"),
                                );
                                continue 'rows;
                            }
                        }
                    };
                    let from_suffix = v.credit;
                    if let (Some(a), Some(b)) = (from_col, from_suffix)
                        && a != b
                    {
                        bad(
                            &mut out,
                            "drcr",
                            "the Dr/Cr column and the amount's own Dr/Cr disagree".into(),
                        );
                        continue 'rows;
                    }
                    let dir = from_col.or(from_suffix);
                    match (dir, v.negative) {
                        (Some(true), true) => {
                            bad(
                                &mut out,
                                "amount",
                                "a negative amount marked as a credit".into(),
                            );
                            continue 'rows;
                        }
                        (Some(true), false) => Some(v.minor),
                        (Some(false), _) => Some(-v.minor),
                        (None, true) => Some(-v.minor),
                        (None, false) => Some(v.minor),
                    }
                }
            }
        } else {
            let one = |out: &mut Parsed, col: &str, i: Option<usize>| -> Result<i64, ()> {
                match parse_amount(cell(i)) {
                    Ok(None) => Ok(0),
                    Ok(Some(a)) if a.negative => {
                        out.errors.push(problem(
                            line_no,
                            Some(col),
                            "a debit/credit column holds a negative amount",
                        ));
                        Err(())
                    }
                    Ok(Some(a)) => Ok(a.minor),
                    Err(e) => {
                        out.errors.push(problem(line_no, Some(col), e));
                        Err(())
                    }
                }
            };
            let Ok(c) = one(&mut out, "credit", cols.credit) else {
                continue 'rows;
            };
            let Ok(d) = one(&mut out, "debit", cols.debit) else {
                continue 'rows;
            };
            match (c != 0, d != 0) {
                (true, true) => {
                    bad(
                        &mut out,
                        "credit",
                        "both a debit and a credit are filled in".into(),
                    );
                    continue 'rows;
                }
                (true, false) => Some(c),
                (false, true) => Some(-d),
                (false, false) => None,
            }
        };
        let Some(amount_minor) = signed.filter(|v| *v != 0) else {
            if out.warnings.len() < 40 {
                out.warnings
                    .push(problem(line_no, None, "row skipped: no amount"));
            }
            continue;
        };
        let reference = cell(cols.reference)
            .trim_matches(|c| c == '\'' || c == '"')
            .trim()
            .to_string();
        if spreadsheet_damaged(&reference) {
            bad(
                &mut out,
                "reference",
                format!(
                    "`{reference}` was rounded by a spreadsheet; export the file again with the reference column as text"
                ),
            );
            continue;
        }
        let narration = squash(cell(cols.narration));
        let refn = squash(&reference);
        let key = format!(
            "{posted_on}|{amount_minor}|{}|{}",
            narration.to_uppercase(),
            refn.to_uppercase()
        );
        let n = seen.entry(key).and_modify(|n| *n += 1).or_insert(1);
        if out.lines.len() >= MAX_ROWS {
            bad(
                &mut out,
                "",
                format!("more than {MAX_ROWS} transactions: split the file by month"),
            );
            break;
        }
        out.lines.push(ParsedLine {
            line_no,
            posted_on,
            value_date,
            amount_minor,
            fingerprint: fingerprint(posted_on, amount_minor, &narration, &refn, *n),
            narration,
            reference: (!refn.is_empty()).then_some(refn),
            balance_minor,
        });
    }
    // only the first problems are listed; the rest are counted
    if out.errors.len() > MAX_REPORTED {
        let more = out.errors.len() - MAX_REPORTED;
        out.errors.truncate(MAX_REPORTED);
        out.errors
            .push(problem(0, None, format!("and {more} more problems")));
    }
    if out.errors.is_empty() && out.lines.is_empty() {
        out.errors.push(problem(
            0,
            None,
            "the file has a header but no transactions",
        ));
    }
    if out.errors.is_empty() {
        check_running_balance(&mut out);
    }
    out
}

/// When every line carries a balance, the balances should follow from the amounts, in file order or reversed.
/// A break is a warning (a filtered export legitimately breaks), never an error.
fn check_running_balance(out: &mut Parsed) {
    if out.lines.len() < 2 || out.lines.iter().any(|l| l.balance_minor.is_none()) {
        return;
    }
    let breaks = |rev: bool| -> Vec<usize> {
        let mut b = Vec::new();
        for w in out.lines.windows(2) {
            let (prev, next) = (&w[0], &w[1]);
            let ok = if rev {
                prev.balance_minor == next.balance_minor.map(|x| x + prev.amount_minor)
            } else {
                next.balance_minor == prev.balance_minor.map(|x| x + next.amount_minor)
            };
            if !ok {
                b.push(next.line_no);
            }
        }
        b
    };
    let (fwd, rev) = (breaks(false), breaks(true));
    let best = if fwd.len() <= rev.len() { fwd } else { rev };
    if !best.is_empty() {
        out.warnings.push(problem(
            best[0],
            Some("balance"),
            format!(
                "the running balance does not follow from the amounts at {} place(s), first here; check the file is complete",
                best.len()
            ),
        ));
    }
}

// ---------- endpoints ----------

#[derive(Deserialize)]
pub struct CsvBody {
    /// The file's contents.
    csv: String,
    /// The file's name, kept with the import record.
    filename: Option<String>,
    /// A name from `GET /v1/bank_statements/layouts`. Default `auto`.
    layout: Option<String>,
    /// Names your own columns; overrides single columns of the layout.
    mapping: Option<ColumnMap>,
    /// A strftime pattern such as `%d/%m/%Y`, for dates the usual patterns do not read.
    date_format: Option<String>,
    /// `,` (default, detected), `;`, `|` or `tab`.
    delimiter: Option<String>,
}

impl CsvBody {
    fn options(&self) -> Options {
        Options {
            layout: self.layout.clone(),
            mapping: self.mapping.clone(),
            date_format: self.date_format.clone(),
            delimiter: self.delimiter.clone(),
        }
    }
}

fn problems_json(p: &[Problem]) -> Value {
    json!(p)
}

fn rejected(parsed: &Parsed) -> Response {
    let first = parsed
        .errors
        .first()
        .map(|e| {
            if e.line > 0 {
                format!("line {}: {}", e.line, e.message)
            } else {
                e.message.clone()
            }
        })
        .unwrap_or_default();
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": {
            "code": "invalid_statement",
            "message": format!("the statement was not imported: {first}"),
            "errors": problems_json(&parsed.errors),
            "warnings": problems_json(&parsed.warnings),
            "stored": false,
        } })),
    )
        .into_response()
}

fn file_hash(csv: &str) -> String {
    hex::encode(Sha256::digest(csv.as_bytes()))
}

/// `POST /v1/bank_statements/preview`: read the file and say what an import would do, storing nothing.
pub async fn preview_csv(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<CsvBody>,
) -> Result<Response, ApiError> {
    auth.require("recon:read")?;
    let parsed = parse_statement(&b.csv, &b.options());
    if !parsed.ok() {
        let mut r = rejected(&parsed);
        // a preview is a 200 with ok=false: the caller asked what would happen
        *r.status_mut() = StatusCode::OK;
        let body = json!({
            "ok": false, "layout": parsed.layout, "rows": 0,
            "errors": problems_json(&parsed.errors), "warnings": problems_json(&parsed.warnings),
        });
        return Ok((StatusCode::OK, Json(body)).into_response());
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let fps: Vec<String> = parsed.lines.iter().map(|l| l.fingerprint.clone()).collect();
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT fingerprint FROM bank_statement_lines WHERE fingerprint = ANY($1)",
    )
    .bind(&fps)
    .fetch_all(&mut *tx)
    .await?;
    let same_file: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM bank_statement_imports WHERE file_sha256 = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(file_hash(&b.csv))
    .fetch_optional(&mut *tx)
    .await?;
    let ((cn, ct), (dn, dt)) = parsed.credits_debits();
    let period = parsed.period();
    let sample: Vec<Value> = parsed
        .lines
        .iter()
        .take(10)
        .map(|l| {
            json!({ "line": l.line_no, "posted_on": l.posted_on, "amount_minor": l.amount_minor,
                    "narration": l.narration, "reference": l.reference })
        })
        .collect();
    Ok(Json(json!({
        "ok": true, "layout": parsed.layout, "header_line": parsed.header_line,
        "rows": parsed.lines.len(),
        "new_rows": parsed.lines.len() - existing.len().min(parsed.lines.len()),
        "duplicate_rows": existing.len(),
        "credits": { "count": cn, "total_minor": ct },
        "debits": { "count": dn, "total_minor": dt },
        "period_from": period.map(|p| p.0), "period_to": period.map(|p| p.1),
        "already_imported_as": same_file,
        "errors": [], "warnings": problems_json(&parsed.warnings), "sample": sample,
    }))
    .into_response())
}

/// `POST /v1/bank_statements/import_csv` with `{ "csv": "<file contents>", "layout"?, "mapping"?, ... }`.
pub async fn import_csv(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<CsvBody>,
) -> Result<Response, ApiError> {
    auth.require("recon:write")?;
    let parsed = parse_statement(&b.csv, &b.options());
    if !parsed.ok() {
        return Ok(rejected(&parsed));
    }
    let meta = crate::recon::ImportMeta {
        source: "csv".into(),
        layout: parsed.layout.clone(),
        filename: b.filename.clone(),
        file_sha256: file_hash(&b.csv),
        credits_minor: parsed.credits_debits().0.1,
        debits_minor: parsed.credits_debits().1.1,
        period: parsed.period(),
    };
    let warnings = problems_json(&parsed.warnings);
    let lines = parsed.into_bank_lines();
    let mut out = store_statement(&s, &auth, lines, Some(meta)).await?;
    out["warnings"] = warnings;
    Ok(Json(out).into_response())
}

pub async fn layouts(auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    Ok(Json(layouts_json()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(csv: &str) -> Parsed {
        let p = parse_statement(csv, &Options::default());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        p
    }

    fn with(csv: &str, layout: &str) -> Parsed {
        parse_statement(
            csv,
            &Options {
                layout: Some(layout.into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn hdfc_style_with_debit_and_credit_columns() {
        let csv = "Account No,1234\nDate,Narration,Chq./Ref.No.,Value Dt,Withdrawal Amt.,Deposit Amt.,Closing Balance\n01/10/26,\"UPI-ACME, SETTLEMENT\",'UTR998877,01/10/26,,\"1,23,456.50\",9\n02/10/26,Bank charges,,02/10/26,10.00,,8\nClosing balance,,,,,,8\n";
        let p = ok(csv);
        let l = &p.lines;
        assert_eq!(l.len(), 2);
        assert_eq!(
            (l[0].amount_minor, l[0].reference.as_deref()),
            (12_345_650, Some("UTR998877"))
        );
        assert_eq!(l[0].narration, "UPI-ACME, SETTLEMENT");
        assert_eq!(l[1].amount_minor, -1000);
        assert_eq!(l[1].reference, None);
        assert_eq!(p.header_line, 2);
        assert!(
            !p.warnings.is_empty(),
            "the footer is noted, not silently dropped"
        );
        // a named layout reads the same file
        assert_eq!(with(csv, "hdfc").lines.len(), 2);
    }

    #[test]
    fn single_amount_column_with_dr_cr_and_stable_fingerprints() {
        let csv = "Txn Date,Description,Amount,Dr/Cr\n2026-10-01,A,100.00,CR\n2026-10-01,A,100.00,CR\n2026-10-02,B,40.00,DR\n";
        let a = ok(csv);
        let b = ok(csv);
        assert_eq!(
            a.lines.iter().map(|x| x.amount_minor).collect::<Vec<_>>(),
            vec![10_000, 10_000, -4_000]
        );
        let fa: Vec<_> = a.lines.iter().map(|x| x.fingerprint.clone()).collect();
        assert_eq!(
            fa,
            b.lines
                .iter()
                .map(|x| x.fingerprint.clone())
                .collect::<Vec<_>>()
        );
        assert_ne!(fa[0], fa[1], "two identical rows are two transactions");
    }

    #[test]
    fn a_two_digit_year_is_read_as_this_century_by_every_layout() {
        let csv = "Date,Narration,Credit\n01/10/26,x,5.00\n";
        assert_eq!(ok(csv).lines[0].posted_on.to_string(), "2026-10-01");
        let csv = "Date,Narration,Credit\n01-10-26,x,5.00\n";
        assert_eq!(ok(csv).lines[0].posted_on.to_string(), "2026-10-01");
        let p = parse_statement(
            "Date,Narration,Credit\n01/10/0026,x,5.00\n",
            &Options::default(),
        );
        assert!(!p.errors.is_empty(), "year 26 is not a date");
    }

    #[test]
    fn overlapping_files_share_fingerprints_for_the_same_transactions() {
        let a = ok(
            "Date,Narration,Reference,Debit,Credit\n01/10/2026,X,R1,,5.00\n02/10/2026,Y,R2,,6.00\n",
        );
        let b = ok(
            "Date,Narration,Reference,Debit,Credit\n02/10/2026,Y,R2,,6.00\n03/10/2026,Z,R3,,7.00\n",
        );
        assert_eq!(a.lines[1].fingerprint, b.lines[0].fingerprint);
        assert_ne!(a.lines[0].fingerprint, b.lines[1].fingerprint);
    }

    #[test]
    fn refuses_to_guess() {
        let bad = |csv: &str| !parse_statement(csv, &Options::default()).errors.is_empty();
        assert!(bad("hello\nworld"));
        assert!(bad("Date,Narration,Deposit\n31/31/2026,x,10.00\n"));
        assert!(bad("Date,Narration,Deposit\n01/10/2026,x,ten\n"));
        assert!(bad("Date,Narration,Deposit\n"));
        assert!(bad(""));
    }

    #[test]
    fn every_problem_is_reported_with_its_line_and_nothing_is_half_read() {
        let csv = "Date,Narration,Debit,Credit\n01/10/2026,ok,,10.00\n32/10/2026,bad date,,5.00\n02/10/2026,two,3.00,4.00\n03/10/2026,comma,,\"1,50\"\n04/10/2026,dec,,1.005\n";
        let p = parse_statement(csv, &Options::default());
        let lines: Vec<_> = p.errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![3, 4, 5, 6], "{:?}", p.errors);
        assert!(!p.ok());
        assert_eq!(p.errors[0].column.as_deref(), Some("date"));
    }

    #[test]
    fn amounts_in_the_ways_indian_banks_write_them() {
        let a = |s: &str| {
            parse_amount(s)
                .unwrap()
                .map(|a| (a.minor, a.negative, a.credit))
        };
        assert_eq!(a("1,23,456.50"), Some((12_345_650, false, None)));
        assert_eq!(a("123,456.50"), Some((12_345_650, false, None)));
        assert_eq!(a("₹ 1,000"), Some((100_000, false, None)));
        assert_eq!(a("INR 12.5"), Some((1_250, false, None)));
        assert_eq!(a("-40.00"), Some((4_000, true, None)));
        assert_eq!(a("(40.00)"), Some((4_000, true, None)));
        assert_eq!(a("500.00 Dr"), Some((50_000, false, Some(false))));
        assert_eq!(a("500.00CR"), Some((50_000, false, Some(true))));
        assert_eq!(a(".50"), Some((50, false, None)));
        assert_eq!(a(""), None);
        for bad in [
            "1,50",
            "12,34,5",
            "1.2.3",
            "1,2,3",
            "abc",
            "1e3",
            "1.005",
            "99999999999999999999",
        ] {
            assert!(parse_amount(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn dr_cr_conflicts_are_errors() {
        let p = parse_statement(
            "Date,Description,Amount,Dr/Cr\n01/10/2026,A,-5.00,CR\n02/10/2026,B,5.00 Dr,CR\n03/10/2026,C,5.00,maybe\n",
            &Options::default(),
        );
        assert_eq!(p.errors.len(), 3, "{:?}", p.errors);
        let q = ok(
            "Date,Description,Amount,Dr/Cr\n01/10/2026,A,-5.00,DR\n02/10/2026,B,5.00 Dr,\n03/10/2026,C,9,C\n",
        );
        assert_eq!(
            q.lines.iter().map(|l| l.amount_minor).collect::<Vec<_>>(),
            vec![-500, -500, 900]
        );
    }

    #[test]
    fn a_spreadsheet_rounded_reference_is_refused() {
        let p = parse_statement(
            "Date,Narration,Reference,Credit\n01/10/2026,x,4.04223E+11,10.00\n",
            &Options::default(),
        );
        assert_eq!(p.errors[0].column.as_deref(), Some("reference"));
    }

    #[test]
    fn custom_mapping_delimiter_and_date_format() {
        let csv = "Posted;Memo;Money in;Money out;Ref\n10.01.2026;Hello;5,00;;X1\n11.01.2026;Bye;;2,00;X2\n";
        // semicolons are detected, but "5,00" is a decimal comma: refused rather than read as 500
        let p = parse_statement(
            csv,
            &Options {
                mapping: Some(ColumnMap {
                    date: Some("Posted".into()),
                    narration: Some("Memo".into()),
                    credit: Some("Money in".into()),
                    debit: Some("Money out".into()),
                    reference: Some("#5".into()),
                    ..Default::default()
                }),
                date_format: Some("%d.%m.%Y".into()),
                ..Default::default()
            },
        );
        assert_eq!(p.errors.len(), 2, "{:?}", p.errors);
        let csv2 = csv.replace("5,00", "5.00").replace("2,00", "2.00");
        let p = parse_statement(
            &csv2,
            &Options {
                mapping: Some(ColumnMap {
                    date: Some("Posted".into()),
                    narration: Some("Memo".into()),
                    credit: Some("Money in".into()),
                    debit: Some("Money out".into()),
                    reference: Some("#5".into()),
                    ..Default::default()
                }),
                date_format: Some("%d.%m.%Y".into()),
                ..Default::default()
            },
        );
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.lines[1].amount_minor, -200);
        assert_eq!(p.lines[1].reference.as_deref(), Some("X2"));
        assert_eq!(p.lines[0].posted_on.to_string(), "2026-01-10");
    }

    #[test]
    fn a_named_layout_that_does_not_fit_says_which_columns_are_missing() {
        let p = with("Date,Narration,Amount\n01/10/2026,x,5.00\n", "hdfc");
        assert!(p.errors[0].message.contains("not found"), "{:?}", p.errors);
        let p = with("x", "nosuchbank");
        assert!(p.errors[0].message.contains("unknown layout"));
    }

    #[test]
    fn quoted_fields_may_span_lines_and_an_unclosed_quote_is_refused() {
        let p = ok("Date,Narration,Credit\n01/10/2026,\"two\nlines, and a \"\"quote\"\"\",1.00\n");
        assert_eq!(p.lines[0].narration, "two lines, and a \"quote\"");
        let p = parse_statement(
            "Date,Narration,Credit\n01/10/2026,\"open,1.00\n",
            &Options::default(),
        );
        assert!(p.errors[0].message.contains("never closed"));
    }

    #[test]
    fn a_bom_crlf_and_tab_files_are_read() {
        let p = ok("\u{feff}Date\tNarration\tCredit\r\n01/10/2026\tx\t5.00\r\n");
        assert_eq!(p.lines[0].amount_minor, 500);
    }

    #[test]
    fn a_broken_running_balance_is_a_warning() {
        let p = ok(
            "Date,Narration,Credit,Debit,Balance\n01/10/2026,a,10.00,,110.00\n02/10/2026,b,5.00,,115.00\n03/10/2026,c,5.00,,999.00\n",
        );
        assert!(
            p.warnings
                .iter()
                .any(|w| w.column.as_deref() == Some("balance"))
        );
        let p = ok(
            "Date,Narration,Credit,Debit,Balance\n01/10/2026,a,10.00,,110.00\n02/10/2026,b,5.00,,115.00\n",
        );
        assert!(p.warnings.is_empty());
    }
}
