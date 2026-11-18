//! Routing automation, all of it opt-in per business and all of it behind one kill switch:
//!
//! - **shadow mode**: a candidate policy is evaluated for every new attempt and its decision is stored next to the one
//!   applied. It never changes routing and never calls a provider (`record_shadow`, `shadow_report`);
//! - **cost-aware routing**: the cost model (`attach_costs`) and the settings the router's `CostPolicy` is built from;
//! - **soft retry**: the settings only; the retry itself is in `softretry.rs`;
//! - the **provider scorecard** (`scorecard`), read-only.
//!
//! The kill switch (`paused`) turns off shadow, soft retry, cost-aware routing, failover and the circuit breaker's
//! effect on routing at once: new payments are routed by the business's own rules and the lowest configured fee,
//! exactly as before any automation existed. Every change is an audit entry.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection};
use zyvor_providers::{
    Method,
    router::{Candidate, Cost, CostPolicy, Decision, Request, Rule},
};

use crate::{AppState, auth::Auth, error::ApiError, fee_check, registry::Routing, tenant};

// ---------- settings ----------

/// Per-business routing automation settings. No stored row means everything off and the kill switch not pulled.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Automation {
    /// The kill switch.
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub paused_by: Option<String>,
    pub paused_at: Option<DateTime<Utc>>,
    pub shadow_enabled: bool,
    /// The candidate policy shadow mode evaluates: `{"rules": [...], "cost_aware": true}`. Without `rules` the
    /// business's live rules are used; `cost_aware` defaults to true (the usual question is "what if I turned it on").
    pub shadow_policy: Option<Value>,
    pub soft_retry_enabled: bool,
    /// Total attempts a payment may have, the first one included.
    pub soft_retry_max_attempts: i32,
    pub soft_retry_window_seconds: i32,
    pub cost_aware_enabled: bool,
    pub cost_success_floor_bps: i32,
    pub cost_premium_bps: i32,
    pub cost_min_fee_samples: i32,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            paused: false,
            paused_reason: None,
            paused_by: None,
            paused_at: None,
            shadow_enabled: false,
            shadow_policy: None,
            soft_retry_enabled: false,
            soft_retry_max_attempts: 2,
            soft_retry_window_seconds: 600,
            cost_aware_enabled: false,
            cost_success_floor_bps: 8_000,
            cost_premium_bps: 500,
            cost_min_fee_samples: 30,
        }
    }
}

/// The candidate policy of shadow mode.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShadowPolicy {
    /// Routing rules to evaluate. `None` means the business's live rules.
    #[serde(default)]
    pub rules: Option<Vec<Rule>>,
    /// Evaluate with cost-aware routing (default true).
    #[serde(default = "yes")]
    pub cost_aware: bool,
}
fn yes() -> bool {
    true
}

impl Automation {
    pub fn shadow_active(&self) -> bool {
        !self.paused && self.shadow_enabled
    }
    pub fn soft_retry_active(&self) -> bool {
        !self.paused && self.soft_retry_enabled
    }
    /// The cost policy applied to real routing.
    pub fn cost_policy(&self) -> Option<CostPolicy> {
        (!self.paused && self.cost_aware_enabled).then(|| self.policy())
    }
    fn policy(&self) -> CostPolicy {
        CostPolicy {
            floor_bps: self.cost_success_floor_bps as u32,
            premium_bps: self.cost_premium_bps as u32,
        }
    }
    pub fn shadow(&self) -> ShadowPolicy {
        self.shadow_policy
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(ShadowPolicy {
                rules: None,
                cost_aware: true,
            })
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        let bad = |m: &str| {
            Err(ApiError::invalid(
                "invalid_automation_settings",
                m.to_string(),
            ))
        };
        if !(2..=5).contains(&self.soft_retry_max_attempts) {
            return bad(
                "soft_retry_max_attempts must be 2 to 5 (total attempts, the first included)",
            );
        }
        if !(30..=86_400).contains(&self.soft_retry_window_seconds) {
            return bad("soft_retry_window_seconds must be 30 to 86400");
        }
        if !(0..=10_000).contains(&self.cost_success_floor_bps) {
            return bad("cost_success_floor_bps must be 0 to 10000");
        }
        if !(0..=10_000).contains(&self.cost_premium_bps) {
            return bad("cost_premium_bps must be 0 to 10000");
        }
        if !(1..=100_000).contains(&self.cost_min_fee_samples) {
            return bad("cost_min_fee_samples must be 1 to 100000");
        }
        if let Some(p) = &self.shadow_policy
            && serde_json::from_value::<ShadowPolicy>(p.clone()).is_err()
        {
            return bad(
                "shadow_policy must look like {\"rules\": [routing rules], \"cost_aware\": true}",
            );
        }
        Ok(())
    }
}

type AutomationRow = (
    bool,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    bool,
    Option<Value>,
    bool,
    i32,
    i32,
    bool,
    i32,
    i32,
    i32,
);

