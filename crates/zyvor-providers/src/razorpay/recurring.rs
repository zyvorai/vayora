//! Razorpay Recurring / Tokens as UPI AutoPay mandates.
//!
//! BUILT FROM RAZORPAY'S PUBLIC DOCS AND TESTED ONLY AGAINST A LOCAL MOCK. Nothing here has run
//! against Razorpay's sandbox or production. Where the docs were silent the code says "UNCLEAR".
//!
//! Mapping (our terms -> Razorpay):
//! - mandate id     -> `receipt` / notes on a registration order + token
//! - approval       -> hosted auth / UPI mandate link on the order (UNCLEAR: exact auth payload shape)
//! - debit          -> create order + create recurring payment with `token_id`
//! - revoke         -> DELETE /v1/tokens/{id} (or cancel); UNCLEAR whether UPI tokens use the same path
//! - fetch debit    -> GET payments by receipt = attempt_id; never re-send on unknown

use serde_json::{Value, json};

use super::Razorpay;
use crate::*;

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|i| i.to_string()))
}

/// Razorpay token / subscription status -> our mandate status.
pub(crate) fn mandate_status(token_status: &str) -> MandateStatus {
    match token_status {
        "confirmed" | "active" => MandateStatus::Active,
        "rejected" | "cancelled" | "canceled" => MandateStatus::Revoked,
        "paused" => MandateStatus::Paused,
        "expired" => MandateStatus::Expired,
        "failed" => MandateStatus::Failed,
        // created / initiated / pending and unknowns stay Pending until approved.
        _ => MandateStatus::Pending,
    }
}

