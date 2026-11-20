// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Payout batches: many payouts proposed, approved and sent as one run, with every item still a normal payout.
//!
//! A batch is built from a CSV file, a JSON list, or the vendors that have a balance. Building it is all or nothing:
//! every problem is reported with its line number and nothing is stored while any remain (`POST /v1/payout_batches/preview`
//! runs the same checks and stores nothing). Each item becomes a row in `payouts` waiting for approval, so the
//! existing rules still hold for it: a different person approves (never the person who proposed it), the ledger is
//! posted once when it is paid, the bank reference is unique. **Every item of a batch needs that approval, whatever the
//! tenant's approval threshold for single payouts says.** The approver may leave items out; the rest are approved
//! together.
//!
//! Checks run when a batch is proposed and again when it is approved and when each item is sent: the vendor must be
//! active, the amount within the vendor's minimum and the tenant's limits, the payee not blocked, and for a payout from a
//! vendor's balance the money must be there and past its hold days.
//!
//! Sending (acquire mode, through the payout rail) claims each item before calling the rail, so two sends at once
//! never pay an item twice, and the payout's id is the rail's idempotency key, so a retry after a failure is the same
//! instruction. An item whose outcome the rail could not tell us is `unknown`: it is never retried, only asked about
//! (`/resolve`, and the worker), the same rule payments follow. In orchestrate mode the batch report is the list of
//! transfers to make from your bank, and each is recorded with `POST /v1/payouts/{id}/mark_paid`.
//!
//! Whether an item is paid is read from its payout, so a payout paid by hand shows as paid here. The report and the
//! batch show the bank statement lines (`bankmatch`) that matched each paid item.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use zyvor_core::new_id;
use zyvor_settle::{PayoutInstruction, PayoutRailStatus, RailError, RailKind};

use crate::{
    ApiError, AppState,
    auth::Auth,
    bankcsv::{Problem, detect_delimiter, norm_header, problem, read_records},
    clock, money_mode, onboarding, risk, tenant,
};

/// Most items in one batch.
pub const MAX_ITEMS: usize = 1000;
const MAX_CSV_BYTES: usize = 1_000_000;
/// A failed item is retried by hand up to this many times in all.
const MAX_ATTEMPTS: i32 = 5;
/// Errors listed in one response; the count of the rest is still given.
const MAX_REPORTED: usize = 100;

// ---------- policy ----------

/// Limits a person with `admin` sets, and the one switch that lets small scheduled batches skip approval. All empty by
/// default: no limit, and every batch waits for a second person.
#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PayoutPolicy {
    /// Refuse one payout above this.
    pub max_payout_minor: Option<i64>,
    /// Refuse a batch whose total is above this.
    pub max_batch_total_minor: Option<i64>,
    /// Refuse a batch with more items than this.
    pub max_batch_items: Option<i64>,
    /// Refuse paying one vendor more than this in any 24 hours (counting payouts still waiting).
    pub max_vendor_per_24h_minor: Option<i64>,
    /// A batch the worker proposes from a schedule is approved without a person when its total is at most this.
    /// Unset (the default): scheduled batches always wait for approval.
    pub auto_approve_scheduled_under_minor: Option<i64>,
}

impl PayoutPolicy {
    pub fn validate(&self) -> Result<(), ApiError> {
        let pos = |v: Option<i64>| v.is_none_or(|n| n > 0 && n <= zyvor_core::MAX_AMOUNT_MINOR);
        if !pos(self.max_payout_minor)
            || !pos(self.max_batch_total_minor)
            || !pos(self.max_vendor_per_24h_minor)
            || !pos(self.auto_approve_scheduled_under_minor)
            || self
                .max_batch_items
                .is_some_and(|n| !(1..=MAX_ITEMS as i64).contains(&n))
        {
            return Err(ApiError::invalid(
                "invalid_payout_policy",
                format!("limits are positive whole numbers (max_batch_items 1 to {MAX_ITEMS})"),
            ));
        }
        Ok(())
    }
}

pub async fn load_policy(conn: &mut PgConnection) -> Result<PayoutPolicy, sqlx::Error> {
    let v: Option<Value> = sqlx::query_scalar("SELECT payout_policy FROM tenant_settings LIMIT 1")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

/// `GET /v1/payout_policy`
pub async fn get_policy(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(json!(load_policy(&mut tx).await?)))
}

/// `PUT /v1/payout_policy`: replaces the whole policy. Admin only: it decides what money may move without a person.
pub async fn put_policy(
    State(s): State<AppState>,
    auth: Auth,
    Json(p): Json<PayoutPolicy>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    p.validate()?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("UPDATE tenant_settings SET payout_policy = $2 WHERE tenant_id = $1")
        .bind(&auth.tenant_id)
        .bind(json!(p))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_policy.changed",
        None,
        json!(p),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!(p)))
}

// ---------- the vendor and tenant rules every payout meets ----------

pub struct VendorRow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub archived: bool,
    pub hold_days: i32,
    pub min_payout_minor: i64,
    pub details_enc: Option<Vec<u8>>,
    pub holder_name: Option<String>,
}

/// Read and lock a vendor (so two payouts cannot spend the same balance).
pub async fn lock_vendor(
    conn: &mut PgConnection,
    id: &str,
) -> Result<Option<VendorRow>, sqlx::Error> {
    let r: Option<(String, String, String, bool, i32, i64, Option<Vec<u8>>, Option<String>)> =
        sqlx::query_as(
            "SELECT id, name, status, archived_at IS NOT NULL, hold_days, min_payout_minor, details_enc, bank_holder_name FROM vendors WHERE id = $1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(r.map(
        |(id, name, status, archived, hold_days, min_payout_minor, details_enc, holder_name)| {
            VendorRow {
                id,
                name,
                status,
                archived,
                hold_days,
                min_payout_minor,
                details_enc,
                holder_name,
            }
        },
    ))
}

pub struct Gate<'a> {
    policy: PayoutPolicy,
    today: NaiveDate,
    risk: risk::Rules,
    crypto: &'a crate::Crypto,
}

impl<'a> Gate<'a> {
    pub async fn load(
        conn: &mut PgConnection,
        crypto: &'a crate::Crypto,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            policy: load_policy(conn).await?,
            today: clock::today(conn).await?,
            risk: risk::load(conn).await?,
            crypto,
        })
    }
}

#[derive(Clone, Copy)]
pub enum Mode {
    /// A payout about to be created, with what this batch already asks of the same vendor.
    New { already_minor: i64 },
    /// A payout that already exists (its own amount is already reserved): approval and sending.
    Recheck,
}

type Found = Vec<(&'static str, String)>;

/// Everything that stops a payout to this vendor, each as `(code, plain sentence)`. Empty means it may go ahead.
pub async fn problems(
    conn: &mut PgConnection,
    gate: &Gate<'_>,
    v: &VendorRow,
    amount: i64,
    currency: &str,
    from_balance: bool,
    mode: Mode,
) -> Result<Found, sqlx::Error> {
    let mut out: Found = Vec::new();
    if v.archived {
        out.push(("vendor_archived", "this vendor is archived".into()));
    }
    if v.status != "active" {
        out.push((
            "vendor_not_active",
            format!(
                "this vendor is {}; only an active vendor can be paid",
                v.status
            ),
        ));
    }
    if amount < v.min_payout_minor {
        out.push((
            "below_minimum_payout",
            format!(
                "this vendor's minimum payout is {}",
                crate::ai::money(v.min_payout_minor, currency)
            ),
        ));
    }
    if gate.policy.max_payout_minor.is_some_and(|m| amount > m) {
        out.push((
            "above_payout_limit",
            format!(
                "a single payout is limited to {}",
                crate::ai::money(gate.policy.max_payout_minor.unwrap_or(0), currency)
            ),
        ));
    }
    if let (Mode::New { already_minor }, Some(cap)) = (mode, gate.policy.max_vendor_per_24h_minor) {
        let recent: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM payouts WHERE vendor_id = $1 AND currency = $2 AND status IN ('pending_approval', 'approved', 'paid') AND created_at > now() - interval '24 hours'")
            .bind(&v.id)
            .bind(currency)
            .fetch_one(&mut *conn)
            .await?;
        if recent + already_minor + amount > cap {
            out.push((
                "over_vendor_daily_limit",
                format!(
                    "paying this vendor more than {} in 24 hours is not allowed ({} already counted)",
                    crate::ai::money(cap, currency),
                    crate::ai::money(recent + already_minor, currency)
                ),
            ));
        }
    }
    match v
        .details_enc
        .as_deref()
        .and_then(|b| gate.crypto.decrypt(b))
    {
        None => out.push((
            "no_bank_details",
            "this vendor has no bank account or UPI id to pay".into(),
        )),
        Some(bytes) => {
            let d: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if d["vpa"]
                .as_str()
                .is_some_and(|vpa| risk::vpa_blocked(&gate.risk, vpa))
            {
                out.push((
                    "blocked_payee",
                    "this vendor's UPI id is on your blocked list".into(),
                ));
            }
        }
    }
    if from_balance {
        let (free, held, eligible) =
            crate::splits::eligible(conn, &v.id, currency, v.hold_days, gate.today).await?;
        match mode {
            Mode::New { already_minor } => {
                if amount + already_minor > eligible {
                    out.push((
                        "insufficient_balance",
                        if held > 0 {
                            format!(
                                "this vendor has {} free to pay out now ({} more is held back for {} days after each payment)",
                                crate::ai::money(eligible.max(0), currency),
                                crate::ai::money(held, currency),
                                v.hold_days
                            )
                        } else {
                            format!(
                                "this vendor has {} free to pay out; the rest of what they are owed is already promised to other payouts",
                                crate::ai::money(free.max(0), currency)
                            )
                        },
                    ));
                }
            }
            Mode::Recheck => {
                // the payout's own amount is already set aside, so what is left must not be negative
                if free < 0 {
                    out.push((
                        "insufficient_balance",
                        "this vendor's balance no longer covers the payouts promised against it (a refund or dispute took part of it back)".into(),
                    ));
                }
            }
        }
    }
    Ok(out)
}

// ---------- reading a file ----------

#[derive(Debug, Clone)]
pub struct ItemSpec {
    pub line: usize,
    pub vendor: String,
    pub amount_minor: i64,
    pub purpose: Option<String>,
    pub reference: Option<String>,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub specs: Vec<ItemSpec>,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
}

/// Rupees with at most two decimals to paise. Nothing that is not plainly an amount: no separators, no sign, no
/// currency symbol, no exponent (a spreadsheet that has turned 12500 into 1.25E+04 has lost the number).
pub fn parse_rupees(s: &str) -> Result<i64, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("amount is empty".into());
    }
    let (whole, frac) = match t.split_once('.') {
        Some((w, f)) => (w, f),
        None => (t, ""),
    };
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
        || frac.len() > 2
        || (t.contains('.') && frac.is_empty())
    {
        return Err(format!(
            "`{t}` is not an amount: use digits with at most two decimals, like 12500 or 12500.50 (no commas, signs or symbols)"
        ));
    }
    let w: i64 = whole
        .parse()
        .map_err(|_| format!("`{t}` is too large to be an amount"))?;
    let f: i64 = format!("{frac:0<2}").parse().unwrap_or(0);
    w.checked_mul(100)
        .and_then(|v| v.checked_add(f))
        .filter(|v| *v > 0 && *v <= zyvor_core::MAX_AMOUNT_MINOR)
        .ok_or_else(|| format!("`{t}` must be more than zero and within the supported limit"))
}

