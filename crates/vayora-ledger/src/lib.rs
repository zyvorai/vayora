//! Double-entry ledger: every transaction's debits must equal its credits.

use sqlx::PgConnection;
use vayora_core::Money;

pub mod accounts {
    /// Asset: funds the UPI network owes us (debit-normal).
    pub const UPI_CLEARING: &str = "upi_clearing";
    /// Liability: funds we owe the merchant (credit-normal).
    pub const MERCHANT_PAYABLE: &str = "merchant_payable";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub account: String,
    pub side: Side,
    pub amount: Money,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("transaction is unbalanced")]
    Unbalanced,
    #[error("transaction needs at least two entries")]
    TooFewEntries,
    #[error("amount overflow")]
    Overflow,
}

/// A balanced set of entries. Construct only through [`Transaction::new`].
#[derive(Debug, Clone)]
pub struct Transaction {
    entries: Vec<Entry>,
}

impl Transaction {
    pub fn new(entries: Vec<Entry>) -> Result<Self, LedgerError> {
        if entries.len() < 2 {
            return Err(LedgerError::TooFewEntries);
        }
        let (mut debit, mut credit) = (0i64, 0i64);
        for e in &entries {
            let total = match e.side {
                Side::Debit => &mut debit,
                Side::Credit => &mut credit,
            };
            *total = total
                .checked_add(e.amount.paise())
                .ok_or(LedgerError::Overflow)?;
        }
        if debit != credit {
            return Err(LedgerError::Unbalanced);
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

impl Transaction {
    /// Persist atomically on `conn` (pass a DB transaction so it commits with
    /// the business change). `reference` is unique, so a repeated post of the
    /// same event fails instead of double-counting.
    pub async fn post(&self, conn: &mut PgConnection, reference: &str) -> Result<(), StoreError> {
        let id = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO ledger_transactions (id, reference) VALUES ($1, $2)")
            .bind(id)
            .bind(reference)
            .execute(&mut *conn)
            .await?;
        for e in &self.entries {
            let side = match e.side {
                Side::Debit => "debit",
                Side::Credit => "credit",
            };
            sqlx::query(
                "INSERT INTO ledger_entries (transaction_id, account, side, amount_paise) VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(&e.account)
            .bind(side)
            .bind(e.amount.paise())
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }
}

/// Total (debits, credits) in paise for an account.
pub async fn totals(conn: &mut PgConnection, account: &str) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT COALESCE(SUM(amount_paise) FILTER (WHERE side = 'debit'), 0)::BIGINT,
                COALESCE(SUM(amount_paise) FILTER (WHERE side = 'credit'), 0)::BIGINT
         FROM ledger_entries WHERE account = $1",
    )
    .bind(account)
    .fetch_one(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(account: &str, side: Side, p: i64) -> Entry {
        Entry {
            account: account.into(),
            side,
            amount: Money::from_paise(p).unwrap(),
        }
    }

    #[test]
    fn balanced_ok() {
        assert!(
            Transaction::new(vec![
                e("customer", Side::Debit, 500),
                e("merchant", Side::Credit, 500)
            ])
            .is_ok()
        );
    }

    #[test]
    fn unbalanced_rejected() {
        let r = Transaction::new(vec![e("a", Side::Debit, 500), e("b", Side::Credit, 499)]);
        assert_eq!(r.unwrap_err(), LedgerError::Unbalanced);
    }

    #[test]
    fn single_entry_rejected() {
        assert_eq!(
            Transaction::new(vec![e("a", Side::Debit, 0)]).unwrap_err(),
            LedgerError::TooFewEntries
        );
    }
}
