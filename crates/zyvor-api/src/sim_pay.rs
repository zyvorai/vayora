//! A simulated payer for the iPhone payer app and tests: it approves or declines the sandbox UPI payload
//! the simulator issued. No money moves and no bank is involved. Off unless `ZYVOR_SIM_PAYER=1`, and it
//! only ever touches the in-process simulator, so a live provider can never be paid through it.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, error::ApiError};

/// The payee the simulator puts in its payloads; anything else is not ours to decide.
const SIM_PAYEE: &str = "sandbox@zyvor-sim";

#[derive(Deserialize)]
pub struct Pay {
    /// The `upi://pay?...` payload scanned from the merchant's QR.
    pub upi_uri: String,
    /// `false` plays a payer who declines.
    #[serde(default = "yes")]
    pub approve: bool,
}

fn yes() -> bool {
    true
}

/// `pa` and `tn` (our attempt id) from a `upi://pay` payload.
fn parse(uri: &str) -> Option<(String, String)> {
    let q = uri.strip_prefix("upi://pay?")?;
    let (mut pa, mut tn) = (None, None);
    for kv in q.split('&') {
        let (k, v) = kv.split_once('=')?;
        match k {
            "pa" => pa = Some(v.to_string()),
            "tn" => tn = Some(v.to_string()),
            _ => {}
        }
    }
    Some((pa?, tn?))
}

pub async fn pay(State(s): State<AppState>, Json(b): Json<Pay>) -> Result<Json<Value>, ApiError> {
    if !s.sim_payer {
        return Err(ApiError::not_found("route"));
    }
    let (pa, attempt) = parse(&b.upi_uri)
        .filter(|(pa, _)| pa == SIM_PAYEE)
        .ok_or_else(|| {
            ApiError::invalid("not_a_sim_payment", "this is not a simulator UPI payload")
        })?;
    let _ = pa;
    let status = s.sim.payer_decide(&attempt, b.approve).ok_or_else(|| {
        ApiError::conflict("nothing_to_pay", "no pending simulated payment for this QR")
    })?;
    Ok(Json(json!({ "status": status })))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_payee_and_attempt() {
        let (pa, tn) =
            parse("upi://pay?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=499.00&cu=INR&tn=att_1")
                .unwrap();
        assert_eq!((pa.as_str(), tn.as_str()), ("sandbox@zyvor-sim", "att_1"));
    }

    #[test]
    fn refuses_other_schemes_and_missing_fields() {
        assert!(parse("https://evil.example/pay?pa=a&tn=b").is_none());
        assert!(parse("upi://pay?pa=sandbox@zyvor-sim").is_none());
    }
}