/// A cell a spreadsheet would run as a formula, or that has control characters in it.
fn unsafe_cell(s: &str) -> Option<&'static str> {
    if s.chars().any(|c| c.is_control()) {
        return Some("has a control character");
    }
    match s.chars().next() {
        Some('=' | '+' | '-' | '@') => {
            Some("starts with =, +, - or @, which a spreadsheet would run as a formula")
        }
        _ => None,
    }
}

const VENDOR_NAMES: &[&str] = &[
    "vendor",
    "vendorid",
    "vendorref",
    "vendorreference",
    "payee",
    "beneficiary",
];
const AMOUNT_NAMES: &[&str] = &["amount", "amountinr", "amountrs", "amountrupees"];
const MINOR_NAMES: &[&str] = &["amountminor", "paise", "amountpaise"];
const PURPOSE_NAMES: &[&str] = &["purpose", "narration", "description", "note", "remarks"];
const REF_NAMES: &[&str] = &[
    "reference",
    "ref",
    "externalref",
    "yourreference",
    "invoice",
];
const CCY_NAMES: &[&str] = &["currency", "ccy"];

/// Read a payout file. Strict: every line must have as many cells as the header, every amount must be plainly an
/// amount, and any problem fails the file. Two lines that are exactly alike are an error (a pasted line twice).
pub fn parse_csv(text: &str, delimiter: Option<&str>) -> Parsed {
    let mut out = Parsed::default();
    let err = |out: &mut Parsed, line, col: Option<&str>, m: String| {
        out.errors.push(problem(line, col, m))
    };
    if text.len() > MAX_CSV_BYTES {
        err(
            &mut out,
            0,
            None,
            format!(
                "the file is larger than {} KB; split it",
                MAX_CSV_BYTES / 1000
            ),
        );
        return out;
    }
    let delim = match delimiter.map(str::trim) {
        None | Some("") => detect_delimiter(text),
        Some("," | "comma") => ',',
        Some(";") => ';',
        Some("|") => '|',
        Some("tab" | "\\t") => '\t',
        Some(o) => {
            err(
                &mut out,
                0,
                None,
                format!("delimiter `{o}` is not one of , ; | tab"),
            );
            return out;
        }
    };
    let records = match read_records(text, delim) {
        Ok(r) => r,
        Err(e) => {
            err(&mut out, 0, None, e);
            return out;
        }
    };
    let Some((hline, header)) = records.first() else {
        err(&mut out, 0, None, "the file is empty".into());
        return out;
    };
    let heads: Vec<String> = header.iter().map(|h| norm_header(h)).collect();
    let find = |names: &[&str]| -> Vec<usize> {
        heads
            .iter()
            .enumerate()
            .filter(|(_, h)| names.contains(&h.as_str()))
            .map(|(i, _)| i)
            .collect()
    };
    let pick = |label: &str, names: &[&str], required: bool, out: &mut Parsed| -> Option<usize> {
        let f = find(names);
        match f.len() {
            1 => Some(f[0]),
            0 if required => {
                out.errors.push(problem(
                    *hline,
                    None,
                    format!("no `{label}` column in the header"),
                ));
                None
            }
            0 => None,
            _ => {
                out.errors.push(problem(
                    *hline,
                    Some(label),
                    format!("more than one column looks like `{label}`"),
                ));
                None
            }
        }
    };
    let vendor_c = pick("vendor", VENDOR_NAMES, true, &mut out);
    let amount_c = pick("amount", AMOUNT_NAMES, false, &mut out);
    let minor_c = pick("amount_minor", MINOR_NAMES, false, &mut out);
    let purpose_c = pick("purpose", PURPOSE_NAMES, false, &mut out);
    let ref_c = pick("reference", REF_NAMES, false, &mut out);
    let ccy_c = pick("currency", CCY_NAMES, false, &mut out);
    if amount_c.is_none() == minor_c.is_none()
        && out
            .errors
            .iter()
            .all(|e| e.line != *hline || !e.message.contains("amount"))
    {
        err(&mut out, *hline, None, "the header needs exactly one of an `amount` (rupees) or an `amount_minor` (paise) column".into());
    }
    let known: BTreeSet<usize> = [vendor_c, amount_c, minor_c, purpose_c, ref_c, ccy_c]
        .into_iter()
        .flatten()
        .collect();
    for (i, h) in header.iter().enumerate() {
        if !known.contains(&i) {
            out.warnings.push(problem(
                *hline,
                Some(h),
                format!("column `{h}` is not used"),
            ));
        }
    }
    if !out.errors.is_empty() {
        return out;
    }
    let (vendor_c, _) = (vendor_c.unwrap_or(0), ());
    let mut seen: HashMap<(String, i64, String, String), usize> = HashMap::new();
    let rows = &records[1..];
    if rows.is_empty() {
        err(
            &mut out,
            0,
            None,
            "the file has a header but no lines".into(),
        );
        return out;
    }
    if rows.len() > MAX_ITEMS {
        err(
            &mut out,
            0,
            None,
            format!("{} lines; a batch holds at most {MAX_ITEMS}", rows.len()),
        );
        return out;
    }
    for (line, row) in rows {
        let line = *line;
        if row.len() != header.len() {
            err(
                &mut out,
                line,
                None,
                format!(
                    "has {} cells but the header has {}; a number with a comma in it (12,500) must be in quotes or have the comma removed",
                    row.len(),
                    header.len()
                ),
            );
            continue;
        }
        let cell = |i: usize| row[i].trim();
        let mut bad = false;
        for (name, i) in [
            ("vendor", Some(vendor_c)),
            ("purpose", purpose_c),
            ("reference", ref_c),
        ] {
            if let Some(i) = i
                && let Some(why) = unsafe_cell(cell(i))
            {
                err(&mut out, line, Some(name), format!("`{}` {why}", cell(i)));
                bad = true;
            }
        }
        let vendor = cell(vendor_c).to_string();
        if vendor.is_empty() {
            err(&mut out, line, Some("vendor"), "vendor is empty".into());
            bad = true;
        }
        let amount = match (amount_c, minor_c) {
            (Some(i), _) => parse_rupees(cell(i)),
            (None, Some(i)) => {
                let t = cell(i);
                if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
                    t.parse::<i64>()
                        .ok()
                        .filter(|v| *v > 0 && *v <= zyvor_core::MAX_AMOUNT_MINOR)
                        .ok_or_else(|| {
                            format!("`{t}` must be more than zero and within the supported limit")
                        })
                } else {
                    Err(format!("`{t}` is not a whole number of paise"))
                }
            }
            (None, None) => Err("no amount".into()),
        };
        let amount = match amount {
            Ok(a) => a,
            Err(m) => {
                err(&mut out, line, Some("amount"), m);
                bad = true;
                0
            }
        };
        if let Some(i) = ccy_c
            && !cell(i).is_empty()
            && !cell(i).eq_ignore_ascii_case("INR")
        {
            err(
                &mut out,
                line,
                Some("currency"),
                format!("`{}`: a batch pays in INR", cell(i)),
            );
            bad = true;
        }
        let purpose = purpose_c
            .map(|i| cell(i).to_string())
            .filter(|p| !p.is_empty());
        if purpose.as_deref().is_some_and(|p| p.chars().count() > 120) {
            err(
                &mut out,
                line,
                Some("purpose"),
                "purpose is up to 120 characters".into(),
            );
            bad = true;
        }
        let reference = ref_c.map(|i| cell(i).to_string()).filter(|p| !p.is_empty());
        if reference.as_deref().is_some_and(|p| p.chars().count() > 60) {
            err(
                &mut out,
                line,
                Some("reference"),
                "reference is up to 60 characters".into(),
            );
            bad = true;
        }
        if bad {
            continue;
        }
        let key = (
            vendor.to_lowercase(),
            amount,
            purpose.clone().unwrap_or_default(),
            reference.clone().unwrap_or_default(),
        );
        if let Some(first) = seen.insert(key, line) {
            err(
                &mut out,
                line,
                None,
                format!(
                    "is exactly like line {first} (same vendor, amount, purpose and reference); a repeated payment needs a different reference"
                ),
            );
            continue;
        }
        out.specs.push(ItemSpec {
            line,
            vendor,
            amount_minor: amount,
            purpose,
            reference,
        });
    }
    out
}

// ---------- planning ----------

#[derive(Debug, Clone)]
pub struct PlannedItem {
    pub line: usize,
    pub vendor_id: String,
    pub vendor_name: String,
    pub amount_minor: i64,
    pub purpose: String,
    pub reference: Option<String>,
    /// Set for scheduled items: the payout's own reference, so the same run is never proposed twice.
    pub ext_ref: Option<String>,
    pub schedule_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<PlannedItem>,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
}

impl Plan {
    pub fn total(&self) -> i64 {
        self.items.iter().map(|i| i.amount_minor).sum()
    }
}

