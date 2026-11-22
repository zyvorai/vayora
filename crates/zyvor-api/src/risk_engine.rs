// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! The risk rules engine. It extends the hard limits and the score in `risk.rs`; it does not replace them.
//!
//! **What it looks at**, per payment attempt: the customer id, a device token the checkout sends in the
//! `X-Zyvor-Device` header (opaque, at most 128 characters; Zyvor does not make one), the customer's phone and
//! e-mail, the payer's network address (only when `ZYVOR_TRUST_PROXY_HOPS` or the console's signed header says what it
//! is), the card (a hash of the network-token reference, never a card number), the UPI id, the amount and the
//! method. Each value is turned into a keyed hash (`keyed.rs`) before it is stored or compared; no phone number,
//! address or e-mail is kept in clear by this engine, none appears in a log line, and the signal rows are deleted after
//! seven days.
//!
//! **Rules** (a business's own, created through the API):
//! - *velocity*: for one dimension (customer, device, phone, email, ip, card, vpa), over a window of 60 seconds to
//!   7 days, count the attempts (`attempts`), add up the amounts (`amount_minor`), or count the distinct customers
//!   (`distinct_customers`, for a device, card, phone, email, ip or vpa shared by many people). It fires when the
//!   total **including the attempt being made** is above `max`.
//! - *amount*: fires when the amount is above `above_minor` or below `below_minor`, optionally for one method.
//!
//! Each rule has an action, `review` or `decline`. **Lists**: a block-list entry (a hashed value in one dimension)
//! declines; an allow-list entry stops the velocity and amount rules from firing for that attempt. Block wins over
//! allow. The allow-list does not switch off the hard limits in `risk.rs` (ceiling, attempts, blocked VPAs, score).
//!
//! **Decision**: the strongest action among the rules that fired. `decline` refuses the attempt with
//! `risk_blocked` (audited, `payment.risk_blocked`). `review` lets the payment go ahead **and** opens a case
//! (`risk_review`, see `GET /v1/risk/review_queue`) so a person can look at it, and raises `risk.review_required`;
//! this is a review after the fact, not a hold, because a payer at checkout cannot be held for a person. A merchant who
//! wants a hold uses manual capture and decides when the case is read. Every decision that fired anything is stored
//! with the names of the rules and the numbers they saw (`GET /v1/risk/decisions`), and `POST /v1/risk/evaluate`
//! shows what would happen for given values without recording anything.
//!
//! A rule that needs a signal the attempt did not carry (no device token, no customer) is skipped and listed under
//! `skipped`, rather than treated as passed or failed.
//!
//! Not in here: any machine-learned model, a card-network score, device fingerprinting beyond the token you send.
//! Limits are only as good as the signals: a person who changes device token, phone and card for each attempt is not
//! caught by velocity on those.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::{
    ApiError, AppState,
    auth::Auth,
    cases::open_case,
    keyed,
    risk::{self, Action},
    tenant,
};

pub const MAX_RULES: i64 = 50;
pub const MAX_LIST_ENTRIES: i64 = 5_000;
pub const RETENTION_DAYS: i32 = 7;
pub const DEVICE_HEADER: &str = "x-zyvor-device";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Customer,
    Device,
    Phone,
    Email,
    Ip,
    Card,
    Vpa,
}

pub const DIMS: [Dim; 7] = [
    Dim::Customer,
    Dim::Device,
    Dim::Phone,
    Dim::Email,
    Dim::Ip,
    Dim::Card,
    Dim::Vpa,
];

impl Dim {
    pub fn parse(s: &str) -> Option<Self> {
        DIMS.into_iter().find(|d| d.as_str() == s)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Dim::Customer => "customer",
            Dim::Device => "device",
            Dim::Phone => "phone",
            Dim::Email => "email",
            Dim::Ip => "ip",
            Dim::Card => "card",
            Dim::Vpa => "vpa",
        }
    }
    /// A fixed column name, never user text.
    fn column(self) -> &'static str {
        match self {
            Dim::Customer => "customer_h",
            Dim::Device => "device_h",
            Dim::Phone => "phone_h",
            Dim::Email => "email_h",
            Dim::Ip => "ip_h",
            Dim::Card => "card_h",
            Dim::Vpa => "vpa_h",
        }
    }
}

/// What one attempt carries. Values are in clear only for the moment they are hashed.
#[derive(Debug, Clone, Default)]
pub struct Signals {
    pub amount_minor: i64,
    pub method: String,
    pub customer: Option<String>,
    pub device: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub ip: Option<String>,
    pub card: Option<String>,
    pub vpa: Option<String>,
}

