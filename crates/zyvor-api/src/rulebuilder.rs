//! A no-code routing rule builder on top of the router's `prefer` / `exclude` rules.
//!
//! A rule is `when` (conditions) and `then` (what to do): conditions on method, amount band, currency, card
//! network, time of day and new or returning customer, and the action `prefer`, `exclude` and / or
//! `prefer_success_rate`. The router (`zyvor_providers::router::choose`) is unchanged: for each payment this module
//! keeps the rules whose conditions hold and hands them to the router as ordinary rules, so every hard filter
//! (provider disabled, capability matrix, `allowed_providers`), the circuit breaker, failover and the "unknown payment
//! blocks retries" rule apply exactly as before. A rule cannot send a payment somewhere the router would refuse.
//!
//! Rule sets are immutable numbered versions with one active pointer per business (`routing_rule_versions`,
//! `routing_rule_active`). Activating, rolling back and deactivating are audited. With no active version the older
//! `tenant_settings.routing.rules` apply, unchanged. With one, it replaces them (fees still come from the settings).
//!
//! `POST /v1/routing/dry_run` says which provider a payment would take, with the full decision, without creating
//! anything or claiming a circuit-breaker probe.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::Currency;
use zyvor_providers::{
    Method,
    router::{self, Candidate, Decision, NoRoute, Rule},
};

use crate::{ApiError, AppState, auth::Auth, instrument, registry, tenant};

pub const KNOWN_PROVIDERS: &[&str] = &[
    "razorpay",
    "cashfree",
    "phonepe",
    "payu",
    "custom",
    "sponsor_upi",
    "sim",
];
pub const MAX_RULES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimeWindow {
    /// `HH:MM`, inclusive.
    pub from: String,
    /// `HH:MM`, exclusive. Earlier than `from` means the window crosses midnight.
    pub to: String,
    /// Minutes ahead of UTC the clock times are in. Default 330 (India).
    #[serde(default = "ist")]
    pub utc_offset_minutes: i32,
}

fn ist() -> i32 {
    330
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct When {
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub min_amount_minor: Option<i64>,
    #[serde(default)]
    pub max_amount_minor: Option<i64>,
    #[serde(default)]
    pub currencies: Vec<String>,
    #[serde(default)]
    pub card_networks: Vec<String>,
    /// `new` and / or `returning`.
    #[serde(default)]
    pub segments: Vec<String>,
    #[serde(default)]
    pub time_window: Option<TimeWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Then {
    #[serde(default)]
    pub prefer: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub prefer_success_rate: bool,
    #[serde(default)]
    pub success_rate_margin_bps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuilderRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub when: When,
    pub then: Then,
}

/// What the rules are judged against.
pub struct RuleCtx<'a> {
    pub method: Method,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub card_network: Option<&'a str>,
    pub segment: Option<&'a str>,
    pub at: DateTime<Utc>,
}

fn hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn in_window(w: &TimeWindow, at: DateTime<Utc>) -> bool {
    let (Some(from), Some(to)) = (hhmm(&w.from), hhmm(&w.to)) else {
        return false;
    };
    let local = at + chrono::Duration::minutes(i64::from(w.utc_offset_minutes));
    let now = local.hour() * 60 + local.minute();
    if from < to {
        from <= now && now < to
    } else {
        now >= from || now < to
    }
}

impl When {
    /// Why the rule does not apply to this payment, or `None` when it does.
    pub fn mismatch(&self, c: &RuleCtx) -> Option<String> {
        if !self.methods.is_empty() && !self.methods.iter().any(|m| m == c.method.as_str()) {
            return Some(format!("method is {}", c.method.as_str()));
        }
        if self.min_amount_minor.is_some_and(|v| c.amount_minor < v) {
            return Some("amount is below the minimum".into());
        }
        if self.max_amount_minor.is_some_and(|v| c.amount_minor > v) {
            return Some("amount is above the maximum".into());
        }
        if !self.currencies.is_empty() && !self.currencies.iter().any(|x| x == c.currency) {
            return Some(format!("currency is {}", c.currency));
        }
        if !self.card_networks.is_empty() && !c.card_networks_match(&self.card_networks) {
            return Some(match c.card_network {
                Some(n) => format!("card network is {n}"),
                None => "the card network is not known".into(),
            });
        }
        if !self.segments.is_empty()
            && !c
                .segment
                .is_some_and(|s| self.segments.iter().any(|x| x == s))
        {
            return Some(match c.segment {
                Some(s) => format!("customer is {s}"),
                None => "the payment has no customer to judge new or returning by".into(),
            });
        }
        if let Some(w) = &self.time_window
            && !in_window(w, c.at)
        {
            return Some(format!(
                "outside {}-{} (UTC{:+})",
                w.from,
                w.to,
                w.utc_offset_minutes as f64 / 60.0
            ));
        }
        None
    }
}

impl RuleCtx<'_> {
    fn card_networks_match(&self, list: &[String]) -> bool {
        self.card_network
            .is_some_and(|n| list.iter().any(|x| x == n))
    }
}

