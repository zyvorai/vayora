// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Read-only ledger invariant checker.
//!
//! [`check`] takes the rows exactly as stored (text side, text currency, `i64` amount) and reports every
//! way they break the ledger rules, without modifying anything and without trusting the write path:
//!
//! * every transaction balances in every currency (debits = credits), and has at least two entries;
//! * amounts are positive, sides are `debit`/`credit`, currencies are supported;
//! * every entry belongs to a known transaction and every transaction has entries;
//! * a transaction that says it `reverses` another really mirrors it, exists, is not itself and is not a
//!   second reversal of the same transaction;
//! * the trial balance (all debits against all credits) is equal per currency;
//! * (warning only) an account whose normal balance is known does not sit on the wrong side.
//!
//! Sums use `i128`, so no stored data can overflow the checker itself. The function is pure: the caller
//! loads the rows (see the API crate) and this decides.

use std::collections::{BTreeMap, BTreeSet};

use zyvor_core::Currency;

/// A stored ledger transaction header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxnRow {
    pub id: String,
    pub reference: String,
    pub reverses: Option<String>,
}

/// A stored ledger entry, with side and currency as raw text so bad values are reported, not hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    pub transaction_id: String,
    pub account: String,
    pub side: String,
    pub amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// A rule the ledger guarantees is broken.
    Error,
    /// Not a ledger rule, but a balance direction that normally means a business-level mistake.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Unbalanced,
    TooFewEntries,
    NoEntries,
    NonPositiveAmount,
    InvalidSide,
    UnsupportedCurrency,
    OrphanEntry,
    DuplicateReference,
    ReversalTargetMissing,
    ReversalOfSelf,
    ReversalNotMirror,
    DuplicateReversal,
    TrialBalanceMismatch,
    UnexpectedBalanceDirection,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Unbalanced => "unbalanced",
            Kind::TooFewEntries => "too_few_entries",
            Kind::NoEntries => "no_entries",
            Kind::NonPositiveAmount => "non_positive_amount",
            Kind::InvalidSide => "invalid_side",
            Kind::UnsupportedCurrency => "unsupported_currency",
            Kind::OrphanEntry => "orphan_entry",
            Kind::DuplicateReference => "duplicate_reference",
            Kind::ReversalTargetMissing => "reversal_target_missing",
            Kind::ReversalOfSelf => "reversal_of_self",
            Kind::ReversalNotMirror => "reversal_not_mirror",
            Kind::DuplicateReversal => "duplicate_reversal",
            Kind::TrialBalanceMismatch => "trial_balance_mismatch",
            Kind::UnexpectedBalanceDirection => "unexpected_balance_direction",
        }
    }
    pub fn severity(self) -> Severity {
        match self {
            Kind::UnexpectedBalanceDirection => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub kind: Kind,
    /// The transaction (or, for account-level findings, the account) the finding is about.
    pub subject: String,
    pub detail: String,
}

/// Totals for one currency across the whole ledger, in minor units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyTotals {
    pub currency: String,
    pub debits: i128,
    pub credits: i128,
}

impl CurrencyTotals {
    pub fn balanced(&self) -> bool {
        self.debits == self.credits
    }
}

/// Net balance of one account in one currency, signed in the account's own direction is left to the reader:
/// `debits - credits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountBalance {
    pub account: String,
    pub currency: String,
    pub debits: i128,
    pub credits: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub transactions_checked: usize,
    pub entries_checked: usize,
    /// Every violation, sorted by kind then subject (deterministic output).
    pub violations: Vec<Violation>,
    /// One row per currency seen, sorted by currency code.
    pub trial_balance: Vec<CurrencyTotals>,
    pub accounts: Vec<AccountBalance>,
}

impl Report {
    /// No error-severity violation. Warnings do not make a ledger unsound.
    pub fn ok(&self) -> bool {
        !self
            .violations
            .iter()
            .any(|v| v.kind.severity() == Severity::Error)
    }
}

/// Which side an account normally carries, from its name, where the ledger has a rule for it.
/// `Some(true)` = credit-normal (balance is credits - debits), `Some(false)` = debit-normal.
pub fn normal_credit(account: &str) -> Option<bool> {
    if account.starts_with("merchant_payable:") || account.starts_with("vendor_payable:") {
        Some(true)
    } else if account == crate::accounts::ESCROW_AVAILABLE
        || account == crate::accounts::ESCROW_IN_FLIGHT
    {
        Some(false)
    } else {
        None
    }
}