/// Resolve vendors, lock them (in id order, so two batches cannot deadlock), and run every rule on every line.
/// Nothing is written. A vendor is named by its id or its `external_ref`.
pub async fn plan(
    conn: &mut PgConnection,
    crypto: &crate::Crypto,
    specs: Vec<ItemSpec>,
    currency: &str,
    from_balance: bool,
    default_purpose: Option<&str>,
) -> Result<Plan, ApiError> {
    let mut out = Plan::default();
    if specs.len() > MAX_ITEMS {
        out.errors.push(problem(
            0,
            None,
            format!("{} lines; a batch holds at most {MAX_ITEMS}", specs.len()),
        ));
        return Ok(out);
    }
    let mut resolved: BTreeMap<String, Option<String>> = BTreeMap::new();
    for sp in &specs {
        if !resolved.contains_key(&sp.vendor) {
            let id: Option<String> = sqlx::query_scalar("SELECT id FROM vendors WHERE id = $1 OR external_ref = $1 ORDER BY (id = $1) DESC LIMIT 1")
                .bind(&sp.vendor)
                .fetch_optional(&mut *conn)
                .await?;
            resolved.insert(sp.vendor.clone(), id);
        }
    }
    let ids: BTreeSet<String> = resolved.values().flatten().cloned().collect();
    let mut vendors: HashMap<String, VendorRow> = HashMap::new();
    for id in &ids {
        if let Some(v) = lock_vendor(conn, id).await? {
            vendors.insert(id.clone(), v);
        }
    }
    let gate = Gate::load(conn, crypto).await?;
    let mut already: HashMap<String, i64> = HashMap::new();
    for sp in specs {
        let Some(Some(vid)) = resolved.get(&sp.vendor) else {
            out.errors.push(problem(
                sp.line,
                Some("vendor"),
                format!("no vendor with id or reference `{}`", sp.vendor),
            ));
            continue;
        };
        let Some(v) = vendors.get(vid) else { continue };
        let Some(purpose) = sp
            .purpose
            .clone()
            .or_else(|| default_purpose.map(str::to_string))
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
        else {
            out.errors.push(problem(
                sp.line,
                Some("purpose"),
                "purpose is required (a purpose column, or default_purpose for the whole batch)"
                    .to_string(),
            ));
            continue;
        };
        let used = already.get(vid).copied().unwrap_or(0);
        let found = problems(
            conn,
            &gate,
            v,
            sp.amount_minor,
            currency,
            from_balance,
            Mode::New {
                already_minor: used,
            },
        )
        .await?;
        if found.is_empty() {
            *already.entry(vid.clone()).or_default() += sp.amount_minor;
            // a similar payout a moment ago is worth a second look, not a refusal
            let dup: Option<String> = sqlx::query_scalar("SELECT id FROM payouts WHERE vendor_id = $1 AND amount_minor = $2 AND currency = $3 AND status IN ('pending_approval', 'approved', 'paid') AND created_at > now() - interval '24 hours' LIMIT 1")
                .bind(vid)
                .bind(sp.amount_minor)
                .bind(currency)
                .fetch_optional(&mut *conn)
                .await?;
            if let Some(d) = dup {
                out.warnings.push(problem(
                    sp.line,
                    Some("amount"),
                    format!(
                        "{} was paid or proposed the same amount in the last 24 hours (payout {d})",
                        v.name
                    ),
                ));
            }
            out.items.push(PlannedItem {
                line: sp.line,
                vendor_id: vid.clone(),
                vendor_name: v.name.clone(),
                amount_minor: sp.amount_minor,
                purpose,
                reference: sp.reference,
                ext_ref: None,
                schedule_id: None,
            });
        } else {
            for (code, m) in found {
                out.errors
                    .push(problem(sp.line, Some(code), format!("{}: {m}", v.name)));
            }
        }
    }
    let total = out.total();
    if out.errors.is_empty() {
        if out.items.is_empty() {
            out.errors.push(problem(0, None, "there is nothing to pay"));
        }
        if gate
            .policy
            .max_batch_items
            .is_some_and(|m| out.items.len() as i64 > m)
        {
            out.errors.push(problem(
                0,
                None,
                format!(
                    "{} items; your policy allows at most {} in a batch",
                    out.items.len(),
                    gate.policy.max_batch_items.unwrap_or(0)
                ),
            ));
        }
        if gate.policy.max_batch_total_minor.is_some_and(|m| total > m) {
            out.errors.push(problem(
                0,
                None,
                format!(
                    "the batch totals {}; your policy allows at most {}",
                    crate::ai::money(total, currency),
                    crate::ai::money(gate.policy.max_batch_total_minor.unwrap_or(0), currency)
                ),
            ));
        }
    }
    Ok(out)
}

/// The same content gives the same fingerprint, whatever order the lines are in and whatever the file is called.
pub fn fingerprint(
    currency: &str,
    from_balance: bool,
    batch_ref: Option<&str>,
    items: &[PlannedItem],
) -> String {
    let mut lines: Vec<String> = items
        .iter()
        .map(|i| {
            format!(
                "{}|{}|{}|{}|{}",
                i.vendor_id,
                i.amount_minor,
                i.purpose,
                i.reference.as_deref().unwrap_or(""),
                i.schedule_id.as_deref().unwrap_or("")
            )
        })
        .collect();
    lines.sort();
    let text = format!(
        "v1|{currency}|{from_balance}|{}|{}",
        batch_ref.unwrap_or(""),
        lines.join("\n")
    );
    hex::encode(Sha256::digest(text.as_bytes()))
}

// ---------- storing a batch ----------

pub struct Meta<'a> {
    pub name: &'a str,
    pub batch_ref: Option<&'a str>,
    pub source: &'a str,
    pub currency: &'a str,
    pub from_balance: bool,
    pub rail: &'a str,
    pub file_name: Option<&'a str>,
    pub note: Option<&'a str>,
    pub created_by: &'a str,
    pub fingerprint: &'a str,
    /// Approve it as it is made (a scheduled batch under the tenant's limit): who is recorded as approving.
    pub auto_approve: bool,
}

/// The id of an open or approved batch with this content, if there is one.
pub async fn existing_with(
    conn: &mut PgConnection,
    fp: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM payout_batches WHERE fingerprint = $1 AND status IN ('pending_approval', 'approved')")
        .bind(fp)
        .fetch_optional(conn)
        .await
}

/// Write the batch, its items and their payouts. The caller holds the transaction and has checked the plan.
pub async fn persist(
    tx: &mut PgConnection,
    tenant_id: &str,
    plan: &Plan,
    m: &Meta<'_>,
) -> Result<String, ApiError> {
    let id = new_id("pbt");
    let status = if m.auto_approve {
        "approved"
    } else {
        "pending_approval"
    };
    sqlx::query("INSERT INTO payout_batches (id, tenant_id, name, batch_ref, source, status, currency, from_balance, rail, item_count, total_minor, fingerprint, file_name, note, created_by, approved_by, approved_at, auto_approved) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)")
        .bind(&id).bind(tenant_id).bind(m.name).bind(m.batch_ref).bind(m.source).bind(status)
        .bind(m.currency).bind(m.from_balance).bind(m.rail).bind(plan.items.len() as i32).bind(plan.total())
        .bind(m.fingerprint).bind(m.file_name).bind(m.note).bind(m.created_by)
        .bind(m.auto_approve.then_some("system:auto-approve"))
        .bind(m.auto_approve.then(chrono::Utc::now))
        .bind(m.auto_approve)
        .execute(&mut *tx)
        .await?;
    for it in &plan.items {
        let pid = new_id("pay");
        let ext = it
            .ext_ref
            .clone()
            .unwrap_or_else(|| format!("{id}:{}", it.line));
        sqlx::query("INSERT INTO payouts (id, tenant_id, vendor_id, amount_minor, currency, status, purpose, external_ref, requested_by, source, approved_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(&pid).bind(tenant_id).bind(&it.vendor_id).bind(it.amount_minor).bind(m.currency)
            .bind(status).bind(&it.purpose).bind(&ext).bind(m.created_by)
            .bind(if m.from_balance { "balance" } else { "own_funds" })
            .bind(m.auto_approve.then_some("system:auto-approve"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO payout_batch_items (id, tenant_id, batch_id, line_no, vendor_id, payout_id, amount_minor, purpose, reference, schedule_id) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(new_id("pbi")).bind(tenant_id).bind(&id).bind(it.line as i32).bind(&it.vendor_id).bind(&pid)
            .bind(it.amount_minor).bind(&it.purpose).bind(&it.reference).bind(&it.schedule_id)
            .execute(&mut *tx)
            .await?;
    }
    tenant::audit(
        tx,
        tenant_id,
        m.created_by,
        "payout_batch.created",
        Some(&id),
        json!({ "source": m.source, "items": plan.items.len(), "total_minor": plan.total(), "auto_approved": m.auto_approve }),
    )
    .await?;
    tenant::emit(
        tx,
        tenant_id,
        if m.auto_approve {
            "payout_batch.approved"
        } else {
            "payout_batch.approval_required"
        },
        json!({ "batch_id": id, "items": plan.items.len(), "total_minor": plan.total(), "currency": m.currency, "source": m.source }),
    )
    .await?;
    Ok(id)
}

// ---------- reading a batch ----------

const STATUS_CASE: &str = "CASE WHEN i.excluded THEN 'excluded' WHEN p.status = 'paid' THEN 'paid' WHEN p.status = 'rejected' THEN 'rejected' WHEN p.status = 'canceled' THEN 'canceled' WHEN p.status = 'pending_approval' THEN 'pending_approval' ELSE COALESCE(i.rail_state, 'approved') END";

#[derive(Default, Debug)]
struct Counts(BTreeMap<String, (i64, i64)>);

impl Counts {
    fn n(&self, k: &str) -> i64 {
        self.0.get(k).map_or(0, |v| v.0)
    }
    fn open(&self) -> i64 {
        [
            "pending_approval",
            "approved",
            "sending",
            "failed",
            "unknown",
        ]
        .iter()
        .map(|k| self.n(k))
        .sum()
    }
    fn json(&self) -> Value {
        json!(
            self.0
                .iter()
                .map(|(k, v)| (k.clone(), json!({ "count": v.0, "amount_minor": v.1 })))
                .collect::<serde_json::Map<_, _>>()
        )
    }
}

/// What the batch is doing: its decision while it waits or was refused, and afterwards what its payouts say.
fn effective_status(stored: &str, c: &Counts) -> &'static str {
    match stored {
        "pending_approval" => "pending_approval",
        "rejected" => "rejected",
        "canceled" => "canceled",
        _ if c.open() == 0 => "completed",
        _ if c.n("unknown") + c.n("failed") > 0 => "attention",
        _ if c.n("sending") > 0 => "processing",
        _ => "approved",
    }
}

async fn counts_of(conn: &mut PgConnection, batch_id: &str) -> Result<Counts, sqlx::Error> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {STATUS_CASE}, count(*), COALESCE(SUM(i.amount_minor), 0)::BIGINT FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id WHERE i.batch_id = $1 GROUP BY 1"
    )))
    .bind(batch_id)
    .fetch_all(conn)
    .await?;
    Ok(Counts(
        rows.into_iter().map(|(k, n, a)| (k, (n, a))).collect(),
    ))
}

async fn items_of(conn: &mut PgConnection, batch_id: &str) -> Result<Vec<Value>, sqlx::Error> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT jsonb_build_object(
            'id', i.id, 'line', i.line_no, 'vendor_id', i.vendor_id, 'vendor_name', v.name, 'payout_id', i.payout_id,
            'amount_minor', i.amount_minor, 'purpose', i.purpose, 'reference', i.reference, 'schedule_id', i.schedule_id,
            'status', {STATUS_CASE}, 'excluded', i.excluded, 'excluded_reason', i.excluded_reason,
            'attempts', i.attempts, 'last_error', i.last_error, 'rail_ref', i.rail_ref, 'bank_ref', p.bank_ref,
            'requested_by', p.requested_by, 'approved_by', p.approved_by,
            'bank_match', CASE WHEN m.bank_line_id IS NULL THEN NULL ELSE jsonb_build_object(
                'status', m.status, 'rule', m.rule, 'bank_line_id', l.id, 'posted_on', l.posted_on,
                'amount_minor', l.amount_minor, 'narration', l.narration, 'reference', {ref_expr}) END)
         FROM payout_batch_items i
         JOIN payouts p ON p.id = i.payout_id
         JOIN vendors v ON v.id = i.vendor_id
         LEFT JOIN LATERAL (SELECT bank_line_id, status, rule FROM bank_line_matches
                            WHERE subject_type = 'payout' AND subject_id = p.id AND status IN ('confirmed', 'suggested')
                            ORDER BY (status = 'confirmed') DESC, id DESC LIMIT 1) m ON true
         LEFT JOIN bank_statement_lines l ON l.id = m.bank_line_id
         WHERE i.batch_id = $1 ORDER BY i.line_no",
        ref_expr = crate::bankmatch::REF_EXPR
    )))
    .bind(batch_id)
    .fetch_all(conn)
    .await
}

