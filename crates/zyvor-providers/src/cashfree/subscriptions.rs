//! Cashfree Subscriptions (v2026-01-01) as UPI AutoPay mandates.
//!
//! BUILT FROM CASHFREE'S PUBLIC DOCS AND TESTED ONLY AGAINST A LOCAL MOCK. Nothing here has run against
//! Cashfree's sandbox or production. Where the docs were silent or contradictory the code says so
//! ("UNCLEAR") rather than guessing quietly; the list is repeated in the pull request.
//!
//! Mapping (our terms -> Cashfree):
//! - mandate id            -> `subscription_id` (so a lost create/revoke is resolved by asking)
//! - mandate               -> an `ON_DEMAND` subscription with `plan_max_amount` = the per-debit cap and
//!   `subscription_expiry_time` = end of the mandate's end date (IST). Zyvor
//!   schedules every debit and enforces the frequency itself, so a `PERIODIC` plan
//!   (Cashfree-driven) is deliberately not used. Cashfree cannot express our start
//!   date or frequency on an on-demand plan; Zyvor enforces both.
//! - payer approval        -> an `AUTH` payment (`POST /subscriptions/pay`, UPI `link`) whose response carries
//!   per-app mandate links, plus the `subscription_session_id` for Cashfree's hosted
//!   checkout
//! - debit                 -> a `CHARGE` payment (`POST /subscriptions/pay`); `payment_id` = our attempt id
//! - revoke                -> `POST /subscriptions/{id}/manage` `{action: CANCEL}`
//! - pause / resume        -> not sent to Cashfree: the docs say `PAUSE` is unsupported for `ON_DEMAND`, and
//!   Zyvor treats a merchant pause as its own hold (the trait has no pause call)
//!
//! Idempotency: Cashfree takes `x-idempotency-key` (UUID). We derive it from our own ids, so a retry of the
//! same operation repeats the same key. A debit whose outcome is unknown is never re-sent: the caller asks
//! with `fetch_debit`, which looks the charge up by `payment_id`.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zyvor_core::Currency;

use super::{Cashfree, rupees};
use crate::*;

/// The API version Subscriptions are called with.
const SUBS_VERSION: &str = "2026-01-01";

/// Plan name shown to the payer (Cashfree allows 40 characters).
const PLAN_NAME: &str = "Zyvor AutoPay";

/// Authorization amount, in minor units. UNCLEAR: the docs show a non-zero `authorization_amount` with
/// `authorization_amount_refund: true` for UPI but do not state the minimum for an on-demand UPI plan.
const AUTH_AMOUNT_MINOR: i64 = 100;

/// A deterministic UUID-shaped key from our own ids, so the same operation always repeats the same key.
/// (Cashfree documents the header as a UUID. This is a SHA-256 truncated into a v5-shaped UUID.)
pub(crate) fn idempotency_key(scope: &str, id: &str) -> String {
    let digest = Sha256::digest(format!("zyvor-cashfree-subscriptions:{scope}:{id}").as_bytes());
    let mut b: [u8; 16] = digest[..16].try_into().expect("16 bytes");
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| v.as_i64().map(|i| i.to_string()))
}

/// Cashfree `subscription_status` -> our mandate status. Statuses are those listed on Cashfree's
/// "Create Subscription" page; an unlisted value maps to `Pending`, which changes nothing on our side.
pub(crate) fn mandate_status(subscription_status: &str, auth_status: &str) -> MandateStatus {
    match subscription_status {
        "ACTIVE" => MandateStatus::Active,
        // The payer never approved before the authorization link expired.
        "LINK_EXPIRED" => MandateStatus::Failed,
        // The plan ran out of cycles or the subscription reached its expiry.
        "COMPLETED" | "EXPIRED" | "CARD_EXPIRED" => MandateStatus::Expired,
        // The payer cancelled in their UPI app, or the merchant cancelled.
        "CUSTOMER_CANCELLED" | "CANCELLED" => MandateStatus::Revoked,
        // UNCLEAR: `ON_HOLD` is documented as a status-change trigger but not defined. Treated as a pause so
        // Zyvor stops scheduling debits; resuming it on our side does not lift Cashfree's hold.
        "ON_HOLD" | "PAUSED" | "CUSTOMER_PAUSED" => MandateStatus::Paused,
        // INITIALIZED / BANK_APPROVAL_PENDING (and anything unknown): not approved yet.
        // UNCLEAR: whether a failed authorization moves the subscription to a failed status or leaves it
        // INITIALIZED. We read `authorisation_details.authorization_status == FAILED` as a failed mandate.
        _ if auth_status == "FAILED" => MandateStatus::Failed,
        _ => MandateStatus::Pending,
    }
}