impl Signals {
    fn get(&self, d: Dim) -> Option<&str> {
        match d {
            Dim::Customer => &self.customer,
            Dim::Device => &self.device,
            Dim::Phone => &self.phone,
            Dim::Email => &self.email,
            Dim::Ip => &self.ip,
            Dim::Card => &self.card,
            Dim::Vpa => &self.vpa,
        }
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct VelocityParams {
    /// customer, device, phone, email, ip, card or vpa.
    pub dimension: String,
    /// `attempts`, `amount_minor` or `distinct_customers`.
    pub metric: String,
    /// 60 to 604800.
    pub window_secs: i64,
    /// Fires when the total, including this attempt, is above this.
    pub max: i64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct AmountParams {
    pub above_minor: Option<i64>,
    pub below_minor: Option<i64>,
    /// Only for this method (`upi`, `card`, `netbanking`, `wallet`); all methods when absent.
    pub method: Option<String>,
}

pub fn validate_velocity(p: &VelocityParams) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::invalid("invalid_risk_rule", m.to_string()));
    let Some(d) = Dim::parse(&p.dimension) else {
        return bad("dimension must be customer, device, phone, email, ip, card or vpa");
    };
    if !matches!(
        p.metric.as_str(),
        "attempts" | "amount_minor" | "distinct_customers"
    ) {
        return bad("metric must be attempts, amount_minor or distinct_customers");
    }
    if p.metric == "distinct_customers" && d == Dim::Customer {
        return bad(
            "distinct_customers counts customers per device, phone, email, ip, card or vpa",
        );
    }
    if !(60..=7 * 86_400).contains(&p.window_secs) {
        return bad("window_secs must be between 60 and 604800");
    }
    if p.max < 1 || p.max > zyvor_core::MAX_AMOUNT_MINOR {
        return bad("max must be a positive whole number");
    }
    Ok(())
}

pub fn validate_amount(p: &AmountParams) -> Result<(), ApiError> {
    let bad = |m: &str| Err(ApiError::invalid("invalid_risk_rule", m.to_string()));
    if p.above_minor.is_none() && p.below_minor.is_none() {
        return bad("give above_minor, below_minor or both");
    }
    for v in [p.above_minor, p.below_minor].into_iter().flatten() {
        if !(1..=zyvor_core::MAX_AMOUNT_MINOR).contains(&v) {
            return bad("amounts must be positive whole minor units");
        }
    }
    if p.above_minor.zip(p.below_minor).is_some_and(|(a, b)| b > a) {
        // below 100 or above 50 would fire on everything between
        return bad("below_minor is above above_minor, so every amount would fire");
    }
    if p.method
        .as_deref()
        .is_some_and(|m| !matches!(m, "upi" | "card" | "netbanking" | "wallet"))
    {
        return bad("method must be upi, card, netbanking or wallet");
    }
    Ok(())
}

/// The decision for one attempt.
#[derive(Debug, Clone)]
pub struct Decision {
    pub action: Action,
    pub fired: Vec<Value>,
    pub skipped: Vec<String>,
    /// Which dimension's allow-list entry matched, if one did.
    pub allowlisted: Option<&'static str>,
}

impl Decision {
    pub fn json(&self) -> Value {
        json!({ "action": self.action.as_str(), "fired": self.fired, "skipped": self.skipped, "allowlisted": self.allowlisted })
    }
    /// The first rule that asked for the winning action, as a short label for the refusal.
    fn label(&self) -> String {
        self.fired
            .iter()
            .find(|f| f["action"] == self.action.as_str())
            .and_then(|f| f["rule"].as_str())
            .unwrap_or("risk_engine")
            .to_string()
    }
}

struct Hashed {
    by_dim: Vec<(Dim, String)>,
}

impl Hashed {
    fn new(key: &[u8; 32], sig: &Signals) -> Self {
        Self {
            by_dim: DIMS
                .into_iter()
                .filter_map(|d| sig.get(d).map(|v| (d, keyed::hash(key, d.as_str(), v))))
                .collect(),
        }
    }
    fn get(&self, d: Dim) -> Option<&str> {
        self.by_dim
            .iter()
            .find(|(x, _)| *x == d)
            .map(|(_, h)| h.as_str())
    }
}

/// Evaluate the business's rules and lists for these signals. Reads only; the caller records.
pub async fn evaluate(
    conn: &mut PgConnection,
    crypto: &crate::Crypto,
    tenant_id: &str,
    sig: &Signals,
) -> Result<Decision, ApiError> {
    let key = keyed::tenant_key(conn, crypto, tenant_id).await?;
    let h = Hashed::new(&key, sig);
    let mut d = Decision {
        action: Action::Allow,
        fired: Vec::new(),
        skipped: Vec::new(),
        allowlisted: None,
    };
    // lists first: block beats allow
    for list in ["block", "allow"] {
        for (dim, hash) in &h.by_dim {
            let hit: Option<String> = sqlx::query_scalar(
                "SELECT id FROM risk_list_entries WHERE list = $1 AND dimension = $2 AND value_hash = $3 AND (expires_at IS NULL OR expires_at > now())",
            )
            .bind(list)
            .bind(dim.as_str())
            .bind(hash)
            .fetch_optional(&mut *conn)
            .await?;
            if hit.is_some() {
                if list == "block" {
                    d.action = Action::Decline;
                    d.fired.push(json!({ "rule": format!("blocklist:{}", dim.as_str()), "kind": "list", "action": "decline", "dimension": dim.as_str() }));
                } else if d.allowlisted.is_none() {
                    d.allowlisted = Some(dim.as_str());
                }
            }
        }
        if list == "block" && d.action == Action::Decline {
            return Ok(d);
        }
    }
    if let Some(dim) = d.allowlisted {
        d.fired.push(json!({ "rule": format!("allowlist:{dim}"), "kind": "list", "action": "allow", "dimension": dim }));
        return Ok(d);
    }
    let rules: Vec<(String, String, Value, String)> = sqlx::query_as(
        "SELECT name, kind, params, action FROM risk_rules WHERE enabled ORDER BY created_at, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (name, kind, params, action) in rules {
        let Some(action) = Action::parse(&action) else {
            continue;
        };
        match kind.as_str() {
            "velocity" => {
                let Ok(p) = serde_json::from_value::<VelocityParams>(params) else {
                    continue;
                };
                let Some(dim) = Dim::parse(&p.dimension) else {
                    continue;
                };
                let Some(hash) = h.get(dim) else {
                    d.skipped
                        .push(format!("{name}: no {} on this attempt", dim.as_str()));
                    continue;
                };
                let col = dim.column();
                let observed: i64 = match p.metric.as_str() {
                    "attempts" => {
                        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                            "SELECT count(*) FROM risk_signals WHERE {col} = $1 AND at > now() - make_interval(secs => $2::float8)"
                        )))
                        .bind(hash)
                        .bind(p.window_secs as f64)
                        .fetch_one(&mut *conn)
                        .await?
                            + 1
                    }
                    "amount_minor" => {
                        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                            "SELECT COALESCE(sum(amount_minor), 0)::BIGINT FROM risk_signals WHERE {col} = $1 AND at > now() - make_interval(secs => $2::float8)"
                        )))
                        .bind(hash)
                        .bind(p.window_secs as f64)
                        .fetch_one(&mut *conn)
                        .await?
                            + sig.amount_minor
                    }
                    _ => {
                        let Some(cust) = h.get(Dim::Customer) else {
                            d.skipped.push(format!("{name}: no customer on this attempt"));
                            continue;
                        };
                        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                            "SELECT count(*) FROM (SELECT customer_h FROM risk_signals WHERE {col} = $1 AND customer_h IS NOT NULL AND at > now() - make_interval(secs => $2::float8) UNION SELECT $3::text) x"
                        )))
                        .bind(hash)
                        .bind(p.window_secs as f64)
                        .bind(cust)
                        .fetch_one(&mut *conn)
                        .await?
                    }
                };
                if observed > p.max {
                    d.fired.push(json!({ "rule": name, "kind": "velocity", "action": action.as_str(), "dimension": p.dimension, "metric": p.metric, "window_secs": p.window_secs, "observed": observed, "limit": p.max }));
                    d.action = d.action.max(action);
                }
            }
            "amount" => {
                let Ok(p) = serde_json::from_value::<AmountParams>(params) else {
                    continue;
                };
                if p.method.as_deref().is_some_and(|m| m != sig.method) {
                    continue;
                }
                let hit = p.above_minor.is_some_and(|a| sig.amount_minor > a)
                    || p.below_minor.is_some_and(|b| sig.amount_minor < b);
                if hit {
                    d.fired.push(json!({ "rule": name, "kind": "amount", "action": action.as_str(), "observed": sig.amount_minor, "above_minor": p.above_minor, "below_minor": p.below_minor, "method": p.method }));
                    d.action = d.action.max(action);
                }
            }
            _ => {}
        }
    }
    Ok(d)
}

