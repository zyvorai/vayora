//! Double-entry ledger: every transaction's debits must equal its credits.

use vayora_core::Money;

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
