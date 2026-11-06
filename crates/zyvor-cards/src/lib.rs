//! Network-token card charges. **Never** accepts or stores PAN or CVV.
//!
//! Card data stays in a PCI-scoped vault / sponsor hosted fields. Zyvor keeps only a
//! network token reference, last4, brand and expiry for display and charging.

use serde::{Deserialize, Serialize};
use zyvor_core::Currency;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CardsError {
    #[error("PAN or CVV must never be sent to Zyvor")]
    RawCardDataForbidden,
    #[error("invalid token: {0}")]
    InvalidToken(String),
    #[error("token revoked")]
    Revoked,
}

/// Stored instrument — network token only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkToken {
    pub id: String,
    pub network_token_ref: String,
    pub last4: String,
    pub brand: Option<String>,
    pub exp_month: Option<u8>,
    pub exp_year: Option<u16>,
    pub revoked: bool,
}

/// Charge request using a previously vaulted network token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargeWithToken {
    pub token_id: String,
    pub network_token_ref: String,
    pub amount_minor: i64,
    pub currency: Currency,
    pub attempt_id: String,
}

/// Reject any payload that looks like it contains raw card data.
pub fn reject_raw_card_fields(payload: &serde_json::Value) -> Result<(), CardsError> {
    const FORBIDDEN: &[&str] = &[
        "pan",
        "card_number",
        "cardNumber",
        "number",
        "cvv",
        "cvc",
        "security_code",
        "securityCode",
    ];
    if let Some(obj) = payload.as_object() {
        for k in FORBIDDEN {
            if obj.contains_key(*k) {
                return Err(CardsError::RawCardDataForbidden);
            }
        }
    }
    Ok(())
}

pub fn validate_token_ref(t: &NetworkToken) -> Result<(), CardsError> {
    if t.revoked {
        return Err(CardsError::Revoked);
    }
    if t.network_token_ref.trim().is_empty() || t.network_token_ref.len() > 128 {
        return Err(CardsError::InvalidToken("network_token_ref length".into()));
    }
    if t.last4.len() != 4 || !t.last4.chars().all(|c| c.is_ascii_digit()) {
        return Err(CardsError::InvalidToken("last4 must be 4 digits".into()));
    }
    if let Some(m) = t.exp_month
        && !(1..=12).contains(&m)
    {
        return Err(CardsError::InvalidToken("exp_month".into()));
    }
    Ok(())
}

/// Build a charge instruction for the acquirer / sponsor — still no PAN.
pub fn charge_instruction(token: &NetworkToken, req: &ChargeWithToken) -> Result<serde_json::Value, CardsError> {
    validate_token_ref(token)?;
    if token.id != req.token_id && token.network_token_ref != req.network_token_ref {
        return Err(CardsError::InvalidToken("token mismatch".into()));
    }
    if req.amount_minor <= 0 {
        return Err(CardsError::InvalidToken("amount must be positive".into()));
    }
    Ok(serde_json::json!({
        "attempt_id": req.attempt_id,
        "amount_minor": req.amount_minor,
        "currency": req.currency,
        "network_token_ref": token.network_token_ref,
        "last4": token.last4,
        "brand": token.brand,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyvor_core::Currency::Inr;

    #[test]
    fn rejects_pan() {
        let v = serde_json::json!({ "pan": "4111111111111111", "cvv": "123" });
        assert_eq!(reject_raw_card_fields(&v), Err(CardsError::RawCardDataForbidden));
    }

    #[test]
    fn accepts_token_only() {
        let t = NetworkToken {
            id: "tok_1".into(),
            network_token_ref: "nt_abc".into(),
            last4: "4242".into(),
            brand: Some("visa".into()),
            exp_month: Some(12),
            exp_year: Some(2030),
            revoked: false,
        };
        let insn = charge_instruction(
            &t,
            &ChargeWithToken {
                token_id: "tok_1".into(),
                network_token_ref: "nt_abc".into(),
                amount_minor: 100,
                currency: Inr,
                attempt_id: "att_1".into(),
            },
        )
        .unwrap();
        assert_eq!(insn["network_token_ref"], "nt_abc");
        assert!(insn.get("pan").is_none());
    }
}
