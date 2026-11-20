// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Accounting exports: ledger-derived journals as CSV (a generic double-entry layout) and as a Tally-style
//! voucher CSV or XML, plus the GST summary. Read-only: nothing here posts to the ledger or moves money.
//!
//! Every voucher balances (debits equal credits) or the export is refused. Output is deterministic: vouchers are
//! ordered by (IST day, voucher number) with the number collated bytewise, lines by debit-first then posting order,
//! and nothing time-dependent is written. Exports are paged with an opaque cursor, scoped to the tenant by
//! row-level security, and every page is written to the audit trail (who, what range, how much, a SHA-256 of the
//! bytes returned).
//!
//! Where the lines come from:
//!   * `ledger_transactions` / `ledger_entries`: captures, refunds, payouts, splits, disputes and acquire-mode
//!     settlement batches, exactly as posted.
//!   * `settlements` (imported provider settlement records): the ledger does not post these today, so a settlement
//!     voucher (bank in, provider receivable out) and a fee voucher (processing fee out of the receivable) are
//!     BUILT from the record. They are labelled `settlement_record` / `fee_record` sources in the JSON output.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{Duration, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::{ApiError, AppState, auth::Auth, tenant};

/// Most vouchers one page may carry.
pub const MAX_PAGE: i64 = 5_000;
pub const DEFAULT_PAGE: i64 = 1_000;
/// Longest date range one export call may cover.
pub const MAX_RANGE_DAYS: i64 = 366;
/// Code given to a ledger account nobody mapped and the built-in chart does not know.
pub const UNMAPPED_CODE: &str = "9999";

// ---------- account mapping ----------