/// The rules whose conditions hold, as ordinary router rules (their order is kept: the first `prefer` that names an
/// eligible provider wins), and which they were.
pub fn compile(rules: &[BuilderRule], c: &RuleCtx) -> (Vec<Rule>, Vec<Value>) {
    let mut out = Vec::new();
    let mut matched = Vec::new();
    for (i, r) in rules.iter().enumerate() {
        if r.when.mismatch(c).is_some() {
            continue;
        }
        out.push(Rule {
            method: None,
            min_amount_minor: None,
            max_amount_minor: None,
            prefer: r.then.prefer.clone(),
            exclude: r.then.exclude.clone(),
            prefer_success_rate: r.then.prefer_success_rate,
            success_rate_margin_bps: r.then.success_rate_margin_bps,
        });
        matched.push(json!({ "index": i, "name": r.name }));
    }
    (out, matched)
}

// ------------------------------------------------------------------ validation

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct Problem {
    pub rule: Option<usize>,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Serialize, Default)]
pub struct Report {
    pub valid: bool,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
}

fn p(rule: Option<usize>, field: &str, message: impl Into<String>) -> Problem {
    Problem {
        rule,
        field: field.to_string(),
        message: message.into(),
    }
}

/// Check a rule list. `accounts` are the business's provider accounts (name, enabled); a rule that would exclude
/// every one of them is an error, a provider with no account is a warning. Pure.
pub fn validate(rules: &[BuilderRule], accounts: &[(String, bool)]) -> Report {
    let mut r = Report::default();
    if rules.len() > MAX_RULES {
        r.errors
            .push(p(None, "rules", format!("at most {MAX_RULES} rules")));
    }
    for (i, rule) in rules.iter().enumerate() {
        let at = Some(i);
        if rule.name.as_deref().is_some_and(|n| n.len() > 60) {
            r.errors.push(p(at, "name", "at most 60 characters"));
        }
        let w = &rule.when;
        for m in &w.methods {
            if m.parse::<Method>().is_err() {
                r.errors
                    .push(p(at, "when.methods", format!("unknown method {m:?}")));
            }
        }
        if w.min_amount_minor.is_some_and(|v| v < 0) || w.max_amount_minor.is_some_and(|v| v <= 0) {
            r.errors.push(p(
                at,
                "when.min_amount_minor",
                "the amount band must be positive",
            ));
        }
        if let (Some(lo), Some(hi)) = (w.min_amount_minor, w.max_amount_minor)
            && lo > hi
        {
            r.errors.push(p(
                at,
                "when.min_amount_minor",
                "the minimum is above the maximum",
            ));
        }
        for c in &w.currencies {
            if c.parse::<Currency>().is_err() {
                r.errors.push(p(
                    at,
                    "when.currencies",
                    format!("unsupported currency {c:?}"),
                ));
            }
        }
        for n in &w.card_networks {
            if !instrument::NETWORKS.contains(&n.as_str()) {
                r.errors.push(p(
                    at,
                    "when.card_networks",
                    format!("{n:?} is not one of {}", instrument::NETWORKS.join(", ")),
                ));
            }
        }
        if !w.card_networks.is_empty()
            && !w.methods.is_empty()
            && !w.methods.iter().any(|m| m == "card")
        {
            r.errors.push(p(
                at,
                "when.card_networks",
                "a card network condition needs `card` in methods",
            ));
        }
        for s in &w.segments {
            if !matches!(s.as_str(), "new" | "returning") {
                r.errors.push(p(
                    at,
                    "when.segments",
                    format!("{s:?} is not new or returning"),
                ));
            }
        }
        if let Some(tw) = &w.time_window {
            match (hhmm(&tw.from), hhmm(&tw.to)) {
                (Some(a), Some(b)) if a == b => {
                    r.errors.push(p(at, "when.time_window", "from and to are the same, so the window is empty or all day. Leave it out for all day"));
                }
                (Some(_), Some(_)) => {}
                _ => r.errors.push(p(
                    at,
                    "when.time_window",
                    "from and to must be HH:MM (00:00 to 23:59)",
                )),
            }
            if !(-720..=840).contains(&tw.utc_offset_minutes) {
                r.errors.push(p(
                    at,
                    "when.time_window.utc_offset_minutes",
                    "must be between -720 and 840",
                ));
            }
        }
        let t = &rule.then;
        if t.prefer.is_empty() && t.exclude.is_empty() && !t.prefer_success_rate {
            r.errors.push(p(
                at,
                "then",
                "a rule needs something to do: prefer, exclude or prefer_success_rate",
            ));
        }
        if t.success_rate_margin_bps.is_some() && !t.prefer_success_rate {
            r.errors.push(p(
                at,
                "then.success_rate_margin_bps",
                "only used with prefer_success_rate",
            ));
        }
        if t.success_rate_margin_bps.is_some_and(|m| m > 10_000) {
            r.errors
                .push(p(at, "then.success_rate_margin_bps", "at most 10000"));
        }
        for (field, list) in [("then.prefer", &t.prefer), ("then.exclude", &t.exclude)] {
            for name in list {
                if !KNOWN_PROVIDERS.contains(&name.as_str()) {
                    r.errors
                        .push(p(at, field, format!("unknown provider {name:?}")));
                } else if !accounts.is_empty() && !accounts.iter().any(|(a, _)| a == name) {
                    r.warnings.push(p(at, field, format!("{name} has no provider account here, so this has no effect until one is added")));
                }
            }
        }
        for name in &t.prefer {
            if t.exclude.contains(name) {
                r.errors.push(p(
                    at,
                    "then",
                    format!("{name} is both preferred and excluded"),
                ));
            }
        }
        if !accounts.is_empty() && accounts.iter().all(|(a, _)| t.exclude.contains(a)) {
            r.errors.push(p(at, "then.exclude", "this would exclude every provider account, so matching payments could not be routed"));
        }
    }
    r.valid = r.errors.is_empty();
    r
}

