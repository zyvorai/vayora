//! Acquire-mode settlement: escrow batches and payout rails.
//!
//! Zyvor instructs a licensed sponsor bank / PA to move money; this crate builds the batch,
//! posts clearing ledger entries, and talks to a [`PayoutRail`]. It does not hold funds itself.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zyvor_core::Currency;
use zyvor_ledger::{Entry, LedgerError, MoneyMode, Transaction, accounts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RailKind {
    Imps,
    Neft,
    Upi,
}

impl RailKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RailKind::Imps => "imps",
            RailKind::Neft => "neft",
            RailKind::Upi => "upi",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutInstruction {
    pub payout_id: String,
    pub amount_minor: i64,
    pub currency: Currency,
    pub rail: RailKind,
    pub account_number: Option<String>,
    pub ifsc: Option<String>,
    pub vpa: Option<String>,
    pub beneficiary_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutResult {
    pub status: PayoutRailStatus,
    pub rail_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutRailStatus {
    Accepted,
    Settled,
    Failed,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum RailError {
    #[error("rail rejected: {0}")]
    Rejected(String),
    #[error("rail outcome unknown: {0}")]
    Uncertain(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Talks to a sponsor bank / PA payout API.
#[async_trait]
pub trait PayoutRail: Send + Sync {
    async fn send(&self, insn: &PayoutInstruction) -> Result<PayoutResult, RailError>;
    async fn fetch(&self, payout_id: &str) -> Result<PayoutResult, RailError>;
}

/// HTTP payout rail that speaks a small JSON contract (same spirit as PROVIDER_CONTRACT).
pub struct HttpPayoutRail {
    base: String,
    api_key: String,
    http: reqwest::Client,
}

impl HttpPayoutRail {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            base: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http,
        }
    }
}

#[async_trait]
impl PayoutRail for HttpPayoutRail {
    async fn send(&self, insn: &PayoutInstruction) -> Result<PayoutResult, RailError> {
        let res = self
            .http
            .post(format!("{}/v1/payouts", self.base))
            .bearer_auth(&self.api_key)
            .json(insn)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    RailError::Uncertain(e.to_string())
                } else {
                    RailError::Rejected(e.to_string())
                }
            })?;
        let status = res.status();
        let body: serde_json::Value = res.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if status.is_server_error() {
            return Err(RailError::Uncertain(format!("rail {status}")));
        }
        if !status.is_success() {
            return Err(RailError::Rejected(
                body["error"].as_str().unwrap_or("rejected").into(),
            ));
        }
        parse_result(body)
    }

    async fn fetch(&self, payout_id: &str) -> Result<PayoutResult, RailError> {
        let res = self
            .http
            .get(format!("{}/v1/payouts/{payout_id}", self.base))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| RailError::Uncertain(e.to_string()))?;
        let status = res.status();
        let body: serde_json::Value = res.json().await.unwrap_or_else(|_| serde_json::json!({}));
        if status.is_server_error() {
            return Err(RailError::Uncertain(format!("rail {status}")));
        }
        if !status.is_success() {
            return Err(RailError::Rejected(
                body["error"].as_str().unwrap_or("not_found").into(),
            ));
        }
        parse_result(body)
    }
}

fn parse_result(body: serde_json::Value) -> Result<PayoutResult, RailError> {
    let status = match body["status"].as_str() {
        Some("accepted") => PayoutRailStatus::Accepted,
        Some("settled") => PayoutRailStatus::Settled,
        Some("failed") => PayoutRailStatus::Failed,
        Some("unknown") => PayoutRailStatus::Unknown,
        other => {
            return Err(RailError::Uncertain(format!(
                "unrecognised payout status {other:?}"
            )));
        }
    };
    Ok(PayoutResult {
        status,
        rail_ref: body["rail_ref"].as_str().map(str::to_string),
        failure_reason: body["failure_reason"].as_str().map(str::to_string),
        raw: body,
    })
}

/// In-process rail for tests / sandbox: always settles with a synthetic UTR.
#[derive(Default)]
pub struct SimPayoutRail;

