// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Matching bank statement lines to what they pay for: provider settlements and direct payments (credits), payouts
//! and refunds (debits).
//!
//! Two kinds of match, never confused:
//! * a **reference** match: the UTR / RRN / bank reference we hold for the thing is, as a whole token, in the line's
//!   reference column or narration. It is recorded as `confirmed`, with the rule and the key that matched.
//! * an **amount-and-date** match: the same amount within a few days. This is only ever a `suggested` match with its
//!   evidence; it stays that way until a person confirms it (`POST /v1/bank_statements/matches/{id}/confirm`).
//!
//! Every match, confirmed or not, is a row in `bank_line_matches`, so the question "why did this line match that
//! settlement" always has an answer. Unmatched credits become cases (`unmatched_bank_credit`); settlements and payouts
//! the statement should show but does not become cases too (`settlement_no_bank_credit`, `payout_no_bank_debit`).

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::Currency;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, recon, tenant};

/// Shortest reference worth matching on: anything shorter appears by chance.
const MIN_KEY: usize = 6;
/// How many days either side of a settlement or payment the amount-and-date rule looks.
pub const WINDOW_DAYS: i64 = 3;

/// Every SQL string built here is assembled from constants in this file; nothing the caller sends is spliced in.
fn asql(s: String) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(s)
}

fn money(minor: i64) -> String {
    format!("₹{}", zyvor_core::minor_to_decimal(minor, Currency::Inr))
}

/// Upper-case letters and digits only: `hdfcn5-2026/11` and `HDFCN5202611` are the same reference.
pub fn norm_key(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// A reference only counts when it is long enough and has a digit in it (so a word is never a key).
pub fn usable_key(k: &str) -> bool {
    k.len() >= MIN_KEY && k.bytes().any(|b| b.is_ascii_digit())
}

fn tokens(text: &str) -> HashSet<String> {
    let parts: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_ascii_uppercase())
        .collect();
    let mut out = HashSet::new();
    for (i, t) in parts.iter().enumerate() {
        // "UTR998877" and "RRN404223241811" carry their label glued on
        for label in ["UTR", "RRN", "REF"] {
            if let Some(rest) = t.strip_prefix(label)
                && rest.len() >= MIN_KEY
            {
                out.insert(rest.to_string());
            }
        }
        out.insert(t.clone());
        // a reference a bank or a provider writes with a separator in it (UTR_0001 / UTR-0001) is still one reference
        for n in 2..=3 {
            if i + n <= parts.len() {
                out.insert(parts[i..i + n].concat());
            }
        }
    }
    out
}

/// The identifiers a bank line carries: its reference column and the tokens of its narration.
pub struct LineKeys {
    reference: Option<String>,
    reference_tokens: HashSet<String>,
    narration_tokens: HashSet<String>,
}

impl LineKeys {
    pub fn new(reference: Option<&str>, narration: &str) -> Self {
        Self {
            reference: reference.map(norm_key).filter(|r| !r.is_empty()),
            reference_tokens: reference.map(tokens).unwrap_or_default(),
            narration_tokens: tokens(narration),
        }
    }

    /// Which rule makes `key` (already normalised) a hit on this line, if any.
    pub fn hit(&self, key: &str) -> Option<&'static str> {
        if !usable_key(key) {
            return None;
        }
        if self.reference.as_deref() == Some(key) || self.reference_tokens.contains(key) {
            Some("reference_column")
        } else if self.narration_tokens.contains(key) {
            Some("reference_in_narration")
        } else {
            None
        }
    }

    /// Every usable token on the line, to look up against the references we hold.
    pub fn all_keys(&self) -> Vec<String> {
        let mut k: HashSet<&String> = HashSet::new();
        k.extend(self.reference_tokens.iter());
        k.extend(self.narration_tokens.iter());
        if let Some(r) = &self.reference {
            k.insert(r);
        }
        let mut v: Vec<String> = k.into_iter().filter(|k| usable_key(k)).cloned().collect();
        v.sort();
        v.truncate(60);
        v
    }
}

/// SQL for "the reference as the bank gave it": the column, else the old `utr` when it is a real reference.
pub(crate) const REF_EXPR: &str = "COALESCE(l.reference, CASE WHEN l.utr IS NULL OR l.utr ~ '^(csv|mt|camt|st):' THEN NULL ELSE l.utr END)";
/// `l` has no confirmed match, by the new table or by the older link.
const UNMATCHED: &str = "l.matched_settlement_id IS NULL AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.bank_line_id = l.id AND m.status = 'confirmed')";

#[derive(Default)]
pub struct Summary {
    pub settlements_rechecked: usize,
    pub confirmed: usize,
    pub suggested: usize,
    pub cases_opened: usize,
}

type LineRow = (i64, NaiveDate, i64, String, Option<String>);

// ---------- recording matches ----------

