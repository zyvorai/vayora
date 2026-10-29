//! Domain primitives shared across Vayora crates.

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("amount overflow")]
    Overflow,
    #[error("amount must not be negative")]
    Negative,
}

/// Indian rupees stored as integer paise (1 INR = 100 paise). Never a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn from_paise(paise: i64) -> Result<Self, MoneyError> {
        if paise < 0 {
            return Err(MoneyError::Negative);
        }
        Ok(Money(paise))
    }

    pub fn paise(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, other: Money) -> Result<Money, MoneyError> {
        self.0
            .checked_add(other.0)
            .map(Money)
            .ok_or(MoneyError::Overflow)
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, MoneyError> {
        let v = self.0.checked_sub(other.0).ok_or(MoneyError::Overflow)?;
        Money::from_paise(v)
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "₹{}.{:02}", self.0 / 100, self.0 % 100)
    }
}

/// Prefixed, time-ordered identifier, e.g. `pay_01928f...`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_negative() {
        assert_eq!(Money::from_paise(-1), Err(MoneyError::Negative));
    }

    #[test]
    fn arithmetic_is_checked() {
        let max = Money::from_paise(i64::MAX).unwrap();
        assert_eq!(
            max.checked_add(Money::from_paise(1).unwrap()),
            Err(MoneyError::Overflow)
        );
        let a = Money::from_paise(100).unwrap();
        assert_eq!(
            a.checked_sub(Money::from_paise(101).unwrap()),
            Err(MoneyError::Negative)
        );
    }

    #[test]
    fn displays_rupees() {
        assert_eq!(Money::from_paise(12345).unwrap().to_string(), "₹123.45");
    }
}