// ------------------------------------------------------------------ storage and selection

pub async fn active_version(conn: &mut PgConnection) -> Result<Option<i32>, ApiError> {
    Ok(
        sqlx::query_scalar::<_, Option<i32>>("SELECT version FROM routing_rule_active")
            .fetch_optional(conn)
            .await?
            .flatten(),
    )
}

pub async fn rules_of(conn: &mut PgConnection, version: i32) -> Result<Vec<BuilderRule>, ApiError> {
    let v: Value = sqlx::query_scalar("SELECT rules FROM routing_rule_versions WHERE version = $1")
        .bind(version)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("rule set version"))?;
    serde_json::from_value(v).map_err(|_| ApiError::internal())
}

/// Which rules one payment is routed by, and why.
pub struct Selection {
    pub rules: Vec<Rule>,
    /// Used if the variant's rules leave no provider for this payment: the control's.
    pub fallback: Option<Vec<Rule>>,
    pub version: Option<i32>,
    pub matched: Vec<Value>,
    pub experiment: Option<(String, &'static str)>,
}

pub async fn select(
    conn: &mut PgConnection,
    intent_id: &str,
    ctx: &RuleCtx<'_>,
    legacy: &[Rule],
) -> Result<Selection, ApiError> {
    let active = active_version(&mut *conn).await?;
    let control = |rules: Option<Vec<BuilderRule>>| match rules {
        Some(r) => compile(&r, ctx),
        None => (legacy.to_vec(), Vec::new()),
    };
    let control_rules = match active {
        Some(v) => Some(rules_of(&mut *conn, v).await?),
        None => None,
    };
    if let Some(exp) = crate::experiments::running(&mut *conn).await?
        && exp
            .method
            .as_deref()
            .is_none_or(|m| m == ctx.method.as_str())
    {
        let arm = crate::experiments::arm_for(&exp.salt, intent_id, exp.variant_bps);
        let (c_rules, c_matched) = control(control_rules);
        if arm == "variant" {
            let vr = rules_of(&mut *conn, exp.variant_version).await?;
            let (v_rules, v_matched) = compile(&vr, ctx);
            return Ok(Selection {
                rules: v_rules,
                fallback: Some(c_rules),
                version: Some(exp.variant_version),
                matched: v_matched,
                experiment: Some((exp.id, "variant")),
            });
        }
        return Ok(Selection {
            rules: c_rules,
            fallback: None,
            version: active,
            matched: c_matched,
            experiment: Some((exp.id, "control")),
        });
    }
    let (rules, matched) = control(control_rules);
    Ok(Selection {
        rules,
        fallback: None,
        version: active,
        matched,
        experiment: None,
    })
}

/// Choose a provider with the selected rules; a variant that leaves nothing falls back to the control's rules, so an
/// experiment can never make a payment fail that would have been routed. Returns whether it fell back.
pub fn choose_with(
    sel: &Selection,
    req: &router::Request,
    candidates: &[Candidate],
) -> Result<(Decision, bool), NoRoute> {
    match router::choose(req, candidates, &sel.rules) {
        Ok(d) => Ok((d, false)),
        Err(e) => match &sel.fallback {
            Some(f) => router::choose(req, candidates, f).map(|d| (d, true)),
            None => Err(e),
        },
    }
}

/// The stored `route` of an attempt: the router's decision plus which rule set and experiment arm chose.
pub fn route_json(d: &Decision, sel: &Selection, fell_back: bool, offers: Option<Value>) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or(Value::Null);
    v["rule_set"] = json!({
        "version": if fell_back { Value::Null } else { json!(sel.version) },
        "source": if fell_back { "control_fallback" } else if sel.version.is_some() { "rule_set" } else { "settings" },
        "matched_rules": if fell_back { json!([]) } else { json!(sel.matched) },
    });
    if let Some((id, arm)) = &sel.experiment {
        v["experiment"] = json!({ "id": id, "arm": arm, "fell_back_to_control": fell_back });
    }
    if let Some(o) = offers {
        v["offers"] = o;
    }
    v
}