/// The built-in chart: a stable code and name per ledger account (or `prefix:*` family). A tenant mapping
/// overrides any of these. The codes are placeholders to import into your own chart of accounts, not statutory.
pub const DEFAULT_CHART: &[(&str, &str, &str)] = &[
    ("bank", "1000", "Bank"),
    (
        "provider_receivable:*",
        "1200",
        "Payment provider receivable",
    ),
    ("escrow:in_flight", "1210", "Escrow in flight"),
    ("escrow:available", "1220", "Escrow available"),
    ("settlement_clearing", "1230", "Settlement clearing"),
    ("merchant_payable:*", "2100", "Merchant payable"),
    ("vendor_payable:*", "2200", "Vendor payable"),
    ("collections", "4000", "Collections"),
    ("fee_income", "4100", "Fee income"),
    ("processing_fees", "5100", "Payment processing fees"),
    ("payouts", "5200", "Payouts to vendors"),
    ("settlement_adjustments", "5900", "Settlement adjustments"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub code: String,
    pub name: String,
    /// False when neither a tenant mapping nor the built-in chart covered the account.
    pub mapped: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Mapping {
    exact: BTreeMap<String, (String, String)>,
    /// `(prefix including the colon, code, name)`, longest prefix first.
    prefixes: Vec<(String, String, String)>,
}

impl Mapping {
    /// Build from a tenant's overrides: `(ledger_account, code, name)`; a trailing `:*` makes it a prefix.
    pub fn from_overrides(rows: &[(String, String, String)]) -> Self {
        let mut m = Mapping::default();
        for (acct, code, name) in rows {
            match acct.strip_suffix('*') {
                Some(prefix) if prefix.ends_with(':') => {
                    m.prefixes
                        .push((prefix.to_string(), code.clone(), name.clone()));
                }
                _ => {
                    m.exact.insert(acct.clone(), (code.clone(), name.clone()));
                }
            }
        }
        m.prefixes
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        m
    }

    /// Tenant exact, tenant prefix (longest), built-in exact, built-in prefix, else unmapped.
    pub fn resolve(&self, account: &str) -> Resolved {
        if let Some((c, n)) = self.exact.get(account) {
            return Resolved {
                code: c.clone(),
                name: n.clone(),
                mapped: true,
            };
        }
        if let Some((_, c, n)) = self
            .prefixes
            .iter()
            .find(|(p, _, _)| account.starts_with(p))
        {
            return Resolved {
                code: c.clone(),
                name: n.clone(),
                mapped: true,
            };
        }
        for (key, code, name) in DEFAULT_CHART {
            let hit = match key.strip_suffix('*') {
                Some(p) => account.starts_with(p),
                None => *key == account,
            };
            if hit {
                return Resolved {
                    code: (*code).into(),
                    name: (*name).into(),
                    mapped: true,
                };
            }
        }
        Resolved {
            code: UNMAPPED_CODE.into(),
            name: account.to_string(),
            mapped: false,
        }
    }
}

// ---------- vouchers ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub account: String,
    pub debit: i64,
    pub credit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voucher {
    pub date: NaiveDate,
    pub number: String,
    /// `payments`, `refunds`, `payouts`, `settlements`, `fees` or `other`.
    pub kind: &'static str,
    /// `ledger`, `settlement_record` or `fee_record`.
    pub source: &'static str,
    pub currency: String,
    pub narration: String,
    pub lines: Vec<Line>,
}

impl Voucher {
    pub fn totals(&self) -> (i64, i64) {
        (
            self.lines.iter().map(|l| l.debit).sum(),
            self.lines.iter().map(|l| l.credit).sum(),
        )
    }
    pub fn balanced(&self) -> bool {
        let (d, c) = self.totals();
        d == c && !self.lines.is_empty()
    }
}

pub const KINDS: &[&str] = &[
    "payments",
    "refunds",
    "fees",
    "settlements",
    "payouts",
    "other",
];

/// The export kind of a ledger reference, from the prefix the posting code gives it.
pub fn kind_of_reference(reference: &str) -> &'static str {
    let prefix = reference.split(':').next().unwrap_or("");
    match prefix {
        "capture" => "payments",
        "refund" => "refunds",
        "payout" => "payouts",
        "settle_release" | "settle_bank" | "escrow_avail" => "settlements",
        _ => "other",
    }
}

fn narration_for(kind: &str, reference: &str) -> String {
    let what = match kind {
        "payments" => "Payment captured",
        "refunds" => "Refund processed",
        "payouts" => "Payout paid",
        "settlements" => "Settlement",
        "fees" => "Provider fees",
        _ => "Ledger posting",
    };
    format!("{what} {reference}")
}

/// A provider settlement record as stored.
#[derive(Debug, Clone)]
pub struct SettlementRecord {
    pub provider: String,
    pub provider_ref: String,
    pub currency: String,
    pub gross_minor: i64,
    pub refunds_minor: i64,
    pub fee_minor: i64,
    pub net_minor: i64,
    pub settled_on: NaiveDate,
}

/// Signed lines (positive = debit, negative = credit) to vouchers lines; zero lines are dropped.
fn lines_from_signed(signed: &[(String, i64)]) -> Vec<Line> {
    signed
        .iter()
        .filter(|(_, v)| *v != 0)
        .map(|(a, v)| Line {
            account: a.clone(),
            debit: (*v).max(0),
            credit: (-*v).max(0),
        })
        .collect()
}

/// The fee voucher for a settlement record: the processing fee the provider deducted comes out of what the
/// provider holds for you. `None` when no fee was deducted.
pub fn fee_voucher(r: &SettlementRecord) -> Option<Voucher> {
    if r.fee_minor == 0 {
        return None;
    }
    let receivable = zyvor_ledger::accounts::provider_receivable(&r.provider);
    let lines = lines_from_signed(&[
        (
            zyvor_ledger::accounts::PROCESSING_FEES.to_string(),
            r.fee_minor,
        ),
        (receivable, -r.fee_minor),
    ]);
    Some(Voucher {
        date: r.settled_on,
        number: format!("fee:{}:{}", r.provider, r.provider_ref),
        kind: "fees",
        source: "fee_record",
        currency: r.currency.clone(),
        narration: narration_for("fees", &format!("{} {}", r.provider, r.provider_ref)),
        lines,
    })
}

/// The settlement voucher for a record: the bank received `net`; the provider receivable falls by gross less
/// refunds less fees (the fee voucher takes the fees); whatever difference is left (adjustments, reserves, a net
/// the provider stated that does not follow from its own lines) goes to the adjustments account so the voucher
/// always balances and the difference stays visible. `None` when everything is zero.
pub fn settlement_voucher(r: &SettlementRecord) -> Option<Voucher> {
    let receivable_fall = r.gross_minor - r.refunds_minor - r.fee_minor;
    let adjustment = r.net_minor - receivable_fall;
    let lines = lines_from_signed(&[
        (zyvor_ledger::accounts::BANK.to_string(), r.net_minor),
        (
            zyvor_ledger::accounts::provider_receivable(&r.provider),
            -receivable_fall,
        ),
        (zyvor_ledger::accounts::ADJUSTMENTS.to_string(), -adjustment),
    ]);
    if lines.is_empty() {
        return None;
    }
    Some(Voucher {
        date: r.settled_on,
        number: format!("settlement:{}:{}", r.provider, r.provider_ref),
        kind: "settlements",
        source: "settlement_record",
        currency: r.currency.clone(),
        narration: narration_for("settlements", &format!("{} {}", r.provider, r.provider_ref)),
        lines,
    })
}

// ---------- rendering ----------

/// `12345` -> `123.45`; `-5` -> `-0.05`. Integer arithmetic only (every supported currency has 2 decimals).
pub fn decimal(minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let a = minor.unsigned_abs();
    format!("{sign}{}.{:02}", a / 100, a % 100)
}

/// A CSV field: quoted when it holds a comma, quote or line break; a leading `=`, `+`, `-`, `@`, tab or CR is
/// neutralised with a leading apostrophe so a spreadsheet never reads text as a formula.
pub fn csv_field(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

pub const GENERIC_HEADER: &str = "voucher_no,date,kind,line,account_code,account_name,ledger_account,debit,credit,currency,narration\n";

/// The generic double-entry journal: one row per voucher line, debit and credit as decimal amounts.
pub fn render_generic(vouchers: &[Voucher], mapping: &Mapping, header: bool) -> String {
    let mut out = String::new();
    if header {
        out.push_str(GENERIC_HEADER);
    }
    for v in vouchers {
        for (i, l) in v.lines.iter().enumerate() {
            let r = mapping.resolve(&l.account);
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&v.number),
                v.date,
                v.kind,
                i + 1,
                csv_field(&r.code),
                csv_field(&r.name),
                csv_field(&l.account),
                if l.debit > 0 {
                    decimal(l.debit)
                } else {
                    String::new()
                },
                if l.credit > 0 {
                    decimal(l.credit)
                } else {
                    String::new()
                },
                v.currency,
                csv_field(&v.narration),
            ));
        }
    }
    out
}

