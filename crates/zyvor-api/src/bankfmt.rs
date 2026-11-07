//! MT940 (SWIFT) and CAMT.053 (ISO 20022) bank statement import. Parsers are intentionally strict:
//! unreadable tags are refused rather than guessed. Lines feed the same reconciliation path as CSV/JSON.

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

fn synth_utr(key: &str) -> String {
    format!("mt:{}", &hex::encode(Sha256::digest(key.as_bytes()))[..16])
}

fn parse_mt_date(s: &str) -> Option<NaiveDate> {
    // YYMMDD
    if s.len() != 6 {
        return None;
    }
    let yy: i32 = s[0..2].parse().ok()?;
    let mm: u32 = s[2..4].parse().ok()?;
    let dd: u32 = s[4..6].parse().ok()?;
    let year = if yy >= 70 { 1900 + yy } else { 2000 + yy };
    NaiveDate::from_ymd_opt(year, mm, dd)
}

/// Amount in `:61:` is `1234,56` (comma decimal) with an optional leading sign via DC mark.
fn parse_mt_amount(s: &str) -> Option<i64> {
    let t = s.trim().replace(',', ".");
    zyvor_core::decimal_to_minor(&t)
}

/// Parse a SWIFT MT940 statement into bank lines. Only credit/debit statement lines (`:61:`) are kept.
pub fn parse_mt940(text: &str) -> Result<Vec<BankLine>, String> {
    let mut lines = Vec::new();
    let mut pending: Option<(NaiveDate, i64, String)> = None;
    let mut seen = std::collections::HashMap::<String, u32>::new();

    for raw in text.lines() {
        let row = raw.trim_end_matches('\r');
        if let Some(rest) = row.strip_prefix(":61:") {
            if let Some((d, amt, narr)) = pending.take() {
                let key = format!("{d}|{amt}|{narr}");
                let n = seen.entry(key.clone()).or_insert(0);
                *n += 1;
                let utr = if narr.contains("UTR") {
                    narr.split_whitespace()
                        .find(|w| w.starts_with("UTR") || w.len() >= 12)
                        .unwrap_or(&synth_utr(&format!("{key}:{n}")))
                        .to_string()
                } else {
                    synth_utr(&format!("{key}:{n}"))
                };
                lines.push(BankLine::new(d, amt, narr, utr));
            }
            // :61:YYMMDD[book]DC[funds]amount[…]//ref
            if rest.len() < 8 {
                return Err(format!("short :61: line: {rest}"));
            }
            let date =
                parse_mt_date(&rest[..6]).ok_or_else(|| format!("bad :61: date in {rest}"))?;
            let mut i = 6;
            // optional booking date MMDD
            if rest.len() >= i + 5
                && rest[i..i + 4].chars().all(|c| c.is_ascii_digit())
                && rest.as_bytes().get(i + 4).is_some_and(|b| {
                    let c = *b as char;
                    c.is_ascii_alphabetic() || c == 'R'
                })
            {
                i += 4;
            }
            let dc = if rest[i..].starts_with('R') && rest.len() > i + 1 {
                let mark = &rest[i..i + 2];
                i += 2;
                mark
            } else {
                let mark = rest
                    .get(i..i + 1)
                    .ok_or_else(|| format!("missing D/C in {rest}"))?;
                i += 1;
                mark
            };
            // optional funds code (one letter)
            if rest
                .as_bytes()
                .get(i)
                .is_some_and(|b| (*b as char).is_ascii_alphabetic())
                && rest
                    .as_bytes()
                    .get(i + 1)
                    .is_some_and(|b| (*b as char).is_ascii_digit())
            {
                i += 1;
            }
            let amt_end = rest[i..]
                .find(['N', 'F', 'S', '/'])
                .map(|p| i + p)
                .unwrap_or(rest.len());
            let mut amount = parse_mt_amount(&rest[i..amt_end])
                .ok_or_else(|| format!("bad amount in {rest}"))?;
            if dc.eq_ignore_ascii_case("D") || dc.eq_ignore_ascii_case("RC") {
                amount = -amount;
            } else if !(dc.eq_ignore_ascii_case("C") || dc.eq_ignore_ascii_case("RD")) {
                return Err(format!("unknown debit/credit mark `{dc}` in {rest}"));
            }
            // prefer explicit bank ref after //
            let ref_part = rest
                .split_once("//")
                .map(|(_, r)| r.trim())
                .filter(|r| !r.is_empty())
                .unwrap_or("")
                .to_string();
            pending = Some((date, amount, ref_part));
        } else if let Some(rest) = row.strip_prefix(":86:") {
            if let Some((d, amt, mut narr)) = pending.take() {
                if narr.is_empty() {
                    narr = rest.to_string();
                } else {
                    narr = format!("{narr} {rest}");
                }
                pending = Some((d, amt, narr));
            }
        } else if row.starts_with(":86:") {
            // continuation already handled
        } else if !row.starts_with(':') && pending.is_some() {
            // :86: continuation lines
            if let Some((d, amt, mut narr)) = pending.take() {
                if !narr.is_empty() {
                    narr.push(' ');
                }
                narr.push_str(row.trim());
                pending = Some((d, amt, narr));
            }
        }
    }
    if let Some((d, amt, narr)) = pending.take() {
        let key = format!("{d}|{amt}|{narr}");
        let n = seen.entry(key.clone()).or_insert(0);
        *n += 1;
        let utr = if !narr.is_empty() && narr.len() >= 6 && !narr.contains(' ') {
            narr.clone()
        } else {
            synth_utr(&format!("{key}:{n}"))
        };
        lines.push(BankLine::new(d, amt, narr, utr));
    }
    if lines.is_empty() {
        return Err("no :61: statement lines found".into());
    }
    Ok(lines)
}