pub async fn record_assignment(
    conn: &mut PgConnection,
    tenant_id: &str,
    sel: &Selection,
    fell_back: bool,
    attempt_id: &str,
    intent_id: &str,
) -> Result<(), ApiError> {
    if let Some((exp, arm)) = &sel.experiment {
        sqlx::query("INSERT INTO routing_experiment_assignments (attempt_id, tenant_id, experiment_id, intent_id, arm, fell_back) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(attempt_id)
            .bind(tenant_id)
            .bind(exp)
            .bind(intent_id)
            .bind(arm)
            .bind(fell_back)
            .execute(conn)
            .await?;
    }
    Ok(())
}

// ------------------------------------------------------------------ API

#[derive(Deserialize)]
pub struct RulesBody {
    rules: Vec<BuilderRule>,
}

async fn accounts(conn: &mut PgConnection) -> Result<Vec<(String, bool)>, ApiError> {
    Ok(
        sqlx::query_as("SELECT provider, enabled FROM provider_accounts ORDER BY provider")
            .fetch_all(conn)
            .await?,
    )
}

pub async fn validate_rules(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<RulesBody>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let acc = accounts(&mut tx).await?;
    Ok(Json(json!(validate(&b.rules, &acc))))
}

#[derive(Deserialize)]
pub struct NewVersion {
    rules: Vec<BuilderRule>,
    #[serde(default)]
    note: Option<String>,
    /// Make it the active rule set straight away.
    #[serde(default)]
    activate: bool,
}

pub async fn create_version(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewVersion>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("routing:write")?;
    if b.note.as_deref().is_some_and(|n| n.len() > 300) {
        return Err(ApiError::invalid(
            "invalid_rules",
            "note is at most 300 characters",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let acc = accounts(&mut tx).await?;
    let report = validate(&b.rules, &acc);
    if !report.valid {
        let first = report
            .errors
            .first()
            .map(|e| e.message.clone())
            .unwrap_or_default();
        let mut err =
            ApiError::invalid("invalid_rules", format!("the rules are not valid: {first}"));
        err.message = format!(
            "{} ({} problem(s); POST /v1/routing/rule_sets/validate lists them)",
            err.message,
            report.errors.len()
        );
        return Err(err);
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('zyvor.routing_rules'), hashtext($1))")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    let version: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM routing_rule_versions")
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO routing_rule_versions (tenant_id, version, rules, note, created_by) VALUES ($1,$2,$3,$4,$5)")
        .bind(&auth.tenant_id)
        .bind(version)
        .bind(json!(b.rules))
        .bind(&b.note)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "routing_rules.version_created",
        Some(&version.to_string()),
        json!({ "version": version, "rules": b.rules.len(), "note": b.note }),
    )
    .await?;
    if b.activate {
        switch(&mut tx, &auth, Some(version), "routing_rules.activated").await?;
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "version": version, "active": b.activate, "warnings": report.warnings })),
    ))
}