pub async fn load(conn: &mut PgConnection, tenant_id: &str) -> Result<Automation, ApiError> {
    let row: Option<AutomationRow> = sqlx::query_as(
        "SELECT paused, paused_reason, paused_by, paused_at, shadow_enabled, shadow_policy, soft_retry_enabled,
                soft_retry_max_attempts, soft_retry_window_seconds, cost_aware_enabled, cost_success_floor_bps,
                cost_premium_bps, cost_min_fee_samples
         FROM routing_automation WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        Some((paused, pr, pb, pa, se, sp, sre, srm, srw, ce, cf, cp, cm)) => Automation {
            paused,
            paused_reason: pr,
            paused_by: pb,
            paused_at: pa,
            shadow_enabled: se,
            shadow_policy: sp,
            soft_retry_enabled: sre,
            soft_retry_max_attempts: srm,
            soft_retry_window_seconds: srw,
            cost_aware_enabled: ce,
            cost_success_floor_bps: cf,
            cost_premium_bps: cp,
            cost_min_fee_samples: cm,
        },
        None => Automation::default(),
    })
}

async fn save(
    conn: &mut PgConnection,
    tenant_id: &str,
    actor: &str,
    a: &Automation,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO routing_automation (tenant_id, paused, paused_reason, paused_by, paused_at, shadow_enabled, shadow_policy,
            soft_retry_enabled, soft_retry_max_attempts, soft_retry_window_seconds, cost_aware_enabled, cost_success_floor_bps,
            cost_premium_bps, cost_min_fee_samples, updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, now())
         ON CONFLICT (tenant_id) DO UPDATE SET paused = $2, paused_reason = $3, paused_by = $4, paused_at = $5,
            shadow_enabled = $6, shadow_policy = $7, soft_retry_enabled = $8, soft_retry_max_attempts = $9,
            soft_retry_window_seconds = $10, cost_aware_enabled = $11, cost_success_floor_bps = $12, cost_premium_bps = $13,
            cost_min_fee_samples = $14, updated_by = $15, updated_at = now()",
    )
    .bind(tenant_id)
    .bind(a.paused)
    .bind(&a.paused_reason)
    .bind(&a.paused_by)
    .bind(a.paused_at)
    .bind(a.shadow_enabled)
    .bind(&a.shadow_policy)
    .bind(a.soft_retry_enabled)
    .bind(a.soft_retry_max_attempts)
    .bind(a.soft_retry_window_seconds)
    .bind(a.cost_aware_enabled)
    .bind(a.cost_success_floor_bps)
    .bind(a.cost_premium_bps)
    .bind(a.cost_min_fee_samples)
    .bind(actor)
    .execute(conn)
    .await?;
    Ok(())
}

/// What is really in force, in one place for the API, the console and the docs: the kill switch overrides each
/// individual switch, and failover and the breaker live in the health settings.
fn effective(a: &Automation, h: &crate::health::Config) -> Value {
    let on = |setting: bool| !a.paused && setting;
    json!({
        "shadow": on(a.shadow_enabled),
        "soft_retry": on(a.soft_retry_enabled),
        "cost_aware": on(a.cost_aware_enabled),
        "failover": on(h.failover_enabled),
        "circuit_breaker": on(h.breaker_enabled),
        // the rule builder's saved rule sets, routing experiments, and what downtime alerts do beyond recording
        "rule_sets": !a.paused,
        "experiments": !a.paused,
        "alert_notifications": !a.paused,
        "note": if a.paused {
            "The kill switch is on: no routing automation acts. New payments use your own routing rules and the lowest configured fee."
        } else {
            "The kill switch is off. Each automation acts only if its own switch is on; failover and the circuit breaker are set in /v1/routing/health/settings."
        },
    })
}

pub async fn get_automation(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let a = load(&mut tx, &auth.tenant_id).await?;
    let h = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    Ok(Json(
        json!({ "settings": a, "effective": effective(&a, &h) }),
    ))
}

#[derive(Deserialize)]
pub struct AutomationSettingsIn {
    shadow_enabled: Option<bool>,
    /// `{"rules": [...], "cost_aware": true}`; `{}` means the live rules with cost-aware routing.
    shadow_policy: Option<Value>,
    soft_retry_enabled: Option<bool>,
    soft_retry_max_attempts: Option<i32>,
    soft_retry_window_seconds: Option<i32>,
    cost_aware_enabled: Option<bool>,
    cost_success_floor_bps: Option<i32>,
    cost_premium_bps: Option<i32>,
    cost_min_fee_samples: Option<i32>,
}

