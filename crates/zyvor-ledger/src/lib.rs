//! Append-only double-entry journal.
//!
//! * integer minor units, never floats;
//! * every transaction balances **per currency**, enforced in code and again by a
//!   deferred DB constraint trigger;
//! * entries are immutable: corrections post a reversal (same lines, opposite sides);
//! * in **orchestrate** mode this is the accounting record only (who holds funds is the provider);
//! * in **acquire** mode custody accounts track escrow under a licensed sponsor / PA.

use std::collections::BTreeMap;

use sqlx::PgConnection;
use zyvor_core::Currency;

/// How a tenant's money flows. Drives which ledger accounts capture/refund/settle use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MoneyMode {
    /// Merchant's own provider collects and settles; Zyvor never custodies funds.
    #[default]
    Orchestrate,
    /// Zyvor is the merchant-facing PG; funds sit in sponsor escrow until settled.
    Acquire,
}

impl MoneyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            MoneyMode::Orchestrate => "orchestrate",
            MoneyMode::Acquire => "acquire",
        }
    }
}

impl std::str::FromStr for MoneyMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "orchestrate" => Ok(MoneyMode::Orchestrate),
            "acquire" => Ok(MoneyMode::Acquire),
            o => Err(format!("unknown money_mode {o}")),
        }
    }
}

pub mod accounts {
    /// Asset: money the provider holds on the merchant's behalf (debit-normal). Orchestrate mode.
    pub fn provider_receivable(provider: &str) -> String {
        format!("provider_receivable:{provider}")
    }
    /// Asset: funds in flight into escrow (debit-normal). Acquire mode, on capture.
    pub const ESCROW_IN_FLIGHT: &str = "escrow:in_flight";
    /// Asset: cleared escrow available to settle (debit-normal). Acquire mode.
    pub const ESCROW_AVAILABLE: &str = "escrow:available";
    /// Liability: amount owed to a merchant after capture, before settlement (credit-normal).
    pub fn merchant_payable(merchant_ref: &str) -> String {
        format!("merchant_payable:{merchant_ref}")
    }
    /// Clearing account used while a settlement batch is in flight.
    pub const SETTLEMENT_CLEARING: &str = "settlement_clearing";
    /// Income: platform / acquire fees retained (credit-normal).
    pub const FEE_INCOME: &str = "fee_income";
    /// Income/collections recognised from customer payments (credit-normal).
    pub const COLLECTIONS: &str = "collections";
    /// Expense: provider fees and taxes on fees (debit-normal).
    pub const PROCESSING_FEES: &str = "processing_fees";
    /// Asset: merchant bank account (debit-normal).
    pub const BANK: &str = "bank";
    /// Money paid out to vendors from the merchant's bank (debit-normal).
    pub const PAYOUTS: &str = "payouts";
    /// Liability: what the merchant owes a vendor from split payments (credit-normal).
    pub fn vendor_payable(vendor_id: &str) -> String {
        format!("vendor_payable:{vendor_id}")
    }
    /// Settlement adjustments, chargebacks, reserves (either side).
    pub const ADJUSTMENTS: &str = "settlement_adjustments";
}

/// Capture posting lines for the tenant's money mode.
pub fn capture_entries(
    mode: MoneyMode,
    provider: &str,
    merchant_ref: &str,
    amount_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    match mode {
        MoneyMode::Orchestrate => Transaction::new(vec![
            Entry::debit(accounts::provider_receivable(provider), amount_minor, currency),
            Entry::credit(accounts::COLLECTIONS, amount_minor, currency),
        ]),
        MoneyMode::Acquire => Transaction::new(vec![
            Entry::debit(accounts::ESCROW_IN_FLIGHT, amount_minor, currency),
            Entry::credit(accounts::merchant_payable(merchant_ref), amount_minor, currency),
        ]),
    }
}

/// Refund posting lines (reverses capture economics for the mode).
pub fn refund_entries(
    mode: MoneyMode,
    provider: &str,
    merchant_ref: &str,
    amount_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    match mode {
        MoneyMode::Orchestrate => Transaction::new(vec![
            Entry::debit(accounts::COLLECTIONS, amount_minor, currency),
            Entry::credit(accounts::provider_receivable(provider), amount_minor, currency),
        ]),
        MoneyMode::Acquire => Transaction::new(vec![
            Entry::debit(accounts::merchant_payable(merchant_ref), amount_minor, currency),
            Entry::credit(accounts::ESCROW_IN_FLIGHT, amount_minor, currency),
        ]),
    }
}