#[async_trait]
impl PayoutRail for SimPayoutRail {
    async fn send(&self, insn: &PayoutInstruction) -> Result<PayoutResult, RailError> {
        Ok(PayoutResult {
            status: PayoutRailStatus::Settled,
            rail_ref: Some(format!(
                "SIMUTR{}",
                &insn.payout_id[insn.payout_id.len().saturating_sub(8)..]
            )),
            failure_reason: None,
            raw: serde_json::json!({ "sim": true }),
        })
    }

    async fn fetch(&self, payout_id: &str) -> Result<PayoutResult, RailError> {
        Ok(PayoutResult {
            status: PayoutRailStatus::Settled,
            rail_ref: Some(format!(
                "SIMUTR{}",
                &payout_id[payout_id.len().saturating_sub(8)..]
            )),
            failure_reason: None,
            raw: serde_json::json!({ "sim": true }),
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchTotals {
    pub captures_minor: i64,
    pub refunds_minor: i64,
    pub fees_minor: i64,
    pub holds_minor: i64,
}

impl BatchTotals {
    pub fn net(&self) -> i64 {
        self.captures_minor - self.refunds_minor - self.fees_minor - self.holds_minor
    }
}

/// Move in-flight escrow to available after the sponsor confirms capture clearing.
pub fn post_escrow_available(
    amount_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    Transaction::new(vec![
        Entry::debit(accounts::ESCROW_AVAILABLE, amount_minor, currency),
        Entry::credit(accounts::ESCROW_IN_FLIGHT, amount_minor, currency),
    ])
}

/// Release a settlement batch: clear merchant payable into settlement clearing (+ optional fee income).
pub fn post_settlement_release(
    mode: MoneyMode,
    merchant_ref: &str,
    net_minor: i64,
    fee_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    if mode != MoneyMode::Acquire {
        return Err(LedgerError::TooFewEntries);
    }
    let gross = net_minor
        .checked_add(fee_minor)
        .ok_or(LedgerError::Overflow)?;
    if gross <= 0 || net_minor < 0 || fee_minor < 0 {
        return Err(LedgerError::NonPositive);
    }
    let mut entries = vec![Entry::debit(
        accounts::merchant_payable(merchant_ref),
        gross,
        currency,
    )];
    if net_minor > 0 {
        entries.push(Entry::credit(
            accounts::SETTLEMENT_CLEARING,
            net_minor,
            currency,
        ));
    }
    if fee_minor > 0 {
        entries.push(Entry::credit(accounts::FEE_INCOME, fee_minor, currency));
    }
    Transaction::new(entries)
}

/// After the rail confirms the bank credit: clearing → bank (merchant received funds).
pub fn post_settlement_banked(
    net_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    Transaction::new(vec![
        Entry::debit(accounts::SETTLEMENT_CLEARING, net_minor, currency),
        Entry::credit(accounts::BANK, net_minor, currency),
    ])
}

/// Acquire payout from escrow / vendor payable via rail (before rail ACK uses clearing).
pub fn post_acquire_payout_request(
    source_account: &str,
    amount_minor: i64,
    currency: Currency,
) -> Result<Transaction, LedgerError> {
    Transaction::new(vec![
        Entry::debit(source_account.to_string(), amount_minor, currency),
        Entry::credit(accounts::SETTLEMENT_CLEARING, amount_minor, currency),
    ])
}

/// Escrow vs ledger proof: available escrow asset should equal open payables (simplified).
pub fn escrow_proof_ok(escrow_available_net: i64, open_payables_net: i64) -> bool {
    escrow_available_net == open_payables_net
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyvor_core::Currency::Inr;

    #[test]
    fn escrow_available_balances() {
        assert!(post_escrow_available(500, Inr).is_ok());
    }

    #[test]
    fn settlement_release_with_fee() {
        let t = post_settlement_release(MoneyMode::Acquire, "org_x", 970, 30, Inr).unwrap();
        assert_eq!(t.entries().len(), 3);
    }

    #[test]
    fn proof() {
        assert!(escrow_proof_ok(1000, 1000));
        assert!(!escrow_proof_ok(1000, 999));
    }
}