/// Cashfree `payment_status` of a CHARGE -> our payment status. UNCLEAR: the docs give no enum for
/// `payment_status`; `SUCCESS`, `PENDING`, `INITIALIZED`, `FAILED` and `CANCELLED` appear in examples and in
/// the webhook page. Anything else stays `Pending` (and so is resolved by asking, never by re-sending).
pub(crate) fn charge_status(s: &str) -> PaymentStatus {
    match s {
        "SUCCESS" => PaymentStatus::Captured,
        "FAILED" | "CANCELLED" => PaymentStatus::Failed,
        _ => PaymentStatus::Pending,
    }
}

fn failure(v: &Value) -> Option<String> {
    v["failure_details"]["failure_reason"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The IST calendar date now (Cashfree stores schedule dates in IST and ignores the time part).
fn today_ist() -> chrono::NaiveDate {
    let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("offset");
    chrono::Utc::now().with_timezone(&ist).date_naive()
}

/// Per-app mandate links from an AUTH response. The key names (`androidAuthAppLinks`, `iosAuthAppLinks`) come
/// from a docs example whose values were truncated; Android links win over iOS for the same app.
fn auth_links(p: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let data = &p["data"]["payload"]["upiIntentData"];
    for key in ["iosAuthAppLinks", "androidAuthAppLinks"] {
        for (app, link) in data[key].as_object().into_iter().flatten() {
            if link.as_str().is_some_and(|l| !l.is_empty()) {
                out.insert(app.to_lowercase(), link.clone());
            }
        }
    }
    out
}

fn entity_state(v: &Value, authorization: Option<Value>) -> MandateState {
    let auth_status = v["authorisation_details"]["authorization_status"]
        .as_str()
        .unwrap_or_default();
    let raw_status = v["subscription_status"].as_str().unwrap_or_default();
    let status = mandate_status(raw_status, auth_status);
    MandateState {
        failure_reason: (status == MandateStatus::Failed).then(|| {
            format!(
                "cashfree: {}",
                if raw_status.is_empty() {
                    "authorization failed"
                } else {
                    raw_status
                }
            )
        }),
        // cf_subscription_id is stable from creation; the UMN (authorization_reference) only appears after
        // approval and stays in `raw`.
        provider_mandate_ref: text(&v["cf_subscription_id"]),
        authorization,
        status,
        raw: v.clone(),
    }
}

fn charge_state(p: &Value, attempt_id: &str) -> PaymentState {
    let status = charge_status(p["payment_status"].as_str().unwrap_or_default());
    PaymentState {
        failure_reason: if status == PaymentStatus::Failed {
            failure(p).or_else(|| Some("cashfree: charge failed".into()))
        } else {
            None
        },
        status,
        provider_order_ref: Some(attempt_id.to_string()),
        provider_payment_ref: text(&p["cf_payment_id"]),
        raw: p.clone(),
    }
}

impl Cashfree {
    fn subs(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.req_v(m, path, SUBS_VERSION)
    }

    pub(crate) async fn sub_create_mandate(
        &self,
        req: &CreateMandate,
    ) -> Result<MandateState, ProviderError> {
        if req.currency != Currency::Inr {
            return Err(ProviderError::Rejected("UPI AutoPay is in INR".into()));
        }
        let body = json!({
            "subscription_id": req.mandate_id,
            "customer_details": {
                // UNCLEAR: `customer_email` and `customer_phone` are required by Cashfree but a mandate
                // request carries neither reliably. The email is a non-deliverable placeholder (the `.invalid`
                // TLD) and notifications are switched off below; the phone falls back as orders do.
                "customer_email": format!("{}@mandates.invalid", req.mandate_id),
                "customer_phone": req.customer_phone.clone().unwrap_or_else(|| "9999999999".into()),
            },
            "plan_details": {
                "plan_name": PLAN_NAME,
                "plan_type": "ON_DEMAND",
                "plan_currency": "INR",
                "plan_max_amount": rupees(req.max_amount_minor, req.currency),
            },
            "authorization_details": {
                "authorization_amount": rupees(AUTH_AMOUNT_MINOR, req.currency),
                "authorization_amount_refund": true,
                "payment_methods": ["upi"],
            },
            "subscription_meta": { "notification_channel": [] },
            "subscription_expiry_time": format!("{}T23:59:59+05:30", req.end_date),
        });
        let sub = read_json(
            self.subs(reqwest::Method::POST, "/subscriptions")
                .header(
                    "x-idempotency-key",
                    idempotency_key("create", &req.mandate_id),
                )
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        let session = text(&sub["subscription_session_id"]);

        // Ask for the payer's approval links. The subscription already exists, so a failure here must not
        // lose it: the mandate stays pending with the session id, which Cashfree's hosted checkout accepts.
        // UNCLEAR: the docs say the create response "includes an authorisation link" but the response schema
        // has only `subscription_session_id`; whether `upi_id` is optional for channel `link` is not stated.
        let auth = json!({
            "subscription_id": req.mandate_id,
            "subscription_session_id": session,
            "payment_id": format!("{}_auth", req.mandate_id),
            "payment_type": "AUTH",
            "payment_method": { "upi": { "channel": "link" } },
        });
        let sent = self
            .subs(reqwest::Method::POST, "/subscriptions/pay")
            .header(
                "x-idempotency-key",
                idempotency_key("auth", &req.mandate_id),
            )
            .json(&auth)
            .send()
            .await
            .map_err(classify);
        let links = match sent {
            Ok(res) => read_json(res).await.map(|p| auth_links(&p)),
            Err(e) => Err(e),
        }
        .unwrap_or_else(|e| {
            tracing::warn!(mandate = %req.mandate_id, error = %e, "cashfree AUTH link not issued");
            Default::default()
        });
        let authorization = json!({
            "type": "cashfree_upi_autopay",
            "subscription_session_id": session,
            "apps": links,
            "expires_at": null,
        });
        Ok(entity_state(&sub, Some(authorization)))
    }

    pub(crate) async fn sub_fetch_mandate(
        &self,
        mandate_id: &str,
    ) -> Result<MandateState, ProviderError> {
        let res = self
            .subs(
                reqwest::Method::GET,
                &format!("/subscriptions/{mandate_id}"),
            )
            .send()
            .await
            .map_err(classify)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(ProviderError::Rejected("not found".into()));
        }
        Ok(entity_state(&read_json(res).await?, None))
    }

    pub(crate) async fn sub_revoke_mandate(
        &self,
        mandate_id: &str,
    ) -> Result<MandateState, ProviderError> {
        let body = json!({ "subscription_id": mandate_id, "action": "CANCEL" });
        let v = read_json(
            self.subs(
                reqwest::Method::POST,
                &format!("/subscriptions/{mandate_id}/manage"),
            )
            .header("x-idempotency-key", idempotency_key("cancel", mandate_id))
            .json(&body)
            .send()
            .await
            .map_err(classify)?,
        )
        .await?;
        Ok(entity_state(&v, None))
    }

    pub(crate) async fn sub_debit(&self, req: &DebitRequest) -> Result<Initiated, ProviderError> {
        if req.currency != Currency::Inr || req.amount_minor <= 0 {
            return Err(ProviderError::Rejected("invalid_debit_amount".into()));
        }
        // UNCLEAR: `payment_schedule_date` is required for UPI and only its date is used. Whether Cashfree
        // accepts today's date, or needs a day or more ahead for its own pre-debit notification, is not
        // stated (the controlled-notification flow is a separate set of endpoints we do not use). The lead
        // is configurable; it defaults to today.
        let due = today_ist() + chrono::Days::new(self.debit_lead_days);
        let body = json!({
            "subscription_id": req.mandate_id,
            "payment_id": req.attempt_id,
            "payment_type": "CHARGE",
            "payment_amount": rupees(req.amount_minor, req.currency),
            "payment_schedule_date": format!("{due}T00:00:00"),
            "payment_remarks": "UPI AutoPay debit",
        });
        // A timeout or 5xx surfaces as Uncertain from `read_json`/`classify`; the caller then asks with
        // `fetch_debit` and never sends this again.
        let p = read_json(
            self.subs(reqwest::Method::POST, "/subscriptions/pay")
                .header(
                    "x-idempotency-key",
                    idempotency_key("debit", &req.attempt_id),
                )
                .json(&body)
                .send()
                .await
                .map_err(classify)?,
        )
        .await?;
        Ok(Initiated {
            state: charge_state(&p, &req.attempt_id),
            action: ClientAction::None,
        })
    }

    pub(crate) async fn sub_fetch_debit(
        &self,
        attempt_id: &str,
        mandate_id: &str,
    ) -> Result<PaymentState, ProviderError> {
        let res = self
            .subs(
                reqwest::Method::GET,
                &format!("/subscriptions/{mandate_id}/payments/{attempt_id}"),
            )
            .send()
            .await
            .map_err(classify)?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            // Deliberately not `Failed`. UNCLEAR: whether a charge accepted a moment ago is always visible
            // here. Calling it failed could let a second debit go out for the same day, so it stays unknown
            // and an operator decides.
            return Err(ProviderError::Uncertain(
                "cashfree has no record of this charge (yet)".into(),
            ));
        }
        Ok(charge_state(&read_json(res).await?, attempt_id))
    }
}

/// A subscription webhook (`SUBSCRIPTION_*`), or `None` when `ty` is not one. Only CHARGE payments become
/// payment events, matched on our attempt id (`payment_id`). Status changes, authorization results and
/// notifications are `Other` for now: the mandate is brought up to date by refreshing it, not by webhook.
pub(crate) fn webhook_event(ty: &str, v: &Value) -> Option<WebhookEvent> {
    if !ty.starts_with("SUBSCRIPTION_") {
        return None;
    }
    let d = &v["data"];
    let is_charge = d["payment_type"] == "CHARGE";
    let kind = match (ty, is_charge) {
        ("SUBSCRIPTION_PAYMENT_SUCCESS", true) => EventKind::PaymentCaptured,
        ("SUBSCRIPTION_PAYMENT_FAILED" | "SUBSCRIPTION_PAYMENT_CANCELLED", true) => {
            EventKind::PaymentFailed
        }
        _ => EventKind::Other,
    };
    let payment_id = text(&d["payment_id"]);
    let sub_id = text(&d["subscription_id"]);
    Some(WebhookEvent {
        // No event id is sent; the same delivery must dedupe to the same key.
        event_id: format!(
            "{ty}:{}:{}:{}",
            sub_id.unwrap_or_default(),
            payment_id.clone().unwrap_or_default(),
            v["event_time"].as_str().unwrap_or_default()
        ),
        kind,
        order_ref: payment_id,
        provider_payment_ref: text(&d["cf_payment_id"]),
        provider_refund_ref: None,
        refund_ref: None,
        amount_minor: d["payment_amount"]
            .as_f64()
            .map(|f| (f * 100.0).round() as i64),
        failure_reason: (kind == EventKind::PaymentFailed)
            .then(|| failure(d).unwrap_or_else(|| ty.to_lowercase())),
        raw: v.clone(),
    })
}