#[allow(clippy::too_many_arguments)]
async fn suggest(
    conn: &mut PgConnection,
    tenant_id: &str,
    line_id: i64,
    subject_type: &str,
    subject_id: &str,
    rule: &str,
    key: Option<&str>,
    evidence: Value,
) -> Result<bool, ApiError> {
    // never overwrites a decision: a rejected suggestion stays rejected
    let n = sqlx::query(
        "INSERT INTO bank_line_matches (tenant_id, bank_line_id, subject_type, subject_id, status, rule, key_matched, evidence)
         VALUES ($1, $2, $3, $4, 'suggested', $5, $6, $7) ON CONFLICT (tenant_id, bank_line_id, subject_type, subject_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(line_id)
    .bind(subject_type)
    .bind(subject_id)
    .bind(rule)
    .bind(key)
    .bind(evidence)
    .execute(conn)
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Make a match final: record the evidence, link the settlement, retire competing suggestions, and close the
/// cases that said the line or the thing was unexplained.
#[allow(clippy::too_many_arguments)]
pub async fn confirm_match(
    conn: &mut PgConnection,
    tenant_id: &str,
    line_id: i64,
    subject_type: &str,
    subject_id: &str,
    rule: &str,
    key: Option<&str>,
    evidence: Value,
    decided_by: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO bank_line_matches (tenant_id, bank_line_id, subject_type, subject_id, status, rule, key_matched, evidence, decided_by, decided_at)
         VALUES ($1, $2, $3, $4, 'confirmed', $5, $6, $7, $8, now())
         ON CONFLICT (tenant_id, bank_line_id, subject_type, subject_id)
         DO UPDATE SET status = 'confirmed', rule = EXCLUDED.rule, key_matched = EXCLUDED.key_matched, evidence = EXCLUDED.evidence,
                       decided_by = EXCLUDED.decided_by, decided_at = now()",
    )
    .bind(tenant_id)
    .bind(line_id)
    .bind(subject_type)
    .bind(subject_id)
    .bind(rule)
    .bind(key)
    .bind(evidence)
    .bind(decided_by)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE bank_line_matches SET status = 'superseded'
         WHERE status = 'suggested' AND (bank_line_id = $1 OR (subject_type = $2 AND subject_id = $3))",
    )
    .bind(line_id)
    .bind(subject_type)
    .bind(subject_id)
    .execute(&mut *conn)
    .await?;
    let resolution = format!("matched to {subject_type} {subject_id} ({rule})");
    if subject_type == "settlement" {
        sqlx::query("UPDATE bank_statement_lines SET matched_settlement_id = $2 WHERE id = $1")
            .bind(line_id)
            .bind(subject_id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("UPDATE settlements SET bank_line_id = $2 WHERE id = $1")
            .bind(subject_id)
            .bind(line_id)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query(
        "UPDATE recon_cases SET status = 'resolved', resolution = $3, resolved_at = now()
         WHERE status <> 'resolved' AND (
               (kind IN ('unmatched_bank_credit', 'unmatched_bank_debit') AND subject_id = $1)
            OR (kind IN ('settlement_no_bank_credit', 'payout_no_bank_debit') AND subject_id = $2))",
    )
    .bind(line_id.to_string())
    .bind(subject_id)
    .bind(resolution)
    .execute(conn)
    .await?;
    Ok(())
}

fn days_apart(a: NaiveDate, b: NaiveDate) -> i64 {
    (a - b).num_days()
}

// ---------- settlements (credits) ----------

/// Find the bank credit for one settlement. Returns the linked line `(id, amount)` when there is one.
/// A line carrying the settlement's UTR is linked at once (an amount that differs is the caller's case to open);
/// without one, lines with the same amount near the settlement day are only suggested.
pub async fn match_settlement(
    conn: &mut PgConnection,
    tenant_id: &str,
    settlement_id: &str,
    net: i64,
    settled_on: NaiveDate,
    currency: &str,
    utr: Option<&str>,
) -> Result<Option<(i64, i64)>, ApiError> {
    let linked: Option<(i64, i64)> = sqlx::query_as(
        "SELECT l.id, l.amount_minor FROM settlements s JOIN bank_statement_lines l ON l.id = s.bank_line_id WHERE s.id = $1",
    )
    .bind(settlement_id)
    .fetch_optional(&mut *conn)
    .await?;
    if linked.is_some() {
        return Ok(linked);
    }
    let key = utr.map(norm_key).filter(|k| usable_key(k));
    let mut ambiguous = false;
    if let Some(key) = &key {
        let cands: Vec<LineRow> = sqlx::query_as(asql(format!(
            "SELECT l.id, l.posted_on, l.amount_minor, l.narration, {REF_EXPR} FROM bank_statement_lines l
             WHERE l.amount_minor > 0 AND l.currency = $1 AND {UNMATCHED}
               AND (upper(regexp_replace(COALESCE({REF_EXPR}, ''), '[^A-Za-z0-9]', '', 'g')) LIKE '%' || $2 || '%'
                    OR upper(regexp_replace(l.narration, '[^A-Za-z0-9]', '', 'g')) LIKE '%' || $2 || '%')
             ORDER BY l.posted_on, l.id"
        )))
        .bind(currency)
        .bind(key)
        .fetch_all(&mut *conn)
        .await?;
        let hits: Vec<(&LineRow, &'static str)> = cands
            .iter()
            .filter_map(|r| {
                LineKeys::new(r.4.as_deref(), &r.3)
                    .hit(key)
                    .map(|rule| (r, rule))
            })
            .collect();
        let chosen = match hits.len() {
            0 => None,
            1 => Some(hits[0]),
            _ => {
                // the same reference on several credits: only an exact amount settles which
                let exact: Vec<_> = hits.iter().filter(|(r, _)| r.2 == net).collect();
                if exact.len() == 1 {
                    Some(*exact[0])
                } else {
                    None
                }
            }
        };
        if let Some((r, rule)) = chosen {
            let ev = json!({
                "key": key, "rule": rule,
                "line": { "posted_on": r.1, "amount_minor": r.2, "narration": r.3, "reference": r.4 },
                "settlement": { "net_minor": net, "settled_on": settled_on, "utr": utr },
                "amount_equal": r.2 == net, "days_apart": days_apart(r.1, settled_on),
            });
            confirm_match(
                conn,
                tenant_id,
                r.0,
                "settlement",
                settlement_id,
                rule,
                Some(key),
                ev,
                "system",
            )
            .await?;
            return Ok(Some((r.0, r.2)));
        }
        if hits.len() > 1 {
            ambiguous = true;
            for (r, _) in &hits {
                let ev = json!({
                    "key": key, "candidates": hits.len(),
                    "line": { "posted_on": r.1, "amount_minor": r.2, "narration": r.3, "reference": r.4 },
                    "settlement": { "net_minor": net, "settled_on": settled_on, "utr": utr },
                    "note": "the settlement's reference is on more than one credit; confirm the right one",
                });
                suggest(
                    conn,
                    tenant_id,
                    r.0,
                    "settlement",
                    settlement_id,
                    "reference_ambiguous",
                    Some(key),
                    ev,
                )
                .await?;
            }
        }
    }
    if ambiguous {
        return Ok(None);
    }
    // weaker: same amount, near the day. Only a suggestion, whatever the evidence looks like.
    let cands: Vec<LineRow> = sqlx::query_as(asql(format!(
        "SELECT l.id, l.posted_on, l.amount_minor, l.narration, {REF_EXPR} FROM bank_statement_lines l
         WHERE l.amount_minor = $1 AND l.currency = $2 AND {UNMATCHED}
           AND l.posted_on BETWEEN $3::date - {WINDOW_DAYS} AND $3::date + {WINDOW_DAYS}
         ORDER BY abs(l.posted_on - $3::date), l.id LIMIT 5"
    )))
    .bind(net)
    .bind(currency)
    .bind(settled_on)
    .fetch_all(&mut *conn)
    .await?;
    for r in &cands {
        let ev = json!({
            "rule": "amount_date_window",
            "line": { "posted_on": r.1, "amount_minor": r.2, "narration": r.3, "reference": r.4 },
            "settlement": { "net_minor": net, "settled_on": settled_on, "utr": utr },
            "amount_equal": true, "days_apart": days_apart(r.1, settled_on), "window_days": WINDOW_DAYS,
            "competing_candidates": cands.len(),
            "note": "no bank reference matched; the same amount arrived within a few days. A person has to confirm this.",
        });
        suggest(
            conn,
            tenant_id,
            r.0,
            "settlement",
            settlement_id,
            "amount_date_window",
            None,
            ev,
        )
        .await?;
    }
    Ok(None)
}

// ---------- lines: payments, payouts and refunds ----------

type Cand = (String, i64, Option<String>, Option<NaiveDate>);

/// Match the lines nothing yet explains against payments (credits), payouts and refunds (debits).
async fn match_lines(
    conn: &mut PgConnection,
    tenant_id: &str,
    summary: &mut Summary,
) -> Result<(), ApiError> {
    let lines: Vec<LineRow> = sqlx::query_as(asql(format!(
        "SELECT l.id, l.posted_on, l.amount_minor, l.narration, {REF_EXPR} FROM bank_statement_lines l
         WHERE {UNMATCHED} AND l.currency = 'INR' ORDER BY l.posted_on DESC, l.id DESC LIMIT 5000"
    )))
    .fetch_all(&mut *conn)
    .await?;
    for (id, on, amount, narration, reference) in lines {
        let keys = LineKeys::new(reference.as_deref(), &narration);
        let all = keys.all_keys();
        let credit = amount > 0;
        let abs = amount.abs();
        // (subject type, id, amount, key, rule) for the reference matches
        let mut by_key: Vec<(&str, Cand, String, &'static str)> = Vec::new();
        if !all.is_empty() {
            let (st, sql) = if credit {
                (
                    "payment_attempt",
                    "SELECT a.id, a.amount_minor, a.bank_reference, NULL::date FROM payment_attempts a
                     WHERE a.status = 'captured' AND a.bank_reference IS NOT NULL
                       AND upper(regexp_replace(a.bank_reference, '[^A-Za-z0-9]', '', 'g')) = ANY($1)
                       AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'payment_attempt' AND m.subject_id = a.id AND m.status = 'confirmed')",
                )
            } else {
                (
                    "payout",
                    "SELECT p.id, p.amount_minor, p.bank_ref, NULL::date FROM payouts p
                     WHERE p.status = 'paid' AND p.bank_ref IS NOT NULL
                       AND upper(regexp_replace(p.bank_ref, '[^A-Za-z0-9]', '', 'g')) = ANY($1)
                       AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'payout' AND m.subject_id = p.id AND m.status = 'confirmed')",
                )
            };
            let mut found: Vec<(&str, Cand)> = sqlx::query_as::<_, Cand>(sql)
                .bind(&all)
                .fetch_all(&mut *conn)
                .await?
                .into_iter()
                .map(|c| (st, c))
                .collect();
            if !credit {
                let refunds: Vec<Cand> = sqlx::query_as(
                    "SELECT r.id, r.amount_minor, r.provider_refund_ref, NULL::date FROM refunds r
                     WHERE r.status = 'processed' AND r.provider_refund_ref IS NOT NULL
                       AND upper(regexp_replace(r.provider_refund_ref, '[^A-Za-z0-9]', '', 'g')) = ANY($1)
                       AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'refund' AND m.subject_id = r.id AND m.status = 'confirmed')",
                )
                .bind(&all)
                .fetch_all(&mut *conn)
                .await?;
                found.extend(refunds.into_iter().map(|c| ("refund", c)));
            }
            for (st, c) in found {
                let key = c.2.as_deref().map(norm_key).unwrap_or_default();
                if let Some(rule) = keys.hit(&key) {
                    by_key.push((st, c, key, rule));
                }
            }
        }
        let mut handled = false;
        if by_key.len() == 1 {
            let (st, c, key, rule) = &by_key[0];
            let ev = json!({
                "key": key, "rule": rule,
                "line": { "posted_on": on, "amount_minor": amount, "narration": narration, "reference": reference },
                "subject": { "amount_minor": c.1, "reference": c.2 },
                "amount_equal": c.1 == abs,
            });
            if c.1 == abs {
                confirm_match(conn, tenant_id, id, st, &c.0, rule, Some(key), ev, "system").await?;
                summary.confirmed += 1;
            } else {
                // the reference fits but the money does not: a person decides
                let mut ev = ev;
                ev["note"] = json!(format!(
                    "the reference matches but the bank moved {} and we recorded {}",
                    money(abs),
                    money(c.1)
                ));
                if suggest(
                    conn,
                    tenant_id,
                    id,
                    st,
                    &c.0,
                    "reference_amount_differs",
                    Some(key),
                    ev,
                )
                .await?
                {
                    summary.suggested += 1;
                }
            }
            handled = true;
        } else if by_key.len() > 1 {
            for (st, c, key, _) in &by_key {
                let ev = json!({
                    "key": key, "candidates": by_key.len(),
                    "line": { "posted_on": on, "amount_minor": amount, "narration": narration, "reference": reference },
                    "subject": { "amount_minor": c.1, "reference": c.2 },
                    "note": "the line carries more than one reference we know; confirm the right one",
                });
                if suggest(
                    conn,
                    tenant_id,
                    id,
                    st,
                    &c.0,
                    "reference_ambiguous",
                    Some(key),
                    ev,
                )
                .await?
                {
                    summary.suggested += 1;
                }
            }
            handled = true;
        }
        if handled || credit {
            // credits are matched to settlements from the settlement's side (`match_settlement`)
            continue;
        }
        // a debit with no reference: same amount as a payout or refund paid about then is only a suggestion
        for (st, sql) in [
            (
                "payout",
                "SELECT p.id, p.amount_minor, p.bank_ref, (p.updated_at AT TIME ZONE 'Asia/Kolkata')::date FROM payouts p
                 WHERE p.status = 'paid' AND p.amount_minor = $1 AND p.currency = 'INR'
                   AND (p.updated_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $2::date - 3 AND $2::date + 3
                   AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'payout' AND m.subject_id = p.id AND m.status = 'confirmed')
                 ORDER BY p.id LIMIT 5",
            ),
            (
                "refund",
                "SELECT r.id, r.amount_minor, r.provider_refund_ref, (r.updated_at AT TIME ZONE 'Asia/Kolkata')::date FROM refunds r
                 WHERE r.status = 'processed' AND r.amount_minor = $1 AND r.currency = 'INR'
                   AND (r.updated_at AT TIME ZONE 'Asia/Kolkata')::date BETWEEN $2::date - 3 AND $2::date + 3
                   AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'refund' AND m.subject_id = r.id AND m.status = 'confirmed')
                 ORDER BY r.id LIMIT 5",
            ),
        ] {
            let cands: Vec<Cand> = sqlx::query_as(sql).bind(abs).bind(on).fetch_all(&mut *conn).await?;
            let n = cands.len();
            for c in cands {
                let ev = json!({
                    "rule": "amount_date_window",
                    "line": { "posted_on": on, "amount_minor": amount, "narration": narration, "reference": reference },
                    "subject": { "amount_minor": c.1, "reference": c.2, "date": c.3 },
                    "amount_equal": true, "days_apart": c.3.map(|d| days_apart(on, d)), "window_days": WINDOW_DAYS,
                    "competing_candidates": n,
                    "note": "no bank reference matched; the same amount left the account within a few days. A person has to confirm this.",
                });
                if suggest(conn, tenant_id, id, st, &c.0, "amount_date_window", None, ev).await? {
                    summary.suggested += 1;
                }
            }
        }
    }
    Ok(())
}

/// A case for every credit nothing explains. Closes itself when the line is matched.
async fn open_credit_cases(conn: &mut PgConnection, tenant_id: &str) -> Result<usize, ApiError> {
    let orphans: Vec<(i64, i64, NaiveDate, String, i64)> = sqlx::query_as(asql(format!(
        "SELECT l.id, l.amount_minor, l.posted_on, l.narration,
                (SELECT count(*) FROM bank_line_matches m WHERE m.bank_line_id = l.id AND m.status = 'suggested')
         FROM bank_statement_lines l WHERE {UNMATCHED} AND l.amount_minor > 0"
    )))
    .fetch_all(&mut *conn)
    .await?;
    let mut opened = 0;
    for (id, amount, on, narr, suggestions) in orphans {
        let hint = if suggestions > 0 {
            format!(
                " {suggestions} possible match(es) on amount and date are waiting for a person to confirm."
            )
        } else {
            String::new()
        };
        if open_case(
            conn,
            tenant_id,
            "unmatched_bank_credit",
            "bank_line",
            &id.to_string(),
            None,
            Some(amount),
            &format!(
                "{} credited on {on} ({narr}) matches no provider settlement.{hint}",
                money(amount)
            ),
            json!({ "bank_line_id": id }),
        )
        .await?
        {
            opened += 1;
        }
    }
    Ok(opened)
}

/// After lines are stored: settlements get another look at the new lines, then lines are matched, then cases.
pub async fn run(conn: &mut PgConnection, tenant_id: &str) -> Result<Summary, ApiError> {
    let mut summary = Summary::default();
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bank_line_matches WHERE status = 'confirmed' AND subject_type = 'settlement'",
    )
    .fetch_one(&mut *conn)
    .await?;
    let suggested_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM bank_line_matches WHERE status = 'suggested'")
            .fetch_one(&mut *conn)
            .await?;
    let waiting: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM settlements WHERE bank_line_id IS NULL ORDER BY settled_on, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for id in &waiting {
        recon::evaluate(conn, tenant_id, id).await?;
    }
    summary.settlements_rechecked = waiting.len();
    let after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM bank_line_matches WHERE status = 'confirmed' AND subject_type = 'settlement'",
    )
    .fetch_one(&mut *conn)
    .await?;
    summary.confirmed = (after - before).max(0) as usize;
    let mut lines = Summary::default();
    match_lines(conn, tenant_id, &mut lines).await?;
    summary.confirmed += lines.confirmed;
    let suggested_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM bank_line_matches WHERE status = 'suggested'")
            .fetch_one(&mut *conn)
            .await?;
    summary.suggested = (suggested_after - suggested_before).max(0) as usize;
    summary.cases_opened = open_credit_cases(conn, tenant_id).await?;
    Ok(summary)
}