/// The bank side of a batch: how many paid items have a statement line that matched, and how many only have a
/// suggestion a person has not yet confirmed.
fn reconciliation(items: &[Value]) -> Value {
    let paid: Vec<&Value> = items.iter().filter(|i| i["status"] == "paid").collect();
    let state = |i: &&Value| i["bank_match"]["status"].as_str().map(str::to_string);
    let confirmed: Vec<&&Value> = paid
        .iter()
        .filter(|i| state(i).as_deref() == Some("confirmed"))
        .collect();
    let suggested = paid
        .iter()
        .filter(|i| state(i).as_deref() == Some("suggested"))
        .count();
    let matched_minor: i64 = confirmed
        .iter()
        .map(|i| i["bank_match"]["amount_minor"].as_i64().unwrap_or(0).abs())
        .sum();
    json!({
        "paid_items": paid.len(),
        "matched_items": confirmed.len(),
        "matched_minor": matched_minor,
        "suggested_items": suggested,
        "unmatched_items": paid.len() - confirmed.len() - suggested,
    })
}

async fn batch_json(
    conn: &mut PgConnection,
    id: &str,
    with_items: bool,
) -> Result<Value, ApiError> {
    let b: Option<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'name', name, 'batch_ref', batch_ref, 'source', source, 'decision', status,
            'currency', currency, 'from_balance', from_balance, 'rail', rail, 'item_count', item_count, 'total_minor', total_minor,
            'file_name', file_name, 'note', note, 'created_by', created_by, 'approved_by', approved_by, 'approved_at', approved_at,
            'decision_note', decision_note, 'auto_approved', auto_approved, 'fingerprint', fingerprint, 'created_at', created_at)
         FROM payout_batches WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let mut b = b.ok_or_else(|| ApiError::not_found("payout batch"))?;
    let counts = counts_of(conn, id).await?;
    b["status"] = json!(effective_status(
        b["decision"].as_str().unwrap_or(""),
        &counts
    ));
    b["items_by_status"] = counts.json();
    b["paid_minor"] = json!(counts.0.get("paid").map_or(0, |v| v.1));
    let items = items_of(conn, id).await?;
    b["bank_reconciliation"] = reconciliation(&items);
    if with_items {
        b["items"] = json!(items);
    }
    Ok(b)
}

/// `GET /v1/payout_batches/{id}`
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(batch_json(&mut tx, &id, true).await?))
}

#[derive(Deserialize)]
pub struct PayoutBatchListQuery {
    status: Option<String>,
}

/// `GET /v1/payout_batches`
pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<PayoutBatchListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM payout_batches ORDER BY created_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut data = Vec::new();
    for id in ids {
        let b = batch_json(&mut tx, &id, false).await?;
        if q.status.as_deref().is_none_or(|s| b["status"] == s) {
            data.push(b);
        }
    }
    Ok(Json(json!({ "data": data })))
}

// ---------- creating ----------

#[derive(Deserialize)]
pub struct ItemIn {
    /// The vendor's id, or the reference you gave it.
    vendor: String,
    amount_minor: i64,
    purpose: Option<String>,
    reference: Option<String>,
}

#[derive(Deserialize)]
pub struct NewBatch {
    name: Option<String>,
    /// Your own reference for the run. The same content is refused while a batch with it is open or approved; a second
    /// identical run needs a different reference.
    batch_ref: Option<String>,
    /// The file's contents (columns: vendor, amount or amount_minor, purpose, reference, currency).
    csv: Option<String>,
    filename: Option<String>,
    /// `,` (detected by default), `;`, `|` or `tab`.
    delimiter: Option<String>,
    /// Or the lines as a list.
    items: Option<Vec<ItemIn>>,
    /// The purpose of lines that have none.
    default_purpose: Option<String>,
    /// Pay out of each vendor's balance from split payments rather than your own funds.
    from_balance: Option<bool>,
    /// `imps` (default), `neft` or `upi`.
    rail: Option<String>,
    note: Option<String>,
}

fn rejected(errors: &[Problem], warnings: &[Problem]) -> Response {
    let first = errors
        .first()
        .map(|e| {
            if e.line > 0 {
                format!("line {}: {}", e.line, e.message)
            } else {
                e.message.clone()
            }
        })
        .unwrap_or_default();
    let shown: Vec<&Problem> = errors.iter().take(MAX_REPORTED).collect();
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": {
            "code": "invalid_payout_batch",
            "message": format!("the batch was not created: {first}"),
            "errors": shown, "errors_total": errors.len(), "warnings": warnings, "stored": false,
        } })),
    )
        .into_response()
}

fn clean_text(v: &Option<String>, max: usize) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().filter(|c| !c.is_control()).take(max).collect())
}

fn rail_of(v: &Option<String>) -> Result<&'static str, ApiError> {
    match v.as_deref().unwrap_or("imps") {
        "imps" => Ok(RailKind::Imps.as_str()),
        "neft" => Ok(RailKind::Neft.as_str()),
        "upi" => Ok(RailKind::Upi.as_str()),
        _ => Err(ApiError::invalid(
            "invalid_rail",
            "rail must be imps, neft or upi",
        )),
    }
}

/// Parse, plan and (when `commit`) store. Returns the response for the caller.
async fn build(s: &AppState, auth: &Auth, b: NewBatch, commit: bool) -> Result<Response, ApiError> {
    auth.require(if commit {
        "payouts:write"
    } else {
        "payments:read"
    })?;
    if b.csv.is_some() == b.items.is_some() {
        return Err(ApiError::invalid(
            "invalid_payout_batch",
            "give either csv (the file's contents) or items (a list), not both and not neither",
        ));
    }
    let rail = rail_of(&b.rail)?;
    let from_balance = b.from_balance.unwrap_or(false);
    let batch_ref = clean_text(&b.batch_ref, 60);
    let default_purpose = clean_text(&b.default_purpose, 120);
    let (source, file_name) = if b.csv.is_some() {
        ("csv", clean_text(&b.filename, 120))
    } else {
        ("items", None)
    };
    let parsed = match (&b.csv, &b.items) {
        (Some(text), _) => parse_csv(text, b.delimiter.as_deref()),
        (None, Some(items)) => {
            let mut p = Parsed::default();
            if items.is_empty() || items.len() > MAX_ITEMS {
                p.errors.push(problem(
                    0,
                    None,
                    format!("give between 1 and {MAX_ITEMS} items"),
                ));
            }
            for (i, it) in items.iter().enumerate() {
                let line = i + 1;
                let mut bad = false;
                for (name, v) in [
                    ("vendor", Some(it.vendor.as_str())),
                    ("purpose", it.purpose.as_deref()),
                    ("reference", it.reference.as_deref()),
                ] {
                    if let Some(v) = v
                        && let Some(why) = unsafe_cell(v.trim())
                    {
                        p.errors
                            .push(problem(line, Some(name), format!("`{v}` {why}")));
                        bad = true;
                    }
                }
                if it.amount_minor <= 0 || it.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
                    p.errors.push(problem(
                        line,
                        Some("amount_minor"),
                        "amount_minor must be positive and within the supported limit".to_string(),
                    ));
                    bad = true;
                }
                if !bad {
                    p.specs.push(ItemSpec {
                        line,
                        vendor: it.vendor.trim().to_string(),
                        amount_minor: it.amount_minor,
                        purpose: clean_text(&it.purpose, 120),
                        reference: clean_text(&it.reference, 60),
                    });
                }
            }
            p
        }
        (None, None) => Parsed::default(),
    };
    if !parsed.errors.is_empty() {
        return Ok(finish_rejected(commit, &parsed.errors, &parsed.warnings));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    onboarding::require_approved(&mut tx).await?;
    let mut plan = plan(
        &mut tx,
        &s.crypto,
        parsed.specs,
        "INR",
        from_balance,
        default_purpose.as_deref(),
    )
    .await?;
    plan.warnings.extend(parsed.warnings);
    finish(
        s,
        auth,
        tx,
        plan,
        commit,
        Meta {
            name: &clean_text(&b.name, 80)
                .unwrap_or_else(|| file_name.clone().unwrap_or_else(|| "Payout batch".into())),
            batch_ref: batch_ref.as_deref(),
            source,
            currency: "INR",
            from_balance,
            rail,
            file_name: file_name.as_deref(),
            note: clean_text(&b.note, 300).as_deref(),
            created_by: &auth.actor,
            fingerprint: "",
            auto_approve: false,
        },
    )
    .await
}

/// A preview is a 200 that says what would happen; a real attempt is a 422.
fn finish_rejected(commit: bool, errors: &[Problem], warnings: &[Problem]) -> Response {
    if commit {
        return rejected(errors, warnings);
    }
    let shown: Vec<&Problem> = errors.iter().take(MAX_REPORTED).collect();
    (
        StatusCode::OK,
        Json(json!({ "ok": false, "errors": shown, "errors_total": errors.len(), "warnings": warnings, "items": [], "item_count": 0 })),
    )
        .into_response()
}

async fn finish(
    s: &AppState,
    auth: &Auth,
    mut tx: sqlx::Transaction<'static, sqlx::Postgres>,
    plan: Plan,
    commit: bool,
    mut m: Meta<'_>,
) -> Result<Response, ApiError> {
    if !plan.errors.is_empty() {
        return Ok(finish_rejected(commit, &plan.errors, &plan.warnings));
    }
    let fp = fingerprint(m.currency, m.from_balance, m.batch_ref, &plan.items);
    let dup = existing_with(&mut tx, &fp).await?;
    if let Some(d) = &dup
        && commit
    {
        return Err(ApiError::conflict(
            "duplicate_batch",
            format!(
                "the same payouts are already in batch {d}; to pay them again, give this one a different batch_ref"
            ),
        ));
    }
    if let Some(r) = m.batch_ref {
        let used: Option<String> =
            sqlx::query_scalar("SELECT id FROM payout_batches WHERE batch_ref = $1")
                .bind(r)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(u) = used {
            let msg = format!("batch_ref `{r}` is already used by batch {u}");
            if commit {
                return Err(ApiError::conflict("batch_ref_used", msg));
            }
            let mut w = plan.warnings;
            w.push(problem(0, Some("batch_ref"), msg));
            return Ok(preview_json(&plan.items, w, &fp, dup, m.currency));
        }
    }
    if !commit {
        // nothing was written: the locks and the checks are all that happened
        drop(tx);
        return Ok(preview_json(
            &plan.items,
            plan.warnings,
            &fp,
            dup,
            m.currency,
        ));
    }
    m.fingerprint = &fp;
    let id = persist(&mut tx, &auth.tenant_id, &plan, &m).await?;
    let mut out = batch_json(&mut tx, &id, true).await?;
    out["warnings"] = json!(plan.warnings);
    tx.commit().await?;
    let _ = s;
    Ok((StatusCode::CREATED, Json(out)).into_response())
}

