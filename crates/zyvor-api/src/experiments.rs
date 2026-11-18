//! Routing experiments: send a share of payments through a different saved rule set and compare success rates.
//!
//! The split is by hash of the experiment's salt and the payment intent id, so every attempt on one payment is in
//! the same arm (a retry or failover never changes arm), and the same intent always lands in the same arm. The
//! control arm is the rule set that is active when the experiment starts; activating, rolling back or deactivating a
//! rule set is refused while one runs, so the comparison stays valid. The variant is an ordinary saved version, so
//! every router guard applies to it too. If the variant's rules leave a payment with no provider, the control's rules
//! route it and the attempt is left out of the results (`fell_back`): an experiment cannot make a payment fail.
//!
//! Guardrails: nothing is concluded before BOTH arms have `min_sample` resolved attempts (default 100). The variant
//! is then stopped automatically only when it is clearly worse: its success rate is at least `min_drop_bps` below the
//! control's (default 300, that is 3 percentage points) AND a one-sided two-proportion z-test says so at z >= 2.33
//! (about 99% one-sided; `z_x100` 233). The check runs in the worker and on `POST .../evaluate`, never on a read.
//! Looking repeatedly raises the chance of a false stop, so the threshold is deliberately strict. A variant that
//! looks better is only reported: it is never promoted by itself.
//!
//! What counts: successes are captured or authorised attempts; failures are provider or bank side failures and
//! declines of unknown cause; attempts with an unknown outcome count against the arm (as the circuit breaker counts
//! them); payer declines and merchant cancellations are left out of the rates, shown on their own.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use zyvor_core::new_id;
use zyvor_providers::Method;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, tenant};

pub struct Running {
    pub id: String,
    pub method: Option<String>,
    pub salt: String,
    pub variant_version: i32,
    pub variant_bps: i32,
}

pub async fn running(conn: &mut PgConnection) -> Result<Option<Running>, ApiError> {
    let r: Option<(String, Option<String>, String, i32, i32)> = sqlx::query_as(
        "SELECT id, method, salt, variant_version, variant_bps FROM routing_experiments WHERE status = 'running'",
    )
    .fetch_optional(conn)
    .await?;
    Ok(
        r.map(|(id, method, salt, variant_version, variant_bps)| Running {
            id,
            method,
            salt,
            variant_version,
            variant_bps,
        }),
    )
}

/// 0..9999, the same for the same salt and intent.
pub fn bucket(salt: &str, intent_id: &str) -> u32 {
    let h = Sha256::digest(format!("{salt}:{intent_id}").as_bytes());
    (u64::from_be_bytes(h[..8].try_into().expect("8 bytes")) % 10_000) as u32
}

