// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Payer-instrument rules: only let a listed payer bank or UPI handle pay, and refund a payment that came from
//! anywhere else.
//!
//! A rule is `{ "allowed_banks": ["HDFC", "SBI"], "allowed_vpa_handles": ["okhdfcbank"] }` on a payment link
//! (`instrument_rules`, or `PUT /v1/payment_links/{id}/instrument_rules`) or on a payment intent
//! (`PUT /v1/payment_intents/{id}/instrument_rules`). An intent's own rule wins over its link's.
//!
//! **What this can and cannot do.** The rule is applied after the money has been captured, not before: the
//! payer's bank or UPI id is only known when the provider says so on the payment record or webhook, and no
//! provider adapter here lets us refuse a payer's UPI id before they pay. So a disallowed payment is captured,
//! then refunded through the normal refund flow (the merchant's refund approval threshold applies, and the refund
//! is requested by `system:instrument_rules`), a reconciliation case is opened, and `payment.instrument_refused`
//! is raised. Whether a refund returns to the payer's own account is the bank's business.
//!
//! **When the provider does not say who paid**, nothing can be checked. The result is `unverifiable`, the payment
//! is kept, and that is visible in `GET /v1/instrument_checks`. It is never a silent pass presented as a check.
//! Today Razorpay (`vpa`, `bank`) and Cashfree (`payment_method.upi.upi_id`,
//! `payment_method.netbanking.netbanking_bank_code`) are read from their documented payment fields, and the
//! simulator reports a VPA and a guessed bank; PayU, PhonePe and custom providers report nothing here. The
//! Razorpay and Cashfree readings were checked against this repository's mocks only, not a live account.
//!
//! **How the two lists combine.** Each list is applied only to the attribute the provider reported. If it
//! reported a VPA, the VPA handle must be on `allowed_vpa_handles` (when that list is set). If it reported a
//! bank, the bank must be on `allowed_banks` (when that list is set). A payment is disallowed when any applicable
//! list rejects it, allowed when at least one list was applicable and all accepted, and unverifiable otherwise.
//! A payer paying by UPI from a bank not on `allowed_banks` is only caught if the provider reports the bank.
//!
//! The worker runs `sweep_tenant` every cycle; a payment is checked once.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zyvor_providers::PayerInstrument;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, tenant};

#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    /// Bank codes or names as your provider reports them, like `HDFC` or `SBI`. Compared ignoring case.
    #[serde(default)]
    pub allowed_banks: Vec<String>,
    /// UPI handles (the part after the `@`), like `okhdfcbank`. A leading `@` is accepted. Compared ignoring case.
    #[serde(default)]
    pub allowed_vpa_handles: Vec<String>,
}

impl Rules {
    pub fn is_empty(&self) -> bool {
        self.allowed_banks.is_empty() && self.allowed_vpa_handles.is_empty()
    }
    /// The lists as they are stored: trimmed, lower-case, handles without `@`, no duplicates.
    pub fn normalised(&self) -> Rules {
        let clean = |v: &Vec<String>, at: bool| {
            let mut out: Vec<String> = v
                .iter()
                .map(|x| {
                    let x = x.trim().to_lowercase();
                    if at {
                        x.trim_start_matches('@').to_string()
                    } else {
                        x
                    }
                })
                .collect();
            out.sort();
            out.dedup();
            out
        };
        Rules {
            allowed_banks: clean(&self.allowed_banks, false),
            allowed_vpa_handles: clean(&self.allowed_vpa_handles, true),
        }
    }
}

pub fn validate(r: &Rules) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::invalid("invalid_instrument_rules", m.to_string()));
    if r.allowed_banks.len() > 50 || r.allowed_vpa_handles.len() > 50 {
        return bad("at most 50 entries in each list");
    }
    let n = r.normalised();
    if n.allowed_banks.iter().any(|b| {
        b.is_empty()
            || b.len() > 40
            || !b
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '&' | '.'))
    }) {
        return bad("a bank is a code or name of letters and digits, up to 40 characters");
    }
    if n.allowed_vpa_handles.iter().any(|h| {
        h.len() < 2 || h.len() > 30 || !h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
    }) {
        return bad("a UPI handle is the part after the @, like okhdfcbank");
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Disallowed(String),
    Unverifiable,
}