/// Settlements and payouts the bank statement should show by now but does not. Only counts as missing once a
/// statement covers the period (a statement not yet imported is not a missing credit).
pub async fn scan_unbanked(
    conn: &mut PgConnection,
    tenant_id: &str,
    older_than_days: i32,
) -> Result<(usize, usize), ApiError> {
    let latest: Option<NaiveDate> =
        sqlx::query_scalar("SELECT max(posted_on) FROM bank_statement_lines")
            .fetch_one(&mut *conn)
            .await?;
    let Some(latest) = latest else {
        return Ok((0, 0));
    };
    let stl: Vec<(String, String, i64, NaiveDate)> = sqlx::query_as(
        "SELECT id, provider, net_minor, settled_on FROM settlements
         WHERE bank_line_id IS NULL AND net_minor > 0 AND settled_on + $1 <= $2::date",
    )
    .bind(older_than_days)
    .bind(latest)
    .fetch_all(&mut *conn)
    .await?;
    let mut n_stl = 0;
    for (id, provider, net, on) in stl {
        let suggestions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM bank_line_matches WHERE subject_type = 'settlement' AND subject_id = $1 AND status = 'suggested'",
        )
        .bind(&id)
        .fetch_one(&mut *conn)
        .await?;
        let hint = if suggestions > 0 {
            format!(
                " {suggestions} credit(s) match on amount and date and wait for a person to confirm."
            )
        } else {
            String::new()
        };
        if open_case(
            conn,
            tenant_id,
            "settlement_no_bank_credit",
            "settlement",
            &id,
            Some(net),
            Some(0),
            &format!(
                "{} settled {} on {on}, but no bank credit with its reference is on the statements imported up to {latest}.{hint}",
                crate::timeline::provider_title(&provider),
                money(net)
            ),
            json!({ "settlement_id": id, "statement_through": latest }),
        )
        .await?
        {
            n_stl += 1;
        }
    }
    let pay: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT id, amount_minor, COALESCE(bank_ref, '') FROM payouts
         WHERE status = 'paid' AND (updated_at AT TIME ZONE 'Asia/Kolkata')::date + $1 <= $2::date
           AND NOT EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.subject_type = 'payout' AND m.subject_id = payouts.id AND m.status = 'confirmed')",
    )
    .bind(older_than_days)
    .bind(latest)
    .fetch_all(&mut *conn)
    .await?;
    let mut n_pay = 0;
    for (id, amount, bank_ref) in pay {
        if open_case(
            conn,
            tenant_id,
            "payout_no_bank_debit",
            "payout",
            &id,
            Some(amount),
            Some(0),
            &format!("Payout {id} of {} was recorded as paid (reference {bank_ref}) but no bank debit for it is on the statements imported up to {latest}.", money(amount)),
            json!({ "payout_id": id, "statement_through": latest }),
        )
        .await?
        {
            n_pay += 1;
        }
    }
    Ok((n_stl, n_pay))
}