pub const TALLY_CSV_HEADER: &str =
    "Date,Voucher Type,Voucher Number,Ledger Name,Debit,Credit,Narration\n";

/// A flat voucher CSV in Tally's field vocabulary (date as `YYYYMMDD`, voucher type, voucher number, ledger name,
/// narration). Tally itself imports XML, not CSV; this layout is for spreadsheet-based or third-party converters.
pub fn render_tally_csv(vouchers: &[Voucher], mapping: &Mapping, header: bool) -> String {
    let mut out = String::new();
    if header {
        out.push_str(TALLY_CSV_HEADER);
    }
    for v in vouchers {
        for l in &v.lines {
            let r = mapping.resolve(&l.account);
            out.push_str(&format!(
                "{},Journal,{},{},{},{},{}\n",
                v.date.format("%Y%m%d"),
                csv_field(&v.number),
                csv_field(&r.name),
                if l.debit > 0 {
                    decimal(l.debit)
                } else {
                    String::new()
                },
                if l.credit > 0 {
                    decimal(l.credit)
                } else {
                    String::new()
                },
                csv_field(&v.narration),
            ));
        }
    }
    out
}

/// Tally import XML. The envelope and the voucher fields follow the sample in Tally's public help pages
/// (header `VERSION 1 / TALLYREQUEST Import / TYPE Data / ID Vouchers`; `DATE` as `YYYYMMDD`; `LEDGERENTRIES.LIST`
/// with `LEDGERNAME`, `ISDEEMEDPOSITIVE` and `AMOUNT`, where a debit is `Yes` with a negative amount and a credit
/// is `No` with a positive amount). Vouchers are Journals; that voucher type is not in the published samples (they
/// show Sales), so this has NOT been run against Tally.
pub fn render_tally_xml(vouchers: &[Voucher], mapping: &Mapping) -> String {
    let mut o = String::new();
    o.push_str("<ENVELOPE>\n <HEADER>\n  <VERSION>1</VERSION>\n  <TALLYREQUEST>Import</TALLYREQUEST>\n  <TYPE>Data</TYPE>\n  <ID>Vouchers</ID>\n </HEADER>\n <BODY>\n  <DESC>\n  </DESC>\n  <DATA>\n   <TALLYMESSAGE>\n");
    for v in vouchers {
        o.push_str(&format!(
            "    <VOUCHER VCHTYPE=\"Journal\" ACTION=\"Create\">\n     <DATE>{}</DATE>\n     <VOUCHERTYPENAME>Journal</VOUCHERTYPENAME>\n     <VOUCHERNUMBER>{}</VOUCHERNUMBER>\n     <NARRATION>{}</NARRATION>\n",
            v.date.format("%Y%m%d"),
            xml_escape(&v.number),
            xml_escape(&v.narration),
        ));
        for l in &v.lines {
            let r = mapping.resolve(&l.account);
            let (yes, amount) = if l.debit > 0 {
                ("Yes", format!("-{}", decimal(l.debit)))
            } else {
                ("No", decimal(l.credit))
            };
            o.push_str(&format!(
                "     <LEDGERENTRIES.LIST>\n      <LEDGERNAME>{}</LEDGERNAME>\n      <ISDEEMEDPOSITIVE>{yes}</ISDEEMEDPOSITIVE>\n      <AMOUNT>{amount}</AMOUNT>\n     </LEDGERENTRIES.LIST>\n",
                xml_escape(&r.name),
            ));
        }
        o.push_str("    </VOUCHER>\n");
    }
    o.push_str("   </TALLYMESSAGE>\n  </DATA>\n </BODY>\n</ENVELOPE>\n");
    o
}

pub fn cursor_encode(day: NaiveDate, reference: &str) -> String {
    hex::encode(format!("{day}\u{1f}{reference}"))
}

pub fn cursor_decode(c: &str) -> Option<(NaiveDate, String)> {
    let raw = String::from_utf8(hex::decode(c).ok()?).ok()?;
    let (d, r) = raw.split_once('\u{1f}')?;
    Some((
        NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()?,
        r.to_string(),
    ))
}

// ---------- loading ----------

async fn load_mapping(conn: &mut PgConnection) -> Result<Mapping, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT ledger_account, code, name FROM account_mappings ORDER BY ledger_account",
    )
    .fetch_all(conn)
    .await?;
    Ok(Mapping::from_overrides(&rows))
}

#[derive(Debug)]
pub struct Page {
    pub vouchers: Vec<Voucher>,
    pub next: Option<String>,
}