/// Change automation settings; only the fields sent change. Needs `admin`. The kill switch has its own routes.
pub async fn put_automation(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<AutomationSettingsIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur = load(&mut tx, &auth.tenant_id).await?;
    let new = Automation {
        shadow_enabled: b.shadow_enabled.unwrap_or(cur.shadow_enabled),
        shadow_policy: b.shadow_policy.or_else(|| cur.shadow_policy.clone()),
        soft_retry_enabled: b.soft_retry_enabled.unwrap_or(cur.soft_retry_enabled),
        soft_retry_max_attempts: b
            .soft_retry_max_attempts
            .unwrap_or(cur.soft_retry_max_attempts),
        soft_retry_window_seconds: b
            .soft_retry_window_seconds
            .unwrap_or(cur.soft_retry_window_seconds),
        cost_aware_enabled: b.cost_aware_enabled.unwrap_or(cur.cost_aware_enabled),
        cost_success_floor_bps: b
            .cost_success_floor_bps
            .unwrap_or(cur.cost_success_floor_bps),
        cost_premium_bps: b.cost_premium_bps.unwrap_or(cur.cost_premium_bps),
        cost_min_fee_samples: b.cost_min_fee_samples.unwrap_or(cur.cost_min_fee_samples),
        ..cur.clone()
    };
    new.validate()?;
    save(&mut tx, &auth.tenant_id, &auth.actor, &new).await?;
    // one entry per automation that was switched, so "who turned cost-aware routing on" has a direct answer
    for (name, was, is) in [
        ("shadow", cur.shadow_enabled, new.shadow_enabled),
        ("soft_retry", cur.soft_retry_enabled, new.soft_retry_enabled),
        ("cost_aware", cur.cost_aware_enabled, new.cost_aware_enabled),
    ] {
        if was != is {
            tenant::audit(
                &mut tx,
                &auth.tenant_id,
                &auth.actor,
                &format!(
                    "routing_automation.{name}_{}",
                    if is { "enabled" } else { "disabled" }
                ),
                None,
                json!({ "automation": name, "enabled": is, "kill_switch_on": new.paused }),
            )
            .await?;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "routing_automation.settings_updated",
        None,
        json!({ "before": cur, "after": new }),
    )
    .await?;
    let h = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "settings": new, "effective": effective(&new, &h) }),
    ))
}

#[derive(Deserialize)]
pub struct PauseIn {
    /// Why the kill switch is being pulled (kept in the audit trail).
    reason: Option<String>,
}

/// The kill switch on: every routing automation stops acting for this business at once. Idempotent. Needs `admin`.
pub async fn pause(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<PauseIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let reason = b
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if reason.as_deref().is_some_and(|r| r.len() > 500) {
        return Err(ApiError::invalid(
            "invalid_automation_settings",
            "reason is at most 500 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur = load(&mut tx, &auth.tenant_id).await?;
    if !cur.paused {
        let new = Automation {
            paused: true,
            paused_reason: reason.clone(),
            paused_by: Some(auth.actor.clone()),
            paused_at: Some(Utc::now()),
            ..cur.clone()
        };
        save(&mut tx, &auth.tenant_id, &auth.actor, &new).await?;
        tenant::audit(
            &mut tx,
            &auth.tenant_id,
            &auth.actor,
            "routing_automation.paused",
            None,
            json!({
                "reason": reason,
                "stops": ["shadow", "soft_retry", "cost_aware", "failover", "circuit_breaker"],
                "settings_before": cur,
            }),
        )
        .await?;
    }
    let a = load(&mut tx, &auth.tenant_id).await?;
    let h = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "settings": a, "effective": effective(&a, &h) }),
    ))
}

/// The kill switch off: each automation acts again if its own switch is on. Idempotent. Needs `admin`.
pub async fn resume(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur = load(&mut tx, &auth.tenant_id).await?;
    if cur.paused {
        let new = Automation {
            paused: false,
            paused_reason: None,
            paused_by: None,
            paused_at: None,
            ..cur.clone()
        };
        save(&mut tx, &auth.tenant_id, &auth.actor, &new).await?;
        tenant::audit(
            &mut tx,
            &auth.tenant_id,
            &auth.actor,
            "routing_automation.resumed",
            None,
            json!({ "was_paused_by": cur.paused_by, "was_paused_at": cur.paused_at, "reason_then": cur.paused_reason }),
        )
        .await?;
    }
    let a = load(&mut tx, &auth.tenant_id).await?;
    let h = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "settings": a, "effective": effective(&a, &h) }),
    ))
}

// ---------- the cost model ----------

/// Fees the provider actually deducted, per (provider, method), from settlement lines (the same source as
/// `GET /v1/analytics/cost`), over the last 90 days.
#[derive(Debug, Clone, Copy, Default)]
pub struct Observed {
    pub payments: i64,
    pub fee_minor: i64,
    pub volume_minor: i64,
}

/// Everything the cost estimate needs, loaded once per routing decision.
pub struct CostModel {
    rules: Vec<fee_check::Rule>,
    observed: BTreeMap<(String, String), Observed>,
    today: NaiveDate,
    /// Fewer observed payments than this and the observed rate is not used.
    min_fee_samples: i64,
}