// ---------- the settlement explained ----------

pub async fn bank_match_json(
    conn: &mut PgConnection,
    settlement_id: &str,
    bank_line: Option<i64>,
) -> Result<Option<Value>, ApiError> {
    let Some(line) = bank_line else {
        return Ok(None);
    };
    let row: Option<(i64, NaiveDate, i64, String, Option<String>, Option<i64>, Option<String>)> =
        sqlx::query_as(asql(format!(
            "SELECT l.id, l.posted_on, l.amount_minor, l.narration, {REF_EXPR}, l.import_id, l.fingerprint FROM bank_statement_lines l WHERE l.id = $1"
        )))
        .bind(line)
        .fetch_optional(&mut *conn)
        .await?;
    let Some((id, on, amount, narration, reference, import_id, _)) = row else {
        return Ok(None);
    };
    let m: Option<(i64, String, Option<String>, Value, Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT id, rule, key_matched, evidence, decided_by, decided_at FROM bank_line_matches
         WHERE subject_type = 'settlement' AND subject_id = $1 AND bank_line_id = $2 AND status = 'confirmed'",
    )
    .bind(settlement_id)
    .bind(line)
    .fetch_optional(&mut *conn)
    .await?;
    let (match_id, rule, key, evidence, by, at) = match m {
        Some((i, r, k, e, b, a)) => (Some(i), r, k, e, b, a),
        None => (
            None,
            "recorded_before_evidence".to_string(),
            None,
            json!({}),
            None,
            None,
        ),
    };
    Ok(Some(json!({
        "bank_line_id": id, "posted_on": on, "amount_minor": amount, "narration": narration,
        // `utr` kept for older clients; `reference` is the same value
        "utr": reference, "reference": reference, "import_id": import_id,
        "match_id": match_id, "rule": rule, "key_matched": key, "evidence": evidence,
        "confirmed_by": by, "confirmed_at": at,
        "confirmed_on": if rule == "amount_date_window" { "person" } else { "reference" },
    })))
}