fn preview_json(
    items: &[PlannedItem],
    warnings: Vec<Problem>,
    fp: &str,
    dup: Option<String>,
    currency: &str,
) -> Response {
    let total: i64 = items.iter().map(|i| i.amount_minor).sum();
    Json(json!({
        "ok": true, "errors": [], "warnings": warnings, "currency": currency,
        "item_count": items.len(), "total_minor": total, "fingerprint": fp,
        // true when committing this now would be refused as a repeat
        "duplicate_of": dup,
        "needs_approval": "every item waits for a person other than the one who proposes the batch",
        "items": items.iter().map(|i| json!({ "line": i.line, "vendor_id": i.vendor_id, "vendor_name": i.vendor_name, "amount_minor": i.amount_minor, "purpose": i.purpose, "reference": i.reference })).collect::<Vec<_>>(),
        "stored": false,
    }))
    .into_response()
}

/// `POST /v1/payout_batches`: propose a batch from a CSV or a list. All or nothing.
pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewBatch>,
) -> Result<Response, ApiError> {
    build(&s, &auth, b, true).await
}

/// `POST /v1/payout_batches/preview`: the same checks and the same answer, storing nothing.
pub async fn preview(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewBatch>,
) -> Result<Response, ApiError> {
    build(&s, &auth, b, false).await
}

#[derive(Deserialize)]
pub struct FromBalances {
    name: Option<String>,
    batch_ref: Option<String>,
    /// Only these vendors (each must be payable); without it, every active vendor with something to pay.
    vendor_ids: Option<Vec<String>>,
    default_purpose: Option<String>,
    rail: Option<String>,
    note: Option<String>,
    /// Check and report without storing.
    dry_run: Option<bool>,
}

/// `POST /v1/payout_batches/from_balances`: pay each vendor what they are owed from split payments, past their hold
/// days and at least their minimum payout. Vendors with nothing payable are listed as skipped, not paid.
pub async fn from_balances(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<FromBalances>,
) -> Result<Response, ApiError> {
    let commit = !b.dry_run.unwrap_or(false);
    auth.require(if commit {
        "payouts:write"
    } else {
        "payments:read"
    })?;
    let rail = rail_of(&b.rail)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    onboarding::require_approved(&mut tx).await?;
    let explicit = b.vendor_ids.is_some();
    let candidates: Vec<String> = match &b.vendor_ids {
        Some(v) => {
            let set: BTreeSet<String> = v.iter().map(|x| x.trim().to_string()).collect();
            set.into_iter().collect()
        }
        None => sqlx::query_scalar("SELECT v.id FROM vendors v WHERE v.status = 'active' AND v.archived_at IS NULL AND EXISTS (SELECT 1 FROM ledger_entries e WHERE e.account = 'vendor_payable:' || v.id) ORDER BY v.id LIMIT 2000")
            .fetch_all(&mut *tx)
            .await?,
    };
    if candidates.is_empty() || candidates.len() > 2000 {
        return Err(ApiError::invalid(
            "invalid_payout_batch",
            "name between 1 and 2000 vendors, or leave vendor_ids out",
        ));
    }
    let today = clock::today(&mut tx).await?;
    let mut specs = Vec::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();
    for (i, vid) in candidates.iter().enumerate() {
        let line = i + 1;
        let Some(v) = lock_vendor(&mut tx, vid).await? else {
            errors.push(problem(line, Some("vendor"), format!("no vendor {vid}")));
            continue;
        };
        let (_, held, eligible) =
            crate::splits::eligible(&mut tx, vid, "INR", v.hold_days, today).await?;
        let why = if v.status != "active" || v.archived {
            Some(format!(
                "vendor is {}",
                if v.archived { "archived" } else { &v.status }
            ))
        } else if eligible <= 0 {
            Some(if held > 0 {
                format!(
                    "{} is held back until its hold days pass",
                    crate::ai::money(held, "INR")
                )
            } else {
                "nothing owed".to_string()
            })
        } else if eligible < v.min_payout_minor {
            Some(format!(
                "{} is below the vendor's minimum payout",
                crate::ai::money(eligible, "INR")
            ))
        } else {
            None
        };
        match why {
            None => specs.push(ItemSpec {
                line,
                vendor: vid.clone(),
                amount_minor: eligible,
                purpose: None,
                reference: None,
            }),
            Some(w) if explicit => {
                errors.push(problem(line, Some("vendor"), format!("{}: {w}", v.name)))
            }
            Some(w) => skipped.push(json!({ "vendor_id": vid, "name": v.name, "reason": w })),
        }
    }
    if !errors.is_empty() {
        return Ok(finish_rejected(commit, &errors, &[]));
    }
    let default_purpose =
        clean_text(&b.default_purpose, 120).or_else(|| Some("Vendor balance payout".to_string()));
    let batch_ref = clean_text(&b.batch_ref, 60);
    let plan = plan(
        &mut tx,
        &s.crypto,
        specs,
        "INR",
        true,
        default_purpose.as_deref(),
    )
    .await?;
    let name = clean_text(&b.name, 80).unwrap_or_else(|| format!("Vendor balances {today}"));
    let note = clean_text(&b.note, 300);
    let mut resp = finish(
        &s,
        &auth,
        tx,
        plan,
        commit,
        Meta {
            name: &name,
            batch_ref: batch_ref.as_deref(),
            source: "balances",
            currency: "INR",
            from_balance: true,
            rail,
            file_name: None,
            note: note.as_deref(),
            created_by: &auth.actor,
            fingerprint: "",
            auto_approve: false,
        },
    )
    .await?;
    if !skipped.is_empty() {
        // add what was skipped to the JSON the caller gets
        let (parts, body) = resp.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .map_err(|_| ApiError::internal())?;
        let mut v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        v["skipped"] = json!(skipped);
        resp = (parts.status, Json(v)).into_response();
    }
    Ok(resp)
}

// ---------- deciding ----------

#[derive(Deserialize, Default)]
pub struct ApproveIn {
    /// Items to leave out of this approval (their payouts are canceled).
    #[serde(default)]
    exclude: Vec<String>,
    reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct DecisionIn {
    note: Option<String>,
}

async fn lock_batch(
    conn: &mut PgConnection,
    id: &str,
) -> Result<(String, String, String, String, bool), ApiError> {
    let r: Option<(String, String, String, String, bool)> = sqlx::query_as("SELECT status, created_by, currency, rail, from_balance FROM payout_batches WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await?;
    r.ok_or_else(|| ApiError::not_found("payout batch"))
}

fn separation() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "separation_of_duties",
        "the person who proposed a batch cannot approve or reject it",
    )
}

/// `POST /v1/payout_batches/{id}/approve`: approve every item except those left out. A different person from the
/// proposer. The checks run again first; if any item fails, nothing is approved and the failing items are listed.
pub async fn approve(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<ApproveIn>>,
) -> Result<Response, ApiError> {
    auth.require("payouts:approve")?;
    let b = body.map(|j| j.0).unwrap_or_default();
    let reason = clean_text(&b.reason, 200);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (status, created_by, currency, _rail, from_balance) = lock_batch(&mut tx, &id).await?;
    if status != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("batch is {status}"),
        ));
    }
    if created_by == auth.actor {
        return Err(separation());
    }
    onboarding::require_approved(&mut tx).await?;
    // leave out what the approver named
    let known: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM payout_batch_items WHERE batch_id = $1 AND NOT excluded",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    if let Some(bad) = b.exclude.iter().find(|e| !known.contains(e)) {
        return Err(ApiError::invalid(
            "invalid_exclusion",
            format!("item {bad} is not an item of this batch (or is already left out)"),
        ));
    }
    for item in &b.exclude {
        exclude_in_tx(
            &mut tx,
            &auth,
            &id,
            item,
            reason.as_deref().unwrap_or("left out when approving"),
        )
        .await?;
    }
    let rows: Vec<(String, i32, String, String, i64)> = sqlx::query_as(
        "SELECT i.id, i.line_no, i.vendor_id, i.payout_id, i.amount_minor FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id
         WHERE i.batch_id = $1 AND NOT i.excluded AND p.status = 'pending_approval' ORDER BY i.line_no FOR UPDATE OF p",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Err(ApiError::conflict(
            "nothing_to_approve",
            "every item was left out; reject the batch instead",
        ));
    }
    // the checks again, now: a vendor may have been suspended, a balance spent, a limit lowered
    let gate = Gate::load(&mut tx, &s.crypto).await?;
    let mut failing = Vec::new();
    let mut vendors: BTreeSet<String> = rows.iter().map(|r| r.2.clone()).collect();
    let mut locked: HashMap<String, VendorRow> = HashMap::new();
    while let Some(v) = vendors.pop_first() {
        if let Some(row) = lock_vendor(&mut tx, &v).await? {
            locked.insert(v, row);
        }
    }
    let mut total = 0i64;
    for (item_id, line, vendor_id, _payout, amount) in &rows {
        total += amount;
        let Some(v) = locked.get(vendor_id) else {
            continue;
        };
        let found = problems(
            &mut tx,
            &gate,
            v,
            *amount,
            &currency,
            from_balance,
            Mode::Recheck,
        )
        .await?;
        if !found.is_empty() {
            failing.push(json!({ "item_id": item_id, "line": line, "vendor_id": vendor_id, "vendor_name": v.name, "problems": found.iter().map(|(c, m)| json!({ "code": c, "message": m })).collect::<Vec<_>>() }));
        }
    }
    if !failing.is_empty() {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": {
                "code": "items_failed_checks",
                "message": "nothing was approved: some items no longer pass the checks; leave them out (exclude) or fix the cause",
                "items": failing,
            } })),
        )
            .into_response());
    }
    if gate.policy.max_batch_total_minor.is_some_and(|m| total > m) {
        return Err(ApiError::conflict(
            "above_batch_limit",
            format!(
                "the items left total {}, above your policy's limit",
                crate::ai::money(total, &currency)
            ),
        ));
    }
    sqlx::query("UPDATE payouts SET status = 'approved', approved_by = $2, updated_at = now() WHERE id = ANY($1) AND status = 'pending_approval'")
        .bind(rows.iter().map(|r| r.3.clone()).collect::<Vec<_>>())
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE payout_batches SET status = 'approved', approved_by = $2, approved_at = now(), item_count = $3, total_minor = $4, decision_note = $5, updated_at = now() WHERE id = $1")
        .bind(&id).bind(&auth.actor).bind(rows.len() as i32).bind(total).bind(&reason)
        .execute(&mut *tx)
        .await?;
    for (_, _, _, payout, amount) in &rows {
        tenant::emit(
            &mut tx,
            &auth.tenant_id,
            "payout.approved",
            json!({ "payout_id": payout, "amount_minor": amount, "currency": currency }),
        )
        .await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_batch.approved",
        Some(&id),
        json!({ "items": rows.len(), "left_out": b.exclude.len(), "total_minor": total }),
    )
    .await?;
    tenant::emit(&mut tx, &auth.tenant_id, "payout_batch.approved", json!({ "batch_id": id, "items": rows.len(), "total_minor": total, "currency": currency, "source": "approval" })).await?;
    let out = batch_json(&mut tx, &id, true).await?;
    tx.commit().await?;
    Ok(Json(out).into_response())
}