/// Move the active pointer (None = back to the settings rules). Refused while an experiment is running, so the
/// comparison it is making stays valid.
async fn switch(
    conn: &mut PgConnection,
    auth: &Auth,
    to: Option<i32>,
    action: &str,
) -> Result<(), ApiError> {
    if crate::experiments::running(&mut *conn).await?.is_some() {
        return Err(ApiError::conflict(
            "experiment_running",
            "a routing experiment is running; stop it before changing the active rule set",
        ));
    }
    let cur = active_version(&mut *conn).await?;
    sqlx::query(
        "INSERT INTO routing_rule_active (tenant_id, version, previous_version, activated_by) VALUES ($1, $2, NULL, $3)
         ON CONFLICT (tenant_id) DO UPDATE SET previous_version = $4, version = $2, activated_by = $3, activated_at = now()",
    )
    .bind(&auth.tenant_id)
    .bind(to)
    .bind(&auth.actor)
    .bind(cur)
    .execute(&mut *conn)
    .await?;
    tenant::audit(
        conn,
        &auth.tenant_id,
        &auth.actor,
        action,
        Some(
            &to.map(|v| v.to_string())
                .unwrap_or_else(|| "settings".into()),
        ),
        json!({ "from_version": cur, "to_version": to }),
    )
    .await?;
    Ok(())
}

pub async fn list_versions(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let active: Option<(Option<i32>, Option<i32>, DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT version, previous_version, activated_at, activated_by FROM routing_rule_active",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let rows: Vec<(i32, Option<String>, String, DateTime<Utc>, i32)> = sqlx::query_as(
        "SELECT version, note, created_by, created_at, jsonb_array_length(rules) FROM routing_rule_versions ORDER BY version DESC LIMIT 200",
    )
    .fetch_all(&mut *tx)
    .await?;
    let cur = active.as_ref().and_then(|a| a.0);
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(v, note, by, at, n)| json!({ "version": v, "note": note, "created_by": by, "created_at": at, "rules": n, "active": cur == Some(v) }))
        .collect();
    Ok(Json(json!({
        "active_version": cur,
        "previous_version": active.as_ref().and_then(|a| a.1),
        "activated_at": active.as_ref().map(|a| a.2),
        "activated_by": active.as_ref().map(|a| a.3.clone()),
        "source": if cur.is_some() { "rule_set" } else { "settings" },
        "data": data,
    })))
}

pub async fn get_version(
    State(s): State<AppState>,
    auth: Auth,
    Path(version): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(Value, Option<String>, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT rules, note, created_by, created_at FROM routing_rule_versions WHERE version = $1",
    )
    .bind(version)
    .fetch_optional(&mut *tx)
    .await?;
    let (rules, note, by, at) = row.ok_or_else(|| ApiError::not_found("rule set version"))?;
    let active = active_version(&mut tx).await?;
    Ok(Json(
        json!({ "version": version, "rules": rules, "note": note, "created_by": by, "created_at": at, "active": active == Some(version) }),
    ))
}

pub async fn activate(
    State(s): State<AppState>,
    auth: Auth,
    Path(version): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('zyvor.routing_rules'), hashtext($1))")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM routing_rule_versions WHERE version = $1)",
    )
    .bind(version)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(ApiError::not_found("rule set version"));
    }
    // activating again what is active changes nothing
    if active_version(&mut tx).await? == Some(version) {
        return Ok(Json(json!({ "active_version": version, "changed": false })));
    }
    switch(&mut tx, &auth, Some(version), "routing_rules.activated").await?;
    tx.commit().await?;
    Ok(Json(json!({ "active_version": version, "changed": true })))
}

/// Back to the rule set (or the settings rules) that was active before the current one.
pub async fn rollback(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('zyvor.routing_rules'), hashtext($1))")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    let row: Option<(Option<i32>, Option<i32>)> =
        sqlx::query_as("SELECT version, previous_version FROM routing_rule_active")
            .fetch_optional(&mut *tx)
            .await?;
    let Some((cur, prev)) = row else {
        return Err(ApiError::conflict(
            "nothing_to_roll_back",
            "no rule set has been activated yet",
        ));
    };
    if cur.is_none() {
        return Err(ApiError::conflict(
            "nothing_to_roll_back",
            "the settings rules are already in force",
        ));
    }
    switch(&mut tx, &auth, prev, "routing_rules.rolled_back").await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "active_version": prev, "rolled_back_from": cur, "source": if prev.is_some() { "rule_set" } else { "settings" } }),
    ))
}

