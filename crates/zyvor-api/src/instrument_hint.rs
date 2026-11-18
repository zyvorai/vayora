//! What the payer used, as far as offers, routing rules and the downtime alerts care: a card network class, a UPI
//! handle or app, a bank. Never a card number: only the first six or eight digits (the BIN) may be sent, and only to
//! work out a network class and match an offer's BIN range; the BIN itself is not stored.
//!
//! Where it comes from, and how far to trust it (`source`):
//! - `card_token`: the brand of a saved card token in our own database;
//! - `vpa`: the handle of the UPI ID the payer typed (what is after the `@`);
//! - `caller_hint`: what the caller said (an API client, or the public checkout page). Nothing checks it against the
//!   provider, so an offer keyed on a hint is a pricing nudge and not enforcement. See docs/zyvor/OFFERS.md.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::ApiError;

pub const NETWORKS: &[&str] = &["visa", "mastercard", "rupay", "amex", "diners", "other"];

/// What a caller may send with an attempt (all optional).
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct InstrumentIn {
    /// One of visa, mastercard, rupay, amex, diners, other.
    #[serde(default)]
    pub card_network: Option<String>,
    /// The first 6 or 8 digits of the card (the BIN), to class the card. Anything longer is refused.
    #[serde(default)]
    pub card_bin: Option<String>,
    /// A UPI app the payer chose, for example `gpay` or `phonepe`.
    #[serde(default)]
    pub upi_app: Option<String>,
    /// The bank for a netbanking payment, for example `hdfc`.
    #[serde(default)]
    pub bank: Option<String>,
}

/// The cleaned facts. `bin6` lives only in memory for one evaluation; it is never put into `to_json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instrument {
    pub card_network: Option<String>,
    pub bin6: Option<String>,
    pub upi_app: Option<String>,
    pub upi_handle: Option<String>,
    pub bank: Option<String>,
    pub source: Option<&'static str>,
}

/// The network class of a BIN, from the well-known issuer prefixes. Anything else is `other`.
pub fn network_of_bin(bin: &str) -> &'static str {
    let n = |k: usize| bin.get(..k).and_then(|p| p.parse::<u32>().ok());
    match (n(1), n(2), n(3), n(4)) {
        (Some(4), ..) => "visa",
        (_, Some(51..=55), ..) | (_, _, _, Some(2221..=2720)) => "mastercard",
        (_, Some(34 | 37), ..) => "amex",
        (_, Some(36 | 38), ..) | (_, _, Some(300..=305), _) => "diners",
        // RuPay ranges: 60, 65, 81, 82, 508
        (_, Some(60 | 65 | 81 | 82), ..) | (_, _, Some(508), _) => "rupay",
        _ => "other",
    }
}

fn word(v: &str, what: &str, max: usize) -> Result<String, ApiError> {
    let w = v.trim().to_lowercase();
    if w.is_empty()
        || w.len() > max
        || !w
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ApiError::invalid(
            "invalid_instrument",
            format!("{what} must be 1-{max} letters, digits, - _ or ."),
        ));
    }
    Ok(w)
}

pub fn normalise_network(v: &str) -> Result<String, ApiError> {
    let w = v.trim().to_lowercase().replace(' ', "");
    let w = match w.as_str() {
        "master" | "mc" => "mastercard".to_string(),
        "americanexpress" => "amex".to_string(),
        "dinersclub" => "diners".to_string(),
        other => other.to_string(),
    };
    if NETWORKS.contains(&w.as_str()) {
        Ok(w)
    } else {
        Err(ApiError::invalid(
            "invalid_instrument",
            format!("card_network must be one of {}", NETWORKS.join(", ")),
        ))
    }
}

/// The handle of a UPI ID, lower case, with the `@` (`name@okaxis` gives `@okaxis`).
pub fn upi_handle(vpa: &str) -> Option<String> {
    vpa.split_once('@')
        .map(|(_, h)| h.trim().to_lowercase())
        .filter(|h| !h.is_empty())
        .map(|h| format!("@{h}"))
}