pub async fn suggested_for_settlement(
    conn: &mut PgConnection,
    settlement_id: &str,
) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<(i64, String, Option<String>, Value, i64, NaiveDate, i64, String, Option<String>)> = sqlx::query_as(asql(format!(
        "SELECT m.id, m.rule, m.key_matched, m.evidence, l.id, l.posted_on, l.amount_minor, l.narration, {REF_EXPR}
         FROM bank_line_matches m JOIN bank_statement_lines l ON l.id = m.bank_line_id
         WHERE m.subject_type = 'settlement' AND m.subject_id = $1 AND m.status = 'suggested' ORDER BY m.id"
    )))
    .bind(settlement_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(mid, rule, key, ev, lid, on, amt, narr, r)| {
            json!({ "match_id": mid, "rule": rule, "key_matched": key, "evidence": ev,
                    "line": { "id": lid, "posted_on": on, "amount_minor": amt, "narration": narr, "reference": r },
                    "status": "suggested" })
        })
        .collect())
}

// ---------- endpoints ----------

fn line_select(extra_where: &str) -> String {
    format!(
        "SELECT jsonb_build_object(
            'id', l.id, 'posted_on', l.posted_on, 'value_date', l.value_date, 'amount_minor', l.amount_minor,
            'direction', CASE WHEN l.amount_minor > 0 THEN 'credit' ELSE 'debit' END,
            'currency', l.currency, 'narration', l.narration, 'reference', {REF_EXPR},
            'import_id', l.import_id, 'line_no', l.line_no, 'balance_minor', l.balance_minor,
            'status', CASE WHEN NOT ({UNMATCHED}) THEN 'matched'
                           WHEN EXISTS (SELECT 1 FROM bank_line_matches m WHERE m.bank_line_id = l.id AND m.status = 'suggested') THEN 'suggested'
                           ELSE 'unmatched' END,
            'match', COALESCE(
                (SELECT jsonb_build_object('id', m.id, 'subject_type', m.subject_type, 'subject_id', m.subject_id, 'rule', m.rule,
                                           'key_matched', m.key_matched, 'confirmed_by', m.decided_by)
                   FROM bank_line_matches m WHERE m.bank_line_id = l.id AND m.status = 'confirmed'),
                CASE WHEN l.matched_settlement_id IS NOT NULL THEN jsonb_build_object('subject_type', 'settlement', 'subject_id', l.matched_settlement_id, 'rule', 'recorded_before_evidence') END),
            'suggestions', (SELECT COALESCE(jsonb_agg(jsonb_build_object('id', m.id, 'subject_type', m.subject_type, 'subject_id', m.subject_id,
                                                                         'rule', m.rule, 'evidence', m.evidence) ORDER BY m.id), '[]'::jsonb)
                              FROM bank_line_matches m WHERE m.bank_line_id = l.id AND m.status = 'suggested')
         ) FROM bank_statement_lines l {extra_where}"
    )
}

