//! Shared primitives for Zyvor Payments: currencies, ids, environments.
//! Money is always an integer count of minor units (paise for INR) plus a currency.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Currency {
    #[serde(rename = "INR")]
    Inr,
    #[serde(rename = "USD")]
    Usd,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Inr => "INR",
            Currency::Usd => "USD",
        }
    }
    /// Minor units per major unit (both supported currencies use 2 decimals).
    pub fn minor_per_major(self) -> i64 {
        100
    }
}

impl std::str::FromStr for Currency {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "INR" => Ok(Currency::Inr),
            "USD" => Ok(Currency::Usd),
            other => Err(format!("unsupported currency {other}")),
        }
    }
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    Test,
    Live,
}

impl Env {
    pub fn as_str(self) -> &'static str {
        match self {
            Env::Test => "test",
            Env::Live => "live",
        }
    }
}

/// Prefixed, time-ordered identifier, e.g. `pi_01928f...`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::now_v7().simple())
}

/// Render minor units as a decimal string in major units ("499.00") without floats.
pub fn minor_to_decimal(minor: i64, currency: Currency) -> String {
    let per = currency.minor_per_major();
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs() as i64;
    format!("{sign}{}.{:02}", abs / per, abs % per)
}

/// Parse "499.00" / "499.5" / "499" into minor units; rejects anything ambiguous.
pub fn decimal_to_minor(s: &str) -> Option<i64> {
    if s.ends_with('.') {
        return None;
    }
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > 2
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: i64 = whole.parse().ok()?;
    let f: i64 = format!("{frac:0<2}").parse().ok()?;
    w.checked_mul(100)?.checked_add(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_roundtrip_without_floats() {
        assert_eq!(minor_to_decimal(49900, Currency::Inr), "499.00");
        assert_eq!(minor_to_decimal(5, Currency::Inr), "0.05");
        assert_eq!(minor_to_decimal(-1250, Currency::Inr), "-12.50");
        assert_eq!(decimal_to_minor("499.00"), Some(49900));
        assert_eq!(decimal_to_minor("499.5"), Some(49950));
        assert_eq!(decimal_to_minor("0.07"), Some(7));
        for bad in ["", ".5", "1.", "1.234", "-1", "1e3", "abc"] {
            assert_eq!(decimal_to_minor(bad), None, "{bad}");
        }
    }

    #[test]
    fn currency_parses() {
        assert_eq!("INR".parse::<Currency>(), Ok(Currency::Inr));
        assert!("EUR".parse::<Currency>().is_err());
        assert_eq!(serde_json::to_string(&Currency::Usd).unwrap(), "\"USD\"");
    }
}
