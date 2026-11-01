//! A simulated "circle" for the iPhone payer app, like `sim_mandate.rs` is for AutoPay: a primary payer lets a delegate pay from their
//! account up to a limit, and the simulator plays the bank that keeps the books. No money moves. Off unless `ZYVOR_SIM_PAYER=1`, and it
//! only ever touches the in-process simulator. The real rules of UPI Circle are not verified here.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_providers::sim::CircleDeny;

use crate::{AppState, error::ApiError};

const SIM_PAYEE: &str = "sandbox@zyvor-sim";

#[derive(Deserialize)]
pub struct Create {
    /// Who is lending their account (a name or UPI ID the phones show).
    pub primary: String,
    /// Who may pay from it.
    pub delegate: String,
    pub max_per_payment_minor: i64,
    pub total_limit_minor: i64,
}

#[derive(Deserialize)]
pub struct Revoke {
    pub circle_id: String,
}

#[derive(Deserialize)]
pub struct Pay {
    pub circle_id: String,
    /// The `upi://pay?...` payload the delegate scanned from the merchant's QR.
    pub upi_uri: String,
}

/// Our attempt id (`tn`) from a simulator `upi://pay` payload; anything else is not ours to approve.
fn attempt_of(uri: &str) -> Option<String> {
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
    (pa? == SIM_PAYEE).then_some(tn?)
}

fn enabled(s: &AppState) -> Result<(), ApiError> {
    if s.sim_payer {
        Ok(())
    } else {
        Err(ApiError::not_found("route"))
    }
}

pub async fn create(
    State(s): State<AppState>,
    Json(b): Json<Create>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    enabled(&s)?;
    let id = s
        .sim
        .circle_create(
            &b.primary,
            &b.delegate,
            b.max_per_payment_minor,
            b.total_limit_minor,
        )
        .map_err(|m| ApiError::invalid("invalid_circle", m))?;
    Ok((StatusCode::CREATED, Json(json!({ "circle_id": id }))))
}

/// What the primary's own bank shows: the limits, what was spent and by which payment.
pub async fn view(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    enabled(&s)?;
    let v = s
        .sim
        .circle_view(&id)
        .ok_or_else(|| ApiError::not_found("circle"))?;
    Ok(Json(json!(v)))
}

pub async fn revoke(
    State(s): State<AppState>,
    Json(b): Json<Revoke>,
) -> Result<Json<Value>, ApiError> {
    enabled(&s)?;
    if !s.sim.circle_revoke(&b.circle_id) {
        return Err(ApiError::conflict(
            "nothing_to_revoke",
            "no live circle with that id",
        ));
    }
    Ok(Json(json!({ "status": "revoked" })))
}

/// The delegate pays a merchant QR through a circle. The amount is read from the payment, never from the request.
pub async fn pay(State(s): State<AppState>, Json(b): Json<Pay>) -> Result<Json<Value>, ApiError> {
    enabled(&s)?;
    let attempt = attempt_of(&b.upi_uri).ok_or_else(|| {
        ApiError::invalid("not_a_sim_payment", "this is not a simulator UPI payload")
    })?;
    match s.sim.circle_pay(&b.circle_id, &attempt) {
        Ok((status, remaining)) => Ok(Json(
            json!({ "status": status, "remaining_minor": remaining }),
        )),
        Err(CircleDeny::Unknown) => Err(ApiError::not_found("circle")),
        Err(CircleDeny::Revoked) => Err(ApiError::conflict(
            "circle_revoked",
            "the account holder has taken this circle back",
        )),
        Err(CircleDeny::OverPaymentLimit) => Err(ApiError::invalid(
            "circle_payment_limit",
            "this payment is more than the circle allows at once",
        )),
        Err(CircleDeny::OverTotalLimit) => Err(ApiError::invalid(
            "circle_total_limit",
            "this payment is more than what is left of the circle",
        )),
        Err(CircleDeny::AlreadyPaid) => Err(ApiError::conflict(
            "already_paid",
            "this payment was already made through the circle",
        )),
        Err(CircleDeny::NotAnIntentPayment) => Err(ApiError::conflict(
            "nothing_to_pay",
            "no pending simulated payment for this QR",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::attempt_of;

    #[test]
    fn reads_the_attempt_from_a_simulator_payload() {
        assert_eq!(
            attempt_of("upi://pay?pa=sandbox@zyvor-sim&pn=X&am=10.00&tn=att_1").as_deref(),
            Some("att_1")
        );
    }

    #[test]
    fn other_payees_and_schemes_are_not_ours() {
        assert!(attempt_of("upi://pay?pa=someone@bank&tn=att_1").is_none());
        assert!(attempt_of("upi://mandate?pa=sandbox@zyvor-sim&tn=mnd_1").is_none());
        assert!(attempt_of("https://evil.example/?pa=sandbox@zyvor-sim&tn=a").is_none());
    }
}