#[derive(Deserialize)]
pub struct LineQuery {
    /// `unmatched`, `suggested` or `matched`.
    status: Option<String>,
    /// `credit` or `debit`.
    direction: Option<String>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    import_id: Option<i64>,
    limit: Option<i64>,
}

/// `GET /v1/bank_statements/lines`
pub async fn list_lines(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<LineQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    if let Some(st) = &q.status
        && !matches!(st.as_str(), "unmatched" | "suggested" | "matched")
    {
        return Err(ApiError::invalid(
            "invalid_status",
            "status is unmatched, suggested or matched",
        ));
    }
    if let Some(d) = &q.direction
        && !matches!(d.as_str(), "credit" | "debit")
    {
        return Err(ApiError::invalid(
            "invalid_direction",
            "direction is credit or debit",
        ));
    }
    let sql = format!(
        "SELECT x FROM ({}) AS t(x) WHERE ($1::text IS NULL OR x->>'status' = $1)
           AND ($2::text IS NULL OR x->>'direction' = $2)
           AND ($3::date IS NULL OR (x->>'posted_on')::date >= $3) AND ($4::date IS NULL OR (x->>'posted_on')::date <= $4)
           AND ($5::bigint IS NULL OR (x->>'import_id')::bigint = $5)
         ORDER BY (x->>'posted_on') DESC, (x->>'id')::bigint DESC LIMIT $6",
        line_select("")
    );
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(&q.status)
        .bind(&q.direction)
        .bind(q.from)
        .bind(q.to)
        .bind(q.import_id)
        .bind(q.limit.unwrap_or(50).clamp(1, 200))
        .fetch_all(&mut *tx)
        .await?;
    Ok(Json(json!({ "data": rows })))
}