/// Leave one item out of a batch that is waiting: its payout is canceled and the batch shrinks.
async fn exclude_in_tx(
    tx: &mut PgConnection,
    auth: &Auth,
    batch_id: &str,
    item_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    let r: Option<(String, i64)> = sqlx::query_as("SELECT payout_id, amount_minor FROM payout_batch_items WHERE id = $1 AND batch_id = $2 AND NOT excluded FOR UPDATE")
        .bind(item_id)
        .bind(batch_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (payout, amount) = r.ok_or_else(|| ApiError::not_found("batch item"))?;
    sqlx::query("UPDATE payouts SET status = 'canceled', updated_at = now() WHERE id = $1 AND status = 'pending_approval'")
        .bind(&payout)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE payout_batch_items SET excluded = true, excluded_by = $2, excluded_reason = $3 WHERE id = $1")
        .bind(item_id)
        .bind(&auth.actor)
        .bind(reason)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE payout_batches SET item_count = item_count - 1, total_minor = GREATEST(total_minor - $2, 1), updated_at = now() WHERE id = $1")
        .bind(batch_id)
        .bind(amount)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_batch.item_excluded",
        Some(batch_id),
        json!({ "item_id": item_id, "payout_id": payout, "reason": reason }),
    )
    .await?;
    Ok(())
}

/// `POST /v1/payout_batches/{id}/items/{item}/exclude`: leave one line out while the batch waits for approval.
pub async fn exclude_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, item)): Path<(String, String)>,
    body: Option<Json<DecisionIn>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")
        .or_else(|_| auth.require("payouts:write"))?;
    let note = clean_text(&body.and_then(|j| j.0.note), 200).unwrap_or_else(|| "left out".into());
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (status, ..) = lock_batch(&mut tx, &id).await?;
    if status != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("batch is {status}; only a waiting batch can lose an item"),
        ));
    }
    exclude_in_tx(&mut tx, &auth, &id, &item, &note).await?;
    let out = batch_json(&mut tx, &id, true).await?;
    tx.commit().await?;
    Ok(Json(out))
}

async fn close(
    s: &AppState,
    auth: &Auth,
    id: &str,
    approve_scope: bool,
    to: &str,
    payout_to: &str,
    note: Option<String>,
) -> Result<Json<Value>, ApiError> {
    if approve_scope {
        auth.require("payouts:approve")?;
    } else {
        auth.require("payouts:write")?;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (status, created_by, ..) = lock_batch(&mut tx, id).await?;
    if status != "pending_approval" {
        return Err(ApiError::conflict(
            "not_pending_approval",
            format!("batch is {status}"),
        ));
    }
    if approve_scope && created_by == auth.actor {
        return Err(separation());
    }
    sqlx::query("UPDATE payouts SET status = $2, approved_by = CASE WHEN $2 = 'rejected' THEN $3 ELSE approved_by END, updated_at = now() WHERE status = 'pending_approval' AND id IN (SELECT payout_id FROM payout_batch_items WHERE batch_id = $1)")
        .bind(id)
        .bind(payout_to)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE payout_batches SET status = $2, decision_note = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(to)
        .bind(&note)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        &format!("payout_batch.{to}"),
        Some(id),
        json!({ "note": note }),
    )
    .await?;
    if to == "rejected" {
        tenant::emit(
            &mut tx,
            &auth.tenant_id,
            "payout_batch.rejected",
            json!({ "batch_id": id }),
        )
        .await?;
    }
    let out = batch_json(&mut tx, id, true).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// `POST /v1/payout_batches/{id}/reject`: every item is rejected. A different person from the proposer.
pub async fn reject(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<DecisionIn>>,
) -> Result<Json<Value>, ApiError> {
    let note = clean_text(&body.and_then(|j| j.0.note), 200);
    close(&s, &auth, &id, true, "rejected", "rejected", note).await
}

/// `POST /v1/payout_batches/{id}/cancel`: withdraw a batch that has not been approved (anyone who may propose one).
/// Its content can then be proposed again.
pub async fn cancel(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<DecisionIn>>,
) -> Result<Json<Value>, ApiError> {
    let note = clean_text(&body.and_then(|j| j.0.note), 200);
    close(&s, &auth, &id, false, "canceled", "canceled", note).await
}

// ---------- guards the single-payout endpoints use ----------

/// A payout in a batch is approved, rejected and sent with its batch.
pub(crate) async fn refuse_if_batched(
    conn: &mut PgConnection,
    payout_id: &str,
) -> Result<(), ApiError> {
    let b: Option<String> =
        sqlx::query_scalar("SELECT batch_id FROM payout_batch_items WHERE payout_id = $1")
            .bind(payout_id)
            .fetch_optional(conn)
            .await?;
    match b {
        Some(b) => Err(ApiError::conflict(
            "in_batch",
            format!("this payout belongs to payout batch {b}; use the batch"),
        )),
        None => Ok(()),
    }
}

/// A payout of an approved batch can be canceled by hand until a rail call is in flight or its outcome is unknown.
pub(crate) async fn refuse_cancel_if_batched(
    conn: &mut PgConnection,
    payout_id: &str,
) -> Result<(), ApiError> {
    let r: Option<(String, String, Option<String>)> = sqlx::query_as("SELECT b.id, b.status, i.rail_state FROM payout_batch_items i JOIN payout_batches b ON b.id = i.batch_id WHERE i.payout_id = $1 FOR UPDATE OF i")
        .bind(payout_id)
        .fetch_optional(conn)
        .await?;
    match r {
        None => Ok(()),
        Some((b, st, _)) if st == "pending_approval" => Err(ApiError::conflict(
            "in_batch",
            format!("this payout waits in batch {b}; leave it out of the batch instead"),
        )),
        Some((_, _, Some(rs))) if rs != "failed" => Err(ApiError::conflict(
            "rail_in_progress",
            format!("the rail call for this payout is {rs}; it cannot be canceled"),
        )),
        Some(_) => Ok(()),
    }
}

/// While the rail is being called for a payout, nobody records it as paid by hand.
pub(crate) async fn refuse_if_sending(
    conn: &mut PgConnection,
    payout_id: &str,
) -> Result<(), ApiError> {
    let r: Option<Option<String>> = sqlx::query_scalar(
        "SELECT rail_state FROM payout_batch_items WHERE payout_id = $1 FOR UPDATE",
    )
    .bind(payout_id)
    .fetch_optional(conn)
    .await?;
    if r.flatten().as_deref() == Some("sending") {
        return Err(ApiError::conflict(
            "rail_in_progress",
            "the rail is being called for this payout; wait for the outcome",
        ));
    }
    Ok(())
}

/// After a payout is paid: when it was the last open item of its batch, tell the merchant once.
pub(crate) async fn after_paid(
    conn: &mut PgConnection,
    tenant_id: &str,
    payout_id: &str,
) -> Result<(), ApiError> {
    let b: Option<String> =
        sqlx::query_scalar("SELECT batch_id FROM payout_batch_items WHERE payout_id = $1")
            .bind(payout_id)
            .fetch_optional(&mut *conn)
            .await?;
    match b {
        Some(batch) => check_complete(conn, tenant_id, &batch).await,
        None => Ok(()),
    }
}

/// Tell the merchant (once) that a batch is done, when nothing in it is open any more.
async fn check_complete(
    conn: &mut PgConnection,
    tenant_id: &str,
    batch: &str,
) -> Result<(), ApiError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM payout_batches WHERE id = $1")
            .bind(batch)
            .fetch_optional(&mut *conn)
            .await?;
    if status.as_deref() != Some("approved") {
        return Ok(());
    }
    let counts = counts_of(conn, batch).await?;
    if counts.open() == 0 {
        let first = sqlx::query("UPDATE payout_batches SET completed_notified_at = now() WHERE id = $1 AND completed_notified_at IS NULL")
            .bind(batch)
            .execute(&mut *conn)
            .await?
            .rows_affected();
        if first == 1 {
            let (paid, minor) = counts.0.get("paid").copied().unwrap_or((0, 0));
            tenant::emit(
                conn,
                tenant_id,
                "payout_batch.completed",
                json!({ "batch_id": batch, "paid_items": paid, "paid_minor": minor }),
            )
            .await?;
        }
    }
    Ok(())
}

/// Look again once the sends of a batch are committed: payouts paid at the same moment by different requests each
/// saw the others as still open, so the last word is said here, from a fresh view.
async fn notify_if_complete(s: &AppState, tenant_id: &str, batch: &str) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    check_complete(&mut tx, tenant_id, batch).await?;
    tx.commit().await?;
    Ok(())
}

// ---------- sending ----------

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Skipped(&'static str),
    Paid,
    Failed,
    Unknown,
}

enum Outcome {
    Paid(String),
    Failed(String),
    Unknown(String),
}

/// What the rail said, as one of three outcomes. An answer that is not clearly paid or clearly failed is unknown.
/// A refusal to a `send` is a failure; a refusal to a `fetch` ("no such payout") proves nothing, so it stays unknown.
fn classify(
    r: Result<zyvor_settle::PayoutResult, RailError>,
    payout_id: &str,
    fetched: bool,
) -> Outcome {
    match r {
        Ok(res) => match res.status {
            PayoutRailStatus::Settled => Outcome::Paid(res.rail_ref.unwrap_or_else(|| {
                format!("RAIL{}", &payout_id[payout_id.len().saturating_sub(8)..])
            })),
            PayoutRailStatus::Accepted => {
                Outcome::Unknown("accepted by the rail, not yet settled".into())
            }
            PayoutRailStatus::Failed => Outcome::Failed(
                res.failure_reason
                    .unwrap_or_else(|| "the rail refused it".into()),
            ),
            PayoutRailStatus::Unknown => Outcome::Unknown(
                res.failure_reason
                    .unwrap_or_else(|| "the rail could not say".into()),
            ),
        },
        Err(RailError::Rejected(m)) if !fetched => Outcome::Failed(m),
        Err(RailError::Unsupported(m)) if !fetched => Outcome::Failed(m),
        Err(e) => Outcome::Unknown(e.to_string()),
    }
}

fn short(m: &str) -> String {
    m.chars().filter(|c| !c.is_control()).take(300).collect()
}