impl CostModel {
    pub async fn load(
        conn: &mut PgConnection,
        tenant_id: &str,
        min_fee_samples: i32,
    ) -> Result<Self, ApiError> {
        let rules = fee_check::load_rules(&mut *conn).await?;
        let today = crate::clock::today(&mut *conn).await?;
        let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT a.provider, a.method, COUNT(*)::BIGINT, COALESCE(SUM(a.amount_minor), 0)::BIGINT, COALESCE(SUM(f.fee), 0)::BIGINT
             FROM payment_attempts a
             JOIN LATERAL (SELECT -SUM(si.amount_minor)::BIGINT AS fee FROM settlement_items si
                           WHERE si.attempt_id = a.id AND si.kind = 'fee') f ON f.fee IS NOT NULL
             WHERE a.tenant_id = $1 AND a.status = 'captured' AND a.created_at > now() - interval '90 days'
             GROUP BY a.provider, a.method",
        )
        .bind(tenant_id)
        .fetch_all(conn)
        .await?;
        Ok(Self {
            rules,
            observed: rows
                .into_iter()
                .map(|(p, m, n, v, f)| {
                    (
                        (p, m),
                        Observed {
                            payments: n,
                            volume_minor: v,
                            fee_minor: f,
                        },
                    )
                })
                .collect(),
            today,
            min_fee_samples: i64::from(min_fee_samples),
        })
    }

    /// The estimated cost of one payment, in this order of trust:
    /// 1. `observed`: the provider's actual deductions, as a share of the volume they were deducted on, applied to
    ///    this amount, once at least `min_fee_samples` payments have a fee line;
    /// 2. `fee_rule`: the merchant's contract rule in force today (percentage, flat, cap, GST on the fee);
    /// 3. `configured`: the routing fee table.
    ///
    /// All integer arithmetic, rounded half up. `None` when there is no basis at all.
    pub fn estimate(
        &self,
        provider: &str,
        method: Method,
        amount_minor: i64,
        configured: Option<zyvor_providers::router::Fee>,
    ) -> Option<Cost> {
        if let Some(o) = self
            .observed
            .get(&(provider.to_string(), method.as_str().to_string()))
            && o.payments >= self.min_fee_samples
            && o.volume_minor > 0
        {
            let minor = (i128::from(amount_minor) * i128::from(o.fee_minor)
                + i128::from(o.volume_minor) / 2)
                / i128::from(o.volume_minor);
            return Some(Cost {
                minor: minor as i64,
                basis: "observed",
            });
        }
        if let Some(rule) = fee_check::pick_rule(&self.rules, provider, method.as_str(), self.today)
        {
            return Some(Cost {
                minor: fee_check::expected_fee(rule, amount_minor).total_minor,
                basis: "fee_rule",
            });
        }
        configured.map(|f| Cost {
            minor: f.estimate(amount_minor),
            basis: "configured",
        })
    }

    pub fn observed(&self, provider: &str, method: &str) -> Option<Observed> {
        self.observed
            .get(&(provider.to_string(), method.to_string()))
            .copied()
    }
}

/// Set `cost` on every candidate (used by cost-aware routing and by shadow mode).
pub async fn attach_costs(
    conn: &mut PgConnection,
    tenant_id: &str,
    auto: &Automation,
    routing: &Routing,
    method: Method,
    amount_minor: i64,
    candidates: &mut [Candidate],
) -> Result<(), ApiError> {
    let model = CostModel::load(conn, tenant_id, auto.cost_min_fee_samples).await?;
    for c in candidates.iter_mut() {
        c.cost = model.estimate(
            &c.provider,
            method,
            amount_minor,
            routing
                .fees
                .get(&c.provider)
                .and_then(|m| m.get(&method))
                .copied(),
        );
    }
    Ok(())
}

// ---------- shadow mode ----------

/// Evaluate the candidate policy for an attempt that was just routed, and store the result next to the real decision.
/// Never changes routing, never calls a provider and never fails the payment: it runs in a savepoint, so a problem
/// here rolls back only the shadow row.
#[allow(clippy::too_many_arguments)]
pub async fn record_shadow(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    attempt_id: &str,
    auto: &Automation,
    routing: &Routing,
    actual_req: &Request,
    actual: &Decision,
    candidates: &[Candidate],
) {
    if let Err(e) = try_record_shadow(
        conn, tenant_id, intent_id, attempt_id, auto, routing, actual_req, actual, candidates,
    )
    .await
    {
        tracing::warn!(attempt = attempt_id, error = ?e.message, "shadow decision not recorded");
    }
}