/// Build the facts for one attempt. A saved card token's brand wins over a hint.
pub fn build(
    hint: Option<&InstrumentIn>,
    vpa: Option<&str>,
    token_brand: Option<&str>,
) -> Result<Instrument, ApiError> {
    let mut i = Instrument::default();
    if let Some(h) = hint {
        if let Some(bin) = h.card_bin.as_deref() {
            let bin = bin.trim();
            if !(6..=8).contains(&bin.len()) || !bin.chars().all(|c| c.is_ascii_digit()) {
                return Err(ApiError::invalid(
                    "invalid_instrument",
                    "card_bin must be the first 6 to 8 digits only. Never send a full card number",
                ));
            }
            i.bin6 = Some(bin[..6].to_string());
            i.card_network = Some(network_of_bin(bin).to_string());
        }
        if let Some(n) = h.card_network.as_deref() {
            let n = normalise_network(n)?;
            // a BIN and a network that disagree is a mistake, not something to guess about
            if i.card_network
                .as_deref()
                .is_some_and(|b| b != n && b != "other")
            {
                return Err(ApiError::invalid(
                    "invalid_instrument",
                    "card_network does not match the card_bin",
                ));
            }
            i.card_network = Some(n);
        }
        if let Some(a) = h.upi_app.as_deref() {
            i.upi_app = Some(word(a, "upi_app", 24)?);
        }
        if let Some(b) = h.bank.as_deref() {
            i.bank = Some(word(b, "bank", 32)?);
        }
        i.source = Some("caller_hint");
    }
    if let Some(h) = vpa.and_then(upi_handle) {
        i.upi_handle = Some(h);
        i.source.get_or_insert("vpa");
    }
    if let Some(b) = token_brand
        && let Ok(n) = normalise_network(b)
    {
        i.card_network = Some(n);
        i.bin6 = None;
        i.source = Some("card_token");
    }
    Ok(i)
}

impl Instrument {
    /// What is stored on the attempt and shown: no BIN, no card number.
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in [
            ("card_network", &self.card_network),
            ("upi_app", &self.upi_app),
            ("upi_handle", &self.upi_handle),
            ("bank", &self.bank),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), json!(v));
            }
        }
        if let (false, Some(s)) = (m.is_empty(), self.source) {
            m.insert("source".into(), json!(s));
        }
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bins_are_classed_by_prefix() {
        assert_eq!(network_of_bin("411111"), "visa");
        assert_eq!(network_of_bin("555555"), "mastercard");
        assert_eq!(network_of_bin("222100"), "mastercard");
        assert_eq!(network_of_bin("272099"), "mastercard");
        assert_eq!(network_of_bin("378282"), "amex");
        assert_eq!(network_of_bin("606000"), "rupay");
        assert_eq!(network_of_bin("508500"), "rupay");
        assert_eq!(network_of_bin("360000"), "diners");
        assert_eq!(network_of_bin("999999"), "other");
    }

    #[test]
    fn a_card_number_is_refused_and_the_bin_is_not_stored() {
        let long = InstrumentIn {
            card_bin: Some("4111111111111111".into()),
            ..Default::default()
        };
        assert!(build(Some(&long), None, None).is_err());
        let ok = InstrumentIn {
            card_bin: Some("411111".into()),
            ..Default::default()
        };
        let i = build(Some(&ok), None, None).unwrap();
        assert_eq!(i.card_network.as_deref(), Some("visa"));
        assert_eq!(i.bin6.as_deref(), Some("411111"));
        assert!(!i.to_json().to_string().contains("411111"));
    }

    #[test]
    fn conflicting_network_and_bin_is_refused_and_a_token_wins() {
        let bad = InstrumentIn {
            card_bin: Some("411111".into()),
            card_network: Some("rupay".into()),
            ..Default::default()
        };
        assert!(build(Some(&bad), None, None).is_err());
        let hint = InstrumentIn {
            card_network: Some("rupay".into()),
            ..Default::default()
        };
        let i = build(Some(&hint), None, Some("Visa")).unwrap();
        assert_eq!(i.card_network.as_deref(), Some("visa"));
        assert_eq!(i.source, Some("card_token"));
    }

    #[test]
    fn upi_handle_comes_from_the_vpa() {
        let i = build(None, Some("Asha@OkAxis"), None).unwrap();
        assert_eq!(i.upi_handle.as_deref(), Some("@okaxis"));
        assert_eq!(i.source, Some("vpa"));
        assert_eq!(build(None, None, None).unwrap().to_json(), json!({}));
    }
}