/// Apply the rules to what the provider reported. Pure; see the module docs for how the lists combine.
pub fn evaluate(rules: &Rules, payer: Option<&PayerInstrument>) -> Verdict {
    let r = rules.normalised();
    let Some(p) = payer else {
        return Verdict::Unverifiable;
    };
    let mut applicable = false;
    if !r.allowed_vpa_handles.is_empty()
        && let Some(v) = &p.vpa
    {
        applicable = true;
        let handle = v
            .rsplit_once('@')
            .map(|(_, h)| h.to_lowercase())
            .unwrap_or_default();
        if !r.allowed_vpa_handles.contains(&handle) {
            return Verdict::Disallowed(format!("UPI handle @{handle} is not on the allowed list"));
        }
    }
    if !r.allowed_banks.is_empty()
        && let Some(b) = &p.bank
    {
        applicable = true;
        if !r.allowed_banks.contains(&b.trim().to_lowercase()) {
            return Verdict::Disallowed(format!("payer bank {b} is not on the allowed list"));
        }
    }
    if applicable {
        Verdict::Allowed
    } else {
        Verdict::Unverifiable
    }
}

/// The payer's instrument from everything a payment attempt has recorded: its creation response and each
/// polled or webhook body kept under the source's name.
pub fn payer_from_attempt_raw(provider: &str, raw: &Value) -> Option<PayerInstrument> {
    let mut found = PayerInstrument::default();
    let mut take = |v: &Value| {
        if let Some(p) = zyvor_providers::payer_instrument(provider, v) {
            found.vpa = found.vpa.take().or(p.vpa);
            found.bank = found.bank.take().or(p.bank);
        }
    };
    take(raw);
    if let Some(o) = raw.as_object() {
        for v in o.values().filter(|v| v.is_object()) {
            take(v);
        }
    }
    (found.vpa.is_some() || found.bank.is_some()).then_some(found)
}

fn handle_of(v: &str) -> Option<String> {
    v.rsplit_once('@').map(|(_, h)| h.to_lowercase())
}

#[derive(Debug, Default)]
pub struct Swept {
    pub checked: usize,
    pub refused: usize,
    pub unverifiable: usize,
}

/// One pass for a business: check captured payments that fall under a rule and have not been checked, and finish the
/// refund of any disallowed one whose refund did not get made (a provider outage, a restart).
pub async fn sweep_tenant(s: &AppState, tenant_id: &str, env: &str) -> Result<Swept, ApiError> {
    let mut out = Swept::default();
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let todo: Vec<(String, String, String, Value, Value, Option<String>)> = sqlx::query_as(
        "SELECT a.id, a.intent_id, a.provider, a.raw, COALESCE(i.instrument_rules, l.instrument_rules), l.id
           FROM payment_attempts a
           JOIN payment_intents i ON i.id = a.intent_id
           LEFT JOIN payment_links l ON l.id = i.metadata->>'payment_link'
          WHERE a.status = 'captured' AND a.updated_at > now() - interval '7 days'
            AND COALESCE(i.instrument_rules, l.instrument_rules) IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM instrument_checks c WHERE c.attempt_id = a.id)
          ORDER BY a.updated_at LIMIT 50",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (attempt_id, intent_id, provider, raw, rules, link_id) in todo {
        let rules: Rules = serde_json::from_value(rules).unwrap_or_default();
        let payer = payer_from_attempt_raw(&provider, &raw);
        let verdict = evaluate(&rules, payer.as_ref());
        let (result, detail) = match &verdict {
            Verdict::Allowed => ("allowed", String::new()),
            Verdict::Unverifiable => (
                "unverifiable",
                "the provider did not report the payer's bank or UPI id, so the rule could not be applied".to_string(),
            ),
            Verdict::Disallowed(why) => ("disallowed", why.clone()),
        };
        // the handle only, never the name part of the UPI id
        sqlx::query("INSERT INTO instrument_checks (tenant_id, attempt_id, intent_id, link_id, result, payer_bank, payer_vpa_handle, detail) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
            .bind(tenant_id)
            .bind(&attempt_id)
            .bind(&intent_id)
            .bind(&link_id)
            .bind(result)
            .bind(payer.as_ref().and_then(|p| p.bank.clone()))
            .bind(payer.as_ref().and_then(|p| p.vpa.as_deref()).and_then(handle_of))
            .bind(&detail)
            .execute(&mut *tx)
            .await?;
        out.checked += 1;
        match verdict {
            Verdict::Unverifiable => out.unverifiable += 1,
            Verdict::Allowed => {}
            Verdict::Disallowed(why) => {
                out.refused += 1;
                open_case(
                    &mut tx,
                    tenant_id,
                    "disallowed_instrument",
                    "payment_attempt",
                    &attempt_id,
                    None,
                    None,
                    &format!("A payment came from a payer bank or UPI handle your rule does not allow ({why}). It is being refunded."),
                    json!({ "intent_id": intent_id, "link_id": link_id }),
                )
                .await?;
                tenant::emit(
                    &mut tx,
                    tenant_id,
                    "payment.instrument_refused",
                    json!({ "intent_id": intent_id, "attempt_id": attempt_id, "reason": why }),
                )
                .await?;
                tenant::audit(
                    &mut tx,
                    tenant_id,
                    "system:instrument_rules",
                    "payment.instrument_refused",
                    Some(&attempt_id),
                    json!({ "reason": why }),
                )
                .await?;
            }
        }
    }
    tx.commit().await?;
    finish_refunds(s, tenant_id, env).await?;
    Ok(out)
}

