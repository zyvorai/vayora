//! UPI gateway abstraction plus a deterministic simulator.
//!
//! A real adapter (sponsor PSP bank / NPCI) implements [`UpiGateway`]; nothing
//! else in the system knows which one is in use.

use async_trait::async_trait;
use vayora_core::Money;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectRequest {
    pub payment_id: String,
    pub vpa: String,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectOutcome {
    Success { upi_ref: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    pub payment_id: String,
    pub upi_ref: String,
    pub amount: Money,
}

#[derive(Debug, thiserror::Error)]
pub enum UpiError {
    #[error("upi gateway unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait UpiGateway: Send + Sync {
    /// Ask the payer's UPI app to approve a collect of `amount`.
    async fn collect(&self, req: CollectRequest) -> Result<CollectOutcome, UpiError>;
    /// Reverse (part of) a successful collect. Returns the refund's UPI reference.
    async fn refund(&self, req: RefundRequest) -> Result<String, UpiError>;
}

/// Shape check for a Virtual Payment Address: `handle@psp`.
pub fn is_valid_vpa(vpa: &str) -> bool {
    let Some((handle, psp)) = vpa.split_once('@') else {
        return false;
    };
    let handle_ok = (2..=64).contains(&handle.len())
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let psp_ok = (2..=32).contains(&psp.len()) && psp.chars().all(|c| c.is_ascii_alphabetic());
    handle_ok && psp_ok
}

/// Deterministic simulator driven by the VPA handle:
/// * `fail...@x`    -> declined, insufficient funds
/// * `timeout...@x` -> declined, payer did not approve in time
/// * anything else  -> success
///
/// Same inputs, same outcome. No network, no money.
#[derive(Debug, Default, Clone)]
pub struct SimulatedUpi;

fn fake_rrn() -> String {
    // 12-digit retrieval reference number, like a real UPI RRN.
    format!("{:012}", uuid::Uuid::now_v7().as_u128() % 1_000_000_000_000)
}

#[async_trait]
impl UpiGateway for SimulatedUpi {
    async fn collect(&self, req: CollectRequest) -> Result<CollectOutcome, UpiError> {
        let handle = req.vpa.split('@').next().unwrap_or_default();
        Ok(if handle.starts_with("fail") {
            CollectOutcome::Failed {
                reason: "insufficient_funds".into(),
            }
        } else if handle.starts_with("timeout") {
            CollectOutcome::Failed {
                reason: "payer_timeout".into(),
            }
        } else {
            CollectOutcome::Success {
                upi_ref: fake_rrn(),
            }
        })
    }

    async fn refund(&self, _req: RefundRequest) -> Result<String, UpiError> {
        Ok(fake_rrn())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(vpa: &str) -> CollectRequest {
        CollectRequest {
            payment_id: "pay_1".into(),
            vpa: vpa.into(),
            amount: Money::from_paise(100).unwrap(),
        }
    }

    #[test]
    fn vpa_shapes() {
        assert!(is_valid_vpa("asha.k@okvayora"));
        for bad in ["", "a@b", "noat", "x y@upi", "name@up1", "@upi", "name@"] {
            assert!(!is_valid_vpa(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn outcomes_follow_handle() {
        let sim = SimulatedUpi;
        assert!(
            matches!(sim.collect(req("asha@upi")).await.unwrap(), CollectOutcome::Success { upi_ref } if upi_ref.len() == 12)
        );
        assert_eq!(
            sim.collect(req("fail.me@upi")).await.unwrap(),
            CollectOutcome::Failed {
                reason: "insufficient_funds".into()
            }
        );
        assert_eq!(
            sim.collect(req("timeout@upi")).await.unwrap(),
            CollectOutcome::Failed {
                reason: "payer_timeout".into()
            }
        );
    }
}
