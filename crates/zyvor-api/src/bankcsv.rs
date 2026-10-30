//! Bank statement CSV import. Banks disagree on column names, date formats and whether debits and
//! credits share a column, so the parser finds the header row, maps columns by name and refuses
//! anything it can't read rather than guessing: a wrong sign on a credit is a wrong reconciliation.

use axum::{Json, extract::State};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    ApiError, AppState,
    auth::Auth,
    recon::{BankLine, ImportBank, import_bank},
};

/// Split one CSV record, honouring double quotes and `""` escapes.
fn split_record(line: &str) -> Vec<String> {
    let (mut out, mut cur, mut q) = (Vec::new(), String::new(), false);
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match (c, q) {
            ('"', true) if it.peek() == Some(&'"') => {
                cur.push('"');
                it.next();
            }
            ('"', _) => q = !q,
            (',', false) => out.push(std::mem::take(&mut cur).trim().to_string()),
            (c, _) => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    [
        "%d/%m/%Y", "%d-%m-%Y", "%d/%m/%y", "%d-%m-%y", "%Y-%m-%d", "%d-%b-%Y", "%d %b %Y",
        "%d-%b-%y",
    ]
    .iter()
    .find_map(|f| NaiveDate::parse_from_str(s.trim(), f).ok())
}

/// "1,23,456.50" -> paise. Empty means no amount in this column.
fn parse_amount(s: &str) -> Result<Option<i64>, String> {
    let t: String = s
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    let t = t.trim_start_matches('₹').trim_start_matches("INR");
    if t.is_empty() || t == "-" {
        return Ok(None);
    }
    let (neg, digits) = match t.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, t),
    };
    let digits = if digits.starts_with('.') {
        format!("0{digits}")
    } else {
        digits.to_string()
    };
    let m =
        zyvor_core::decimal_to_minor(&digits).ok_or_else(|| format!("unreadable amount `{s}`"))?;
    Ok(Some(if neg { -m } else { m }))
}

fn find(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|h| {
        let h = h.to_lowercase();
        names.iter().any(|n| h == *n || h.starts_with(n))
    })
}