#[allow(clippy::too_many_arguments)]
async fn try_record_shadow(
    conn: &mut PgConnection,
    tenant_id: &str,
    intent_id: &str,
    attempt_id: &str,
    auto: &Automation,
    routing: &Routing,
    actual_req: &Request,
    actual: &Decision,
    candidates: &[Candidate],
) -> Result<(), ApiError> {
    let policy = auto.shadow();
    let rules: &[Rule] = policy.rules.as_deref().unwrap_or(&routing.rules);
    let shadow_req = Request {
        cost: policy.cost_aware.then(|| auto.policy()),
        ..actual_req.clone()
    };
    let result = zyvor_providers::router::choose(&shadow_req, candidates, rules);
    let cost_of = |provider: &str| {
        candidates
            .iter()
            .find(|c| c.provider == provider)
            .and_then(|c| c.cost)
    };
    let (shadow_provider, shadow_expl, shadow_json, shadow_error) = match &result {
        Ok(d) => (
            Some(d.provider.clone()),
            Some(d.explanation.clone()),
            serde_json::to_value(d).ok(),
            None,
        ),
        Err(e) => (None, None, None, Some(e.to_string())),
    };
    let a_cost = cost_of(&actual.provider);
    let s_cost = shadow_provider.as_deref().and_then(cost_of);
    let mut sp = conn.begin().await?;
    sqlx::query(
        "INSERT INTO routing_shadow_decisions (tenant_id, intent_id, attempt_id, method, amount_minor, actual_provider,
            shadow_provider, agree, actual_fee_minor, shadow_fee_minor, fee_basis, actual_explanation, shadow_explanation,
            shadow_decision, shadow_error)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
         ON CONFLICT (attempt_id) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(intent_id)
    .bind(attempt_id)
    .bind(actual_req.method.as_str())
    .bind(actual_req.amount_minor)
    .bind(&actual.provider)
    .bind(&shadow_provider)
    .bind(shadow_provider.as_deref() == Some(actual.provider.as_str()))
    .bind(a_cost.map(|c| c.minor))
    .bind(s_cost.map(|c| c.minor))
    .bind(json!({ "actual": a_cost.map(|c| c.basis), "shadow": s_cost.map(|c| c.basis) }))
    .bind(&actual.explanation)
    .bind(&shadow_expl)
    .bind(&shadow_json)
    .bind(&shadow_error)
    .execute(&mut *sp)
    .await?;
    sp.commit().await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ReportQuery {
    pub days: Option<i32>,
    pub method: Option<String>,
}

fn days_of(q: &Option<i32>, default: i32) -> Result<i32, ApiError> {
    let d = q.unwrap_or(default);
    if (1..=90).contains(&d) {
        Ok(d)
    } else {
        Err(ApiError::invalid("invalid_days", "days must be 1 to 90"))
    }
}

fn method_of(q: &Option<String>) -> Result<Option<String>, ApiError> {
    match q.as_deref() {
        None | Some("") => Ok(None),
        Some(m) => m
            .parse::<Method>()
            .map(|m| Some(m.as_str().to_string()))
            .map_err(|e: String| ApiError::invalid("invalid_method", e)),
    }
}

/// Shadow versus actual over a window: how often the candidate policy agrees with what was applied, where it
/// differs, the estimated fee difference, and what actually happened to the payments that were sent.
pub async fn shadow_report(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ReportQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let days = days_of(&q.days, 7)?;
    let method = method_of(&q.method)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let auto = load(&mut tx, &auth.tenant_id).await?;

    let (total, agreed, no_route): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COUNT(*) FILTER (WHERE agree), COUNT(*) FILTER (WHERE shadow_provider IS NULL)
         FROM routing_shadow_decisions
         WHERE tenant_id = $1 AND created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR method = $3)",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_one(&mut *tx)
    .await?;
    let (cmp_n, actual_fee, shadow_fee): (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(actual_fee_minor), 0)::BIGINT, COALESCE(SUM(shadow_fee_minor), 0)::BIGINT
         FROM routing_shadow_decisions
         WHERE tenant_id = $1 AND created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR method = $3)
           AND actual_fee_minor IS NOT NULL AND shadow_fee_minor IS NOT NULL",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_one(&mut *tx)
    .await?;
    let pairs: Vec<(String, String, Option<String>, i64, i64, i64)> = sqlx::query_as(
        "SELECT method, actual_provider, shadow_provider, COUNT(*),
                COUNT(*) FILTER (WHERE actual_fee_minor IS NOT NULL AND shadow_fee_minor IS NOT NULL),
                COALESCE(SUM(actual_fee_minor - shadow_fee_minor) FILTER (WHERE actual_fee_minor IS NOT NULL AND shadow_fee_minor IS NOT NULL), 0)::BIGINT
         FROM routing_shadow_decisions
         WHERE tenant_id = $1 AND created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR method = $3)
           AND NOT agree
         GROUP BY method, actual_provider, shadow_provider ORDER BY COUNT(*) DESC, method, actual_provider",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;
    // What happened to the payments that were really sent (the provider that actually took them).
    let outcomes: Vec<(String, String, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT d.actual_provider, d.method, COUNT(*),
                COUNT(*) FILTER (WHERE a.status IN ('captured', 'authorized')),
                COUNT(*) FILTER (WHERE a.status = 'failed' AND a.failure_class IS DISTINCT FROM 'payer_declined' AND a.failure_class IS DISTINCT FROM 'cancelled'),
                COUNT(*) FILTER (WHERE a.status = 'failed' AND a.failure_class = 'payer_declined'),
                COUNT(*) FILTER (WHERE a.status = 'unknown'),
                COUNT(*) FILTER (WHERE NOT d.agree)
         FROM routing_shadow_decisions d JOIN payment_attempts a ON a.id = d.attempt_id
         WHERE d.tenant_id = $1 AND d.created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR d.method = $3)
         GROUP BY d.actual_provider, d.method ORDER BY d.actual_provider, d.method",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;
    let recent: Vec<(String, String, i64, String, Option<String>, Option<String>, Option<String>, Option<i64>, Option<i64>, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT d.attempt_id, d.method, d.amount_minor, d.actual_provider, d.shadow_provider, d.actual_explanation, d.shadow_explanation,
                d.actual_fee_minor, d.shadow_fee_minor, a.status, d.created_at
         FROM routing_shadow_decisions d JOIN payment_attempts a ON a.id = d.attempt_id
         WHERE d.tenant_id = $1 AND d.created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR d.method = $3)
           AND NOT d.agree
         ORDER BY d.created_at DESC, d.id DESC LIMIT 20",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;

    Ok(Json(json!({
        "window_days": days,
        "method": method,
        "shadow": {
            "enabled": auto.shadow_enabled,
            "kill_switch_on": auto.paused,
            "recording_now": auto.shadow_active(),
            "policy": auto.shadow(),
        },
        "decisions": total,
        "agreed": agreed,
        "differed": total - agreed,
        "no_route_in_shadow": no_route,
        "agreement_rate_bps": (total > 0).then(|| agreed * 10_000 / total),
        // Estimated with the fee model (observed fees, then your fee rules, then the configured fee table), on the
        // decisions where both providers have a basis. Positive: the shadow choice would have cost less.
        "estimated_fee": {
            "comparable_decisions": cmp_n,
            "actual_total_minor": actual_fee,
            "shadow_total_minor": shadow_fee,
            "difference_minor": actual_fee - shadow_fee,
            "meaning": "difference_minor is what was applied minus what shadow would have chosen: positive means the shadow choice would have cost less. An estimate, not a settlement figure.",
        },
        "differences": pairs.into_iter().map(|(m, a, sh, n, cmp, diff)| json!({
            "method": m, "applied_provider": a, "shadow_provider": sh, "decisions": n,
            "fee_comparable": cmp, "estimated_fee_difference_minor": diff,
        })).collect::<Vec<_>>(),
        "outcome_by_provider": outcomes.into_iter().map(|(p, m, n, ok, f, payer, unk, diff)| json!({
            "provider": p, "method": m, "payments_sent": n, "succeeded": ok, "provider_failures": f,
            "payer_declines": payer, "unresolved": unk, "of_which_shadow_would_have_chosen_another": diff,
        })).collect::<Vec<_>>(),
        "recent_differences": recent.into_iter().map(|(att, m, amt, ap, sp, ae, se, af, sf, st, at)| json!({
            "attempt_id": att, "method": m, "amount_minor": amt, "applied_provider": ap, "shadow_provider": sp,
            "applied_explanation": ae, "shadow_explanation": se,
            "applied_fee_minor": af, "shadow_fee_minor": sf, "attempt_status": st, "at": at,
        })).collect::<Vec<_>>(),
        "limits": "Shadow mode cannot know how the other provider would have done: outcomes shown are those of the provider that really took each payment. It never sends a payment and never contacts a provider.",
    })))
}