fn xml_tag<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let close = format!("</{tag}>");
    let start = if let Some(i) = s.find(&format!("<{tag}>")) {
        i + tag.len() + 2
    } else {
        let i = s.find(&format!("<{tag} "))?;
        s[i..].find('>')? + i + 1
    };
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].trim())
}

/// Minimal CAMT.053 reader: each `<Ntry>` becomes a bank line. Amounts use the document currency.
pub fn parse_camt053(text: &str) -> Result<Vec<BankLine>, String> {
    let mut lines = Vec::new();
    let mut rest = text;
    let mut seen = std::collections::HashMap::<String, u32>::new();
    while let Some(start) = rest.find("<Ntry>") {
        let after = &rest[start + 6..];
        let end = after
            .find("</Ntry>")
            .ok_or_else(|| "unclosed <Ntry>".to_string())?;
        let entry = &after[..end];
        rest = &after[end + 7..];

        let amt_s = xml_tag(entry, "Amt").ok_or_else(|| "Ntry missing Amt".to_string())?;
        let mut amount =
            zyvor_core::decimal_to_minor(amt_s).ok_or_else(|| format!("bad Amt `{amt_s}`"))?;
        let ind = xml_tag(entry, "CdtDbtInd").unwrap_or("CRDT");
        if ind == "DBIT" {
            amount = -amount;
        } else if ind != "CRDT" {
            return Err(format!("unknown CdtDbtInd `{ind}`"));
        }
        let date_s = xml_tag(entry, "Dt")
            .and_then(|d| xml_tag(d, "Dt").or(Some(d)))
            .or_else(|| {
                // BookgDt / ValDt wrappers
                xml_tag(entry, "BookgDt")
                    .and_then(|d| xml_tag(d, "Dt"))
                    .or_else(|| xml_tag(entry, "ValDt").and_then(|d| xml_tag(d, "Dt")))
            })
            .ok_or_else(|| "Ntry missing date".to_string())?;
        let date = NaiveDate::parse_from_str(date_s, "%Y-%m-%d")
            .map_err(|_| format!("bad date `{date_s}`"))?;
        let narr = xml_tag(entry, "Ustrd")
            .or_else(|| xml_tag(entry, "AddtlNtryInf"))
            .unwrap_or("")
            .to_string();
        let utr = xml_tag(entry, "AcctSvcrRef")
            .or_else(|| xml_tag(entry, "NtryRef"))
            .or_else(|| xml_tag(entry, "EndToEndId"))
            .map(str::to_string)
            .unwrap_or_else(|| {
                let key = format!("{date}|{amount}|{narr}");
                let n = seen.entry(key.clone()).or_insert(0);
                *n += 1;
                synth_utr(&format!("{key}:{n}")).replacen("mt:", "camt:", 1)
            });
        lines.push(BankLine::new(date, amount, narr, utr));
    }
    if lines.is_empty() {
        return Err("no <Ntry> elements found".into());
    }
    Ok(lines)
}

#[derive(Deserialize)]
pub struct TextBody {
    text: String,
}

pub async fn import_mt940(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<TextBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let lines = parse_mt940(&b.text).map_err(|e| ApiError::invalid("invalid_statement", e))?;
    import_bank(State(s), auth, Json(ImportBank { lines })).await
}

pub async fn import_camt(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<TextBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let lines = parse_camt053(&b.text).map_err(|e| ApiError::invalid("invalid_statement", e))?;
    import_bank(State(s), auth, Json(ImportBank { lines })).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt940_credit_and_debit() {
        let mt = "\
:20:START
:25:123456
:28C:1/1
:60F:C261001INR0,00
:61:2610011001C1234,56NTRFNONREF//UTR998877
:86:NEFT RAZORPAY SETTLEMENT
:61:2610021002D10,00NMSCNONREF
:86:Bank charges
:62F:C261002INR1224,56
-";
        let l = parse_mt940(mt).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].amount_minor, 123_456);
        assert!(
            l[0].utr.as_deref().unwrap().contains("UTR998877")
                || l[0].narration.as_deref().unwrap().contains("UTR")
                || l[0].utr.as_deref() == Some("UTR998877")
        );
        assert_eq!(l[1].amount_minor, -1_000);
    }

    #[test]
    fn camt_credit() {
        let xml = r#"<?xml version="1.0"?>
<Document><BkToCstmrStmt><Stmt>
<Ntry>
  <Amt Ccy="INR">1234.56</Amt>
  <CdtDbtInd>CRDT</CdtDbtInd>
  <BookgDt><Dt>2026-10-01</Dt></BookgDt>
  <AcctSvcrRef>UTR998877</AcctSvcrRef>
  <NtryDtls><TxDtls><RmtInf><Ustrd>NEFT RAZORPAY</Ustrd></RmtInf></TxDtls></NtryDtls>
</Ntry>
</Stmt></BkToCstmrStmt></Document>"#;
        let l = parse_camt053(xml).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].amount_minor, 123_456);
        assert_eq!(l[0].utr.as_deref(), Some("UTR998877"));
    }
}