/// Stop using any rule set: the older `routing.rules` in the settings apply again.
pub async fn deactivate(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('zyvor.routing_rules'), hashtext($1))")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    if active_version(&mut tx).await?.is_none() {
        return Ok(Json(json!({ "active_version": null, "changed": false })));
    }
    switch(&mut tx, &auth, None, "routing_rules.deactivated").await?;
    tx.commit().await?;
    Ok(Json(json!({ "active_version": null, "changed": true })))
}

#[derive(Deserialize)]
pub struct DryRun {
    method: String,
    amount_minor: i64,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    card_network: Option<String>,
    #[serde(default)]
    card_bin: Option<String>,
    #[serde(default)]
    customer_id: Option<String>,
    /// `new` or `returning`, instead of a customer.
    #[serde(default)]
    segment: Option<String>,
    /// The moment to judge time-of-day rules at (default: now).
    #[serde(default)]
    at: Option<DateTime<Utc>>,
    /// Try rules that are not saved.
    #[serde(default)]
    rules: Option<Vec<BuilderRule>>,
    /// Try a saved version instead of the active one.
    #[serde(default)]
    version: Option<i32>,
    /// With a running experiment: which arm this payment (intent) would be in.
    #[serde(default)]
    intent_id: Option<String>,
    /// With a running experiment: force `control` or `variant`.
    #[serde(default)]
    arm: Option<String>,
}

/// Which provider would this payment take, and why. Creates nothing, claims no circuit-breaker probe.
pub async fn dry_run(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<DryRun>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let method: Method = b
        .method
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_method", e))?;
    let currency: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be a positive integer within the supported limit",
        ));
    }
    let hint = instrument::InstrumentIn {
        card_network: b.card_network.clone(),
        card_bin: b.card_bin.clone(),
        ..Default::default()
    };
    let ins = instrument::build(Some(&hint), None, None)?;
    if let Some(sg) = &b.segment
        && !matches!(sg.as_str(), "new" | "returning")
    {
        return Err(ApiError::invalid(
            "invalid_segment",
            "segment is new or returning",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let seg: Option<&'static str> = match (&b.segment, &b.customer_id) {
        (Some(sg), _) => Some(if sg == "new" { "new" } else { "returning" }),
        (None, Some(c)) => {
            if !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)",
            )
            .bind(c)
            .fetch_one(&mut *tx)
            .await?
            {
                return Err(ApiError::not_found("customer"));
            }
            crate::offers::segment(&mut tx, Some(c), None).await?
        }
        _ => None,
    };
    let ctx = RuleCtx {
        method,
        amount_minor: b.amount_minor,
        currency: currency.code(),
        card_network: ins.card_network.as_deref(),
        segment: seg,
        at: b.at.unwrap_or_else(Utc::now),
    };
    let routing = registry::routing(&mut tx, &auth.tenant_id).await?;
    let mut experiment = Value::Null;
    let (sel, source): (Selection, Value) = if let Some(rules) = &b.rules {
        let acc = accounts(&mut tx).await?;
        let rep = validate(rules, &acc);
        if !rep.valid {
            return Err(ApiError::invalid(
                "invalid_rules",
                format!("the rules are not valid: {}", rep.errors[0].message),
            ));
        }
        let (r, m) = compile(rules, &ctx);
        (
            Selection {
                rules: r,
                fallback: None,
                version: None,
                matched: m,
                experiment: None,
            },
            json!({ "kind": "inline" }),
        )
    } else if let Some(v) = b.version {
        let rules = rules_of(&mut tx, v).await?;
        let (r, m) = compile(&rules, &ctx);
        (
            Selection {
                rules: r,
                fallback: None,
                version: Some(v),
                matched: m,
                experiment: None,
            },
            json!({ "kind": "version", "version": v }),
        )
    } else {
        // the same selection a real payment gets, including an experiment's arm
        let intent = b.intent_id.clone().unwrap_or_else(|| "dry-run".into());
        let mut sel = select(&mut tx, &intent, &ctx, &routing.rules).await?;
        if let (Some(forced), Some((id, _))) = (b.arm.as_deref(), sel.experiment.clone()) {
            let exp = crate::experiments::running(&mut tx)
                .await?
                .ok_or_else(ApiError::internal)?;
            let want_variant = match forced {
                "variant" => true,
                "control" => false,
                _ => {
                    return Err(ApiError::invalid(
                        "invalid_arm",
                        "arm is control or variant",
                    ));
                }
            };
            let active = active_version(&mut tx).await?;
            let (rules, version) = if want_variant {
                (Some(exp.variant_version), Some(exp.variant_version))
            } else {
                (active, active)
            };
            let compiled = match rules {
                Some(v) => compile(&rules_of(&mut tx, v).await?, &ctx),
                None => (routing.rules.clone(), Vec::new()),
            };
            sel = Selection {
                rules: compiled.0,
                fallback: None,
                version,
                matched: compiled.1,
                experiment: Some((id, if want_variant { "variant" } else { "control" })),
            };
        }
        if let Some((id, arm)) = &sel.experiment {
            experiment = json!({ "id": id, "arm": arm });
        }
        let src = if sel.version.is_some() {
            json!({ "kind": "rule_set", "version": sel.version })
        } else {
            json!({ "kind": "settings" })
        };
        (sel, src)
    };
    let mut candidates =
        registry::candidates(&s, &mut tx, &auth.tenant_id, &auth.env, &routing).await?;
    let cfg = crate::health::load_config(&mut tx, &auth.tenant_id).await?;
    crate::health::attach_signals(&mut tx, &auth.tenant_id, &cfg, method, &mut candidates).await?;
    let req = router::Request {
        method,
        amount_minor: b.amount_minor,
        currency,
        allowed: None,
        upi_intent: false,
        avoid: Vec::new(),
    };
    let (decision, fell_back) = choose_with(&sel, &req, &candidates)
        .map_err(|e| ApiError::invalid("no_route", e.to_string()))?;
    let skipped: Vec<Value> = rules_not_matched(&b, &ctx, &mut tx, &sel).await?;
    Ok(Json(json!({
        "dry_run": true,
        "provider": decision.provider,
        "explanation": decision.explanation,
        "would_probe": decision.probe,
        "decision": decision,
        "rule_source": source,
        "matched_rules": if fell_back { json!([]) } else { json!(sel.matched) },
        "unmatched_rules": skipped,
        "experiment": experiment,
        "fell_back_to_control": fell_back,
        "notes": ["Nothing was created. No circuit-breaker probe was claimed; a real payment decides that at the moment it is made."],
    })))
}

