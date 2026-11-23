// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `POST /v1/recon/ledger_check`: compare a (simulated) provider settlement file with what the ledger posted.
//!
//! The caller sends the file's lines (a payment or refund, the provider's reference, an integer amount in minor
//! units and a currency) plus the window of ledger postings to look at. Every line and every posting lands in
//! exactly one category, with exact integer comparison (no tolerance, no rounding):
//!
//! * `matched`: same kind, reference, amount and currency, once on each side;
//! * `amount_mismatch` / `currency_mismatch`: the reference is on both sides but the figures differ;
//! * `missing_in_ledger`: the file has it, no capture or refund was posted for that reference;
//! * `missing_in_file`: the ledger posted it inside the window, the file does not list it;
//! * `duplicate_in_file` / `duplicate_in_ledger`: the same kind and reference more than once (never matched, so a
//!   double payment cannot hide behind a single posting);
//! * `invalid_line`: not a whole positive amount, an unknown kind or currency, an empty reference.
//!
//! Strictly read-only: nothing is stored, no case is opened, no ledger row is touched. The classification is a pure
//! function ([`classify`]); the handler only loads the postings. A simulated file only: no bank or provider is called.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

/// Lines accepted in one request (a bigger file is refused whole, not truncated).
pub const MAX_LINES: usize = 5_000;
/// Longest window of ledger postings one request may look at.
pub const MAX_WINDOW_DAYS: i64 = 31;
/// Entries listed per category; the counts always cover everything.
pub const MAX_LISTED: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Payment,
    Refund,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Payment => "payment",
            Kind::Refund => "refund",
        }
    }
}

/// One line of the file, as sent: the fields stay loose so a malformed one is reported, not fatal.
#[derive(Debug, Clone, Deserialize)]
pub struct FileLine {
    pub kind: Value,
    pub provider_ref: Value,
    pub amount_minor: Value,
    pub currency: Option<Value>,
}

/// What the ledger posted for one capture or refund.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub kind: Kind,
    pub provider_ref: String,
    pub amount_minor: i64,
    pub currency: String,
    /// The ledger transaction reference (`capture:<attempt>` or `refund:<refund>`).
    pub posting_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    index: usize,
    kind: Kind,
    provider_ref: String,
    amount_minor: i64,
    currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub matched: Vec<Value>,
    pub amount_mismatch: Vec<Value>,
    pub currency_mismatch: Vec<Value>,
    pub missing_in_ledger: Vec<Value>,
    pub missing_in_file: Vec<Value>,
    pub duplicate_in_file: Vec<Value>,
    pub duplicate_in_ledger: Vec<Value>,
    pub invalid_line: Vec<Value>,
    /// (currency, file payments, file refunds, ledger payments, ledger refunds), all minor units, i128 sums.
    pub totals: BTreeMap<String, [i128; 4]>,
}

impl Outcome {
    /// Everything matched exactly: nothing in any other category.
    pub fn ok(&self) -> bool {
        self.amount_mismatch.is_empty()
            && self.currency_mismatch.is_empty()
            && self.missing_in_ledger.is_empty()
            && self.missing_in_file.is_empty()
            && self.duplicate_in_file.is_empty()
            && self.duplicate_in_ledger.is_empty()
            && self.invalid_line.is_empty()
    }
}

fn parse(index: usize, l: &FileLine) -> Result<Line, String> {
    let kind = match l.kind.as_str() {
        Some("payment") => Kind::Payment,
        Some("refund") => Kind::Refund,
        _ => return Err("kind must be \"payment\" or \"refund\"".into()),
    };
    let provider_ref = match l.provider_ref.as_str() {
        Some(r) if !r.trim().is_empty() && r.len() <= 200 => r.to_string(),
        _ => {
            return Err("provider_ref must be a non-empty string of at most 200 characters".into());
        }
    };
    // a JSON number that is not a whole i64 (1.5, 1e3, a string, null) is not an amount in minor units
    let amount_minor = match l.amount_minor.as_i64() {
        Some(a) if a > 0 => a,
        _ => return Err("amount_minor must be a whole positive number of minor units".into()),
    };
    let currency = match l.currency.as_ref().map(|c| c.as_str()) {
        None => "INR".to_string(),
        Some(Some(c)) if c.parse::<zyvor_core::Currency>().is_ok() => c.to_string(),
        _ => return Err("currency must be INR or USD".into()),
    };
    Ok(Line {
        index,
        kind,
        provider_ref,
        amount_minor,
        currency,
    })
}