/// Write down what the rail said. For paid: the payout is marked paid and posted to the ledger in the same
/// transaction as the item, so the two cannot disagree. Only an item that is `sending` or `unknown` can be settled
/// this way, so a second call with the same answer changes nothing.
async fn record(
    s: &AppState,
    tenant_id: &str,
    actor: &str,
    item_id: &str,
    out: Outcome,
) -> Result<Step, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let r: Option<(String, Option<String>)> = sqlx::query_as("SELECT i.payout_id, i.rail_state FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id WHERE i.id = $1 FOR UPDATE OF i, p")
        .bind(item_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((payout_id, state)) = r else {
        return Ok(Step::Skipped("gone"));
    };
    if !matches!(state.as_deref(), Some("sending" | "unknown")) {
        return Ok(Step::Skipped("not_waiting_for_an_outcome"));
    }
    match out {
        Outcome::Paid(rail_ref) => {
            let done = crate::payouts::finalize_paid(
                &mut tx,
                tenant_id,
                actor,
                &payout_id,
                &rail_ref,
                Some("rail"),
            )
            .await;
            match done {
                Ok(_) => {
                    sqlx::query("UPDATE payout_batch_items SET rail_state = NULL, rail_ref = $2, last_error = NULL WHERE id = $1")
                        .bind(item_id)
                        .bind(&rail_ref)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    let batch: Option<String> = {
                        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
                        sqlx::query_scalar("SELECT batch_id FROM payout_batch_items WHERE id = $1")
                            .bind(item_id)
                            .fetch_optional(&mut *tx)
                            .await?
                    };
                    if let Some(b) = batch {
                        notify_if_complete(s, tenant_id, &b).await?;
                    }
                    Ok(Step::Paid)
                }
                Err(e) => {
                    // the rail says it paid but we could not record it (for example its reference is already used):
                    // never send again; a person looks
                    drop(tx);
                    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
                    sqlx::query("UPDATE payout_batch_items SET rail_state = 'unknown', rail_ref = $2, last_error = $3 WHERE id = $1")
                        .bind(item_id)
                        .bind(&rail_ref)
                        .bind(short(&format!("the rail reports it paid (reference {rail_ref}) but it could not be recorded: {}", e.message)))
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    Ok(Step::Unknown)
                }
            }
        }
        Outcome::Failed(m) => {
            sqlx::query("UPDATE payout_batch_items SET rail_state = 'failed', last_error = $2 WHERE id = $1")
                .bind(item_id)
                .bind(short(&m))
                .execute(&mut *tx)
                .await?;
            tenant::audit(
                &mut tx,
                tenant_id,
                actor,
                "payout_batch.item_failed",
                Some(&payout_id),
                json!({ "item_id": item_id, "reason": short(&m) }),
            )
            .await?;
            tx.commit().await?;
            Ok(Step::Failed)
        }
        Outcome::Unknown(m) => {
            sqlx::query("UPDATE payout_batch_items SET rail_state = 'unknown', last_error = $2 WHERE id = $1")
                .bind(item_id)
                .bind(short(&m))
                .execute(&mut *tx)
                .await?;
            tenant::audit(
                &mut tx,
                tenant_id,
                actor,
                "payout_batch.item_unknown",
                Some(&payout_id),
                json!({ "item_id": item_id, "reason": short(&m) }),
            )
            .await?;
            tx.commit().await?;
            Ok(Step::Unknown)
        }
    }
}

/// Send one item to the rail, or retry a failed one. The item is claimed (`sending`) in its own committed
/// transaction before the rail is called, so a second caller sees it claimed and does nothing; the payout id is the
/// rail's idempotency key.
pub async fn process_item(
    s: &AppState,
    tenant_id: &str,
    actor: &str,
    item_id: &str,
    retrying: bool,
) -> Result<Step, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let r: Option<(String, String, Option<String>, i32, bool, String, i64, String, String, String)> = sqlx::query_as(
        "SELECT i.payout_id, i.vendor_id, i.rail_state, i.attempts, i.excluded, p.status, p.amount_minor, p.currency, b.rail, b.id
         FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id JOIN payout_batches b ON b.id = i.batch_id
         WHERE i.id = $1 FOR UPDATE OF i, p",
    )
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((
        payout_id,
        vendor_id,
        rail_state,
        attempts,
        excluded,
        pstatus,
        amount,
        currency,
        rail,
        _batch,
    )) = r
    else {
        return Ok(Step::Skipped("gone"));
    };
    if excluded || pstatus != "approved" {
        return Ok(Step::Skipped("not_approved"));
    }
    match (retrying, rail_state.as_deref()) {
        (false, None) => {}
        (true, Some("failed")) => {
            if attempts >= MAX_ATTEMPTS {
                return Ok(Step::Skipped("attempts_exhausted"));
            }
        }
        (_, Some("sending")) => return Ok(Step::Skipped("already_sending")),
        (_, Some("unknown")) => return Ok(Step::Skipped("outcome_unknown")),
        _ => return Ok(Step::Skipped("not_ready")),
    }
    onboarding::require_approved(&mut tx).await?;
    let from_balance: bool =
        sqlx::query_scalar("SELECT source = 'balance' FROM payouts WHERE id = $1")
            .bind(&payout_id)
            .fetch_one(&mut *tx)
            .await?;
    let gate = Gate::load(&mut tx, &s.crypto).await?;
    let Some(vendor) = lock_vendor(&mut tx, &vendor_id).await? else {
        return Ok(Step::Skipped("gone"));
    };
    let found = problems(
        &mut tx,
        &gate,
        &vendor,
        amount,
        &currency,
        from_balance,
        Mode::Recheck,
    )
    .await?;
    let details: Value = vendor
        .details_enc
        .as_deref()
        .and_then(|b| s.crypto.decrypt(b))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let rail_kind = match rail.as_str() {
        "neft" => RailKind::Neft,
        "upi" => RailKind::Upi,
        _ => RailKind::Imps,
    };
    let usable = match rail_kind {
        RailKind::Upi => details["vpa"].as_str().is_some(),
        _ => details["account_number"].as_str().is_some() && details["ifsc"].as_str().is_some(),
    };
    let stop = found.first().map(|(c, m)| format!("{c}: {m}")).or_else(|| {
        (!usable).then(|| {
            format!(
                "the vendor has no {} to pay on the {rail} rail",
                if rail_kind == RailKind::Upi {
                    "UPI id"
                } else {
                    "bank account"
                }
            )
        })
    });
    if let Some(why) = stop {
        sqlx::query("UPDATE payout_batch_items SET rail_state = 'failed', last_error = $2, last_attempt_at = now() WHERE id = $1")
            .bind(item_id)
            .bind(short(&why))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Step::Failed);
    }
    sqlx::query("UPDATE payout_batch_items SET rail_state = 'sending', attempts = attempts + 1, last_attempt_at = now(), last_error = NULL WHERE id = $1")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        tenant_id,
        actor,
        "payout_batch.item_sent",
        Some(&payout_id),
        json!({ "item_id": item_id, "attempt": attempts + 1, "retry": retrying }),
    )
    .await?;
    tx.commit().await?;

    let insn = PayoutInstruction {
        payout_id: payout_id.clone(),
        amount_minor: amount,
        currency: currency.parse().map_err(|_| ApiError::internal())?,
        rail: rail_kind,
        account_number: details["account_number"].as_str().map(str::to_string),
        ifsc: details["ifsc"].as_str().map(str::to_string),
        vpa: details["vpa"].as_str().map(str::to_string),
        beneficiary_name: vendor.holder_name.clone().unwrap_or(vendor.name.clone()),
    };
    let result = crate::settle::rail_send(s, &insn).await;
    record(
        s,
        tenant_id,
        actor,
        item_id,
        classify(result, &payout_id, false),
    )
    .await
}

/// Ask the rail about an item whose outcome is unknown; never sends.
pub async fn resolve_item(
    s: &AppState,
    tenant_id: &str,
    actor: &str,
    item_id: &str,
) -> Result<Step, ApiError> {
    let payout: Option<String> = {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        sqlx::query_scalar("SELECT payout_id FROM payout_batch_items WHERE id = $1 AND rail_state IN ('unknown', 'sending')")
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await?
    };
    let Some(payout) = payout else {
        return Ok(Step::Skipped("nothing_to_resolve"));
    };
    let result = crate::settle::rail_fetch(s, &payout).await;
    // an item still unknown after asking stays unknown: no state change
    let out = classify(result, &payout, true);
    if let Outcome::Unknown(m) = &out {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        sqlx::query("UPDATE payout_batch_items SET rail_state = 'unknown', last_error = $2 WHERE id = $1 AND rail_state IN ('unknown', 'sending')")
            .bind(item_id)
            .bind(short(m))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Step::Unknown);
    }
    record(s, tenant_id, actor, item_id, out).await
}

async fn ready_batch(s: &AppState, auth: &Auth, id: &str) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (status, ..) = lock_batch(&mut tx, id).await?;
    if status != "approved" {
        return Err(ApiError::conflict(
            "not_approved",
            format!("batch is {status}; only an approved batch can be sent"),
        ));
    }
    if money_mode::load_for_tenant(&mut tx, &auth.tenant_id).await?
        != zyvor_ledger::MoneyMode::Acquire
    {
        return Err(ApiError::conflict(
            "orchestrate_mode",
            "use mark_paid for each payout in orchestrate mode (the batch report lists the transfers); rail payouts are acquire-only",
        ));
    }
    onboarding::require_approved(&mut tx).await?;
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct RetryIn {
    /// Only these items; without it, every failed item.
    item_ids: Option<Vec<String>>,
}