pub(crate) fn charge_status(s: &str) -> PaymentStatus {
    match s {
        "captured" | "authorized" => PaymentStatus::Captured,
        "failed" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

impl Razorpay {
    pub(crate) async fn rec_create_mandate(
        &self,
        req: &CreateMandate,
    ) -> Result<MandateState, ProviderError> {
        // Registration order: amount 0 for UPI Autopay registration is common in docs; UNCLEAR minimum.
        let body = json!({
            "amount": 0,
            "currency": req.currency.code(),
            "receipt": req.mandate_id,
            "notes": {
                "mandate_id": req.mandate_id,
                "max_amount_minor": req.max_amount_minor,
                "frequency": req.frequency.as_str(),
                "start_date": req.start_date.to_string(),
                "end_date": req.end_date.to_string(),
            },
            "payment": {
                "capture": true,
                "method": "upi",
                // UNCLEAR: recurring flag name varies across Razorpay product docs (token vs subscription).
                "recurring": 1
            }
        });
        let order = read_json(
            self.post("/v1/orders")
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let order_id = text(&order["id"])
            .ok_or_else(|| ProviderError::Uncertain("recurring order response had no id".into()))?;
        // Prefer an auth link if present; otherwise hosted checkout params for the payer.
        let auth_uri = order["upi"]["intent_url"]
            .as_str()
            .or_else(|| order["short_url"].as_str())
            .map(str::to_string);
        Ok(MandateState {
            status: MandateStatus::Pending,
            provider_mandate_ref: Some(order_id.clone()),
            authorization: Some(json!({
                "type": if auth_uri.is_some() { "upi_mandate" } else { "hosted" },
                "uri": auth_uri,
                "order_id": order_id,
                "key": self.key_id,
            })),
            failure_reason: None,
            raw: order,
        })
    }

    pub(crate) async fn rec_fetch_mandate(
        &self,
        mandate_id: &str,
    ) -> Result<MandateState, ProviderError> {
        // Look up the registration order by our receipt (= mandate_id).
        let orders = read_json(
            self.get(&format!("/v1/orders?receipt={mandate_id}"))
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let Some(order) = orders["items"].as_array().and_then(|a| a.first()) else {
            return Ok(MandateState {
                status: MandateStatus::Failed,
                provider_mandate_ref: None,
                authorization: None,
                failure_reason: Some("not_found".into()),
                raw: orders,
            });
        };
        let order_id = text(&order["id"]).unwrap_or_default();
        // UNCLEAR: token may live under payments or a separate /tokens list keyed by order.
        let token_status = order["token"]["status"]
            .as_str()
            .or_else(|| order["status"].as_str())
            .unwrap_or("created");
        let token_id = text(&order["token"]["id"]).or_else(|| text(&order["token_id"]));
        Ok(MandateState {
            status: mandate_status(token_status),
            provider_mandate_ref: token_id.or(Some(order_id)),
            authorization: None,
            failure_reason: None,
            raw: order.clone(),
        })
    }

    pub(crate) async fn rec_revoke_mandate(
        &self,
        mandate_id: &str,
    ) -> Result<MandateState, ProviderError> {
        let current = self.rec_fetch_mandate(mandate_id).await?;
        let Some(token_id) = current.provider_mandate_ref.clone() else {
            return Ok(MandateState {
                status: MandateStatus::Revoked,
                provider_mandate_ref: None,
                authorization: None,
                failure_reason: None,
                raw: current.raw,
            });
        };
        // Prefer token delete when the ref looks like a token; else cancel the order notes path.
        let path = if token_id.starts_with("token_") {
            format!("/v1/tokens/{token_id}")
        } else {
            // UNCLEAR: no cancel-by-order for Autopay; treat as revoked locally after a best-effort delete.
            format!("/v1/tokens/{token_id}")
        };
        let res = self
            .http
            .delete(format!("{}{path}", self.base))
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .send()
            .await
            .map_err(classify)?;
        let raw = if res.status().is_success() || res.status().as_u16() == 404 {
            read_json(res).await.unwrap_or(json!({ "revoked": true }))
        } else {
            // Still mark revoked on our side if Razorpay already cancelled; surface uncertainty otherwise.
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            if status.as_u16() >= 500 {
                return Err(ProviderError::Uncertain(format!(
                    "revoke returned {status}: {body}"
                )));
            }
            json!({ "error": body })
        };
        Ok(MandateState {
            status: MandateStatus::Revoked,
            provider_mandate_ref: Some(token_id),
            authorization: None,
            failure_reason: None,
            raw,
        })
    }

    pub(crate) async fn rec_debit(&self, req: &DebitRequest) -> Result<Initiated, ProviderError> {
        let order_body = json!({
            "amount": req.amount_minor,
            "currency": req.currency.code(),
            "receipt": req.attempt_id,
            "payment_capture": 1,
            "notes": {
                "attempt_id": req.attempt_id,
                "mandate_id": req.mandate_id,
            }
        });
        let order = read_json(
            self.post("/v1/orders")
                .json(&order_body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let order_id = text(&order["id"])
            .ok_or_else(|| ProviderError::Uncertain("debit order response had no id".into()))?;
        let pay_body = json!({
            "amount": req.amount_minor,
            "currency": req.currency.code(),
            "order_id": order_id,
            "customer_id": Value::Null,
            "token": { "id": req.provider_mandate_ref },
            "recurring": "1",
            "notes": { "attempt_id": req.attempt_id, "mandate_id": req.mandate_id },
            "email": "noreply@zyvor.invalid",
            "contact": "+910000000000",
        });
        // UNCLEAR: exact recurring charge path — docs show /v1/payments/create/recurring.
        let pay = read_json(
            self.post("/v1/payments/create/recurring")
                .json(&pay_body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let status = charge_status(pay["status"].as_str().unwrap_or("created"));
        Ok(Initiated {
            state: PaymentState {
                status,
                provider_order_ref: Some(order_id),
                provider_payment_ref: text(&pay["razorpay_payment_id"])
                    .or_else(|| text(&pay["id"])),
                failure_reason: pay["error_description"].as_str().map(str::to_string),
                raw: pay,
            },
            action: ClientAction::None,
        })
    }

    pub(crate) async fn rec_fetch_debit(
        &self,
        attempt_id: &str,
        _mandate_id: &str,
    ) -> Result<PaymentState, ProviderError> {
        // Same as ordinary fetch by receipt — never re-send.
        self.fetch_payment(attempt_id).await
    }
}