async fn insert_signal(
    conn: &mut PgConnection,
    crypto: &crate::Crypto,
    tenant_id: &str,
    intent_id: &str,
    sig: &Signals,
) -> Result<(), ApiError> {
    let key = keyed::tenant_key(conn, crypto, tenant_id).await?;
    let h = Hashed::new(&key, sig);
    sqlx::query("INSERT INTO risk_signals (id, tenant_id, intent_id, amount_minor, customer_h, device_h, phone_h, email_h, ip_h, card_h, vpa_h) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(zyvor_core::new_id("rs"))
        .bind(tenant_id)
        .bind(intent_id)
        .bind(sig.amount_minor)
        .bind(h.get(Dim::Customer))
        .bind(h.get(Dim::Device))
        .bind(h.get(Dim::Phone))
        .bind(h.get(Dim::Email))
        .bind(h.get(Dim::Ip))
        .bind(h.get(Dim::Card))
        .bind(h.get(Dim::Vpa))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn record_decision(
    conn: &mut PgConnection,
    tenant_id: &str,
    subject_type: &str,
    subject_id: &str,
    action: Action,
    score: Option<i64>,
    fired: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO risk_decisions (id, tenant_id, subject_type, subject_id, action, score, fired) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(zyvor_core::new_id("rd"))
        .bind(tenant_id)
        .bind(subject_type)
        .bind(subject_id)
        .bind(action.as_str())
        .bind(score.map(|s| s as i32))
        .bind(fired)
        .execute(conn)
        .await?;
    Ok(())
}

/// Called by `payments::start_attempt` after the hard limits pass. Records what was seen, then either lets the
/// attempt go (`None`, having opened a review case if a rule asked for one) or returns the refusal
/// `(rule label, detail)`; the caller abandons its transaction on a refusal, so the decision is stored separately.
#[allow(clippy::too_many_arguments)]
pub async fn screen_attempt(
    s: &AppState,
    tx: &mut PgConnection,
    tenant_id: &str,
    headers: &HeaderMap,
    intent_id: &str,
    customer_id: Option<&str>,
    vpa: Option<&str>,
    card_token_id: Option<&str>,
    amount_minor: i64,
    method: &str,
) -> Result<Option<(String, Value)>, ApiError> {
    let mut sig = Signals {
        amount_minor,
        method: method.to_string(),
        customer: customer_id.map(str::to_string),
        vpa: vpa.map(str::to_string),
        device: headers
            .get(DEVICE_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .map(str::to_string),
        ip: crate::ratelimit::vouched_client_ip(s.actor_secret.as_deref(), headers)
            .or_else(|| {
                crate::ratelimit::client_ip(headers, &axum::http::Extensions::new(), s.trusted_hops)
            })
            .map(|ip| ip.to_string()),
        ..Default::default()
    };
    if let Some(c) = customer_id {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT phone, email FROM customers WHERE id = $1")
                .bind(c)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((phone, email)) = row {
            sig.phone = phone;
            sig.email = email;
        }
    }
    if let Some(t) = card_token_id {
        sig.card = sqlx::query_scalar("SELECT network_token_ref FROM card_tokens WHERE id = $1")
            .bind(t)
            .fetch_optional(&mut *tx)
            .await?;
    }
    let d = evaluate(tx, &s.crypto, tenant_id, &sig).await?;
    if !d.fired.is_empty() {
        let fired = json!(d.fired);
        match d.action {
            Action::Decline => {
                // the caller's transaction is about to be abandoned, so store the decision on its own
                if let Ok(mut own) = tenant::begin(&s.pool, tenant_id).await {
                    let _ = record_decision(
                        &mut own,
                        tenant_id,
                        "payment_intent",
                        intent_id,
                        d.action,
                        None,
                        &fired,
                    )
                    .await;
                    let _ = own.commit().await;
                }
                return Ok(Some((
                    d.label(),
                    json!({ "intent_id": intent_id, "decision": d.json() }),
                )));
            }
            Action::Review => {
                record_decision(
                    tx,
                    tenant_id,
                    "payment_intent",
                    intent_id,
                    d.action,
                    None,
                    &fired,
                )
                .await?;
                open_case(
                    tx,
                    tenant_id,
                    "risk_review",
                    "payment_intent",
                    intent_id,
                    None,
                    None,
                    &format!("A risk rule asked for a person to look at this payment ({}). It was allowed to go ahead.", d.label()),
                    json!({ "decision": d.json() }),
                )
                .await?;
                tenant::emit(
                    tx,
                    tenant_id,
                    "risk.review_required",
                    json!({ "subject_type": "payment_intent", "subject_id": intent_id, "rule": d.label() }),
                )
                .await?;
            }
            Action::Allow => {
                record_decision(
                    tx,
                    tenant_id,
                    "payment_intent",
                    intent_id,
                    d.action,
                    None,
                    &fired,
                )
                .await?;
            }
        }
    }
    insert_signal(tx, &s.crypto, tenant_id, intent_id, &sig).await?;
    Ok(None)
}

/// The worker's housekeeping: drop signal rows past retention.
pub async fn sweep_tenant(s: &AppState, tenant_id: &str) -> Result<u64, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let n = sqlx::query("DELETE FROM risk_signals WHERE at < now() - make_interval(days => $1)")
        .bind(RETENTION_DAYS)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok(n)
}

// ---- API: rules ------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NewRiskRule {
    /// A short unique name; it is what the decision and the refusal show.
    name: String,
    /// `velocity` or `amount`.
    kind: String,
    /// The rule's parameters: see `VelocityParams` and `AmountParams` in the docs.
    params: Value,
    /// `review` or `decline`. (Allow is what the allow-list is for.)
    action: String,
}

/// `POST /v1/risk/rules`
pub async fn create_rule(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewRiskRule>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("risk:write")?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 60 || name.chars().any(char::is_control) {
        return Err(ApiError::invalid(
            "invalid_risk_rule",
            "name is 1-60 characters",
        ));
    }
    let action = Action::parse(&b.action)
        .filter(|a| *a != Action::Allow)
        .ok_or_else(|| {
            ApiError::invalid("invalid_risk_rule", "action must be review or decline")
        })?;
    match b.kind.as_str() {
        "velocity" => {
            let p: VelocityParams = serde_json::from_value(b.params.clone())
                .map_err(|e| ApiError::invalid("invalid_risk_rule", format!("params: {e}")))?;
            validate_velocity(&p)?;
        }
        "amount" => {
            let p: AmountParams = serde_json::from_value(b.params.clone())
                .map_err(|e| ApiError::invalid("invalid_risk_rule", format!("params: {e}")))?;
            validate_amount(&p)?;
        }
        _ => {
            return Err(ApiError::invalid(
                "invalid_risk_rule",
                "kind must be velocity or amount",
            ));
        }
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM risk_rules")
        .fetch_one(&mut *tx)
        .await?;
    if n >= MAX_RULES {
        return Err(ApiError::conflict(
            "too_many_rules",
            format!("at most {MAX_RULES} rules"),
        ));
    }
    let id = zyvor_core::new_id("rr");
    sqlx::query("INSERT INTO risk_rules (id, tenant_id, name, kind, params, action, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(&id).bind(&auth.tenant_id).bind(name).bind(&b.kind).bind(&b.params).bind(action.as_str()).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "risk_rule.created",
        Some(&id),
        json!({ "name": name, "kind": b.kind, "action": action.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": name, "kind": b.kind, "params": b.params, "action": action.as_str(), "enabled": true }),
        ),
    ))
}

/// `GET /v1/risk/rules`
pub async fn list_rules(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, Value, String, bool, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, name, kind, params, action, enabled, created_at FROM risk_rules ORDER BY created_at, id",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, kind, params, action, enabled, at)| json!({
        "id": id, "name": name, "kind": kind, "params": params, "action": action, "enabled": enabled, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// `DELETE /v1/risk/rules/{id}`
pub async fn delete_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("DELETE FROM risk_rules WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("risk rule"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "risk_rule.deleted",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "deleted": true })))
}

// ---- API: lists ------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct NewEntry {
    /// `allow` or `block`.
    list: String,
    /// customer, device, phone, email, ip, card or vpa.
    dimension: String,
    /// The value to match. It is hashed at once and never stored.
    value: String,
    /// Your own short note, shown in the list; do not put the value in it.
    note: Option<String>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn mask_hint(dim: Dim, v: &str) -> String {
    match dim {
        Dim::Email => match v.split_once('@') {
            Some((l, d)) => format!("{}***@{d}", l.chars().take(1).collect::<String>()),
            None => "***".into(),
        },
        Dim::Phone => {
            let n = keyed::normalise("phone", v);
            format!(
                "******{}",
                n.chars()
                    .skip(n.len().saturating_sub(4))
                    .collect::<String>()
            )
        }
        Dim::Vpa => crate::verify::mask_vpa(v),
        _ => "***".into(),
    }
}

/// `POST /v1/risk/lists`
pub async fn add_entry(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewEntry>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("risk:write")?;
    let dim = Dim::parse(&b.dimension).ok_or_else(|| {
        ApiError::invalid(
            "invalid_list_entry",
            "dimension must be customer, device, phone, email, ip, card or vpa",
        )
    })?;
    if !matches!(b.list.as_str(), "allow" | "block") {
        return Err(ApiError::invalid(
            "invalid_list_entry",
            "list must be allow or block",
        ));
    }
    let value = b.value.trim();
    if value.is_empty() || value.len() > 200 {
        return Err(ApiError::invalid(
            "invalid_list_entry",
            "value is 1-200 characters",
        ));
    }
    if b.note.as_deref().is_some_and(|n| n.len() > 40) {
        return Err(ApiError::invalid(
            "invalid_list_entry",
            "note is at most 40 characters",
        ));
    }
    if b.expires_at.is_some_and(|e| e <= chrono::Utc::now()) {
        return Err(ApiError::invalid(
            "invalid_list_entry",
            "expires_at must be in the future",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM risk_list_entries")
        .fetch_one(&mut *tx)
        .await?;
    if n >= MAX_LIST_ENTRIES {
        return Err(ApiError::conflict(
            "too_many_entries",
            format!("at most {MAX_LIST_ENTRIES} list entries"),
        ));
    }
    let key = keyed::tenant_key(&mut tx, &s.crypto, &auth.tenant_id).await?;
    let hash = keyed::hash(&key, dim.as_str(), value);
    let label = match &b.note {
        Some(n) if !n.trim().is_empty() => format!("{} ({})", mask_hint(dim, value), n.trim()),
        _ => mask_hint(dim, value),
    };
    let id = zyvor_core::new_id("rl");
    let inserted = sqlx::query("INSERT INTO risk_list_entries (id, tenant_id, list, dimension, value_hash, label, created_by, expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
        .bind(&id).bind(&auth.tenant_id).bind(&b.list).bind(dim.as_str()).bind(&hash).bind(&label).bind(&auth.actor).bind(b.expires_at)
        .execute(&mut *tx).await?.rows_affected();
    if inserted == 0 {
        return Err(ApiError::conflict(
            "already_listed",
            "that value is already on this list",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "risk_list.added",
        Some(&id),
        json!({ "list": b.list, "dimension": dim.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "list": b.list, "dimension": dim.as_str(), "label": label, "expires_at": b.expires_at }),
        ),
    ))
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    list: Option<String>,
}

/// `GET /v1/risk/lists`: entries are shown as a masked hint, never the value.
pub async fn list_entries(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, list, dimension, label, created_by, expires_at, created_at FROM risk_list_entries WHERE ($1::text IS NULL OR list = $1) ORDER BY created_at DESC, id LIMIT 500",
    )
    .bind(&q.list)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, list, dim, label, by, exp, at)| json!({
        "id": id, "list": list, "dimension": dim, "label": label, "created_by": by, "expires_at": exp, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// `DELETE /v1/risk/lists/{id}`
pub async fn delete_entry(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("DELETE FROM risk_list_entries WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("list entry"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "risk_list.removed",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "deleted": true })))
}

// ---- API: dry run, decisions, review queue ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct EvalBody {
    amount_minor: i64,
    method: Option<String>,
    customer_id: Option<String>,
    device: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    ip: Option<String>,
    card: Option<String>,
    vpa: Option<String>,
}

/// `POST /v1/risk/evaluate`: what the rules would decide for these values. Records nothing; the values are hashed for
/// the comparison and dropped.
pub async fn evaluate_dry(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<EvalBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive whole number",
        ));
    }
    let sig = Signals {
        amount_minor: b.amount_minor,
        method: b.method.unwrap_or_else(|| "upi".into()),
        customer: b.customer_id,
        device: b.device,
        phone: b.phone,
        email: b.email,
        ip: b.ip,
        card: b.card,
        vpa: b.vpa,
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let d = evaluate(&mut tx, &s.crypto, &auth.tenant_id, &sig).await?;
    // the tenant key may have been created; keep it so it is not made twice
    tx.commit().await?;
    Ok(Json(json!({ "dry_run": true, "decision": d.json() })))
}

#[derive(Deserialize, Default)]
pub struct DecisionQuery {
    subject_id: Option<String>,
    limit: Option<i64>,
}

/// `GET /v1/risk/decisions`: every decision where a rule or list fired, with the numbers it saw.
pub async fn list_decisions(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<DecisionQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        String,
        String,
        Option<i32>,
        Value,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, subject_type, subject_id, action, score, fired, created_at FROM risk_decisions
          WHERE ($1::text IS NULL OR subject_id = $1) ORDER BY created_at DESC, id LIMIT $2",
    )
    .bind(&q.subject_id)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, st, sid, action, score, fired, at)| json!({
        "id": id, "subject_type": st, "subject_id": sid, "action": action, "score": score, "fired": fired, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/risk/review_queue`: open review cases (payments and orders). Resolve one with
/// `POST /v1/recon/cases/{id}/resolve`.
pub async fn review_queue(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, kind, status, subject_type, subject_id, explanation, assignee, created_at FROM recon_cases
          WHERE kind IN ('risk_review', 'order_risk_review') AND status <> 'resolved' ORDER BY created_at LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, kind, status, st, sid, ex, asg, at)| json!({
        "case_id": id, "kind": kind, "status": status, "subject_type": st, "subject_id": sid, "explanation": ex, "assignee": asg, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

// ---- API: order risk score -------------------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PinTierBody {
    /// Three to six digits, first not zero; the longest matching prefix wins.
    pin_prefix: String,
    /// `low`, `medium` or `high`.
    tier: String,
}

/// `PUT /v1/risk/pin_tiers`
pub async fn set_pin_tier(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<PinTierBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:write")?;
    let p = b.pin_prefix.trim();
    if !(3..=6).contains(&p.len()) || !p.chars().all(|c| c.is_ascii_digit()) || p.starts_with('0') {
        return Err(ApiError::invalid(
            "invalid_pin_prefix",
            "pin_prefix is 3 to 6 digits, not starting with 0",
        ));
    }
    if risk::PinTier::parse(&b.tier).is_none() {
        return Err(ApiError::invalid(
            "invalid_tier",
            "tier must be low, medium or high",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM risk_pin_tiers")
        .fetch_one(&mut *tx)
        .await?;
    if n >= 20_000 {
        return Err(ApiError::conflict(
            "too_many_pin_tiers",
            "at most 20000 tiers",
        ));
    }
    sqlx::query("INSERT INTO risk_pin_tiers (tenant_id, pin_prefix, tier, updated_by) VALUES ($1,$2,$3,$4) ON CONFLICT (tenant_id, pin_prefix) DO UPDATE SET tier = EXCLUDED.tier, updated_by = EXCLUDED.updated_by, updated_at = now()")
        .bind(&auth.tenant_id).bind(p).bind(&b.tier).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({ "pin_prefix": p, "tier": b.tier })))
}

/// `GET /v1/risk/pin_tiers`
pub async fn list_pin_tiers(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT pin_prefix, tier FROM risk_pin_tiers ORDER BY pin_prefix LIMIT 1000",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(p, t)| json!({ "pin_prefix": p, "tier": t })).collect::<Vec<_>>() }),
    ))
}

/// `DELETE /v1/risk/pin_tiers/{pin_prefix}`
pub async fn delete_pin_tier(
    State(s): State<AppState>,
    auth: Auth,
    Path(prefix): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("DELETE FROM risk_pin_tiers WHERE pin_prefix = $1")
        .bind(&prefix)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("pin tier"));
    }
    tx.commit().await?;
    Ok(Json(json!({ "pin_prefix": prefix, "deleted": true })))
}

#[derive(Deserialize)]
pub struct OrderScoreBody {
    /// Six digits.
    pin_code: String,
    order_value_minor: i64,
    /// The buyer has received orders from you before.
    #[serde(default)]
    repeat_customer: bool,
    /// How many earlier orders they refused or that came back undelivered.
    #[serde(default)]
    prior_refusals: u32,
    /// Your order reference. With one, a `review` or `decline` result opens a case in the review queue (once per reference).
    order_ref: Option<String>,
    /// Score from which the order needs a look (default 40).
    review_at: Option<i64>,
    /// Score from which the order should be refused (default 75).
    decline_at: Option<i64>,
}

/// `POST /v1/risk/order_score`: an explainable delivery-risk score for a cash-on-delivery order from what you tell
/// us about it. The formula is in `risk::order_score` and `docs/zyvor/RISK_ENGINE.md`. Nothing is learned, no
/// personal data is stored (the pin code is not kept; only the factors and points are, when an order reference is
/// given).
pub async fn order_score(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<OrderScoreBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("risk:write")?;
    if !risk::valid_pin_code(b.pin_code.trim()) {
        return Err(ApiError::invalid(
            "invalid_pin_code",
            "pin_code must be six digits, not starting with 0",
        ));
    }
    if b.order_value_minor <= 0 || b.order_value_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "order_value_minor must be a positive whole number",
        ));
    }
    if b.prior_refusals > 1_000 {
        return Err(ApiError::invalid(
            "invalid_refusals",
            "prior_refusals is a count",
        ));
    }
    let review_at = b.review_at.unwrap_or(risk::DEFAULT_REVIEW_AT);
    let decline_at = b.decline_at.unwrap_or(risk::DEFAULT_DECLINE_AT);
    if !(1..=100).contains(&review_at) || !(1..=101).contains(&decline_at) || review_at > decline_at
    {
        return Err(ApiError::invalid(
            "invalid_thresholds",
            "review_at is 1-100 and decline_at is up to 101, review_at not above decline_at",
        ));
    }
    if b.order_ref
        .as_deref()
        .is_some_and(|r| r.trim().is_empty() || r.len() > 64)
    {
        return Err(ApiError::invalid(
            "invalid_order_ref",
            "order_ref is 1-64 characters",
        ));
    }
    let pin = b.pin_code.trim();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let tier: Option<String> = sqlx::query_scalar(
        "SELECT tier FROM risk_pin_tiers WHERE $1 LIKE pin_prefix || '%' ORDER BY length(pin_prefix) DESC LIMIT 1",
    )
    .bind(pin)
    .fetch_optional(&mut *tx)
    .await?;
    let score = risk::order_score(
        &risk::OrderInput {
            pin_code: pin.to_string(),
            order_value_minor: b.order_value_minor,
            repeat_customer: b.repeat_customer,
            prior_refusals: b.prior_refusals,
            pin_tier: tier.as_deref().and_then(risk::PinTier::parse),
        },
        review_at,
        decline_at,
    );
    let mut case_id: Option<String> = None;
    if let Some(r) = b.order_ref.as_deref().map(str::trim) {
        let fired = json!(score.contributions);
        record_decision(
            &mut tx,
            &auth.tenant_id,
            "order",
            r,
            score.action,
            Some(score.score),
            &fired,
        )
        .await?;
        if score.action != Action::Allow {
            let opened = open_case(
                &mut tx,
                &auth.tenant_id,
                "order_risk_review",
                "order",
                r,
                None,
                None,
                &format!("Order risk score {} ({}); action {}. See the contributions.", score.score, score.band, score.action.as_str()),
                json!({ "score": score.score, "contributions": score.contributions, "formula": risk::ORDER_FORMULA }),
            )
            .await?;
            if opened {
                tenant::emit(
                    &mut tx,
                    &auth.tenant_id,
                    "risk.review_required",
                    json!({ "subject_type": "order", "subject_id": r, "score": score.score }),
                )
                .await?;
            }
            case_id = sqlx::query_scalar(
                "SELECT id FROM recon_cases WHERE kind = 'order_risk_review' AND subject_id = $1",
            )
            .bind(r)
            .fetch_optional(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    let advice = match score.action {
        Action::Allow => "Ship as usual.",
        Action::Review => "Ask the buyer to confirm, or to pay in advance, before shipping.",
        Action::Decline => "Do not ship cash on delivery; offer prepaid only.",
    };
    Ok(Json(json!({
        "score": score.score, "band": score.band, "action": score.action.as_str(), "advice": advice,
        "contributions": score.contributions, "review_at": score.review_at, "decline_at": score.decline_at,
        "formula": score.formula, "pin_tier": tier, "case_id": case_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vel(dim: &str, metric: &str, window: i64, max: i64) -> VelocityParams {
        VelocityParams {
            dimension: dim.into(),
            metric: metric.into(),
            window_secs: window,
            max,
        }
    }

    #[test]
    fn velocity_rules_are_validated() {
        assert!(validate_velocity(&vel("phone", "attempts", 3600, 5)).is_ok());
        assert!(validate_velocity(&vel("card", "distinct_customers", 86_400, 2)).is_ok());
        assert!(validate_velocity(&vel("customer", "distinct_customers", 3600, 2)).is_err());
        assert!(validate_velocity(&vel("shoe", "attempts", 3600, 5)).is_err());
        assert!(validate_velocity(&vel("phone", "attempts", 59, 5)).is_err());
        assert!(validate_velocity(&vel("phone", "attempts", 8 * 86_400, 5)).is_err());
        assert!(validate_velocity(&vel("phone", "attempts", 3600, 0)).is_err());
        assert!(validate_velocity(&vel("phone", "sum", 3600, 1)).is_err());
    }

    #[test]
    fn amount_rules_are_validated() {
        let a = |above, below, m: Option<&str>| AmountParams {
            above_minor: above,
            below_minor: below,
            method: m.map(str::to_string),
        };
        assert!(validate_amount(&a(Some(100_000), None, None)).is_ok());
        assert!(validate_amount(&a(None, Some(100), Some("upi"))).is_ok());
        assert!(validate_amount(&a(None, None, None)).is_err());
        assert!(validate_amount(&a(Some(0), None, None)).is_err());
        assert!(validate_amount(&a(Some(50), Some(100), None)).is_err());
        assert!(validate_amount(&a(Some(50), None, Some("cheque"))).is_err());
    }

    #[test]
    fn unknown_parameters_are_refused() {
        let bad = serde_json::from_value::<VelocityParams>(
            json!({ "dimension": "phone", "metric": "attempts", "window_secs": 60, "max": 1, "extra": 1 }),
        );
        assert!(bad.is_err());
    }

    #[test]
    fn signals_ignore_blank_values_and_dimensions_round_trip() {
        let s = Signals {
            phone: Some("  ".into()),
            email: Some("a@b.c".into()),
            ..Default::default()
        };
        assert_eq!(s.get(Dim::Phone), None);
        assert_eq!(s.get(Dim::Email), Some("a@b.c"));
        for d in DIMS {
            assert_eq!(Dim::parse(d.as_str()), Some(d));
            assert!(d.column().ends_with("_h"));
        }
    }

    #[test]
    fn the_strongest_action_wins_and_names_its_rule() {
        let d = Decision {
            action: Action::Decline,
            fired: vec![
                json!({ "rule": "soft", "action": "review" }),
                json!({ "rule": "hard", "action": "decline" }),
            ],
            skipped: vec![],
            allowlisted: None,
        };
        assert_eq!(d.label(), "hard");
        assert!(Action::Decline > Action::Review && Action::Review > Action::Allow);
    }

    #[test]
    fn list_hints_do_not_show_the_value() {
        assert_eq!(mask_hint(Dim::Phone, "+91 98765 43210"), "******3210");
        assert_eq!(
            mask_hint(Dim::Email, "ravi@example.com"),
            "r***@example.com"
        );
        assert_eq!(mask_hint(Dim::Ip, "203.0.113.9"), "***");
        assert!(!mask_hint(Dim::Phone, "9876543210").contains("98765"));
    }
}