/// `GET /v1/bank_statements/lines/{id}`: one line with every match it has had, including rejected ones.
pub async fn get_line(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut line: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(line_select("WHERE l.id = $1")))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("bank line"))?;
    let matches: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'subject_type', subject_type, 'subject_id', subject_id, 'status', status, 'rule', rule,
                                   'key_matched', key_matched, 'evidence', evidence, 'decided_by', decided_by, 'decided_at', decided_at, 'created_at', created_at)
         FROM bank_line_matches WHERE bank_line_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let cases: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'kind', kind, 'status', status, 'assignee', assignee)
         FROM recon_cases WHERE subject_type = 'bank_line' AND subject_id = $1::text ORDER BY created_at",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    line["matches"] = json!(matches);
    line["cases"] = json!(cases);
    Ok(Json(line))
}

/// `GET /v1/bank_statements/imports`
pub async fn list_imports(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'source', source, 'layout', layout, 'filename', filename, 'file_sha256', file_sha256,
                                   'rows', rows_total, 'added', rows_added, 'duplicates', rows_duplicate,
                                   'credits_minor', credits_minor, 'debits_minor', debits_minor,
                                   'period_from', period_from, 'period_to', period_to, 'created_by', created_by, 'created_at', created_at)
         FROM bank_statement_imports ORDER BY id DESC LIMIT 100",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": rows })))
}

#[derive(Deserialize, Default)]
pub struct Decision {
    /// Why, kept with the decision.
    note: Option<String>,
}

async fn load_suggested(
    conn: &mut PgConnection,
    id: i64,
) -> Result<(i64, String, String, String, Option<String>, Value), ApiError> {
    let row: Option<(i64, String, String, String, String, Option<String>, Value)> = sqlx::query_as(
        "SELECT bank_line_id, subject_type, subject_id, status, rule, key_matched, evidence FROM bank_line_matches WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let (line, st, sid, status, rule, key, ev) = row.ok_or_else(|| ApiError::not_found("match"))?;
    if status != "suggested" {
        return Err(ApiError::conflict(
            "not_suggested",
            format!("this match is {status}, only a suggested match can be confirmed or rejected"),
        ));
    }
    Ok((line, st, sid, rule, key, ev))
}

/// `POST /v1/bank_statements/matches/{id}/confirm`: a person accepts a suggested match.
pub async fn confirm(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    body: Option<Json<Decision>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let note = body.and_then(|b| b.0.note);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (line, st, sid, rule, key, mut ev) = load_suggested(&mut tx, id).await?;
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bank_line_matches WHERE status = 'confirmed' AND bank_line_id = $1)
             OR (SELECT matched_settlement_id IS NOT NULL FROM bank_statement_lines WHERE id = $1)",
    )
    .bind(line)
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(ApiError::conflict(
            "line_already_matched",
            "this bank line is already matched to something else",
        ));
    }
    let subject_taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bank_line_matches WHERE status = 'confirmed' AND subject_type = $1 AND subject_id = $2)
             OR ($1 = 'settlement' AND (SELECT bank_line_id IS NOT NULL FROM settlements WHERE id = $2))",
    )
    .bind(&st)
    .bind(&sid)
    .fetch_one(&mut *tx)
    .await?;
    if subject_taken {
        return Err(ApiError::conflict(
            "subject_already_matched",
            format!("{st} {sid} already has a bank line"),
        ));
    }
    ev["confirmed_by_person"] = json!(auth.actor);
    ev["note"] = json!(note);
    confirm_match(
        &mut tx,
        &auth.tenant_id,
        line,
        &st,
        &sid,
        &rule,
        key.as_deref(),
        ev,
        &auth.actor,
    )
    .await?;
    if st == "settlement" {
        recon::evaluate(&mut tx, &auth.tenant_id, &sid).await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "bank_match.confirmed",
        Some(&id.to_string()),
        json!({ "bank_line_id": line, "subject_type": st, "subject_id": sid, "rule": rule, "note": note }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "status": "confirmed", "bank_line_id": line, "subject_type": st, "subject_id": sid, "rule": rule, "decided_by": auth.actor }),
    ))
}