/// One page of vouchers for `[from, to]` (IST days, inclusive), after the cursor, in the export order.
pub async fn load_page(
    conn: &mut PgConnection,
    from: NaiveDate,
    to: NaiveDate,
    currency: &str,
    kinds: &[String],
    after: Option<(NaiveDate, String)>,
    limit: i64,
) -> Result<Page, ApiError> {
    let (a_day, a_ref) = match &after {
        Some((d, r)) => (Some(*d), Some(r.clone())),
        None => (None, None),
    };
    let heads: Vec<(NaiveDate, String, String, String)> = sqlx::query_as(
        "SELECT day, ref, src, id FROM (
           SELECT (t.created_at AT TIME ZONE 'Asia/Kolkata')::date AS day, (t.reference COLLATE \"C\") AS ref, 'ledger' AS src, t.id AS id,
                  CASE split_part(t.reference, ':', 1)
                    WHEN 'capture' THEN 'payments' WHEN 'refund' THEN 'refunds' WHEN 'payout' THEN 'payouts'
                    WHEN 'settle_release' THEN 'settlements' WHEN 'settle_bank' THEN 'settlements' WHEN 'escrow_avail' THEN 'settlements'
                    ELSE 'other' END AS kind
           FROM ledger_transactions t
           WHERE EXISTS (SELECT 1 FROM ledger_entries e WHERE e.transaction_id = t.id AND e.currency = $3)
           UNION ALL
           SELECT s.settled_on, ('settlement:' || s.provider || ':' || s.provider_ref) COLLATE \"C\", 'settlement_record', s.id, 'settlements'
           FROM settlements s WHERE s.currency = $3
           UNION ALL
           SELECT s.settled_on, ('fee:' || s.provider || ':' || s.provider_ref) COLLATE \"C\", 'fee_record', s.id, 'fees'
           FROM settlements s WHERE s.currency = $3 AND s.fee_minor <> 0
         ) h
         WHERE day >= $1 AND day <= $2 AND kind = ANY($4)
           AND ($5::date IS NULL OR (day, ref) > ($5::date, $6::text COLLATE \"C\"))
         ORDER BY day, ref LIMIT $7",
    )
    .bind(from)
    .bind(to)
    .bind(currency)
    .bind(kinds)
    .bind(a_day)
    .bind(a_ref)
    .bind(limit + 1)
    .fetch_all(&mut *conn)
    .await?;
    let has_more = heads.len() as i64 > limit;
    let heads: Vec<_> = heads.into_iter().take(limit as usize).collect();
    let next = if has_more {
        heads.last().map(|(d, r, _, _)| cursor_encode(*d, r))
    } else {
        None
    };

    let ledger_ids: Vec<String> = heads
        .iter()
        .filter(|h| h.2 == "ledger")
        .map(|h| h.3.clone())
        .collect();
    let record_ids: Vec<String> = heads
        .iter()
        .filter(|h| h.2 != "ledger")
        .map(|h| h.3.clone())
        .collect();

    let entries: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT transaction_id, account, side, amount_minor FROM ledger_entries
         WHERE transaction_id = ANY($1) AND currency = $2
         ORDER BY transaction_id, (side <> 'debit'), id",
    )
    .bind(&ledger_ids)
    .bind(currency)
    .fetch_all(&mut *conn)
    .await?;
    let mut by_tx: BTreeMap<String, Vec<Line>> = BTreeMap::new();
    for (tx, account, side, amount) in entries {
        by_tx.entry(tx).or_default().push(Line {
            account,
            debit: if side == "debit" { amount } else { 0 },
            credit: if side == "credit" { amount } else { 0 },
        });
    }
    let records: Vec<(String, String, String, String, i64, i64, i64, i64, NaiveDate)> =
        sqlx::query_as(
            "SELECT id, provider, provider_ref, currency, gross_minor, refunds_minor, fee_minor, net_minor, settled_on
             FROM settlements WHERE id = ANY($1)",
        )
        .bind(&record_ids)
        .fetch_all(&mut *conn)
        .await?;
    let by_record: BTreeMap<String, SettlementRecord> = records
        .into_iter()
        .map(|(id, provider, provider_ref, currency, g, r, f, n, on)| {
            (
                id,
                SettlementRecord {
                    provider,
                    provider_ref,
                    currency,
                    gross_minor: g,
                    refunds_minor: r,
                    fee_minor: f,
                    net_minor: n,
                    settled_on: on,
                },
            )
        })
        .collect();

    let mut vouchers = Vec::with_capacity(heads.len());
    for (day, reference, src, id) in heads {
        match src.as_str() {
            "ledger" => {
                let kind = kind_of_reference(&reference);
                vouchers.push(Voucher {
                    date: day,
                    narration: narration_for(kind, &reference),
                    number: reference,
                    kind,
                    source: "ledger",
                    currency: currency.to_string(),
                    lines: by_tx.remove(&id).unwrap_or_default(),
                });
            }
            "fee_record" => {
                if let Some(v) = by_record.get(&id).and_then(fee_voucher) {
                    vouchers.push(v);
                }
            }
            _ => {
                if let Some(v) = by_record.get(&id).and_then(settlement_voucher) {
                    vouchers.push(v);
                }
            }
        }
    }
    Ok(Page { vouchers, next })
}

// ---------- endpoints ----------

fn parse_day(name: &str, v: &Option<String>) -> Result<NaiveDate, ApiError> {
    let v = v.as_deref().ok_or_else(|| {
        ApiError::invalid("invalid_range", format!("{name} is required (YYYY-MM-DD)"))
    })?;
    NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|_| {
        ApiError::invalid(
            "invalid_range",
            format!("{name} must be a date, YYYY-MM-DD"),
        )
    })
}

