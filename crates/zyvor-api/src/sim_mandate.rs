//! A simulated payer for UPI AutoPay mandates, like `sim_pay.rs` is for payments: it approves, declines or
//! revokes a mandate the in-process simulator issued. No bank, no real mandate. Off unless
//! `ZYVOR_SIM_PAYER=1`, and it only ever touches the in-process simulator.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, error::ApiError};

const SIM_PAYEE: &str = "sandbox@zyvor-sim";

#[derive(Deserialize)]
pub struct Decide {
    /// The `upi://mandate?...` link from the mandate's `authorization`.
    pub upi_uri: String,
    /// `false` plays a payer who declines.
    #[serde(default = "yes")]
    pub approve: bool,
}

#[derive(Deserialize)]
pub struct Revoke {
    /// The mandate id (`tn` in the `upi://mandate` link), as the payer sees it in their UPI app.
    pub mandate_id: String,
}

fn yes() -> bool {
    true
}

/// `pa` and `tn` (our mandate id) from a `upi://mandate` link.
fn parse(uri: &str) -> Option<(String, String)> {
    let q = uri.strip_prefix("upi://mandate?")?;
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

pub async fn decide(
    State(s): State<AppState>,
    Json(b): Json<Decide>,
) -> Result<Json<Value>, ApiError> {
    if !s.sim_payer {
        return Err(ApiError::not_found("route"));
    }
    let (_, mandate) = parse(&b.upi_uri)
        .filter(|(pa, _)| pa == SIM_PAYEE)
        .ok_or_else(|| {
            ApiError::invalid("not_a_sim_mandate", "this is not a simulator mandate link")
        })?;
    let status = s.sim.mandate_decide(&mandate, b.approve).ok_or_else(|| {
        ApiError::conflict(
            "nothing_to_approve",
            "no pending simulated mandate for this link",
        )
    })?;
    Ok(Json(json!({ "status": status })))
}

/// What the payer's own bank shows about a mandate: its state and the debits it took (the simulator plays the bank).
pub async fn view(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !s.sim_payer {
        return Err(ApiError::not_found("route"));
    }
    let v = s
        .sim
        .payer_view(&id)
        .ok_or_else(|| ApiError::not_found("mandate"))?;
    Ok(Json(json!(v)))
}

pub async fn revoke(
    State(s): State<AppState>,
    Json(b): Json<Revoke>,
) -> Result<Json<Value>, ApiError> {
    if !s.sim_payer {
        return Err(ApiError::not_found("route"));
    }
    let status = s.sim.payer_revoke(&b.mandate_id).ok_or_else(|| {
        ApiError::conflict(
            "nothing_to_revoke",
            "no live simulated mandate with that id",
        )
    })?;
    Ok(Json(json!({ "status": status })))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_payee_and_mandate() {
        let (pa, tn) = parse("upi://mandate?pa=sandbox@zyvor-sim&pn=Zyvor%20Sandbox&am=1500.00&cu=INR&tn=mnd_1&recur=monthly").unwrap();
        assert_eq!((pa.as_str(), tn.as_str()), ("sandbox@zyvor-sim", "mnd_1"));
    }

    #[test]
    fn a_payment_link_is_not_a_mandate_link() {
        assert!(parse("upi://pay?pa=sandbox@zyvor-sim&tn=att_1").is_none());
        assert!(parse("https://evil.example/mandate?pa=a&tn=b").is_none());
        assert!(parse("upi://mandate?pa=sandbox@zyvor-sim").is_none());
    }
}