async fn run(
    s: &AppState,
    auth: &Auth,
    id: &str,
    kind: &str,
    only: Option<Vec<String>>,
) -> Result<Json<Value>, ApiError> {
    ready_batch(s, auth, id).await?;
    let ids: Vec<String> = {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let sql = match kind {
            "send" => {
                "SELECT i.id FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id WHERE i.batch_id = $1 AND NOT i.excluded AND p.status = 'approved' AND i.rail_state IS NULL ORDER BY i.line_no"
            }
            "retry" => {
                "SELECT i.id FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id WHERE i.batch_id = $1 AND NOT i.excluded AND p.status = 'approved' AND i.rail_state = 'failed' ORDER BY i.line_no"
            }
            _ => {
                "SELECT i.id FROM payout_batch_items i WHERE i.batch_id = $1 AND i.rail_state IN ('unknown', 'sending') ORDER BY i.line_no"
            }
        };
        let all: Vec<String> = sqlx::query_scalar(sql).bind(id).fetch_all(&mut *tx).await?;
        match only {
            Some(o) => all.into_iter().filter(|i| o.contains(i)).collect(),
            None => all,
        }
    };
    let mut tally: BTreeMap<&'static str, i64> = BTreeMap::new();
    for item in &ids {
        let step = match kind {
            "resolve" => resolve_item(s, &auth.tenant_id, &auth.actor, item).await?,
            _ => process_item(s, &auth.tenant_id, &auth.actor, item, kind == "retry").await?,
        };
        let k = match step {
            Step::Paid => "paid",
            Step::Failed => "failed",
            Step::Unknown => "unknown",
            Step::Skipped(r) => r,
        };
        *tally.entry(k).or_default() += 1;
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut out = batch_json(&mut tx, id, true).await?;
    out["run"] = json!({ "action": kind, "attempted": ids.len(), "outcomes": tally });
    Ok(Json(out))
}

/// `POST /v1/payout_batches/{id}/send`: send every approved item that has not been sent (acquire mode). Safe to call
/// again, or twice at once: an item already claimed, paid or unknown is left alone.
pub async fn send(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    run(&s, &auth, &id, "send", None).await
}

/// `POST /v1/payout_batches/{id}/retry`: send the items the rail refused again, with the same payout id (so the same
/// instruction), up to five attempts each. Never touches an item whose outcome is unknown.
pub async fn retry(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<RetryIn>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    run(&s, &auth, &id, "retry", body.and_then(|j| j.0.item_ids)).await
}

/// `POST /v1/payout_batches/{id}/resolve`: ask the rail what became of items whose outcome was unknown. Sends nothing.
pub async fn resolve(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    run(&s, &auth, &id, "resolve", None).await
}

/// The worker's part, per tenant: ask about items left unknown (or stuck sending after a crash) for longer than
/// `resolve_after`, and send the items of batches approved by the tenant's own auto-approve setting.
pub async fn sweep(s: &AppState, tenant_id: &str) -> Result<usize, ApiError> {
    let secs = s.resolve_after.as_secs_f64();
    let (stale, due, acquire): (Vec<String>, Vec<String>, bool) = {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let acquire = money_mode::load_for_tenant(&mut tx, tenant_id).await?
            == zyvor_ledger::MoneyMode::Acquire;
        let stale = sqlx::query_scalar("SELECT id FROM payout_batch_items WHERE rail_state IN ('unknown', 'sending') AND last_attempt_at < now() - make_interval(secs => $1::float8) ORDER BY last_attempt_at LIMIT 50")
            .bind(secs)
            .fetch_all(&mut *tx)
            .await?;
        let due = if acquire {
            sqlx::query_scalar("SELECT i.id FROM payout_batch_items i JOIN payout_batches b ON b.id = i.batch_id JOIN payouts p ON p.id = i.payout_id WHERE b.auto_approved AND b.status = 'approved' AND NOT i.excluded AND p.status = 'approved' AND i.rail_state IS NULL ORDER BY b.created_at, i.line_no LIMIT 50")
                .fetch_all(&mut *tx)
                .await?
        } else {
            Vec::new()
        };
        (stale, due, acquire)
    };
    let mut n = 0;
    if acquire {
        for id in stale {
            resolve_item(s, tenant_id, "system:worker", &id).await?;
            n += 1;
        }
    }
    for id in due {
        // an auto-approved batch is sent by the worker only because the tenant's policy said so
        process_item(s, tenant_id, "system:auto-approve", &id, false).await?;
        n += 1;
    }
    Ok(n)
}

// ---------- the report ----------

fn csv_cell(v: &str) -> String {
    // a cell that starts like a formula is neutralised, since the report is meant to be opened in a spreadsheet
    let mut s = v.to_string();
    if matches!(s.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r')) {
        s.insert(0, '\'');
    }
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

fn rupees(minor: i64) -> String {
    format!("{}.{:02}", minor / 100, minor % 100)
}

/// `GET /v1/payout_batches/{id}/report.csv`: one line per item with where it stands and the bank statement line that
/// matched it. In orchestrate mode this is also the list of transfers to make.
pub async fn report_csv(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let b = batch_json(&mut tx, &id, true).await?;
    let mut out = String::from(
        "line,item_id,payout_id,vendor_id,vendor,amount_inr,purpose,reference,status,attempts,bank_ref,last_error,approved_by,bank_match,bank_line_date,bank_line_amount_inr,bank_line_reference\n",
    );
    let g = |v: &Value| v.as_str().unwrap_or("").to_string();
    for it in b["items"].as_array().into_iter().flatten() {
        let m = &it["bank_match"];
        let cols = [
            it["line"].as_i64().unwrap_or(0).to_string(),
            g(&it["id"]),
            g(&it["payout_id"]),
            g(&it["vendor_id"]),
            g(&it["vendor_name"]),
            rupees(it["amount_minor"].as_i64().unwrap_or(0)),
            g(&it["purpose"]),
            g(&it["reference"]),
            g(&it["status"]),
            it["attempts"].as_i64().unwrap_or(0).to_string(),
            g(&it["bank_ref"]),
            g(&it["last_error"]),
            g(&it["approved_by"]),
            if m.is_null() {
                "none".to_string()
            } else {
                g(&m["status"])
            },
            g(&m["posted_on"]),
            m["amount_minor"]
                .as_i64()
                .map(|a| rupees(a.abs()))
                .unwrap_or_default(),
            g(&m["reference"]),
        ];
        out.push_str(
            &cols
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    let mut res = Response::new(axum::body::Body::from(out));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"payout-batch-{id}.csv\""))
            .map_err(|_| ApiError::internal())?,
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rupees_are_read_strictly() {
        assert_eq!(parse_rupees("12500"), Ok(1_250_000));
        assert_eq!(parse_rupees("12500.5"), Ok(1_250_050));
        assert_eq!(parse_rupees("0.01"), Ok(1));
        assert_eq!(parse_rupees(" 7.25 "), Ok(725));
        for bad in [
            "",
            "0",
            "0.00",
            "-5",
            "+5",
            "12,500",
            "1.234",
            "1.",
            ".5",
            "1e3",
            "1.25E+04",
            "₹5",
            "5 INR",
            "abc",
            "99999999999999999999",
        ] {
            assert!(parse_rupees(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn a_file_is_read_line_by_line_and_fails_as_a_whole() {
        let ok = parse_csv(
            "Vendor,Amount,Purpose,Reference\nven_a,100.50,June,INV-1\nacme,20,June,INV-2\n",
            None,
        );
        assert!(ok.errors.is_empty(), "{:?}", ok.errors);
        assert_eq!(ok.specs.len(), 2);
        assert_eq!((ok.specs[0].amount_minor, ok.specs[0].line), (10_050, 2));

        let bad = parse_csv(
            "vendor,amount,purpose\nven_a,12,500,June\nven_b,abc,June\n=cmd,10,June\nven_c,5,June\nven_c,5,June\n",
            None,
        );
        let lines: Vec<usize> = bad.errors.iter().map(|e| e.line).collect();
        // the comma in 12,500 gives a line with the wrong number of cells; the formula and the repeated line are named
        assert_eq!(lines, vec![2, 3, 4, 6], "{:?}", bad.errors);
        assert!(
            bad.errors[0]
                .message
                .contains("4 cells but the header has 3")
        );
        assert!(bad.errors[3].message.contains("line 5"));
    }

    #[test]
    fn the_header_must_say_what_is_in_each_column() {
        assert!(!parse_csv("vendor,purpose\nv,x\n", None).errors.is_empty());
        assert!(
            !parse_csv("vendor,amount,amount_minor\nv,1,100\n", None)
                .errors
                .is_empty()
        );
        assert!(!parse_csv("amount\n5\n", None).errors.is_empty());
        assert!(!parse_csv("", None).errors.is_empty());
        assert!(
            !parse_csv("vendor,amount\n", None).errors.is_empty(),
            "a header alone is not a batch"
        );
        let semi = parse_csv("vendor;amount_minor;purpose\nv1;12345;x\n", None);
        assert!(semi.errors.is_empty() && semi.specs[0].amount_minor == 12_345);
        let extra = parse_csv("vendor,amount,purpose,colour\nv1,5,x,red\n", None);
        assert!(extra.errors.is_empty() && extra.warnings.len() == 1);
        assert!(
            !parse_csv("vendor,amount,purpose,currency\nv1,5,x,USD\n", None)
                .errors
                .is_empty()
        );
    }

    #[test]
    fn the_fingerprint_ignores_order_and_names_but_not_money() {
        let item = |v: &str, a: i64| PlannedItem {
            line: 1,
            vendor_id: v.into(),
            vendor_name: "n".into(),
            amount_minor: a,
            purpose: "p".into(),
            reference: None,
            ext_ref: None,
            schedule_id: None,
        };
        let a = fingerprint("INR", false, None, &[item("v1", 100), item("v2", 200)]);
        let b = fingerprint("INR", false, None, &[item("v2", 200), item("v1", 100)]);
        assert_eq!(a, b);
        assert_ne!(
            a,
            fingerprint("INR", false, None, &[item("v1", 101), item("v2", 200)])
        );
        assert_ne!(
            a,
            fingerprint("INR", true, None, &[item("v1", 100), item("v2", 200)])
        );
        assert_ne!(
            a,
            fingerprint(
                "INR",
                false,
                Some("run-2"),
                &[item("v1", 100), item("v2", 200)]
            )
        );
    }

    #[test]
    fn batch_status_follows_what_the_items_say() {
        let c = |pairs: &[(&str, i64)]| {
            Counts(
                pairs
                    .iter()
                    .map(|(k, n)| (k.to_string(), (*n, 0)))
                    .collect(),
            )
        };
        assert_eq!(
            effective_status("pending_approval", &c(&[("pending_approval", 3)])),
            "pending_approval"
        );
        assert_eq!(
            effective_status("approved", &c(&[("approved", 2)])),
            "approved"
        );
        assert_eq!(
            effective_status("approved", &c(&[("paid", 1), ("sending", 1)])),
            "processing"
        );
        assert_eq!(
            effective_status("approved", &c(&[("paid", 1), ("failed", 1)])),
            "attention"
        );
        assert_eq!(
            effective_status(
                "approved",
                &c(&[("paid", 1), ("unknown", 1), ("sending", 1)])
            ),
            "attention"
        );
        assert_eq!(
            effective_status(
                "approved",
                &c(&[("paid", 2), ("canceled", 1), ("excluded", 1)])
            ),
            "completed"
        );
        assert_eq!(
            effective_status("rejected", &c(&[("rejected", 2)])),
            "rejected"
        );
    }

    #[test]
    fn report_cells_cannot_run_as_formulas() {
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(rupees(10_005), "100.05");
    }

    #[test]
    fn an_unclear_rail_answer_is_never_a_failure() {
        let res = |status| zyvor_settle::PayoutResult {
            status,
            rail_ref: Some("UTR123".into()),
            failure_reason: None,
            raw: json!({}),
        };
        assert!(
            matches!(classify(Ok(res(PayoutRailStatus::Settled)), "pay_x", false), Outcome::Paid(r) if r == "UTR123")
        );
        assert!(matches!(
            classify(Ok(res(PayoutRailStatus::Accepted)), "pay_x", false),
            Outcome::Unknown(_)
        ));
        assert!(matches!(
            classify(Ok(res(PayoutRailStatus::Failed)), "pay_x", false),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            classify(Err(RailError::Uncertain("timeout".into())), "pay_x", false),
            Outcome::Unknown(_)
        ));
        assert!(matches!(
            classify(Err(RailError::Rejected("no".into())), "pay_x", false),
            Outcome::Failed(_)
        ));
        // asking "what happened to it?" and being told "no such payout" does not prove it was not paid
        assert!(matches!(
            classify(Err(RailError::Rejected("not_found".into())), "pay_x", true),
            Outcome::Unknown(_)
        ));
    }
}