/// Refund every disallowed payment that has no refund yet. Safe to repeat: the refund carries a fixed idempotency key
/// and the reference `instrument:<attempt>`.
async fn finish_refunds(s: &AppState, tenant_id: &str, env: &str) -> Result<(), ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let pending: Vec<(String, String)> = sqlx::query_as(
        "SELECT attempt_id, intent_id FROM instrument_checks WHERE result = 'disallowed' AND refund_id IS NULL ORDER BY checked_at LIMIT 50",
    )
    .fetch_all(&mut *tx)
    .await?;
    drop(tx);
    for (attempt_id, intent_id) in pending {
        let auth = Auth {
            tenant_id: tenant_id.to_string(),
            key_id: "system:instrument_rules".into(),
            env: env.to_string(),
            scopes: vec!["refunds:write".into()],
            actor: "system:instrument_rules".into(),
            branch_id: None,
        };
        let reference = format!("instrument:{attempt_id}");
        let mut h = HeaderMap::new();
        if let Ok(v) = reference.parse() {
            h.insert("idempotency-key", v);
        }
        let body: Option<Json<crate::refunds::CreateRefund>> = serde_json::from_value(
            json!({ "reason": "payer instrument not allowed", "external_ref": reference }),
        )
        .ok()
        .map(Json);
        match crate::refunds::create_refund(
            State(s.clone()),
            auth,
            Path(intent_id.clone()),
            h,
            body,
        )
        .await
        {
            Ok(_) => {
                let mut tx = tenant::begin(&s.pool, tenant_id).await?;
                sqlx::query("UPDATE instrument_checks SET refund_id = (SELECT id FROM refunds WHERE intent_id = $2 AND external_ref = $3 LIMIT 1) WHERE attempt_id = $1")
                    .bind(&attempt_id)
                    .bind(&intent_id)
                    .bind(&reference)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
            }
            Err(e) => {
                tracing::warn!(attempt = %attempt_id, error = ?e.message, "refund of a disallowed payer instrument not made yet; will retry");
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct SetRules {
    /// Empty lists remove the rule.
    #[serde(default)]
    allowed_banks: Vec<String>,
    #[serde(default)]
    allowed_vpa_handles: Vec<String>,
}

fn into_rules(b: SetRules) -> Result<(Rules, Option<Value>), ApiError> {
    let r = Rules {
        allowed_banks: b.allowed_banks,
        allowed_vpa_handles: b.allowed_vpa_handles,
    };
    validate(&r)?;
    let n = r.normalised();
    let stored = (!n.is_empty()).then(|| json!(n));
    Ok((n, stored))
}

/// `PUT /v1/payment_links/{id}/instrument_rules`
pub async fn set_link_rules(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<SetRules>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let (rules, stored) = into_rules(b)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE payment_links SET instrument_rules = $2 WHERE id = $1")
        .bind(&id)
        .bind(&stored)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("payment link"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_link.instrument_rules_set",
        Some(&id),
        json!({ "banks": rules.allowed_banks.len(), "handles": rules.allowed_vpa_handles.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "instrument_rules": stored })))
}

/// `PUT /v1/payment_intents/{id}/instrument_rules`: only while the payment has not succeeded.
pub async fn set_intent_rules(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<SetRules>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let (rules, stored) = into_rules(b)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM payment_intents WHERE id = $1 FOR UPDATE")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    match status.as_deref() {
        None => return Err(ApiError::not_found("payment intent")),
        Some("requires_payment_method" | "processing") => {}
        Some(other) => {
            return Err(ApiError::conflict(
                "intent_not_open",
                format!("the payment is {other}; a rule can only be set before it succeeds"),
            ));
        }
    }
    sqlx::query("UPDATE payment_intents SET instrument_rules = $2 WHERE id = $1")
        .bind(&id)
        .bind(&stored)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payment_intent.instrument_rules_set",
        Some(&id),
        json!({ "banks": rules.allowed_banks.len(), "handles": rules.allowed_vpa_handles.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "instrument_rules": stored })))
}

#[derive(Deserialize, Default)]
pub struct ChecksQuery {
    result: Option<String>,
    limit: Option<i64>,
}