/// Pure classification of `file` against `ledger`. Total: no input panics or overflows.
pub fn classify(file: &[FileLine], ledger: &[Posting]) -> Outcome {
    let mut o = Outcome::default();
    let mut lines: BTreeMap<(Kind, String), Vec<Line>> = BTreeMap::new();
    for (i, raw) in file.iter().enumerate() {
        match parse(i, raw) {
            Ok(l) => lines
                .entry((l.kind, l.provider_ref.clone()))
                .or_default()
                .push(l),
            Err(why) => o.invalid_line.push(json!({ "line": i, "reason": why })),
        }
    }
    let mut posted: BTreeMap<(Kind, String), Vec<&Posting>> = BTreeMap::new();
    for p in ledger {
        posted
            .entry((p.kind, p.provider_ref.clone()))
            .or_default()
            .push(p);
    }
    let slot =
        |k: Kind, ledger_side: bool| usize::from(ledger_side) * 2 + usize::from(k == Kind::Refund);

    for ((kind, r), ls) in &lines {
        for l in ls {
            o.totals.entry(l.currency.clone()).or_default()[slot(*kind, false)] +=
                i128::from(l.amount_minor);
        }
        let ps = posted.get(&(*kind, r.clone()));
        if ls.len() > 1 {
            o.duplicate_in_file.push(json!({
                "kind": kind.as_str(), "provider_ref": r, "lines": ls.iter().map(|l| l.index).collect::<Vec<_>>(),
            }));
            continue;
        }
        let l = &ls[0];
        match ps {
            None => o.missing_in_ledger.push(json!({
                "line": l.index, "kind": kind.as_str(), "provider_ref": r,
                "amount_minor": l.amount_minor, "currency": l.currency,
            })),
            Some(ps) if ps.len() > 1 => o.duplicate_in_ledger.push(json!({
                "kind": kind.as_str(), "provider_ref": r,
                "postings": ps.iter().map(|p| p.posting_ref.clone()).collect::<Vec<_>>(),
            })),
            Some(ps) => {
                let p = ps[0];
                if p.currency != l.currency {
                    o.currency_mismatch.push(json!({
                        "line": l.index, "kind": kind.as_str(), "provider_ref": r, "posting": p.posting_ref,
                        "file_currency": l.currency, "ledger_currency": p.currency,
                    }));
                } else if p.amount_minor != l.amount_minor {
                    o.amount_mismatch.push(json!({
                        "line": l.index, "kind": kind.as_str(), "provider_ref": r, "posting": p.posting_ref,
                        "currency": l.currency, "file_minor": l.amount_minor, "ledger_minor": p.amount_minor,
                        "difference_minor": i128::from(l.amount_minor) - i128::from(p.amount_minor),
                    }));
                } else {
                    o.matched.push(json!({
                        "line": l.index, "kind": kind.as_str(), "provider_ref": r, "posting": p.posting_ref,
                        "amount_minor": l.amount_minor, "currency": l.currency,
                    }));
                }
            }
        }
    }
    for ((kind, r), ps) in &posted {
        for p in ps {
            o.totals.entry(p.currency.clone()).or_default()[slot(*kind, true)] +=
                i128::from(p.amount_minor);
        }
        if lines.contains_key(&(*kind, r.clone())) {
            continue;
        }
        if ps.len() > 1 {
            o.duplicate_in_ledger.push(json!({
                "kind": kind.as_str(), "provider_ref": r,
                "postings": ps.iter().map(|p| p.posting_ref.clone()).collect::<Vec<_>>(),
            }));
        } else {
            o.missing_in_file.push(json!({
                "kind": kind.as_str(), "provider_ref": r, "posting": ps[0].posting_ref,
                "amount_minor": ps[0].amount_minor, "currency": ps[0].currency,
            }));
        }
    }
    o
}