fn parse_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let (f, t) = (parse_day("from", from)?, parse_day("to", to)?);
    if t < f {
        return Err(ApiError::invalid("invalid_range", "to is before from"));
    }
    if t - f > Duration::days(MAX_RANGE_DAYS) {
        return Err(ApiError::invalid(
            "range_too_long",
            format!("a single export covers at most {MAX_RANGE_DAYS} days; split the range"),
        ));
    }
    Ok((f, t))
}

#[derive(Deserialize)]
pub struct JournalQuery {
    /// First IST day, `YYYY-MM-DD`.
    from: Option<String>,
    /// Last IST day, inclusive.
    to: Option<String>,
    /// `csv` (default), `tally_csv`, `tally_xml` or `json`.
    format: Option<String>,
    /// INR by default.
    currency: Option<String>,
    /// Comma list of `payments,refunds,fees,settlements,payouts,other`; default all.
    kinds: Option<String>,
    /// The `x-next-cursor` of the previous page.
    after: Option<String>,
    /// Vouchers per page, 1-5000, default 1000.
    limit: Option<i64>,
}

fn hv(s: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(s).ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `GET /v1/exports/journal`: one page of the journal for a date range. See the module notes.
pub async fn journal(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<JournalQuery>,
) -> Result<Response, ApiError> {
    auth.require("exports:read")?;
    let (from, to) = parse_range(&q.from, &q.to)?;
    let format = q.format.as_deref().unwrap_or("csv");
    if !matches!(format, "csv" | "tally_csv" | "tally_xml" | "json") {
        return Err(ApiError::invalid(
            "invalid_format",
            "format is csv, tally_csv, tally_xml or json",
        ));
    }
    let currency = q.currency.clone().unwrap_or_else(|| "INR".into());
    if !matches!(currency.as_str(), "INR" | "USD") {
        return Err(ApiError::invalid(
            "invalid_currency",
            "currency is INR or USD",
        ));
    }
    let kinds: Vec<String> = match q.kinds.as_deref() {
        None | Some("") => KINDS.iter().map(|k| (*k).to_string()).collect(),
        Some(list) => {
            let mut v = vec![];
            for k in list.split(',').map(str::trim) {
                if !KINDS.contains(&k) {
                    return Err(ApiError::invalid(
                        "invalid_kinds",
                        format!("unknown kind {k}; use {}", KINDS.join(", ")),
                    ));
                }
                v.push(k.to_string());
            }
            v
        }
    };
    let after = match q.after.as_deref() {
        None | Some("") => None,
        Some(c) => Some(cursor_decode(c).ok_or_else(|| {
            ApiError::invalid("invalid_cursor", "after is not a cursor this export gave")
        })?),
    };
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);

    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mapping = load_mapping(&mut tx).await?;
    let page = load_page(&mut tx, from, to, &currency, &kinds, after.clone(), limit).await?;
    for v in &page.vouchers {
        if !v.balanced() {
            tracing::error!(voucher = %v.number, "export voucher does not balance");
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "export_unbalanced",
                format!(
                    "voucher {} does not balance; nothing was exported",
                    v.number
                ),
            ));
        }
    }
    let (mut debit, mut credit, mut lines) = (0i64, 0i64, 0usize);
    let mut unmapped: BTreeSet<String> = BTreeSet::new();
    for v in &page.vouchers {
        let (d, c) = v.totals();
        debit += d;
        credit += c;
        lines += v.lines.len();
        for l in &v.lines {
            if !mapping.resolve(&l.account).mapped {
                unmapped.insert(l.account.clone());
            }
        }
    }
    let first = after.is_none();
    let (body, content_type) = match format {
        "csv" => (render_generic(&page.vouchers, &mapping, first), "text/csv; charset=utf-8"),
        "tally_csv" => (render_tally_csv(&page.vouchers, &mapping, first), "text/csv; charset=utf-8"),
        "tally_xml" => (render_tally_xml(&page.vouchers, &mapping), "application/xml; charset=utf-8"),
        _ => (
            serde_json::to_string(&json!({
                "from": from, "to": to, "currency": currency,
                "vouchers": page.vouchers.iter().map(|v| {
                    let (d, c) = v.totals();
                    json!({
                        "number": v.number, "date": v.date, "kind": v.kind, "source": v.source, "narration": v.narration,
                        "debit_minor": d, "credit_minor": c,
                        "lines": v.lines.iter().map(|l| { let r = mapping.resolve(&l.account); json!({
                            "ledger_account": l.account, "account_code": r.code, "account_name": r.name,
                            "debit_minor": l.debit, "credit_minor": l.credit,
                        }) }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
                "totals": { "vouchers": page.vouchers.len(), "lines": lines, "debit_minor": debit, "credit_minor": credit },
                "unmapped_accounts": unmapped,
                "next_cursor": page.next,
            }))
            .map_err(|_| ApiError::internal())?,
            "application/json",
        ),
    };
    let sha = sha256_hex(body.as_bytes());
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "export.journal",
        Some(format),
        json!({
            "from": from, "to": to, "currency": currency, "format": format, "kinds": kinds,
            "after": q.after, "limit": limit, "vouchers": page.vouchers.len(), "lines": lines,
            "debit_total_minor": debit, "credit_total_minor": credit, "sha256": sha,
            "next_cursor": page.next, "unmapped_accounts": unmapped,
        }),
    )
    .await?;
    tx.commit().await?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    let put = |h: &mut HeaderMap, k: &'static str, v: String| {
        if let Some(v) = hv(&v) {
            h.insert(HeaderName::from_static(k), v);
        }
    };
    put(
        &mut headers,
        "x-export-vouchers",
        page.vouchers.len().to_string(),
    );
    put(&mut headers, "x-export-lines", lines.to_string());
    put(
        &mut headers,
        "x-export-debit-total-minor",
        debit.to_string(),
    );
    put(
        &mut headers,
        "x-export-credit-total-minor",
        credit.to_string(),
    );
    put(&mut headers, "x-export-sha256", sha);
    put(
        &mut headers,
        "x-next-cursor",
        page.next.clone().unwrap_or_default(),
    );
    put(
        &mut headers,
        "x-export-unmapped-accounts",
        unmapped.iter().cloned().collect::<Vec<_>>().join(","),
    );
    if format != "json" {
        let ext = if format == "tally_xml" { "xml" } else { "csv" };
        put(
            &mut headers,
            "content-disposition",
            format!("attachment; filename=\"journal-{from}-{to}-{format}.{ext}\""),
        );
    }
    Ok((StatusCode::OK, headers, body).into_response())
}

// ---------- account mappings ----------

fn valid_account_key(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 100
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-' | '*'))
        && (!a.contains('*') || (a.ends_with(":*") && a.matches('*').count() == 1))
}

fn valid_label(s: &str, max: usize) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() <= max && !t.chars().any(char::is_control)
}

#[derive(Deserialize)]
pub struct NewMapping {
    /// A ledger account (`bank`, `provider_receivable:razorpay`) or a family ending in `:*`.
    ledger_account: String,
    code: String,
    name: String,
}

pub async fn set_mapping(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewMapping>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("exports:write")?;
    if !valid_account_key(&b.ledger_account) {
        return Err(ApiError::invalid(
            "invalid_ledger_account",
            "ledger_account is letters, digits and : _ . -, optionally ending in :* for a family",
        ));
    }
    if !valid_label(&b.code, 40) || !valid_label(&b.name, 120) {
        return Err(ApiError::invalid(
            "invalid_mapping",
            "code (up to 40) and name (up to 120) are required and must be plain text",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO account_mappings (tenant_id, ledger_account, code, name, updated_by) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id, ledger_account) DO UPDATE SET code = EXCLUDED.code, name = EXCLUDED.name, updated_by = EXCLUDED.updated_by, updated_at = now()
         RETURNING id",
    )
    .bind(&auth.tenant_id)
    .bind(&b.ledger_account)
    .bind(b.code.trim())
    .bind(b.name.trim())
    .bind(&auth.actor)
    .fetch_one(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "export.mapping_set",
        Some(&b.ledger_account),
        json!({ "code": b.code.trim(), "name": b.name.trim() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "ledger_account": b.ledger_account, "code": b.code.trim(), "name": b.name.trim() }),
        ),
    ))
}