/// `GET /v1/instrument_checks`: what was checked, including payments that could not be (`unverifiable`).
pub async fn list_checks(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ChecksQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Option<String>, String, Option<String>, Option<String>, Option<String>, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT attempt_id, intent_id, link_id, result, payer_bank, payer_vpa_handle, refund_id, detail, checked_at
           FROM instrument_checks WHERE ($1::text IS NULL OR result = $1) ORDER BY checked_at DESC LIMIT $2",
    )
    .bind(&q.result)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(a, i, l, r, b, h, rf, d, at)| json!({
        "attempt_id": a, "intent_id": i, "link_id": l, "result": r, "payer_bank": b,
        "payer_vpa_handle": h, "refund_id": rf, "detail": d, "checked_at": at })).collect::<Vec<_>>() }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(banks: &[&str], handles: &[&str]) -> Rules {
        Rules {
            allowed_banks: banks.iter().map(|s| s.to_string()).collect(),
            allowed_vpa_handles: handles.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn payer(vpa: Option<&str>, bank: Option<&str>) -> Option<PayerInstrument> {
        Some(PayerInstrument {
            vpa: vpa.map(str::to_string),
            bank: bank.map(str::to_string),
        })
    }

    #[test]
    fn a_listed_handle_or_bank_passes_and_others_do_not() {
        let r = rules(&[], &["@okhdfcbank", "OKSBI"]);
        assert_eq!(
            evaluate(&r, payer(Some("a@OKHDFCBANK"), None).as_ref()),
            Verdict::Allowed
        );
        assert!(matches!(
            evaluate(&r, payer(Some("a@ybl"), None).as_ref()),
            Verdict::Disallowed(_)
        ));
        let b = rules(&["HDFC", "sbi"], &[]);
        assert_eq!(
            evaluate(&b, payer(None, Some("hdfc")).as_ref()),
            Verdict::Allowed
        );
        assert!(matches!(
            evaluate(&b, payer(None, Some("ICIC")).as_ref()),
            Verdict::Disallowed(_)
        ));
    }

    #[test]
    fn what_the_provider_did_not_report_cannot_be_checked() {
        let r = rules(&["HDFC"], &["okhdfcbank"]);
        assert_eq!(evaluate(&r, None), Verdict::Unverifiable);
        assert_eq!(
            evaluate(&r, payer(None, None).as_ref()),
            Verdict::Unverifiable
        );
        // a bank rule alone and a payer known only by VPA: nothing applicable
        assert_eq!(
            evaluate(&rules(&["HDFC"], &[]), payer(Some("a@ybl"), None).as_ref()),
            Verdict::Unverifiable
        );
    }

    #[test]
    fn every_applicable_list_must_accept() {
        let r = rules(&["HDFC"], &["okhdfcbank"]);
        assert_eq!(
            evaluate(&r, payer(Some("a@okhdfcbank"), Some("HDFC")).as_ref()),
            Verdict::Allowed
        );
        assert!(matches!(
            evaluate(&r, payer(Some("a@okhdfcbank"), Some("ICIC")).as_ref()),
            Verdict::Disallowed(_)
        ));
        assert!(matches!(
            evaluate(&r, payer(Some("a@ybl"), Some("HDFC")).as_ref()),
            Verdict::Disallowed(_)
        ));
    }

    #[test]
    fn rules_are_validated_and_normalised() {
        assert!(validate(&rules(&["HDFC"], &["@okhdfcbank"])).is_ok());
        assert!(validate(&rules(&["bad;bank"], &[])).is_err());
        assert!(validate(&rules(&[], &["x"])).is_err());
        assert!(validate(&rules(&[], &["has space"])).is_err());
        let long = vec!["HDFC".to_string(); 51];
        assert!(
            validate(&Rules {
                allowed_banks: long,
                ..Default::default()
            })
            .is_err()
        );
        let n = rules(&["HDFC", "hdfc "], &["@OkHdfc", "okhdfc"]).normalised();
        assert_eq!(n.allowed_banks, vec!["hdfc"]);
        assert_eq!(n.allowed_vpa_handles, vec!["okhdfc"]);
    }

    #[test]
    fn the_payer_is_read_from_any_recorded_body() {
        let raw = json!({ "webhook": { "payload": { "payment": { "entity": { "vpa": "x@okhdfcbank", "bank": null } } } }, "sim": true });
        let p = payer_from_attempt_raw("razorpay", &raw).unwrap();
        assert_eq!(p.vpa.as_deref(), Some("x@okhdfcbank"));
        assert!(payer_from_attempt_raw("razorpay", &json!({ "id": "pay_1" })).is_none());
        assert!(payer_from_attempt_raw("phonepe", &raw).is_none());
        let cf = json!({ "poll": { "data": { "payment": { "payment_method": { "upi": { "upi_id": "y@ybl" } } } } } });
        assert_eq!(
            payer_from_attempt_raw("cashfree", &cf)
                .unwrap()
                .vpa
                .as_deref(),
            Some("y@ybl")
        );
    }
}