/// `POST /v1/bank_statements/matches/{id}/reject`: a person turns a suggestion down. It is not suggested again.
pub async fn reject(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    body: Option<Json<Decision>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let note = body.and_then(|b| b.0.note);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (line, st, sid, rule, _, _) = load_suggested(&mut tx, id).await?;
    sqlx::query("UPDATE bank_line_matches SET status = 'rejected', decided_by = $2, decided_at = now(), evidence = evidence || $3 WHERE id = $1")
        .bind(id)
        .bind(&auth.actor)
        .bind(json!({ "rejected_note": note }))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "bank_match.rejected",
        Some(&id.to_string()),
        json!({ "bank_line_id": line, "subject_type": st, "subject_id": sid, "rule": rule, "note": note }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "rejected" })))
}

/// `POST /v1/bank_statements/rematch`: look again at everything unmatched (after a settlement report arrived, say).
pub async fn rematch(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sum = run(&mut tx, &auth.tenant_id).await?;
    tx.commit().await?;
    Ok(Json(json!({
        "settlements_rechecked": sum.settlements_rechecked,
        "matched": { "confirmed": sum.confirmed, "suggested": sum.suggested },
        "cases_opened": sum.cases_opened,
    })))
}

/// `GET /v1/recon/ageing`: open cases by how long they have waited, and who has them.
pub async fn ageing(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, i64, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT kind,
                CASE WHEN age <= 1 THEN '0-1' WHEN age <= 3 THEN '2-3' WHEN age <= 7 THEN '4-7' WHEN age <= 14 THEN '8-14' ELSE '15+' END,
                count(*)::BIGINT, count(*) FILTER (WHERE assignee IS NULL)::BIGINT, max(age)::BIGINT, sum(abs(COALESCE(actual_minor, expected_minor, 0)))::BIGINT
         FROM (SELECT kind, assignee, actual_minor, expected_minor,
                      GREATEST(0, EXTRACT(day FROM (now() - created_at)))::BIGINT AS age
               FROM recon_cases WHERE status <> 'resolved') c
         GROUP BY 1, 2 ORDER BY 1, 2",
    )
    .fetch_all(&mut *tx)
    .await?;
    let buckets = ["0-1", "2-3", "4-7", "8-14", "15+"];
    let mut by_kind: std::collections::BTreeMap<String, Value> = Default::default();
    let (mut total, mut unassigned, mut oldest) = (0i64, 0i64, 0i64);
    for (kind, bucket, n, un, old, amount) in rows {
        let e = by_kind
            .entry(kind)
            .or_insert_with(|| json!({ "buckets": {}, "open": 0, "unassigned": 0, "oldest_days": 0, "amount_minor": 0 }));
        e["buckets"][&bucket] = json!(n);
        e["open"] = json!(e["open"].as_i64().unwrap_or(0) + n);
        e["unassigned"] = json!(e["unassigned"].as_i64().unwrap_or(0) + un);
        e["oldest_days"] = json!(e["oldest_days"].as_i64().unwrap_or(0).max(old));
        e["amount_minor"] = json!(e["amount_minor"].as_i64().unwrap_or(0) + amount.unwrap_or(0));
        total += n;
        unassigned += un;
        oldest = oldest.max(old);
    }
    Ok(Json(json!({
        "bucket_days": buckets, "open": total, "unassigned": unassigned, "oldest_days": oldest,
        "by_kind": by_kind,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_whole_tokens_not_substrings() {
        let k = LineKeys::new(None, "NEFT-HDFCN52026110112345678-ACME LTD-SETTLEMENT");
        assert_eq!(
            k.hit("HDFCN52026110112345678"),
            Some("reference_in_narration")
        );
        assert_eq!(
            k.hit("HDFCN520261101"),
            None,
            "a prefix of a reference is not the reference"
        );
        assert_eq!(k.hit("ACME"), None, "too short, no digit");
        // a separator inside the reference does not matter
        let k = LineKeys::new(None, "NEFT UTRSETL_00000000000001 RZP");
        assert_eq!(
            k.hit("UTRSETL00000000000001"),
            Some("reference_in_narration")
        );
    }

    #[test]
    fn a_label_glued_to_the_reference_is_peeled_off() {
        let k = LineKeys::new(None, "IMPS UTR998877 from RAZORPAY");
        assert_eq!(k.hit("998877"), Some("reference_in_narration"));
        let k = LineKeys::new(Some("'UTR998877"), "x");
        assert_eq!(k.hit("UTR998877"), Some("reference_column"));
        let k = LineKeys::new(Some("UPI/404223241811/ok"), "x");
        assert_eq!(k.hit("404223241811"), Some("reference_column"));
    }

    #[test]
    fn punctuation_and_case_do_not_matter() {
        assert_eq!(norm_key("hdfcn5-2026/11 99"), "HDFCN520261199");
        assert!(usable_key("SIMUTR20261101"));
        assert!(!usable_key("12345"));
        assert!(!usable_key("SETTLEMENT"));
    }
}