/// Which rules of the rule set in use did not apply, and why (so a person can see why a rule was ignored).
async fn rules_not_matched(
    b: &DryRun,
    ctx: &RuleCtx<'_>,
    conn: &mut PgConnection,
    sel: &Selection,
) -> Result<Vec<Value>, ApiError> {
    let rules = if let Some(r) = &b.rules {
        r.clone()
    } else if let Some(v) = sel.version {
        rules_of(conn, v).await?
    } else {
        return Ok(Vec::new());
    };
    Ok(rules
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            r.when
                .mismatch(ctx)
                .map(|why| json!({ "index": i, "name": r.name, "because": why }))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rule(when: When, then: Then) -> BuilderRule {
        BuilderRule {
            name: None,
            when,
            then,
        }
    }
    fn prefer(p: &str) -> Then {
        Then {
            prefer: vec![p.into()],
            ..Default::default()
        }
    }
    fn ctx(at: DateTime<Utc>) -> RuleCtx<'static> {
        RuleCtx {
            method: Method::Card,
            amount_minor: 50_000,
            currency: "INR",
            card_network: Some("visa"),
            segment: Some("new"),
            at,
        }
    }
    fn accounts() -> Vec<(String, bool)> {
        vec![("razorpay".into(), true), ("cashfree".into(), true)]
    }

    #[test]
    fn time_windows_cross_midnight_in_the_chosen_offset() {
        // 22:00-06:00 India time: 17:00 UTC is 22:30 IST (inside), 02:00 UTC is 07:30 IST (outside)
        let w = TimeWindow {
            from: "22:00".into(),
            to: "06:00".into(),
            utc_offset_minutes: 330,
        };
        assert!(in_window(
            &w,
            Utc.with_ymd_and_hms(2026, 11, 18, 17, 0, 0).unwrap()
        ));
        assert!(in_window(
            &w,
            Utc.with_ymd_and_hms(2026, 11, 18, 21, 0, 0).unwrap()
        )); // 02:30 IST
        assert!(!in_window(
            &w,
            Utc.with_ymd_and_hms(2026, 11, 18, 2, 0, 0).unwrap()
        ));
        // a same-day window is [from, to)
        let d = TimeWindow {
            from: "09:00".into(),
            to: "17:00".into(),
            utc_offset_minutes: 0,
        };
        assert!(in_window(
            &d,
            Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0).unwrap()
        ));
        assert!(!in_window(
            &d,
            Utc.with_ymd_and_hms(2026, 11, 18, 17, 0, 0).unwrap()
        ));
    }

    #[test]
    fn conditions_choose_the_rules_that_reach_the_router() {
        let at = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
        let rules = [
            rule(
                When {
                    card_networks: vec!["visa".into()],
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When {
                    methods: vec!["upi".into()],
                    ..Default::default()
                },
                prefer("cashfree"),
            ),
            rule(
                When {
                    min_amount_minor: Some(100_000),
                    ..Default::default()
                },
                prefer("cashfree"),
            ),
            rule(
                When {
                    segments: vec!["returning".into()],
                    ..Default::default()
                },
                prefer("cashfree"),
            ),
        ];
        let (compiled, matched) = compile(&rules, &ctx(at));
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].prefer, vec!["razorpay".to_string()]);
        assert_eq!(matched[0]["index"], 0);
        // unknown network or unknown segment never matches a rule that asks for one
        let mut c = ctx(at);
        c.card_network = None;
        assert!(rules[0].when.mismatch(&c).is_some());
        c.segment = None;
        assert!(rules[3].when.mismatch(&c).unwrap().contains("no customer"));
    }

    #[test]
    fn validation_reports_every_mistake_with_the_rule_it_is_in() {
        let bad = [
            rule(
                When {
                    methods: vec!["crypto".into()],
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When {
                    min_amount_minor: Some(10),
                    max_amount_minor: Some(5),
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(When::default(), Then::default()),
            rule(
                When::default(),
                Then {
                    prefer: vec!["razorpay".into()],
                    exclude: vec!["razorpay".into()],
                    ..Default::default()
                },
            ),
            rule(When::default(), prefer("nosuchbank")),
            rule(
                When {
                    time_window: Some(TimeWindow {
                        from: "9:00".into(),
                        to: "17:00".into(),
                        utc_offset_minutes: 330,
                    }),
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When {
                    time_window: Some(TimeWindow {
                        from: "10:00".into(),
                        to: "10:00".into(),
                        utc_offset_minutes: 330,
                    }),
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When {
                    methods: vec!["upi".into()],
                    card_networks: vec!["visa".into()],
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When {
                    card_networks: vec!["amexx".into()],
                    ..Default::default()
                },
                prefer("razorpay"),
            ),
            rule(
                When::default(),
                Then {
                    exclude: vec!["razorpay".into(), "cashfree".into()],
                    ..Default::default()
                },
            ),
            rule(
                When::default(),
                Then {
                    prefer_success_rate: false,
                    success_rate_margin_bps: Some(100),
                    prefer: vec!["razorpay".into()],
                    ..Default::default()
                },
            ),
        ];
        let rep = validate(&bad, &accounts());
        assert!(!rep.valid);
        for i in 0..bad.len() {
            assert!(
                rep.errors.iter().any(|e| e.rule == Some(i)),
                "rule {i} should have an error: {:?}",
                rep.errors
            );
        }
        // a good one has none, and a provider with no account is only a warning
        let good = [rule(When::default(), prefer("payu"))];
        let rep = validate(&good, &accounts());
        assert!(rep.valid, "{:?}", rep.errors);
        assert_eq!(rep.warnings.len(), 1);
        let many: Vec<_> = (0..51)
            .map(|_| rule(When::default(), prefer("razorpay")))
            .collect();
        assert!(
            validate(&many, &accounts())
                .errors
                .iter()
                .any(|e| e.rule.is_none())
        );
    }

    #[test]
    fn unknown_fields_are_refused_so_a_typo_does_not_silently_do_nothing() {
        let r: Result<BuilderRule, _> = serde_json::from_value(
            json!({ "when": { "card_network": ["visa"] }, "then": { "prefer": ["razorpay"] } }),
        );
        assert!(r.is_err());
        let ok: BuilderRule = serde_json::from_value(
            json!({ "when": { "card_networks": ["visa"] }, "then": { "prefer": ["razorpay"] } }),
        )
        .unwrap();
        assert_eq!(ok.when.card_networks, vec!["visa".to_string()]);
    }
}