/// Lost-dispute posting lines.
pub fn dispute_lost_entries(
    mode: MoneyMode,
    provider: &str,
    merchant_ref: &str,
    amount_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    match mode {
        MoneyMode::Orchestrate => Transaction::new(vec![
            Entry::debit(accounts::ADJUSTMENTS, amount_minor, currency),
            Entry::credit(accounts::provider_receivable(provider), amount_minor, currency),
        ]),
        MoneyMode::Acquire => Transaction::new(vec![
            Entry::debit(accounts::ADJUSTMENTS, amount_minor, currency),
            Entry::credit(accounts::merchant_payable(merchant_ref), amount_minor, currency),
        ]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Debit => Side::Credit,
            Side::Credit => Side::Debit,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Side::Debit => "debit",
            Side::Credit => "credit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub account: String,
    pub side: Side,
    pub amount_minor: i64,
    pub currency: Currency,
}

impl Entry {
    pub fn debit(account: impl Into<String>, amount_minor: i64, currency: Currency) -> Self {
        Self {
            account: account.into(),
            side: Side::Debit,
            amount_minor,
            currency,
        }
    }
    pub fn credit(account: impl Into<String>, amount_minor: i64, currency: Currency) -> Self {
        Self {
            account: account.into(),
            side: Side::Credit,
            amount_minor,
            currency,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("transaction needs at least two entries")]
    TooFewEntries,
    #[error("entry amounts must be positive")]
    NonPositive,
    #[error("transaction does not balance in {0}")]
    Unbalanced(Currency),
    #[error("amount overflow")]
    Overflow,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// A balanced set of entries. Constructible only through [`Transaction::new`].
#[derive(Debug, Clone)]
pub struct Transaction {
    entries: Vec<Entry>,
}

impl Transaction {
    pub fn new(entries: Vec<Entry>) -> Result<Self, LedgerError> {
        if entries.len() < 2 {
            return Err(LedgerError::TooFewEntries);
        }
        let mut by_ccy: BTreeMap<Currency, (i64, i64)> = BTreeMap::new();
        for e in &entries {
            if e.amount_minor <= 0 {
                return Err(LedgerError::NonPositive);
            }
            let t = by_ccy.entry(e.currency).or_default();
            let slot = if e.side == Side::Debit {
                &mut t.0
            } else {
                &mut t.1
            };
            *slot = slot
                .checked_add(e.amount_minor)
                .ok_or(LedgerError::Overflow)?;
        }
        if let Some((ccy, _)) = by_ccy.iter().find(|(_, (d, c))| d != c) {
            return Err(LedgerError::Unbalanced(*ccy));
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Same lines with sides swapped. Posting this undoes the original without editing history.
    pub fn reversal(&self) -> Transaction {
        Transaction {
            entries: self
                .entries
                .iter()
                .map(|e| Entry {
                    side: e.side.opposite(),
                    ..e.clone()
                })
                .collect(),
        }
    }

    /// Persist on `conn` (use the request's DB transaction). `reference` is unique per tenant, so
    /// re-posting the same business event fails instead of double counting.
    pub async fn post(
        &self,
        conn: &mut PgConnection,
        tenant_id: &str,
        reference: &str,
        reverses: Option<&str>,
    ) -> Result<String, StoreError> {
        let id = zyvor_core::new_id("jt");
        sqlx::query("INSERT INTO ledger_transactions (id, tenant_id, reference, reverses) VALUES ($1, $2, $3, $4)")
            .bind(&id)
            .bind(tenant_id)
            .bind(reference)
            .bind(reverses)
            .execute(&mut *conn)
            .await?;
        for e in &self.entries {
            sqlx::query(
                "INSERT INTO ledger_entries (tenant_id, transaction_id, account, side, amount_minor, currency) VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(tenant_id)
            .bind(&id)
            .bind(&e.account)
            .bind(e.side.as_str())
            .bind(e.amount_minor)
            .bind(e.currency.code())
            .execute(&mut *conn)
            .await?;
        }
        Ok(id)
    }
}

/// (debits, credits) in minor units for one account and currency.
pub async fn totals(
    conn: &mut PgConnection,
    tenant_id: &str,
    account: &str,
    currency: Currency,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side = 'debit'), 0)::BIGINT,
                COALESCE(SUM(amount_minor) FILTER (WHERE side = 'credit'), 0)::BIGINT
         FROM ledger_entries WHERE tenant_id = $1 AND account = $2 AND currency = $3",
    )
    .bind(tenant_id)
    .bind(account)
    .bind(currency.code())
    .fetch_one(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use Currency::{Inr, Usd};

    #[test]
    fn balances_per_currency_not_in_aggregate() {
        // 100 INR debit + 100 USD credit has equal raw totals but is NOT balanced.
        let r = Transaction::new(vec![
            Entry::debit("a", 100, Inr),
            Entry::credit("b", 100, Usd),
        ]);
        assert_eq!(r.unwrap_err(), LedgerError::Unbalanced(Inr));
        let ok = Transaction::new(vec![
            Entry::debit("a", 100, Inr),
            Entry::credit("b", 100, Inr),
            Entry::debit("c", 5, Usd),
            Entry::credit("d", 5, Usd),
        ]);
        assert!(ok.is_ok());
    }

    #[test]
    fn rejects_bad_shapes() {
        assert_eq!(
            Transaction::new(vec![Entry::debit("a", 1, Inr)]).unwrap_err(),
            LedgerError::TooFewEntries
        );
        assert_eq!(
            Transaction::new(vec![Entry::debit("a", 0, Inr), Entry::credit("b", 0, Inr)])
                .unwrap_err(),
            LedgerError::NonPositive
        );
        assert_eq!(
            Transaction::new(vec![
                Entry::debit("a", i64::MAX, Inr),
                Entry::debit("a", 1, Inr),
                Entry::credit("b", 1, Inr)
            ])
            .unwrap_err(),
            LedgerError::Overflow
        );
    }

    #[test]
    fn reversal_is_balanced_and_flips_sides() {
        let t = Transaction::new(vec![
            Entry::debit("a", 700, Inr),
            Entry::credit("b", 700, Inr),
        ])
        .unwrap();
        let r = t.reversal();
        assert_eq!(r.entries()[0].side, Side::Credit);
        assert_eq!(r.entries()[1].side, Side::Debit);
        assert!(Transaction::new(r.entries().to_vec()).is_ok());
    }

    #[test]
    fn capture_orchestrate_vs_acquire() {
        let o = capture_entries(MoneyMode::Orchestrate, "razorpay", "org_1", 100, Inr).unwrap();
        assert_eq!(o.entries()[0].account, "provider_receivable:razorpay");
        let a = capture_entries(MoneyMode::Acquire, "sponsor_upi", "org_1", 100, Inr).unwrap();
        assert_eq!(a.entries()[0].account, "escrow:in_flight");
        assert_eq!(a.entries()[1].account, "merchant_payable:org_1");
    }
}