/// Check `txns` and `entries` (all of one tenant). Pure and total: it never panics on any input.
pub fn check(txns: &[TxnRow], entries: &[EntryRow]) -> Report {
    let mut v: Vec<Violation> = Vec::new();
    let mut push = |kind: Kind, subject: &str, detail: String| {
        v.push(Violation {
            kind,
            subject: subject.to_string(),
            detail,
        })
    };

    let known: BTreeSet<&str> = txns.iter().map(|t| t.id.as_str()).collect();
    let mut by_txn: BTreeMap<&str, Vec<&EntryRow>> = BTreeMap::new();
    for e in entries {
        by_txn.entry(e.transaction_id.as_str()).or_default().push(e);
    }

    // references are unique
    let mut seen_ref: BTreeMap<&str, &str> = BTreeMap::new();
    for t in txns {
        if let Some(first) = seen_ref.insert(t.reference.as_str(), t.id.as_str()) {
            push(
                Kind::DuplicateReference,
                &t.id,
                format!("reference {} is also used by {first}", t.reference),
            );
        }
    }

    // entries without a transaction
    for (id, es) in &by_txn {
        if !known.contains(id) {
            push(
                Kind::OrphanEntry,
                id,
                format!(
                    "{} entries point at a transaction that does not exist",
                    es.len()
                ),
            );
        }
    }

    let mut totals: BTreeMap<String, (i128, i128)> = BTreeMap::new();
    let mut accounts: BTreeMap<(String, String), (i128, i128)> = BTreeMap::new();

    for t in txns {
        let Some(es) = by_txn.get(t.id.as_str()) else {
            push(
                Kind::NoEntries,
                &t.id,
                "transaction has no entries".to_string(),
            );
            continue;
        };
        if es.len() < 2 {
            push(
                Kind::TooFewEntries,
                &t.id,
                format!("transaction has {} entry", es.len()),
            );
        }
        let mut per_ccy: BTreeMap<&str, (i128, i128)> = BTreeMap::new();
        for e in es {
            if e.amount_minor <= 0 {
                push(
                    Kind::NonPositiveAmount,
                    &t.id,
                    format!("{} {} on {}", e.amount_minor, e.currency, e.account),
                );
            }
            if e.currency.parse::<Currency>().is_err() {
                push(
                    Kind::UnsupportedCurrency,
                    &t.id,
                    format!("currency {:?} on {}", e.currency, e.account),
                );
            }
            let amt = i128::from(e.amount_minor);
            let slot = per_ccy.entry(e.currency.as_str()).or_default();
            let acct = accounts
                .entry((e.account.clone(), e.currency.clone()))
                .or_default();
            let tot = totals.entry(e.currency.clone()).or_default();
            match e.side.as_str() {
                "debit" => {
                    slot.0 += amt;
                    acct.0 += amt;
                    tot.0 += amt;
                }
                "credit" => {
                    slot.1 += amt;
                    acct.1 += amt;
                    tot.1 += amt;
                }
                other => push(
                    Kind::InvalidSide,
                    &t.id,
                    format!("side {other:?} on {}", e.account),
                ),
            }
        }
        for (ccy, (d, c)) in &per_ccy {
            if d != c {
                push(
                    Kind::Unbalanced,
                    &t.id,
                    format!("{ccy}: debits {d} != credits {c}"),
                );
            }
        }
    }

    // reversals
    let by_id: BTreeMap<&str, &TxnRow> = txns.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut reversed_by: BTreeMap<&str, &str> = BTreeMap::new();
    for t in txns {
        let Some(target) = t.reverses.as_deref() else {
            continue;
        };
        if target == t.id {
            push(
                Kind::ReversalOfSelf,
                &t.id,
                "transaction reverses itself".to_string(),
            );
            continue;
        }
        if !by_id.contains_key(target) {
            push(
                Kind::ReversalTargetMissing,
                &t.id,
                format!("reverses {target}, which does not exist"),
            );
            continue;
        }
        if let Some(first) = reversed_by.insert(target, t.id.as_str()) {
            push(
                Kind::DuplicateReversal,
                &t.id,
                format!("{target} is already reversed by {first}"),
            );
        }
        let lines = |id: &str, flip: bool| -> Vec<(String, String, i64, String)> {
            let mut l: Vec<_> = by_txn
                .get(id)
                .map(|es| {
                    es.iter()
                        .map(|e| {
                            let side = match (e.side.as_str(), flip) {
                                ("debit", true) => "credit",
                                ("credit", true) => "debit",
                                (s, _) => s,
                            };
                            (
                                e.account.clone(),
                                side.to_string(),
                                e.amount_minor,
                                e.currency.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            l.sort();
            l
        };
        if lines(&t.id, false) != lines(target, true) {
            push(
                Kind::ReversalNotMirror,
                &t.id,
                format!("lines are not the opposite of {target}"),
            );
        }
    }

    let trial_balance: Vec<CurrencyTotals> = totals
        .into_iter()
        .map(|(currency, (debits, credits))| CurrencyTotals {
            currency,
            debits,
            credits,
        })
        .collect();
    for t in &trial_balance {
        if !t.balanced() {
            push(
                Kind::TrialBalanceMismatch,
                &t.currency,
                format!("debits {} != credits {}", t.debits, t.credits),
            );
        }
    }

    let account_rows: Vec<AccountBalance> = accounts
        .into_iter()
        .map(|((account, currency), (debits, credits))| AccountBalance {
            account,
            currency,
            debits,
            credits,
        })
        .collect();
    for a in &account_rows {
        if let Some(credit_normal) = normal_credit(&a.account) {
            let wrong = if credit_normal {
                a.debits > a.credits
            } else {
                a.credits > a.debits
            };
            if wrong {
                push(
                    Kind::UnexpectedBalanceDirection,
                    &a.account,
                    format!(
                        "{} {}: debits {} credits {} on a {}-normal account",
                        a.account,
                        a.currency,
                        a.debits,
                        a.credits,
                        if credit_normal { "credit" } else { "debit" }
                    ),
                );
            }
        }
    }

    v.sort_by(|a, b| (a.kind, &a.subject, &a.detail).cmp(&(b.kind, &b.subject, &b.detail)));
    Report {
        transactions_checked: txns.len(),
        entries_checked: entries.len(),
        violations: v,
        trial_balance,
        accounts: account_rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn t(id: &str) -> TxnRow {
        TxnRow {
            id: id.into(),
            reference: format!("ref:{id}"),
            reverses: None,
        }
    }
    fn e(tx: &str, account: &str, side: &str, amt: i64, ccy: &str) -> EntryRow {
        EntryRow {
            transaction_id: tx.into(),
            account: account.into(),
            side: side.into(),
            amount_minor: amt,
            currency: ccy.into(),
        }
    }
    fn kinds(r: &Report) -> Vec<Kind> {
        r.violations.iter().map(|v| v.kind).collect()
    }

    #[test]
    fn a_clean_ledger_is_ok_and_trial_balance_is_equal() {
        let r = check(
            &[t("a"), t("b")],
            &[
                e("a", "provider_receivable:rz", "debit", 500, "INR"),
                e("a", "collections", "credit", 500, "INR"),
                e("b", "collections", "debit", 200, "INR"),
                e("b", "provider_receivable:rz", "credit", 200, "INR"),
            ],
        );
        assert!(r.ok(), "{:?}", r.violations);
        assert!(r.violations.is_empty());
        assert_eq!(r.trial_balance.len(), 1);
        assert!(r.trial_balance[0].balanced());
        assert_eq!((r.transactions_checked, r.entries_checked), (2, 4));
    }

    #[test]
    fn same_total_in_two_currencies_is_not_balanced() {
        let r = check(
            &[t("a")],
            &[
                e("a", "x", "debit", 100, "INR"),
                e("a", "y", "credit", 100, "USD"),
            ],
        );
        let k = kinds(&r);
        // unbalanced in both currencies and in the trial balance of both
        assert_eq!(k.iter().filter(|k| **k == Kind::Unbalanced).count(), 2);
        assert_eq!(
            k.iter()
                .filter(|k| **k == Kind::TrialBalanceMismatch)
                .count(),
            2
        );
        assert!(!r.ok());
    }

    #[test]
    fn reports_each_hostile_shape() {
        let r = check(
            &[t("lonely"), t("one"), t("neg"), t("bad")],
            &[
                e("one", "a", "debit", 5, "INR"),
                e("neg", "a", "debit", -5, "INR"),
                e("neg", "b", "credit", -5, "INR"),
                e("bad", "a", "sideways", 5, "EUR"),
                e("bad", "b", "credit", 5, "EUR"),
                e("ghost", "a", "debit", 1, "INR"),
            ],
        );
        let k = kinds(&r);
        for want in [
            Kind::NoEntries,
            Kind::TooFewEntries,
            Kind::NonPositiveAmount,
            Kind::InvalidSide,
            Kind::UnsupportedCurrency,
            Kind::OrphanEntry,
            Kind::Unbalanced,
        ] {
            assert!(k.contains(&want), "missing {want:?} in {k:?}");
        }
        assert!(!r.ok());
    }

    #[test]
    fn overflowing_amounts_do_not_overflow_the_checker() {
        let r = check(
            &[t("a")],
            &[
                e("a", "x", "debit", i64::MAX, "INR"),
                e("a", "x", "debit", i64::MAX, "INR"),
                e("a", "y", "credit", i64::MAX, "INR"),
                e("a", "y", "credit", i64::MAX, "INR"),
            ],
        );
        assert!(r.ok());
        assert_eq!(r.trial_balance[0].debits, 2 * i128::from(i64::MAX));
    }

    #[test]
    fn reversals_must_exist_mirror_and_be_unique() {
        let mut rev = t("r");
        rev.reverses = Some("a".into());
        let mut rev2 = t("r2");
        rev2.reverses = Some("a".into());
        let mut missing = t("m");
        missing.reverses = Some("nope".into());
        let mut selfie = t("s");
        selfie.reverses = Some("s".into());
        let base = [
            e("a", "x", "debit", 10, "INR"),
            e("a", "y", "credit", 10, "INR"),
            e("m", "x", "debit", 1, "INR"),
            e("m", "y", "credit", 1, "INR"),
            e("s", "x", "debit", 1, "INR"),
            e("s", "y", "credit", 1, "INR"),
        ];
        // a proper mirror is fine
        let mut ok_entries = base.to_vec();
        ok_entries.push(e("r", "x", "credit", 10, "INR"));
        ok_entries.push(e("r", "y", "debit", 10, "INR"));
        let r = check(
            &[t("a"), rev.clone()],
            &ok_entries[..2]
                .iter()
                .cloned()
                .chain(ok_entries[6..].iter().cloned())
                .collect::<Vec<_>>(),
        );
        assert!(r.violations.is_empty(), "{:?}", r.violations);
        // same-side copy is not a mirror; second reversal; missing target; self
        let mut entries = base.to_vec();
        entries.push(e("r", "x", "debit", 10, "INR"));
        entries.push(e("r", "y", "credit", 10, "INR"));
        entries.push(e("r2", "x", "credit", 10, "INR"));
        entries.push(e("r2", "y", "debit", 10, "INR"));
        let r = check(&[t("a"), rev, rev2, missing, selfie], &entries);
        let k = kinds(&r);
        assert!(k.contains(&Kind::ReversalNotMirror));
        assert!(k.contains(&Kind::DuplicateReversal));
        assert!(k.contains(&Kind::ReversalTargetMissing));
        assert!(k.contains(&Kind::ReversalOfSelf));
    }

    #[test]
    fn duplicate_references_are_reported() {
        let mut b = t("b");
        b.reference = "ref:a".into();
        let r = check(
            &[t("a"), b],
            &[
                e("a", "x", "debit", 1, "INR"),
                e("a", "y", "credit", 1, "INR"),
                e("b", "x", "debit", 1, "INR"),
                e("b", "y", "credit", 1, "INR"),
            ],
        );
        assert_eq!(kinds(&r), vec![Kind::DuplicateReference]);
    }

    #[test]
    fn wrong_direction_balance_is_a_warning_not_an_error() {
        // vendor paid more than owed: vendor_payable ends on the debit side
        let r = check(
            &[t("a")],
            &[
                e("a", "vendor_payable:v1", "debit", 50, "INR"),
                e("a", "bank", "credit", 50, "INR"),
            ],
        );
        assert_eq!(kinds(&r), vec![Kind::UnexpectedBalanceDirection]);
        assert!(r.ok());
        assert_eq!(
            Kind::UnexpectedBalanceDirection.severity(),
            Severity::Warning
        );
    }

    #[test]
    fn output_is_deterministic_regardless_of_input_order() {
        let txns = [t("b"), t("a")];
        let es = [
            e("a", "x", "debit", 3, "INR"),
            e("b", "x", "debit", 3, "INR"),
        ];
        let mut es2 = es.clone().to_vec();
        es2.reverse();
        assert_eq!(check(&txns, &es), check(&[t("a"), t("b")], &es2));
    }

    /// The write path (`Transaction::new`, the capture/refund/dispute helpers and `reversal`) can never
    /// produce rows the checker flags.
    #[test]
    fn everything_the_write_path_builds_passes_the_checker() {
        use crate::{MoneyMode, Side};
        let mut txns = vec![];
        let mut rows = vec![];
        let mut add = |n: usize, tx: crate::Transaction, reverses: Option<String>| {
            let id = format!("t{n}");
            txns.push(TxnRow {
                id: id.clone(),
                reference: format!("r{n}"),
                reverses,
            });
            for en in tx.entries() {
                rows.push(EntryRow {
                    transaction_id: id.clone(),
                    account: en.account.clone(),
                    side: if en.side == Side::Debit {
                        "debit"
                    } else {
                        "credit"
                    }
                    .into(),
                    amount_minor: en.amount_minor,
                    currency: en.currency.code().into(),
                });
            }
        };
        let mut n = 0;
        for mode in [MoneyMode::Orchestrate, MoneyMode::Acquire] {
            let cap = crate::capture_entries(mode, "rz", "org", 12_345, Currency::Inr).unwrap();
            add(n, cap.clone(), None);
            n += 1;
            add(n, cap.reversal(), Some(format!("t{}", n - 1)));
            n += 1;
            add(
                n,
                crate::refund_entries(mode, "rz", "org", 5, Currency::Usd).unwrap(),
                None,
            );
            n += 1;
            add(
                n,
                crate::dispute_lost_entries(mode, "rz", "org", 7, Currency::Inr).unwrap(),
                None,
            );
            n += 1;
        }
        let r = check(&txns, &rows);
        // only warning-level findings are possible (a refund posted without its capture)
        assert!(r.ok(), "{:?}", r.violations);
    }

    /// Build a balanced transaction from arbitrary legs: pair every debit with a credit of equal amount.
    fn balanced_entries(id: &str, legs: &[(u8, u8, u32, bool)]) -> Vec<EntryRow> {
        let mut out = vec![];
        for (a, b, amt, usd) in legs {
            let ccy = if *usd { "USD" } else { "INR" };
            let amt = i64::from(*amt) + 1;
            out.push(e(id, &format!("acct{a}"), "debit", amt, ccy));
            out.push(e(id, &format!("acct{b}"), "credit", amt, ccy));
        }
        out
    }

    proptest! {
        #[test]
        fn any_ledger_built_from_balanced_transactions_has_no_violation(
            txs in proptest::collection::vec(
                proptest::collection::vec((0u8..6, 0u8..6, any::<u32>(), any::<bool>()), 1..5), 0..12)
        ) {
            let mut txns = vec![];
            let mut entries = vec![];
            for (i, legs) in txs.iter().enumerate() {
                let id = format!("t{i}");
                txns.push(t(&id));
                entries.extend(balanced_entries(&id, legs));
            }
            let r = check(&txns, &entries);
            prop_assert!(r.violations.is_empty(), "{:?}", r.violations);
            prop_assert!(r.trial_balance.iter().all(CurrencyTotals::balanced));
        }

        #[test]
        fn nudging_one_amount_is_always_detected_and_never_panics(
            txs in proptest::collection::vec(
                proptest::collection::vec((0u8..6, 0u8..6, any::<u32>(), any::<bool>()), 1..5), 1..8),
            pick in any::<prop::sample::Index>(),
            delta in 1i64..1_000_000,
        ) {
            let mut txns = vec![];
            let mut entries = vec![];
            for (i, legs) in txs.iter().enumerate() {
                let id = format!("t{i}");
                txns.push(t(&id));
                entries.extend(balanced_entries(&id, legs));
            }
            let i = pick.index(entries.len());
            let victim_tx = entries[i].transaction_id.clone();
            entries[i].amount_minor += delta;
            let r = check(&txns, &entries);
            prop_assert!(!r.ok());
            prop_assert!(r.violations.iter().any(|v| v.kind == Kind::Unbalanced && v.subject == victim_tx));
            prop_assert!(r.violations.iter().any(|v| v.kind == Kind::TrialBalanceMismatch));
        }

        #[test]
        fn arbitrary_rows_never_panic(
            rows in proptest::collection::vec(
                ("[a-c]", "[a-c]", "(debit|credit|x)", any::<i64>(), "(INR|USD|EUR)"), 0..30),
            ids in proptest::collection::vec("[a-c]", 0..4),
        ) {
            let txns: Vec<TxnRow> = ids.iter().enumerate().map(|(i, id)| TxnRow {
                id: id.clone(), reference: format!("r{i}"), reverses: Some(id.clone()) }).collect();
            let entries: Vec<EntryRow> = rows.into_iter().map(|(tx, acct, side, amt, ccy)|
                EntryRow { transaction_id: tx, account: acct, side, amount_minor: amt, currency: ccy }).collect();
            let _ = check(&txns, &entries);
        }
    }
}