pub async fn delete_mapping(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("exports:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let acct: Option<String> =
        sqlx::query_scalar("DELETE FROM account_mappings WHERE id = $1 RETURNING ledger_account")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let acct = acct.ok_or_else(|| ApiError::not_found("account mapping"))?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "export.mapping_deleted",
        Some(&acct),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "deleted": true, "ledger_account": acct })))
}

/// `GET /v1/exports/account_mappings`: the built-in chart, this tenant's overrides, and what every ledger
/// account in use resolves to (so an unmapped one is visible before an export).
pub async fn list_mappings(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("exports:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let overrides: Vec<(i64, String, String, String, String)> = sqlx::query_as(
        "SELECT id, ledger_account, code, name, updated_by FROM account_mappings ORDER BY ledger_account",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mapping = Mapping::from_overrides(
        &overrides
            .iter()
            .map(|(_, a, c, n, _)| (a.clone(), c.clone(), n.clone()))
            .collect::<Vec<_>>(),
    );
    let mut accounts: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT account FROM ledger_entries ORDER BY account")
            .fetch_all(&mut *tx)
            .await?;
    // accounts the settlement-record vouchers use
    for a in [
        zyvor_ledger::accounts::BANK.to_string(),
        zyvor_ledger::accounts::PROCESSING_FEES.to_string(),
        zyvor_ledger::accounts::ADJUSTMENTS.to_string(),
    ] {
        if !accounts.contains(&a) {
            accounts.push(a);
        }
    }
    let provs: Vec<String> = sqlx::query_scalar("SELECT DISTINCT provider FROM settlements")
        .fetch_all(&mut *tx)
        .await?;
    for p in provs {
        let a = zyvor_ledger::accounts::provider_receivable(&p);
        if !accounts.contains(&a) {
            accounts.push(a);
        }
    }
    accounts.sort();
    Ok(Json(json!({
        "defaults": DEFAULT_CHART.iter().map(|(a, c, n)| json!({ "ledger_account": a, "code": c, "name": n })).collect::<Vec<_>>(),
        "overrides": overrides.iter().map(|(id, a, c, n, by)| json!({ "id": id, "ledger_account": a, "code": c, "name": n, "updated_by": by })).collect::<Vec<_>>(),
        "effective": accounts.iter().map(|a| { let r = mapping.resolve(a); json!({ "ledger_account": a, "code": r.code, "name": r.name, "mapped": r.mapped }) }).collect::<Vec<_>>(),
        "unmapped_code": UNMAPPED_CODE,
        "note": "Codes are placeholders to import into your own chart of accounts, not statutory. An account with neither a mapping nor a built-in code exports as the unmapped code with its ledger name.",
    })))
}

// ---------- GST summary ----------

#[derive(Deserialize)]
pub struct GstQuery {
    from: Option<String>,
    to: Option<String>,
    /// `csv` (default) or `json`.
    format: Option<String>,
}

/// `GET /v1/exports/gst_summary`: sales and credit notes in the range by supply type and rate, with the net.
pub async fn gst_summary(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<GstQuery>,
) -> Result<Response, ApiError> {
    auth.require("exports:read")?;
    let (from, to) = parse_range(&q.from, &q.to)?;
    let format = q.format.as_deref().unwrap_or("csv");
    if !matches!(format, "csv" | "json") {
        return Err(ApiError::invalid("invalid_format", "format is csv or json"));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows = crate::gst::summary_rows(&mut tx, from, to).await?;
    let (body, content_type) = if format == "csv" {
        (crate::gst::summary_csv(&rows), "text/csv; charset=utf-8")
    } else {
        (
            serde_json::to_string(&json!({
                "from": from, "to": to,
                "rows": rows.iter().map(crate::gst::SummaryRow::to_json).collect::<Vec<_>>(),
                "net": crate::gst::summary_net(&rows).to_json(),
                "note": "A preparation aid from invoices and credit notes in the range; Zyvor does not file returns.",
            }))
            .map_err(|_| ApiError::internal())?,
            "application/json",
        )
    };
    let sha = sha256_hex(body.as_bytes());
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "export.gst_summary",
        Some(format),
        json!({ "from": from, "to": to, "format": format, "rows": rows.len(), "sha256": sha }),
    )
    .await?;
    tx.commit().await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(v) = hv(&sha) {
        headers.insert(HeaderName::from_static("x-export-sha256"), v);
    }
    Ok((StatusCode::OK, headers, body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn rec(gross: i64, refunds: i64, fee: i64, net: i64) -> SettlementRecord {
        SettlementRecord {
            provider: "razorpay".into(),
            provider_ref: "stl_1".into(),
            currency: "INR".into(),
            gross_minor: gross,
            refunds_minor: refunds,
            fee_minor: fee,
            net_minor: net,
            settled_on: d(2026, 11, 1),
        }
    }

    #[test]
    fn settlement_and_fee_vouchers_balance_for_any_numbers() {
        // consistent: net = gross - refunds - fee
        for r in [
            rec(100_000, 0, 2_360, 97_640),
            rec(100_000, 30_000, 1_000, 69_000),
            // the provider's own net does not follow from its lines: the gap lands in adjustments
            rec(100_000, 0, 2_000, 90_000),
            rec(100_000, 0, 0, 100_500),
            // fees returned (negative), net below zero
            rec(0, 5_000, -300, -4_700),
            rec(0, 0, 0, 0),
        ] {
            for v in [settlement_voucher(&r), fee_voucher(&r)]
                .into_iter()
                .flatten()
            {
                assert!(v.balanced(), "{v:?}");
            }
        }
        // a settlement with everything zero is no voucher at all
        assert!(settlement_voucher(&rec(0, 0, 0, 0)).is_none());
        assert!(fee_voucher(&rec(100, 0, 0, 100)).is_none());
    }

    #[test]
    fn settlement_voucher_lines_are_what_they_say() {
        let v = settlement_voucher(&rec(100_000, 30_000, 1_000, 69_000)).unwrap();
        assert_eq!(
            v.lines,
            vec![
                Line {
                    account: "bank".into(),
                    debit: 69_000,
                    credit: 0
                },
                Line {
                    account: "provider_receivable:razorpay".into(),
                    debit: 0,
                    credit: 69_000
                },
            ]
        );
        let f = fee_voucher(&rec(100_000, 30_000, 1_000, 69_000)).unwrap();
        assert_eq!(
            f.lines[0],
            Line {
                account: "processing_fees".into(),
                debit: 1_000,
                credit: 0
            }
        );
        // an unexplained shortfall is shown as an adjustment, not hidden
        let v = settlement_voucher(&rec(100_000, 0, 2_000, 90_000)).unwrap();
        assert!(
            v.lines
                .iter()
                .any(|l| l.account == "settlement_adjustments" && l.debit == 8_000)
        );
    }

    #[test]
    fn mapping_prefers_tenant_over_builtin_and_longest_prefix() {
        let m = Mapping::from_overrides(&[
            ("bank".into(), "1010".into(), "HDFC current".into()),
            (
                "provider_receivable:*".into(),
                "1250".into(),
                "PG receivable".into(),
            ),
            (
                "provider_receivable:razorpay".into(),
                "1251".into(),
                "Razorpay".into(),
            ),
        ]);
        assert_eq!(m.resolve("bank").code, "1010");
        assert_eq!(m.resolve("provider_receivable:razorpay").code, "1251");
        assert_eq!(m.resolve("provider_receivable:cashfree").code, "1250");
        // untouched accounts use the built-in chart; unknown ones are visible, not dropped
        assert_eq!(m.resolve("collections").code, "4000");
        let u = m.resolve("mystery:1");
        assert_eq!(
            (u.code.as_str(), u.mapped, u.name.as_str()),
            (UNMAPPED_CODE, false, "mystery:1")
        );
        assert_eq!(Mapping::default().resolve("vendor_payable:v1").code, "2200");
    }

    #[test]
    fn money_text_is_exact() {
        assert_eq!(decimal(12_345), "123.45");
        assert_eq!(decimal(5), "0.05");
        assert_eq!(decimal(0), "0.00");
        assert_eq!(decimal(-5), "-0.05");
        assert_eq!(decimal(i64::MAX), "92233720368547758.07");
    }

    #[test]
    fn csv_fields_are_quoted_and_formulas_neutralised() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("@x"), "'@x");
    }

    #[test]
    fn generic_csv_is_deterministic_and_columns_line_up() {
        let v = Voucher {
            date: d(2026, 11, 1),
            number: "capture:att_1".into(),
            kind: "payments",
            source: "ledger",
            currency: "INR".into(),
            narration: "Payment captured capture:att_1".into(),
            lines: vec![
                Line {
                    account: "provider_receivable:razorpay".into(),
                    debit: 100_000,
                    credit: 0,
                },
                Line {
                    account: "collections".into(),
                    debit: 0,
                    credit: 100_000,
                },
            ],
        };
        let m = Mapping::default();
        let a = render_generic(std::slice::from_ref(&v), &m, true);
        assert_eq!(a, render_generic(std::slice::from_ref(&v), &m, true));
        let rows: Vec<&str> = a.lines().collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[1],
            "capture:att_1,2026-11-01,payments,1,1200,Payment provider receivable,provider_receivable:razorpay,1000.00,,INR,Payment captured capture:att_1"
        );
        assert_eq!(rows[2].split(',').nth(8), Some("1000.00"));
        // later pages carry no header
        assert!(!render_generic(&[v], &m, false).starts_with("voucher_no"));
    }

    #[test]
    fn tally_xml_follows_the_documented_sign_convention() {
        let v = settlement_voucher(&rec(100_000, 0, 0, 100_000)).unwrap();
        let x = render_tally_xml(&[v], &Mapping::default());
        assert!(x.contains("<TALLYREQUEST>Import</TALLYREQUEST>"));
        assert!(x.contains("<DATE>20261101</DATE>"));
        // debit: deemed positive Yes with a negative amount; credit: No with a positive amount
        assert!(
            x.contains("<ISDEEMEDPOSITIVE>Yes</ISDEEMEDPOSITIVE>\n      <AMOUNT>-1000.00</AMOUNT>")
        );
        assert!(
            x.contains("<ISDEEMEDPOSITIVE>No</ISDEEMEDPOSITIVE>\n      <AMOUNT>1000.00</AMOUNT>")
        );
        // text is escaped
        let mut v = settlement_voucher(&rec(100, 0, 0, 100)).unwrap();
        v.narration = "A & B <x>".into();
        assert!(render_tally_xml(&[v], &Mapping::default()).contains("A &amp; B &lt;x&gt;"));
    }

    #[test]
    fn cursors_round_trip_and_reject_junk() {
        let c = cursor_encode(d(2026, 11, 1), "settlement:rz:stl, 1");
        assert_eq!(
            cursor_decode(&c),
            Some((d(2026, 11, 1), "settlement:rz:stl, 1".to_string()))
        );
        assert_eq!(cursor_decode("zz"), None);
        assert_eq!(cursor_decode(&hex::encode("nope")), None);
    }

    #[test]
    fn kinds_come_from_the_reference_prefix() {
        assert_eq!(kind_of_reference("capture:att_1"), "payments");
        assert_eq!(kind_of_reference("refund:rf_1"), "refunds");
        assert_eq!(kind_of_reference("payout:po_1"), "payouts");
        assert_eq!(kind_of_reference("settle_bank:sbat_1"), "settlements");
        assert_eq!(kind_of_reference("split:x"), "other");
        assert_eq!(kind_of_reference("refund_receipt:x"), "other");
    }

    #[test]
    fn mapping_keys_are_validated() {
        assert!(valid_account_key("bank"));
        assert!(valid_account_key("provider_receivable:*"));
        assert!(!valid_account_key("*"));
        assert!(!valid_account_key("a*b"));
        assert!(!valid_account_key("a b"));
        assert!(!valid_account_key(""));
    }
}