pub fn parse(csv: &str) -> Result<Vec<BankLine>, String> {
    let rows: Vec<Vec<String>> = csv
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(split_record)
        .collect();
    // statements often start with account details; the header is the first row that names a date and an amount
    let hi = rows
        .iter()
        .position(|r| {
            find(r, &["date", "txn date", "transaction date", "value date"]).is_some()
                && (find(r, &["deposit", "credit", "amount", "cr"]).is_some())
        })
        .ok_or(
            "no header row found: expected a date column and a credit/deposit or amount column",
        )?;
    let h = &rows[hi];
    let date = find(h, &["transaction date", "txn date", "date", "value date"]).unwrap();
    let narr = find(h, &["narration", "description", "particulars", "remarks"]);
    let utr = find(
        h,
        &[
            "utr",
            "chq./ref.no",
            "chq/ref no",
            "ref no",
            "reference",
            "ref",
        ],
    );
    let credit = find(h, &["deposit", "credit"]);
    let debit = find(h, &["withdrawal", "debit"]);
    let amount = find(h, &["amount"]);
    let drcr = find(h, &["dr/cr", "type"]);
    if credit.is_none() && amount.is_none() {
        return Err("no credit/deposit or amount column".into());
    }
    let mut out = Vec::new();
    let mut seen: std::collections::HashMap<String, u32> = Default::default();
    for (n, r) in rows.iter().enumerate().skip(hi + 1) {
        let cell = |i: Option<usize>| i.and_then(|i| r.get(i)).map(String::as_str).unwrap_or("");
        let line_no = n + 1;
        let Some(d) = parse_date(cell(Some(date))) else {
            // footers such as "Closing balance" are skipped; a half-parsed data row is an error
            let raw = cell(Some(date));
            if raw.chars().any(|c| c.is_ascii_digit())
                && raw.chars().any(|c| matches!(c, '/' | '-' | ' '))
            {
                return Err(format!(
                    "row {line_no}: unreadable date `{}`",
                    cell(Some(date))
                ));
            }
            continue;
        };
        let signed = if let Some(a) = amount {
            let v = parse_amount(cell(Some(a))).map_err(|e| format!("row {line_no}: {e}"))?;
            v.map(|v| {
                let t = cell(drcr).to_lowercase();
                if t.starts_with("dr") || t.starts_with("debit") {
                    -v.abs()
                } else {
                    v
                }
            })
        } else {
            let c = parse_amount(cell(credit)).map_err(|e| format!("row {line_no}: {e}"))?;
            let dbt = parse_amount(cell(debit)).map_err(|e| format!("row {line_no}: {e}"))?;
            match (c, dbt) {
                (Some(c), None) => Some(c.abs()),
                (None, Some(d)) => Some(-d.abs()),
                (Some(c), Some(d)) => Some(c.abs() - d.abs()),
                (None, None) => None,
            }
        };
        let Some(amount_minor) = signed.filter(|v| *v != 0) else {
            continue;
        };
        let narration = cell(narr).to_string();
        let given = cell(utr).trim_matches('\'').to_string();
        // rows with no reference get a stable synthetic one, so importing the file twice adds nothing
        let utr_final = if given.is_empty() {
            let key = format!("{d}|{amount_minor}|{narration}");
            let k = seen.entry(key.clone()).or_insert(0);
            *k += 1;
            format!(
                "csv:{}:{k}",
                &hex::encode(Sha256::digest(key.as_bytes()))[..16]
            )
        } else {
            given
        };
        out.push(BankLine::new(d, amount_minor, narration, utr_final));
    }
    if out.is_empty() {
        return Err("the file has a header but no transactions".into());
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CsvBody {
    csv: String,
}

/// `POST /v1/bank_statements/import_csv` with `{ "csv": "<file contents>" }`.
pub async fn import_csv(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<CsvBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let lines = parse(&b.csv).map_err(|e| ApiError::invalid("invalid_statement", e))?;
    import_bank(State(s), auth, Json(ImportBank { lines })).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdfc_style_with_debit_and_credit_columns() {
        let csv = "Account No,1234\nDate,Narration,Chq./Ref.No.,Value Dt,Withdrawal Amt.,Deposit Amt.,Closing Balance\n01/10/26,\"UPI-ACME, SETTLEMENT\",'UTR998877,01/10/26,,\"1,23,456.50\",9\n02/10/26,Bank charges,,02/10/26,10.00,,8\nClosing balance,,,,,,8\n";
        let l = parse(csv).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(
            (l[0].amount_minor, l[0].utr.as_deref()),
            (12_345_650, Some("UTR998877"))
        );
        assert_eq!(l[0].narration.as_deref(), Some("UPI-ACME, SETTLEMENT"));
        assert_eq!(l[1].amount_minor, -1000);
    }

    #[test]
    fn single_amount_column_with_dr_cr_and_stable_synthetic_refs() {
        let csv = "Txn Date,Description,Amount,Dr/Cr\n2026-10-01,A,100.00,CR\n2026-10-01,A,100.00,CR\n2026-10-02,B,40.00,DR\n";
        let a = parse(csv).unwrap();
        let b = parse(csv).unwrap();
        assert_eq!(
            a.iter().map(|x| x.amount_minor).collect::<Vec<_>>(),
            vec![10_000, 10_000, -4_000]
        );
        let refs: Vec<_> = a.iter().map(|x| x.utr.clone()).collect();
        assert_eq!(refs, b.iter().map(|x| x.utr.clone()).collect::<Vec<_>>());
        assert_ne!(refs[0], refs[1], "two identical rows are two transactions");
    }

    #[test]
    fn refuses_to_guess() {
        assert!(parse("hello\nworld").is_err());
        assert!(parse("Date,Narration,Deposit\n31/31/2026,x,10.00\n").is_err());
        assert!(parse("Date,Narration,Deposit\n01/10/2026,x,ten\n").is_err());
        assert!(parse("Date,Narration,Deposit\n").is_err());
    }
}