pub fn arm_for(salt: &str, intent_id: &str, variant_bps: i32) -> &'static str {
    if i64::from(bucket(salt, intent_id)) < i64::from(variant_bps) {
        "variant"
    } else {
        "control"
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArmStats {
    pub successes: u32,
    pub failures: u32,
    pub unresolved: u32,
    pub payer_declines: u32,
    pub in_flight: u32,
}

impl ArmStats {
    pub fn samples(&self) -> u32 {
        self.successes + self.failures + self.unresolved
    }
    pub fn rate_bps(&self) -> Option<u32> {
        let n = self.samples();
        (n > 0).then(|| (u64::from(self.successes) * 10_000 / u64::from(n)) as u32)
    }
    fn to_json(self) -> Value {
        json!({
            "samples": self.samples(), "successes": self.successes, "failures": self.failures, "unresolved": self.unresolved,
            "payer_declines": self.payer_declines, "in_flight": self.in_flight, "success_rate_bps": self.rate_bps(),
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    NotEnoughData { control: u32, variant: u32 },
    NoClearDifference { drop_bps: i64 },
    VariantWorse { drop_bps: i64, z_x100: i64 },
    VariantBetter { gain_bps: i64, z_x100: i64 },
}

/// One-sided two-proportion z for "arm a is better than arm b", times 100, rounded. 0 when there is no variance.
fn z_x100(a: &ArmStats, b: &ArmStats) -> i64 {
    let (na, nb) = (f64::from(a.samples()), f64::from(b.samples()));
    if na == 0.0 || nb == 0.0 {
        return 0;
    }
    let (pa, pb) = (f64::from(a.successes) / na, f64::from(b.successes) / nb);
    let pooled = f64::from(a.successes + b.successes) / (na + nb);
    let se = (pooled * (1.0 - pooled) * (1.0 / na + 1.0 / nb)).sqrt();
    if se == 0.0 {
        return 0;
    }
    (((pa - pb) / se) * 100.0).round() as i64
}

/// The whole rule, pure. `min_sample` per arm, `min_drop_bps` in basis points of success rate, `z` times 100.
pub fn judge(
    control: &ArmStats,
    variant: &ArmStats,
    min_sample: u32,
    min_drop_bps: i64,
    z: i64,
) -> Verdict {
    if control.samples() < min_sample || variant.samples() < min_sample {
        return Verdict::NotEnoughData {
            control: control.samples(),
            variant: variant.samples(),
        };
    }
    let (rc, rv) = (
        i64::from(control.rate_bps().unwrap_or(0)),
        i64::from(variant.rate_bps().unwrap_or(0)),
    );
    let drop_bps = rc - rv;
    let zc = z_x100(control, variant);
    if drop_bps >= min_drop_bps && zc >= z {
        return Verdict::VariantWorse {
            drop_bps,
            z_x100: zc,
        };
    }
    let zv = z_x100(variant, control);
    if -drop_bps >= min_drop_bps && zv >= z {
        return Verdict::VariantBetter {
            gain_bps: -drop_bps,
            z_x100: zv,
        };
    }
    Verdict::NoClearDifference { drop_bps }
}

const NOT_CANCELLED: &str = "NOT (p.status = 'failed' AND (p.failure_class IS NOT DISTINCT FROM 'cancelled' OR p.failure_reason IS NOT DISTINCT FROM 'canceled_by_merchant'))";

async fn arm_stats(
    conn: &mut PgConnection,
    exp_id: &str,
) -> Result<(ArmStats, ArmStats), ApiError> {
    let rows: Vec<(String, i64, i64, i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT a.arm,
            COUNT(*) FILTER (WHERE p.status IN ('captured', 'authorized')),
            COUNT(*) FILTER (WHERE p.status = 'failed' AND {NOT_CANCELLED} AND p.failure_class IS DISTINCT FROM 'payer_declined'),
            COUNT(*) FILTER (WHERE p.status = 'unknown'),
            COUNT(*) FILTER (WHERE p.status = 'failed' AND p.failure_class = 'payer_declined'),
            COUNT(*) FILTER (WHERE p.status IN ('created', 'pending'))
         FROM routing_experiment_assignments a JOIN payment_attempts p ON p.id = a.attempt_id
         WHERE a.experiment_id = $1 AND NOT a.fell_back GROUP BY a.arm"
    )))
    .bind(exp_id)
    .fetch_all(conn)
    .await?;
    let mut c = ArmStats::default();
    let mut v = ArmStats::default();
    for (arm, s, f, u, pd, i) in rows {
        let st = ArmStats {
            successes: s as u32,
            failures: f as u32,
            unresolved: u as u32,
            payer_declines: pd as u32,
            in_flight: i as u32,
        };
        if arm == "variant" {
            v = st;
        } else {
            c = st;
        }
    }
    Ok((c, v))
}

struct Exp {
    id: String,
    name: String,
    status: String,
    control_version: Option<i32>,
    variant_version: i32,
    variant_bps: i32,
    method: Option<String>,
    min_sample: i32,
    min_drop_bps: i32,
    z_x100: i32,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    stop_reason: Option<String>,
}

#[allow(clippy::type_complexity)]
async fn load(conn: &mut PgConnection, id: &str, lock: bool) -> Result<Exp, ApiError> {
    let sql = format!(
        "SELECT id, name, status, control_version, variant_version, variant_bps, method, min_sample, min_drop_bps, z_x100, started_at, ended_at, stop_reason FROM routing_experiments WHERE id = $1{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    let r: Option<(
        String,
        String,
        String,
        Option<i32>,
        i32,
        i32,
        Option<String>,
        i32,
        i32,
        i32,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<String>,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?;
    let r = r.ok_or_else(|| ApiError::not_found("routing experiment"))?;
    Ok(Exp {
        id: r.0,
        name: r.1,
        status: r.2,
        control_version: r.3,
        variant_version: r.4,
        variant_bps: r.5,
        method: r.6,
        min_sample: r.7,
        min_drop_bps: r.8,
        z_x100: r.9,
        started_at: r.10,
        ended_at: r.11,
        stop_reason: r.12,
    })
}

fn verdict_json(v: &Verdict) -> Value {
    match v {
        Verdict::NotEnoughData { control, variant } => {
            json!({ "state": "not_enough_data", "control_samples": control, "variant_samples": variant })
        }
        Verdict::NoClearDifference { drop_bps } => {
            json!({ "state": "no_clear_difference", "control_minus_variant_bps": drop_bps })
        }
        Verdict::VariantWorse { drop_bps, z_x100 } => {
            json!({ "state": "variant_worse", "control_minus_variant_bps": drop_bps, "z_x100": z_x100 })
        }
        Verdict::VariantBetter { gain_bps, z_x100 } => {
            json!({ "state": "variant_better", "variant_minus_control_bps": gain_bps, "z_x100": z_x100, "note": "never promoted automatically; activate the variant yourself if you want it" })
        }
    }
}

async fn results_body(
    conn: &mut PgConnection,
    tenant_id: &str,
    e: &Exp,
) -> Result<Value, ApiError> {
    let paused = crate::routing_auto::load(&mut *conn, tenant_id)
        .await?
        .paused;
    let (c, v) = arm_stats(conn, &e.id).await?;
    let verdict = judge(
        &c,
        &v,
        e.min_sample as u32,
        i64::from(e.min_drop_bps),
        i64::from(e.z_x100),
    );
    let by_provider: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT a.arm, p.provider, COUNT(*) FROM routing_experiment_assignments a JOIN payment_attempts p ON p.id = a.attempt_id
         WHERE a.experiment_id = $1 AND NOT a.fell_back GROUP BY a.arm, p.provider ORDER BY a.arm, p.provider",
    )
    .bind(&e.id)
    .fetch_all(&mut *conn)
    .await?;
    let fell_back: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM routing_experiment_assignments WHERE experiment_id = $1 AND fell_back")
        .bind(&e.id)
        .fetch_one(&mut *conn)
        .await?;
    let providers = |arm: &str| -> Value {
        json!(
            by_provider
                .iter()
                .filter(|(a, _, _)| a == arm)
                .map(|(_, p, n)| json!({ "provider": p, "attempts": n }))
                .collect::<Vec<_>>()
        )
    };
    Ok(json!({
        "id": e.id, "name": e.name, "status": e.status,
        "control_version": e.control_version, "variant_version": e.variant_version,
        "variant_percent": f64::from(e.variant_bps) / 100.0, "method": e.method,
        "started_at": e.started_at, "ended_at": e.ended_at, "stop_reason": e.stop_reason,
        "guardrails": {
            "min_sample_per_arm": e.min_sample, "min_drop_bps": e.min_drop_bps, "z_x100": e.z_x100,
            "rule": "Stopped automatically only when both arms have min_sample resolved attempts, the variant's success rate is at least min_drop_bps below the control's, and a one-sided two-proportion z-test gives z >= z_x100 / 100. A better-looking variant is reported, never promoted.",
        },
        "control": { "stats": c.to_json(), "providers": providers("control") },
        "variant": { "stats": v.to_json(), "providers": providers("variant") },
        "left_out_fell_back_to_control": fell_back,
        "verdict": verdict_json(&verdict),
        "would_auto_stop": e.status == "running" && matches!(verdict, Verdict::VariantWorse { .. }),
        "automation_paused": paused,
    }))
}

/// Evaluate a running experiment and stop it if the variant is clearly worse. Safe to run from any number of
/// workers and any number of times: the experiment row is locked and only a `running` one changes. Returns the
/// reason when it stopped it.
pub async fn evaluate_and_apply(
    conn: &mut PgConnection,
    tenant_id: &str,
    exp_id: &str,
) -> Result<Option<String>, ApiError> {
    let e = load(conn, exp_id, true).await?;
    if e.status != "running" {
        return Ok(None);
    }
    let (c, v) = arm_stats(conn, exp_id).await?;
    let Verdict::VariantWorse { drop_bps, z_x100 } = judge(
        &c,
        &v,
        e.min_sample as u32,
        i64::from(e.min_drop_bps),
        i64::from(e.z_x100),
    ) else {
        return Ok(None);
    };
    let reason = format!(
        "variant success rate {} bps against control {} bps ({} bps lower, z = {:.2}) with {} and {} attempts",
        v.rate_bps().unwrap_or(0),
        c.rate_bps().unwrap_or(0),
        drop_bps,
        z_x100 as f64 / 100.0,
        v.samples(),
        c.samples()
    );
    sqlx::query("UPDATE routing_experiments SET status = 'auto_stopped', ended_at = now(), stop_reason = $2 WHERE id = $1")
        .bind(exp_id)
        .bind(&reason)
        .execute(&mut *conn)
        .await?;
    tenant::audit(
        conn,
        tenant_id,
        "system",
        "routing_experiment.auto_stopped",
        Some(exp_id),
        json!({ "reason": reason, "control": c.to_json(), "variant": v.to_json() }),
    )
    .await?;
    open_case(conn, tenant_id, "routing_experiment_stopped", "routing_experiment", exp_id, None, None, &format!("Routing experiment {} was stopped automatically: {reason}. The control rules are in force again.", e.name), json!({ "variant_version": e.variant_version })).await?;
    tenant::emit(
        conn,
        tenant_id,
        "routing.experiment_stopped",
        json!({ "experiment_id": exp_id, "name": e.name, "reason": reason, "auto": true }),
    )
    .await?;
    Ok(Some(reason))
}

/// Worker step: check the business's running experiment, if any.
pub async fn sweep_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<bool, ApiError> {
    let mut tx = tenant::begin(pool, tenant_id).await?;
    // the routing kill switch: no experiment traffic is split while it is on, so there is nothing new to judge
    if crate::routing_auto::load(&mut tx, tenant_id).await?.paused {
        return Ok(false);
    }
    let Some(r) = running(&mut tx).await? else {
        return Ok(false);
    };
    let stopped = evaluate_and_apply(&mut tx, tenant_id, &r.id)
        .await?
        .is_some();
    tx.commit().await?;
    Ok(stopped)
}

#[derive(Deserialize)]
pub struct NewExperiment {
    name: String,
    /// The saved rule set version to try on part of the traffic.
    variant_version: i32,
    /// Share of payments sent to the variant, 1 to 50.
    variant_percent: i32,
    /// Only payments of this method take part.
    #[serde(default)]
    method: Option<String>,
    /// Resolved attempts needed in each arm before anything is concluded (20 to 100000, default 100).
    #[serde(default)]
    min_sample: Option<i32>,
    /// How much lower (basis points) the variant's success rate must be before it is stopped (50 to 10000, default 300).
    #[serde(default)]
    min_drop_bps: Option<i32>,
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewExperiment>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("routing:write")?;
    let bad = |m: &str| ApiError::invalid("invalid_experiment", m.to_string());
    if b.name.trim().is_empty() || b.name.len() > 80 {
        return Err(bad("name must be 1-80 characters"));
    }
    if !(1..=50).contains(&b.variant_percent) {
        return Err(bad("variant_percent must be between 1 and 50"));
    }
    if let Some(m) = &b.method
        && m.parse::<Method>().is_err()
    {
        return Err(bad("unknown method"));
    }
    if b.min_sample.is_some_and(|v| !(20..=100_000).contains(&v))
        || b.min_drop_bps.is_some_and(|v| !(50..=10_000).contains(&v))
    {
        return Err(bad(
            "min_sample must be 20-100000 and min_drop_bps 50-10000",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('zyvor.routing_rules'), hashtext($1))")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM routing_rule_versions WHERE version = $1)",
    )
    .bind(b.variant_version)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(ApiError::not_found("rule set version"));
    }
    let control = crate::rulebuilder::active_version(&mut tx).await?;
    if control == Some(b.variant_version) {
        return Err(bad(
            "the variant is the active rule set; pick another version",
        ));
    }
    let id = new_id("rex");
    sqlx::query(
        "INSERT INTO routing_experiments (id, tenant_id, name, control_version, variant_version, variant_bps, method, salt, min_sample, min_drop_bps, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,COALESCE($9, 100),COALESCE($10, 300),$11)",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(b.name.trim())
    .bind(control)
    .bind(b.variant_version)
    .bind(b.variant_percent * 100)
    .bind(&b.method)
    .bind(new_id("salt"))
    .bind(b.min_sample)
    .bind(b.min_drop_bps)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("experiment_running", "an experiment is already running; stop it first"),
        e => e.into(),
    })?;
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "routing_experiment.started", Some(&id), json!({ "control_version": control, "variant_version": b.variant_version, "variant_percent": b.variant_percent, "method": b.method })).await?;
    let e = load(&mut tx, &id, false).await?;
    let body = results_body(&mut tx, &auth.tenant_id, &e).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(body)))
}

pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, Option<i32>, i32, i32, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, name, status, control_version, variant_version, variant_bps, started_at, ended_at FROM routing_experiments ORDER BY started_at DESC LIMIT 100",
    )
    .fetch_all(&mut *tx)
    .await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, status, c, v, bps, st, en)| json!({ "id": id, "name": name, "status": status, "control_version": c, "variant_version": v, "variant_percent": f64::from(bps) / 100.0, "started_at": st, "ended_at": en }))
        .collect();
    Ok(Json(json!({ "data": data })))
}

pub async fn results(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = load(&mut tx, &id, false).await?;
    Ok(Json(results_body(&mut tx, &auth.tenant_id, &e).await?))
}

pub async fn stop(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let e = load(&mut tx, &id, true).await?;
    if e.status != "running" {
        return Err(ApiError::conflict(
            "experiment_not_running",
            format!("the experiment is {}", e.status),
        ));
    }
    sqlx::query("UPDATE routing_experiments SET status = 'stopped', ended_at = now(), stop_reason = $2 WHERE id = $1")
        .bind(&id)
        .bind(format!("stopped by {}", auth.actor))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "routing_experiment.stopped",
        Some(&id),
        json!({}),
    )
    .await?;
    tenant::emit(&mut tx, &auth.tenant_id, "routing.experiment_stopped", json!({ "experiment_id": id, "name": e.name, "reason": "stopped by a person", "auto": false })).await?;
    let e = load(&mut tx, &id, false).await?;
    let body = results_body(&mut tx, &auth.tenant_id, &e).await?;
    tx.commit().await?;
    Ok(Json(body))
}