pub fn outcome_json(o: &Outcome) -> Value {
    let cat = |v: &Vec<Value>| json!({ "count": v.len(), "items": v.iter().take(MAX_LISTED).collect::<Vec<_>>(), "truncated": v.len() > MAX_LISTED });
    json!({
        "ok": o.ok(),
        "matched": { "count": o.matched.len() },
        "amount_mismatch": cat(&o.amount_mismatch),
        "currency_mismatch": cat(&o.currency_mismatch),
        "missing_in_ledger": cat(&o.missing_in_ledger),
        "missing_in_file": cat(&o.missing_in_file),
        "duplicate_in_file": cat(&o.duplicate_in_file),
        "duplicate_in_ledger": cat(&o.duplicate_in_ledger),
        "invalid_line": cat(&o.invalid_line),
        "totals": o.totals.iter().map(|(c, t)| json!({
            "currency": c,
            "file_payments_minor": t[0].to_string(), "file_refunds_minor": t[1].to_string(),
            "ledger_payments_minor": t[2].to_string(), "ledger_refunds_minor": t[3].to_string(),
        })).collect::<Vec<_>>(),
    })
}

#[derive(Deserialize)]
pub struct Check {
    /// Ledger postings made in `[from, to)` are expected in the file.
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    lines: Vec<FileLine>,
}