// ---------- provider scorecard ----------

#[derive(Deserialize)]
pub struct ScorecardQuery {
    pub days: Option<i32>,
    pub method: Option<String>,
}

/// Per provider and method over a window: success rate (payer declines excluded), failure classes, median time to
/// resolve, unresolved payments, observed effective fee and the last degrade and recovery.
pub async fn scorecard(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ScorecardQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let days = days_of(&q.days, 7)?;
    let method = method_of(&q.method)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cfg = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    let auto = load(&mut tx, &auth.tenant_id).await?;
    let model = CostModel::load(&mut tx, &auth.tenant_id, auto.cost_min_fee_samples).await?;
    let routing = crate::registry::routing(&mut tx, &auth.tenant_id).await?;

    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, i64, i64, i64, i64, i64, i64, Option<f64>)> = sqlx::query_as(
        "SELECT provider, method,
                COUNT(*) FILTER (WHERE status IN ('captured', 'authorized')),
                COUNT(*) FILTER (WHERE status = 'failed' AND failure_class IS DISTINCT FROM 'payer_declined'
                                   AND failure_class IS DISTINCT FROM 'cancelled' AND failure_reason IS DISTINCT FROM 'canceled_by_merchant'),
                COUNT(*) FILTER (WHERE status = 'failed' AND failure_class = 'payer_declined'),
                COUNT(*) FILTER (WHERE status = 'unknown'),
                COUNT(*) FILTER (WHERE status IN ('created', 'pending')),
                COUNT(*) FILTER (WHERE status = 'failed' AND (failure_class = 'cancelled' OR failure_reason = 'canceled_by_merchant')),
                (percentile_cont(0.5) WITHIN GROUP (ORDER BY EXTRACT(EPOCH FROM (updated_at - created_at)))
                    FILTER (WHERE (status = 'failed' AND failure_class IS DISTINCT FROM 'cancelled')
                               OR (status = 'captured' AND refunded_minor = 0 AND reserved_refund_minor = 0)))::float8
         FROM payment_attempts
         WHERE tenant_id = $1 AND created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR method = $3)
         GROUP BY provider, method ORDER BY provider, method",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;
    let classes: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT provider, method, failure_class, COUNT(*) FROM payment_attempts
         WHERE tenant_id = $1 AND status = 'failed' AND created_at > now() - make_interval(days => $2) AND ($3::text IS NULL OR method = $3)
         GROUP BY provider, method, failure_class",
    )
    .bind(&auth.tenant_id)
    .bind(days)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;
    // unresolved right now, whatever their age: they block a retry of their payment until resolved
    let open_unknown: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT provider, method, COUNT(*) FROM payment_attempts
         WHERE tenant_id = $1 AND status = 'unknown' AND ($2::text IS NULL OR method = $2) GROUP BY provider, method",
    )
    .bind(&auth.tenant_id)
    .bind(&method)
    .fetch_all(&mut *tx)
    .await?;
    let health: Vec<(String, String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT provider, method, state, degraded_since FROM provider_health_state WHERE tenant_id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let event_rows: Vec<(String, Option<String>, Option<String>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT action, detail->>'method', subject, created_at FROM audit_events
         WHERE tenant_id = $1 AND action IN ('provider.degraded', 'provider.recovered') ORDER BY created_at DESC, id DESC",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let events: Vec<(String, String, Option<String>, DateTime<Utc>)> = event_rows
        .into_iter()
        .map(|(a, m, p, at)| (a, m.unwrap_or_default(), p, at))
        .collect();
    let accounts: Vec<(String, bool)> = sqlx::query_as(
        "SELECT provider, enabled FROM provider_accounts WHERE tenant_id = $1 ORDER BY provider",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    let enabled = |p: &str| accounts.iter().find(|(n, _)| n == p).map(|(_, e)| *e);
    let mut out = Vec::new();
    for (provider, m, ok, fail, payer, unknown, in_flight, cancelled, median) in rows {
        let samples = ok + fail + unknown;
        let mut by_class: BTreeMap<String, i64> = BTreeMap::new();
        for (p, mm, c, n) in &classes {
            if *p == provider && *mm == m {
                *by_class
                    .entry(c.clone().unwrap_or_else(|| "unclassified".into()))
                    .or_default() += n;
            }
        }
        let st = health.iter().find(|h| h.0 == provider && h.1 == m);
        let last = |action: &str| {
            events
                .iter()
                .find(|e| e.0 == action && e.1 == m && e.2.as_deref() == Some(provider.as_str()))
                .map(|e| e.3)
        };
        let observed = m
            .parse::<Method>()
            .ok()
            .and_then(|_| model.observed(&provider, &m));
        let configured = m.parse::<Method>().ok().and_then(|mm| {
            routing
                .fees
                .get(&provider)
                .and_then(|f| f.get(&mm))
                .map(|f| json!({ "bps": f.bps, "fixed_minor": f.fixed_minor }))
        });
        out.push(json!({
            "provider": provider,
            "enabled": enabled(&provider),
            "method": m,
            "attempts": ok + fail + payer + unknown + in_flight + cancelled,
            "successes": ok,
            "provider_failures": fail,
            "payer_declines": payer,
            "unresolved_in_window": unknown,
            "in_flight": in_flight,
            "cancelled_by_merchant": cancelled,
            // payer declines and merchant cancellations are not the provider's doing and are left out
            "success_rate_bps": (samples >= i64::from(cfg.min_samples) && samples > 0).then(|| ok * 10_000 / samples),
            "samples": samples,
            "min_samples": cfg.min_samples,
            "failure_classes": by_class,
            "median_seconds_to_resolve": median,
            "unresolved_now": open_unknown.iter().find(|u| u.0 == provider && u.1 == m).map_or(0, |u| u.2),
            "observed_fee": observed.map(|o| json!({
                "payments": o.payments, "fee_minor": o.fee_minor, "on_volume_minor": o.volume_minor,
                "effective_rate_bps": (o.volume_minor > 0).then(|| o.fee_minor * 10_000 / o.volume_minor),
                "window_days": 90,
                "used_by_cost_routing": o.payments >= i64::from(auto.cost_min_fee_samples),
            })),
            "configured_fee": configured,
            "breaker_state": st.map_or("healthy", |h| h.2.as_str()),
            "degraded_since": st.and_then(|h| h.3),
            "last_degraded_at": last("provider.degraded"),
            "last_recovered_at": last("provider.recovered"),
        }));
    }
    Ok(Json(json!({
        "window_days": days,
        "method": method,
        "success_rate_rule": "successes over successes + provider failures + unresolved, in basis points. Payer declines and merchant cancellations are left out. Null below min_samples.",
        "median_rule": "Median seconds from creating the attempt to its last update, for failed attempts and for captured attempts that no refund has touched (a refund moves the last-update time, so those are left out).",
        "scorecard": out,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyvor_providers::router::Fee;

    fn model(
        rules: Vec<fee_check::Rule>,
        observed: Vec<((&str, &str), Observed)>,
        min: i64,
    ) -> CostModel {
        CostModel {
            rules,
            observed: observed
                .into_iter()
                .map(|((p, m), o)| ((p.into(), m.into()), o))
                .collect(),
            today: NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            min_fee_samples: min,
        }
    }
    fn rule(provider: &str, bps: i64, flat: i64, gst: Option<i64>) -> fee_check::Rule {
        fee_check::Rule {
            id: "r".into(),
            provider: provider.into(),
            method: "upi".into(),
            bps,
            flat_minor: flat,
            cap_minor: None,
            gst_bps: gst,
            refund_returns_fee: false,
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        }
    }

    #[test]
    fn observed_beats_the_fee_rule_beats_the_configured_fee_and_each_names_itself() {
        let o = Observed {
            payments: 40,
            fee_minor: 4_130,
            volume_minor: 1_000_000,
        };
        let m = model(
            vec![rule("a", 200, 0, None), rule("b", 200, 0, None)],
            vec![(("a", "upi"), o)],
            30,
        );
        let cfg = Some(Fee {
            bps: 300,
            fixed_minor: 0,
        });
        // observed: 41.3 bps of the volume, applied to 100_000 = 413
        let c = m.estimate("a", Method::Upi, 100_000, cfg).unwrap();
        assert_eq!((c.minor, c.basis), (413, "observed"));
        // no observation for b: its fee rule (200 bps, no GST) = 2000
        let c = m.estimate("b", Method::Upi, 100_000, cfg).unwrap();
        assert_eq!((c.minor, c.basis), (2_000, "fee_rule"));
        // neither: the configured table
        let c = m.estimate("c", Method::Upi, 100_000, cfg).unwrap();
        assert_eq!((c.minor, c.basis), (3_000, "configured"));
        // no basis at all
        assert!(m.estimate("c", Method::Upi, 100_000, None).is_none());
    }

    #[test]
    fn too_few_observed_payments_fall_back_to_the_fee_rule() {
        let o = Observed {
            payments: 29,
            fee_minor: 1,
            volume_minor: 1_000_000,
        };
        let m = model(
            vec![rule("a", 100, 0, Some(1_800))],
            vec![(("a", "upi"), o)],
            30,
        );
        let c = m.estimate("a", Method::Upi, 100_000, None).unwrap();
        // 1% = 1000, plus 18% GST on the fee = 1180
        assert_eq!((c.minor, c.basis), (1_180, "fee_rule"));
    }

    #[test]
    fn settings_are_validated_and_the_kill_switch_overrides_every_switch() {
        assert!(Automation::default().validate().is_ok());
        let bad = |f: fn(&mut Automation)| {
            let mut a = Automation::default();
            f(&mut a);
            a.validate().is_err()
        };
        assert!(bad(|a| a.soft_retry_max_attempts = 1));
        assert!(bad(|a| a.soft_retry_max_attempts = 6));
        assert!(bad(|a| a.soft_retry_window_seconds = 5));
        assert!(bad(|a| a.cost_success_floor_bps = 10_001));
        assert!(bad(|a| a.cost_min_fee_samples = 0));
        assert!(bad(|a| a.shadow_policy = Some(json!({ "rules": "nope" }))));
        let mut a = Automation {
            shadow_enabled: true,
            soft_retry_enabled: true,
            cost_aware_enabled: true,
            ..Automation::default()
        };
        assert!(a.shadow_active() && a.soft_retry_active() && a.cost_policy().is_some());
        a.paused = true;
        assert!(!a.shadow_active() && !a.soft_retry_active() && a.cost_policy().is_none());
        // the defaults are all off
        let d = Automation::default();
        assert!(!d.shadow_active() && !d.soft_retry_active() && d.cost_policy().is_none());
    }
}