/// Run the guardrail now (the worker does the same on its cycle).
pub async fn evaluate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    load(&mut tx, &id, false).await?;
    let stopped = evaluate_and_apply(&mut tx, &auth.tenant_id, &id).await?;
    let e = load(&mut tx, &id, false).await?;
    let mut body = results_body(&mut tx, &auth.tenant_id, &e).await?;
    body["auto_stopped_now"] = json!(stopped.is_some());
    tx.commit().await?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(s: u32, f: u32) -> ArmStats {
        ArmStats {
            successes: s,
            failures: f,
            ..Default::default()
        }
    }

    #[test]
    fn the_split_is_stable_and_close_to_the_share() {
        let mut variant = 0;
        for i in 0..20_000 {
            let id = format!("pi_{i}");
            assert_eq!(arm_for("salt", &id, 2_000), arm_for("salt", &id, 2_000));
            if arm_for("salt", &id, 2_000) == "variant" {
                variant += 1;
            }
        }
        // 20% of 20,000 = 4,000; a hash split is not exact, but it is close
        assert!((3_800..4_200).contains(&variant), "{variant}");
        // a different salt splits differently
        assert!(
            (0..200).any(|i| arm_for("a", &format!("pi_{i}"), 5_000)
                != arm_for("b", &format!("pi_{i}"), 5_000))
        );
    }

    #[test]
    fn nothing_is_concluded_below_the_minimum_sample() {
        let v = judge(&arm(95, 5), &arm(10, 80), 100, 300, 233);
        assert_eq!(
            v,
            Verdict::NotEnoughData {
                control: 100,
                variant: 90
            }
        );
        // control alone big enough is still not enough
        assert!(matches!(
            judge(&arm(950, 50), &arm(1, 8), 100, 300, 233),
            Verdict::NotEnoughData { .. }
        ));
    }

    #[test]
    fn a_clearly_worse_variant_stops_and_a_small_wobble_does_not() {
        // 95% against 80% with 200 each: clearly worse
        assert!(matches!(
            judge(&arm(190, 10), &arm(160, 40), 100, 300, 233),
            Verdict::VariantWorse { .. }
        ));
        // 2 points lower is under the 3-point floor whatever the z says
        assert!(matches!(
            judge(&arm(9_500, 500), &arm(9_300, 700), 100, 300, 233),
            Verdict::NoClearDifference { .. }
        ));
        // 4 points lower but on only 100 each: not significant at 99%
        assert!(matches!(
            judge(&arm(95, 5), &arm(91, 9), 100, 300, 233),
            Verdict::NoClearDifference { .. }
        ));
        // both perfect: no variance, nothing to say
        assert!(matches!(
            judge(&arm(150, 0), &arm(150, 0), 100, 300, 233),
            Verdict::NoClearDifference { .. }
        ));
    }

    #[test]
    fn a_better_variant_is_reported_but_never_a_reason_to_stop() {
        match judge(&arm(160, 40), &arm(190, 10), 100, 300, 233) {
            Verdict::VariantBetter { gain_bps, .. } => assert_eq!(gain_bps, 1_500),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unresolved_attempts_count_against_an_arm() {
        let a = ArmStats {
            successes: 80,
            failures: 0,
            unresolved: 20,
            ..Default::default()
        };
        assert_eq!(a.rate_bps(), Some(8_000));
        assert_eq!(ArmStats::default().rate_bps(), None);
    }
}