pub async fn check(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Check>,
) -> Result<Json<Value>, ApiError> {
    auth.require("recon:read")?;
    if b.lines.len() > MAX_LINES {
        return Err(ApiError::invalid(
            "too_many_lines",
            format!("at most {MAX_LINES} lines per request"),
        ));
    }
    if b.to <= b.from || (b.to - b.from) > chrono::Duration::days(MAX_WINDOW_DAYS) {
        return Err(ApiError::invalid(
            "invalid_window",
            format!("`to` must be after `from`, at most {MAX_WINDOW_DAYS} days apart"),
        ));
    }
    let refs: Vec<String> = b
        .lines
        .iter()
        .filter_map(|l| l.provider_ref.as_str().map(str::to_string))
        .collect();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // postings in the window, plus the postings of every reference the file names (even outside the window)
    let rows: Vec<(String, String, i64, String, String)> = sqlx::query_as(
        "SELECT payment, a.provider_payment_ref, COALESCE(SUM(e.amount_minor) FILTER (WHERE e.side = debit), 0)::BIGINT, e.currency, t.reference
         FROM ledger_transactions t
         JOIN ledger_entries e ON e.transaction_id = t.id
         JOIN payment_attempts a ON t.reference = 'capture:' || a.id
         WHERE t.tenant_id = $1 AND a.provider_payment_ref IS NOT NULL
           AND ((t.created_at >= $2 AND t.created_at < $3) OR a.provider_payment_ref = ANY($4))
         GROUP BY a.provider_payment_ref, e.currency, t.reference
         UNION ALL
         SELECT refund, r.provider_refund_ref, COALESCE(SUM(e.amount_minor) FILTER (WHERE e.side = debit), 0)::BIGINT, e.currency, t.reference
         FROM ledger_transactions t
         JOIN ledger_entries e ON e.transaction_id = t.id
         JOIN refunds r ON t.reference = 'refund:' || r.id
         WHERE t.tenant_id = $1 AND r.provider_refund_ref IS NOT NULL
           AND ((t.created_at >= $2 AND t.created_at < $3) OR r.provider_refund_ref = ANY($4))
         GROUP BY r.provider_refund_ref, e.currency, t.reference",
    )
    .bind(&auth.tenant_id)
    .bind(b.from)
    .bind(b.to)
    .bind(&refs)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    let postings: Vec<Posting> = rows
        .into_iter()
        .map(
            |(kind, provider_ref, amount_minor, currency, posting_ref)| Posting {
                kind: if kind == "refund" {
                    Kind::Refund
                } else {
                    Kind::Payment
                },
                provider_ref,
                amount_minor,
                currency,
                posting_ref,
            },
        )
        .collect();
    Ok(Json(outcome_json(&classify(&b.lines, &postings))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn line(kind: &str, r: &str, amt: Value, ccy: Option<&str>) -> FileLine {
        FileLine {
            kind: json!(kind),
            provider_ref: json!(r),
            amount_minor: amt,
            currency: ccy.map(|c| json!(c)),
        }
    }
    fn post(kind: Kind, r: &str, amt: i64, ccy: &str) -> Posting {
        Posting {
            kind,
            provider_ref: r.into(),
            amount_minor: amt,
            currency: ccy.into(),
            posting_ref: format!(
                "{}:{r}",
                if kind == Kind::Payment {
                    "capture"
                } else {
                    "refund"
                }
            ),
        }
    }

    #[test]
    fn every_category() {
        let file = vec![
            line("payment", "p1", json!(500), None),        // matched
            line("payment", "p2", json!(700), None),        // amount mismatch (ledger 600)
            line("payment", "p3", json!(100), Some("USD")), // currency mismatch (ledger INR)
            line("payment", "p4", json!(100), None),        // missing in ledger
            line("payment", "p5", json!(100), None),        // duplicate in file
            line("payment", "p5", json!(100), None),
            line("refund", "r1", json!(50), None), // matched refund
            line("payment", "p6", json!(10), None), // duplicate in ledger
            line("payment", "bad", json!(1.5), None), // invalid
            line("payment", "bad2", json!(-3), None),
            line("payment", "bad3", json!("12"), None),
            line("payment", " ", json!(3), None),
            line("chargeback", "x", json!(3), None),
            line("payment", "bad4", json!(3), Some("EUR")),
        ];
        let ledger = vec![
            post(Kind::Payment, "p1", 500, "INR"),
            post(Kind::Payment, "p2", 600, "INR"),
            post(Kind::Payment, "p3", 100, "INR"),
            post(Kind::Payment, "p5", 100, "INR"),
            post(Kind::Refund, "r1", 50, "INR"),
            post(Kind::Payment, "p6", 10, "INR"),
            post(Kind::Payment, "p6", 10, "INR"),
            post(Kind::Payment, "p7", 9, "INR"), // missing in file
            post(Kind::Refund, "p1", 1, "INR"), // a refund with the payment's reference is a different key: missing in file
        ];
        let o = classify(&file, &ledger);
        assert_eq!(o.matched.len(), 2);
        assert_eq!(o.amount_mismatch.len(), 1);
        assert_eq!(o.amount_mismatch[0]["difference_minor"], 100);
        assert_eq!(o.currency_mismatch.len(), 1);
        assert_eq!(o.missing_in_ledger.len(), 1);
        assert_eq!(o.duplicate_in_file.len(), 1);
        assert_eq!(o.duplicate_in_ledger.len(), 1);
        assert_eq!(o.missing_in_file.len(), 2);
        assert_eq!(o.invalid_line.len(), 6);
        assert!(!o.ok());
    }

    #[test]
    fn a_double_payment_cannot_hide_behind_one_posting() {
        let file = vec![
            line("payment", "p", json!(100), None),
            line("payment", "p", json!(100), None),
        ];
        let o = classify(&file, &[post(Kind::Payment, "p", 100, "INR")]);
        assert_eq!(o.matched.len(), 0);
        assert_eq!(o.duplicate_in_file.len(), 1);
        assert!(!o.ok());
    }

    #[test]
    fn exact_integer_comparison_with_no_tolerance() {
        let o = classify(
            &[line("payment", "p", json!(100_00), None)],
            &[post(Kind::Payment, "p", 100_00 - 1, "INR")],
        );
        assert_eq!(o.amount_mismatch.len(), 1);
        // the largest amounts neither overflow nor lose precision
        let o = classify(
            &[
                line("payment", "p", json!(i64::MAX), None),
                line("payment", "q", json!(i64::MAX), None),
            ],
            &[
                post(Kind::Payment, "p", i64::MAX, "INR"),
                post(Kind::Payment, "q", i64::MAX, "INR"),
            ],
        );
        assert!(o.ok());
        assert_eq!(o.totals["INR"][0], 2 * i128::from(i64::MAX));
        assert_eq!(
            outcome_json(&o)["totals"][0]["file_payments_minor"],
            (2 * i128::from(i64::MAX)).to_string()
        );
    }

    #[test]
    fn json_output_counts_everything_and_caps_the_lists() {
        let file: Vec<FileLine> = (0..300)
            .map(|i| line("payment", &format!("p{i}"), json!(1), None))
            .collect();
        let j = outcome_json(&classify(&file, &[]));
        assert_eq!(j["ok"], false);
        assert_eq!(j["missing_in_ledger"]["count"], 300);
        assert_eq!(
            j["missing_in_ledger"]["items"].as_array().unwrap().len(),
            MAX_LISTED
        );
        assert_eq!(j["missing_in_ledger"]["truncated"], true);
    }

    proptest! {
        /// A file generated from the ledger itself always reconciles; every line and posting is accounted for exactly once.
        #[test]
        fn a_file_made_from_the_ledger_matches_it(
            items in proptest::collection::btree_map("[a-z]{1,6}", (1i64..i64::MAX, any::<bool>(), any::<bool>()), 0..40)
        ) {
            let mut file = vec![];
            let mut ledger = vec![];
            for (r, (amt, refund, usd)) in &items {
                let kind = if *refund { Kind::Refund } else { Kind::Payment };
                let ccy = if *usd { "USD" } else { "INR" };
                file.push(line(kind.as_str(), r, json!(amt), Some(ccy)));
                ledger.push(post(kind, r, *amt, ccy));
            }
            let o = classify(&file, &ledger);
            prop_assert!(o.ok());
            prop_assert_eq!(o.matched.len(), items.len());
        }

        /// Whatever the file says, the totals of the ledger side are the sum of the postings and nothing panics.
        #[test]
        fn arbitrary_input_is_total_and_conserves_counts(
            amts in proptest::collection::vec((0usize..5, any::<i64>(), 0usize..3), 0..40),
            posts in proptest::collection::vec((0usize..5, 1i64..1_000_000, any::<bool>()), 0..40),
        ) {
            let kinds = ["payment", "refund", "x"];
            let file: Vec<FileLine> = amts.iter().map(|(r, a, k)| line(kinds[*k], &format!("r{r}"), json!(a), None)).collect();
            let ledger: Vec<Posting> = posts.iter().map(|(r, a, rf)| post(if *rf { Kind::Refund } else { Kind::Payment }, &format!("r{r}"), *a, "INR")).collect();
            let o = classify(&file, &ledger);
            let ledger_sum: i128 = ledger.iter().map(|p| i128::from(p.amount_minor)).sum();
            let got: i128 = o.totals.get("INR").map(|t| t[2] + t[3]).unwrap_or(0);
            prop_assert_eq!(got, ledger_sum);
            // file side: every line is either invalid or counted in the file totals (valid amounts only)
            let file_sum: i128 = file.iter().filter_map(|l| match (l.kind.as_str(), l.amount_minor.as_i64()) {
                (Some("payment" | "refund"), Some(a)) if a > 0 => Some(i128::from(a)),
                _ => None,
            }).sum();
            let got_file: i128 = o.totals.get("INR").map(|t| t[0] + t[1]).unwrap_or(0);
            prop_assert_eq!(got_file, file_sum);
        }
    }
}
