//! Provider health for routing: a rolling success-rate signal derived from stored payment attempts, a
//! per-(provider, method) circuit breaker, and the failover list for a payment that already failed once.
//!
//! The signal is computed from `payment_attempts` on demand; only the breaker state and the tenant's
//! thresholds are stored. A signal can only change which provider is *preferred* for NEW payments. It never
//! touches an attempt that already exists, and it can never start a second live attempt: that is guarded
//! where attempts are created (`payments::start_attempt`), not here.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_providers::{
    Method,
    router::{Avoid, Breaker, Candidate, Signal},
};

use crate::{AppState, auth::Auth, error::ApiError, tenant};

/// Per-tenant thresholds. No stored row means these defaults.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Config {
    /// Turn the circuit breaker (skip degraded providers for new payments) on or off.
    pub breaker_enabled: bool,
    /// Prefer another eligible provider for the next attempt after a definitive failure.
    pub failover_enabled: bool,
    /// The signal looks at at most this many recent resolved attempts...
    pub window_attempts: i32,
    /// ...no older than this many minutes.
    pub window_minutes: i32,
    /// Fewer resolved attempts than this and the signal is ignored.
    pub min_samples: i32,
    /// Degrade when at least this percent of the window failed or is unresolved.
    pub degrade_failure_pct: i32,
    /// A degraded provider recovers when the window's failure percent is at or below this (or a probe succeeds).
    pub recover_failure_pct: i32,
    /// Seconds a degraded provider is left alone before one payment may probe it.
    pub cooldown_seconds: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            breaker_enabled: true,
            failover_enabled: true,
            window_attempts: 100,
            window_minutes: 30,
            min_samples: 10,
            degrade_failure_pct: 50,
            recover_failure_pct: 20,
            cooldown_seconds: 120,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ApiError> {
        let bad = |m: &str| Err(ApiError::invalid("invalid_health_settings", m.to_string()));
        if !(5..=1000).contains(&self.window_attempts) {
            return bad("window_attempts must be 5 to 1000");
        }
        if !(1..=1440).contains(&self.window_minutes) {
            return bad("window_minutes must be 1 to 1440");
        }
        if !(3..=1000).contains(&self.min_samples) {
            return bad("min_samples must be 3 to 1000");
        }
        if !(1..=100).contains(&self.degrade_failure_pct) {
            return bad("degrade_failure_pct must be 1 to 100");
        }
        if !(0..=99).contains(&self.recover_failure_pct) {
            return bad("recover_failure_pct must be 0 to 99");
        }
        if self.recover_failure_pct >= self.degrade_failure_pct {
            return bad("recover_failure_pct must be below degrade_failure_pct");
        }
        if !(10..=86_400).contains(&self.cooldown_seconds) {
            return bad("cooldown_seconds must be 10 to 86400");
        }
        Ok(())
    }
}

pub async fn load_config(conn: &mut PgConnection, tenant_id: &str) -> Result<Config, ApiError> {
    let row: Option<(bool, bool, i32, i32, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT breaker_enabled, failover_enabled, window_attempts, window_minutes, min_samples, degrade_failure_pct, recover_failure_pct, cooldown_seconds
         FROM routing_health_config WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        Some((be, fe, wa, wm, ms, dp, rp, cs)) => Config {
            breaker_enabled: be,
            failover_enabled: fe,
            window_attempts: wa,
            window_minutes: wm,
            min_samples: ms,
            degrade_failure_pct: dp,
            recover_failure_pct: rp,
            cooldown_seconds: cs,
        },
        None => Config::default(),
    })
}

/// Resolved attempts counted by the signal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub successes: u32,
    pub failures: u32,
    pub unresolved: u32,
    /// Created, pending or authorised-but-not-captured attempts: not yet counted either way.
    pub in_flight: u32,
}

impl Stats {
    pub fn samples(&self) -> u32 {
        self.successes + self.failures + self.unresolved
    }
    /// Percent (rounded down) that failed or are unresolved.
    pub fn failure_pct(&self) -> u32 {
        ((self.failures + self.unresolved) * 100)
            .checked_div(self.samples())
            .unwrap_or(0)
    }
}

const COUNT_SQL: &str = "SELECT COUNT(*) FILTER (WHERE status IN ('captured', 'authorized')),
        COUNT(*) FILTER (WHERE status = 'failed'),
        COUNT(*) FILTER (WHERE status = 'unknown'),
        COUNT(*) FILTER (WHERE status IN ('created', 'pending'))";

/// The rolling window: the newest `window_attempts` resolved attempts that are no older than `window_minutes`.
/// Merchant cancellations are not the provider's doing and are left out. After a recovery the window starts
/// afresh (`not_before`), so the failures that caused the degradation cannot degrade the provider again.
pub async fn window_stats(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    method: &str,
    cfg: &Config,
    not_before: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Stats, ApiError> {
    let sql = format!(
        "{COUNT_SQL} FROM (
            SELECT status FROM payment_attempts
            WHERE tenant_id = $1 AND provider = $2 AND method = $3
              AND created_at > now() - make_interval(mins => $4)
              AND ($6::timestamptz IS NULL OR created_at > $6)
              AND status IN ('captured', 'authorized', 'failed', 'unknown', 'created', 'pending')
              AND NOT (status = 'failed' AND (failure_class IS NOT DISTINCT FROM 'cancelled' OR failure_reason IS NOT DISTINCT FROM 'canceled_by_merchant'))
            ORDER BY created_at DESC LIMIT $5) r"
    );
    let (s, f, u, i): (i64, i64, i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(tenant_id)
        .bind(provider)
        .bind(method)
        .bind(cfg.window_minutes)
        .bind(i64::from(cfg.window_attempts))
        .bind(not_before)
        .fetch_one(conn)
        .await?;
    Ok(Stats {
        successes: s as u32,
        failures: f as u32,
        unresolved: u as u32,
        in_flight: i as u32,
    })
}

/// Attempts created since the provider was degraded: the probes (and anything routed to it as a last resort).
async fn since_stats(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    method: &str,
) -> Result<(Stats, bool), ApiError> {
    let sql = format!(
        "{COUNT_SQL}, COALESCE((SELECT status IN ('captured', 'authorized') FROM payment_attempts a2
                WHERE a2.tenant_id = $1 AND a2.provider = $2 AND a2.method = $3
                  AND a2.created_at >= (SELECT degraded_since FROM provider_health_state WHERE tenant_id = $1 AND provider = $2 AND method = $3)
                  AND a2.status IN ('captured', 'authorized', 'failed')
                  AND NOT (a2.status = 'failed' AND (a2.failure_class IS NOT DISTINCT FROM 'cancelled' OR a2.failure_reason IS NOT DISTINCT FROM 'canceled_by_merchant'))
                ORDER BY a2.created_at DESC LIMIT 1), false)
         FROM payment_attempts
         WHERE tenant_id = $1 AND provider = $2 AND method = $3
           AND created_at >= (SELECT degraded_since FROM provider_health_state WHERE tenant_id = $1 AND provider = $2 AND method = $3)
           AND NOT (status = 'failed' AND (failure_class IS NOT DISTINCT FROM 'cancelled' OR failure_reason IS NOT DISTINCT FROM 'canceled_by_merchant'))"
    );
    let (s, f, u, i, last_ok): (i64, i64, i64, i64, bool) =
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(tenant_id)
            .bind(provider)
            .bind(method)
            .fetch_one(conn)
            .await?;
    Ok((
        Stats {
            successes: s as u32,
            failures: f as u32,
            unresolved: u as u32,
            in_flight: i as u32,
        },
        last_ok,
    ))
}

/// What the breaker has stored for one provider and method.
#[derive(Debug, Clone, Default)]
pub struct Stored {
    pub degraded: bool,
    pub reason: Option<String>,
    pub degraded_since: Option<chrono::DateTime<chrono::Utc>>,
    pub last_probe_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Seconds since the later of `degraded_since` and `last_probe_at`, by the database clock.
    pub quiet_secs: Option<f64>,
    /// When it last recovered (healthy rows only).
    pub recovered_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn stored(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    method: &str,
) -> Result<Stored, ApiError> {
    let row: Option<(
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<f64>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT state, reason, degraded_since, last_probe_at,
                EXTRACT(EPOCH FROM (now() - GREATEST(degraded_since, COALESCE(last_probe_at, degraded_since))))::float8, changed_at
         FROM provider_health_state WHERE tenant_id = $1 AND provider = $2 AND method = $3",
    )
    .bind(tenant_id)
    .bind(provider)
    .bind(method)
    .fetch_optional(conn)
    .await?;
    Ok(match row {
        Some((state, reason, since, probe, quiet, changed)) => Stored {
            degraded: state == "degraded",
            reason,
            degraded_since: since,
            last_probe_at: probe,
            quiet_secs: quiet,
            recovered_at: (state != "degraded").then_some(changed),
        },
        None => Stored::default(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    Degrade(String),
    Recover(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eval {
    pub breaker: Breaker,
    pub transition: Option<Transition>,
}

/// What the probes since degradation say.
#[derive(Debug, Clone, Copy, Default)]
pub struct Since {
    pub stats: Stats,
    /// The newest resolved attempt since degradation succeeded.
    pub last_succeeded: bool,
}

/// The circuit breaker as a pure function. `quiet_secs` is how long the provider has been left alone.
pub fn evaluate(
    cfg: &Config,
    window: &Stats,
    degraded: bool,
    quiet_secs: Option<f64>,
    since: &Since,
) -> Eval {
    if !cfg.breaker_enabled {
        return Eval {
            breaker: Breaker::Healthy,
            transition: degraded
                .then(|| Transition::Recover("the circuit breaker was turned off".into())),
        };
    }
    let enough = window.samples() >= cfg.min_samples as u32;
    let pct = window.failure_pct();
    if !degraded {
        if enough && pct >= cfg.degrade_failure_pct as u32 {
            return Eval {
                breaker: Breaker::Degraded,
                transition: Some(Transition::Degrade(format!(
                    "{} of the last {} resolved attempts failed or are unresolved ({pct}%, threshold {}%)",
                    window.failures + window.unresolved,
                    window.samples(),
                    cfg.degrade_failure_pct
                ))),
            };
        }
        return Eval {
            breaker: Breaker::Healthy,
            transition: None,
        };
    }
    // degraded: recover when a probe succeeded, or the rolling signal itself improved
    if since.last_succeeded {
        return Eval {
            breaker: Breaker::Healthy,
            transition: Some(Transition::Recover(
                "a probe payment after the cool-down succeeded".into(),
            )),
        };
    }
    if enough && pct <= cfg.recover_failure_pct as u32 {
        return Eval {
            breaker: Breaker::Healthy,
            transition: Some(Transition::Recover(format!(
                "the recent failure rate fell to {pct}% (recovery at or below {}%)",
                cfg.recover_failure_pct
            ))),
        };
    }
    // A probe that is still out (pending) or unresolved blocks the next one.
    let probe_out = since.stats.in_flight > 0 || since.stats.unresolved > 0;
    let cooled = quiet_secs.is_some_and(|q| q >= f64::from(cfg.cooldown_seconds));
    Eval {
        breaker: if cooled && !probe_out {
            Breaker::ProbeDue
        } else {
            Breaker::Degraded
        },
        transition: None,
    }
}

fn signal(cfg: &Config, w: &Stats, b: Breaker) -> Signal {
    let n = w.samples();
    Signal {
        samples: n,
        successes: w.successes,
        failures: w.failures,
        unresolved: w.unresolved,
        success_rate_bps: (n >= cfg.min_samples as u32 && n > 0).then(|| w.successes * 10_000 / n),
        min_samples: cfg.min_samples as u32,
        breaker: b,
    }
}

/// Compute the signal and breaker state for every enabled candidate, for the method being routed, and record any
/// degrade / recovery (an audit entry and a webhook event) in the caller's transaction.
pub async fn attach_signals(
    conn: &mut PgConnection,
    tenant_id: &str,
    cfg: &Config,
    method: Method,
    candidates: &mut [Candidate],
) -> Result<(), ApiError> {
    for c in candidates.iter_mut().filter(|c| c.enabled) {
        let m = method.as_str();
        let st = stored(conn, tenant_id, &c.provider, m).await?;
        let window = window_stats(conn, tenant_id, &c.provider, m, cfg, st.recovered_at).await?;
        let since = if st.degraded {
            let (stats, last_succeeded) = since_stats(conn, tenant_id, &c.provider, m).await?;
            Since {
                stats,
                last_succeeded,
            }
        } else {
            Since::default()
        };
        let ev = evaluate(cfg, &window, st.degraded, st.quiet_secs, &since);
        let sig = signal(cfg, &window, ev.breaker);
        if let Some(t) = &ev.transition {
            record_transition(conn, tenant_id, &c.provider, m, t, &sig).await?;
        }
        c.signal = Some(sig);
    }
    Ok(())
}

/// Persist a transition once (concurrent routers race on the row) and tell the merchant.
async fn record_transition(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    method: &str,
    t: &Transition,
    sig: &Signal,
) -> Result<(), ApiError> {
    let (changed, event, reason) = match t {
        Transition::Degrade(why) => {
            let r = sqlx::query(
                "INSERT INTO provider_health_state (tenant_id, provider, method, state, reason, degraded_since, last_probe_at, changed_at)
                 VALUES ($1, $2, $3, 'degraded', $4, now(), NULL, now())
                 ON CONFLICT (tenant_id, provider, method) DO UPDATE
                   SET state = 'degraded', reason = $4, degraded_since = now(), last_probe_at = NULL, changed_at = now()
                   WHERE provider_health_state.state <> 'degraded'",
            )
            .bind(tenant_id)
            .bind(provider)
            .bind(method)
            .bind(why)
            .execute(&mut *conn)
            .await?;
            (r.rows_affected() > 0, "provider.degraded", why)
        }
        Transition::Recover(why) => {
            let r = sqlx::query(
                "UPDATE provider_health_state SET state = 'healthy', reason = $4, degraded_since = NULL, last_probe_at = NULL, changed_at = now()
                 WHERE tenant_id = $1 AND provider = $2 AND method = $3 AND state = 'degraded'",
            )
            .bind(tenant_id)
            .bind(provider)
            .bind(method)
            .bind(why)
            .execute(&mut *conn)
            .await?;
            (r.rows_affected() > 0, "provider.recovered", why)
        }
    };
    if changed {
        let detail =
            json!({ "provider": provider, "method": method, "reason": reason, "signal": sig });
        tenant::audit(
            conn,
            tenant_id,
            "routing",
            event,
            Some(provider),
            detail.clone(),
        )
        .await?;
        tenant::emit(conn, tenant_id, event, detail).await?;
        tracing::warn!(provider, method, reason, event, "provider health changed");
    }
    Ok(())
}

/// Take the one probe slot for a degraded provider. False when someone else already took it (or the provider
/// is no longer degraded), in which case the caller must route without it.
pub async fn claim_probe(
    conn: &mut PgConnection,
    tenant_id: &str,
    provider: &str,
    method: Method,
    cfg: &Config,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "UPDATE provider_health_state SET last_probe_at = now()
         WHERE tenant_id = $1 AND provider = $2 AND method = $3 AND state = 'degraded'
           AND now() - GREATEST(degraded_since, COALESCE(last_probe_at, degraded_since)) >= make_interval(secs => $4)",
    )
    .bind(tenant_id)
    .bind(provider)
    .bind(method.as_str())
    .bind(f64::from(cfg.cooldown_seconds))
    .execute(conn)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Providers that already failed this payment in a way that proves nothing was taken: the provider answered
/// "failed", or refused the request before creating anything. A merchant cancellation does not count (a late
/// capture is still possible there), and neither does anything unresolved.
pub async fn failed_here(conn: &mut PgConnection, intent_id: &str) -> Result<Vec<Avoid>, ApiError> {
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT ON (provider) provider, failure_class, failure_reason
         FROM payment_attempts WHERE intent_id = $1 AND status = 'failed'
           AND failure_class IN ('declined', 'provider_rejected')
         ORDER BY provider, created_at DESC",
    )
    .bind(intent_id)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(provider, class, reason)| Avoid {
            provider,
            reason: format!(
                "{}{}",
                class.unwrap_or_default(),
                reason.map(|r| format!(": {r}")).unwrap_or_default()
            ),
        })
        .collect())
}

// ---------- API ----------

#[derive(Deserialize)]
pub struct HealthQuery {
    pub method: Option<String>,
}

/// Read provider health: the success-rate signal and breaker state per provider and method (read-only: asking
/// never changes state; transitions are recorded when a payment is routed), plus recent degrade / recover events.
pub async fn get_health(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<HealthQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let method: Option<Method> = match q.method.as_deref() {
        Some(m) => Some(
            m.parse()
                .map_err(|e: String| ApiError::invalid("invalid_method", e))?,
        ),
        None => None,
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cfg = load_config(&mut tx, &auth.tenant_id).await?;
    let accounts: Vec<(String, bool)> = sqlx::query_as(
        "SELECT provider, enabled FROM provider_accounts WHERE tenant_id = $1 ORDER BY provider",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut providers = Vec::new();
    for (provider, enabled) in accounts {
        // methods with attempts in the window, or with a stored breaker state
        let mut methods: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT method FROM (
                SELECT method FROM payment_attempts WHERE tenant_id = $1 AND provider = $2 AND created_at > now() - make_interval(mins => $3)
                UNION SELECT method FROM provider_health_state WHERE tenant_id = $1 AND provider = $2) m ORDER BY method",
        )
        .bind(&auth.tenant_id)
        .bind(&provider)
        .bind(cfg.window_minutes)
        .fetch_all(&mut *tx)
        .await?;
        if let Some(m) = method {
            methods.retain(|x| x == m.as_str());
        }
        let mut rows = Vec::new();
        for m in methods {
            let st = stored(&mut tx, &auth.tenant_id, &provider, &m).await?;
            let window = window_stats(
                &mut tx,
                &auth.tenant_id,
                &provider,
                &m,
                &cfg,
                st.recovered_at,
            )
            .await?;
            let since = if st.degraded {
                let (stats, last_succeeded) =
                    since_stats(&mut tx, &auth.tenant_id, &provider, &m).await?;
                Since {
                    stats,
                    last_succeeded,
                }
            } else {
                Since::default()
            };
            let ev = evaluate(&cfg, &window, st.degraded, st.quiet_secs, &since);
            let sig = signal(&cfg, &window, ev.breaker);
            rows.push(json!({
                "method": m,
                "signal": sig,
                "in_flight": window.in_flight,
                "state": match ev.breaker { Breaker::Healthy => "healthy", Breaker::Degraded => "degraded", Breaker::ProbeDue => "probe_due" },
                "stored_state": if st.degraded { "degraded" } else { "healthy" },
                "reason": st.reason,
                "degraded_since": st.degraded_since,
                "last_probe_at": st.last_probe_at,
                "pending_transition": ev.transition.as_ref().map(|t| match t {
                    Transition::Degrade(r) => json!({ "to": "degraded", "reason": r }),
                    Transition::Recover(r) => json!({ "to": "healthy", "reason": r }),
                }),
            }));
        }
        providers.push(json!({ "provider": provider, "enabled": enabled, "methods": rows }));
    }
    let changes: Vec<(String, Option<String>, Value, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT action, subject, detail, created_at FROM audit_events
         WHERE tenant_id = $1 AND action IN ('provider.degraded', 'provider.recovered')
         ORDER BY created_at DESC, id DESC LIMIT 20",
        )
        .bind(&auth.tenant_id)
        .fetch_all(&mut *tx)
        .await?;
    let recent: Vec<Value> = changes
        .into_iter()
        .map(|(a, p, d, at)| json!({ "event": a, "provider": p, "detail": d, "at": at }))
        .collect();
    Ok(Json(
        json!({ "config": cfg, "providers": providers, "recent_changes": recent }),
    ))
}

pub async fn get_settings(State(s): State<AppState>, auth: Auth) -> Result<Json<Config>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(load_config(&mut tx, &auth.tenant_id).await?))
}

#[derive(Deserialize)]
pub struct SettingsIn {
    breaker_enabled: Option<bool>,
    failover_enabled: Option<bool>,
    window_attempts: Option<i32>,
    window_minutes: Option<i32>,
    min_samples: Option<i32>,
    degrade_failure_pct: Option<i32>,
    recover_failure_pct: Option<i32>,
    cooldown_seconds: Option<i32>,
}

/// Change thresholds. Only the fields sent change; the rest keep their current value.
pub async fn put_settings(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<SettingsIn>,
) -> Result<Json<Config>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let cur = load_config(&mut tx, &auth.tenant_id).await?;
    let new = Config {
        breaker_enabled: b.breaker_enabled.unwrap_or(cur.breaker_enabled),
        failover_enabled: b.failover_enabled.unwrap_or(cur.failover_enabled),
        window_attempts: b.window_attempts.unwrap_or(cur.window_attempts),
        window_minutes: b.window_minutes.unwrap_or(cur.window_minutes),
        min_samples: b.min_samples.unwrap_or(cur.min_samples),
        degrade_failure_pct: b.degrade_failure_pct.unwrap_or(cur.degrade_failure_pct),
        recover_failure_pct: b.recover_failure_pct.unwrap_or(cur.recover_failure_pct),
        cooldown_seconds: b.cooldown_seconds.unwrap_or(cur.cooldown_seconds),
    };
    new.validate()?;
    sqlx::query(
        "INSERT INTO routing_health_config (tenant_id, breaker_enabled, failover_enabled, window_attempts, window_minutes, min_samples, degrade_failure_pct, recover_failure_pct, cooldown_seconds, updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now())
         ON CONFLICT (tenant_id) DO UPDATE SET breaker_enabled = $2, failover_enabled = $3, window_attempts = $4, window_minutes = $5,
            min_samples = $6, degrade_failure_pct = $7, recover_failure_pct = $8, cooldown_seconds = $9, updated_by = $10, updated_at = now()",
    )
    .bind(&auth.tenant_id)
    .bind(new.breaker_enabled)
    .bind(new.failover_enabled)
    .bind(new.window_attempts)
    .bind(new.window_minutes)
    .bind(new.min_samples)
    .bind(new.degrade_failure_pct)
    .bind(new.recover_failure_pct)
    .bind(new.cooldown_seconds)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "routing_health.settings_updated",
        None,
        json!({ "before": cur, "after": new }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(new))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(s: u32, f: u32, u: u32) -> Stats {
        Stats {
            successes: s,
            failures: f,
            unresolved: u,
            in_flight: 0,
        }
    }

    #[test]
    fn a_small_sample_never_degrades() {
        let cfg = Config::default();
        let e = evaluate(&cfg, &st(0, 9, 0), false, None, &Since::default());
        assert_eq!(e.breaker, Breaker::Healthy);
        assert!(e.transition.is_none());
    }

    #[test]
    fn crossing_the_threshold_degrades_once_with_a_reason() {
        let cfg = Config::default();
        let e = evaluate(&cfg, &st(5, 4, 1), false, None, &Since::default());
        assert_eq!(e.breaker, Breaker::Degraded);
        assert!(
            matches!(e.transition, Some(Transition::Degrade(ref r)) if r.contains("5 of the last 10"))
        );
        // just under the threshold stays healthy
        let e = evaluate(&cfg, &st(6, 4, 0), false, None, &Since::default());
        assert_eq!(e.breaker, Breaker::Healthy);
    }

    #[test]
    fn degraded_waits_out_the_cooldown_then_allows_one_probe_and_blocks_while_it_is_out() {
        let cfg = Config::default();
        let w = st(2, 18, 0);
        let none = Since::default();
        assert_eq!(
            evaluate(&cfg, &w, true, Some(30.0), &none).breaker,
            Breaker::Degraded
        );
        assert_eq!(
            evaluate(&cfg, &w, true, Some(121.0), &none).breaker,
            Breaker::ProbeDue
        );
        let out = Since {
            stats: Stats {
                in_flight: 1,
                ..Stats::default()
            },
            last_succeeded: false,
        };
        assert_eq!(
            evaluate(&cfg, &w, true, Some(500.0), &out).breaker,
            Breaker::Degraded
        );
        let unknown = Since {
            stats: Stats {
                unresolved: 1,
                ..Stats::default()
            },
            last_succeeded: false,
        };
        assert_eq!(
            evaluate(&cfg, &w, true, Some(500.0), &unknown).breaker,
            Breaker::Degraded
        );
    }

    #[test]
    fn a_successful_probe_or_an_improved_window_recovers() {
        let cfg = Config::default();
        let ok = Since {
            stats: Stats {
                successes: 1,
                ..Stats::default()
            },
            last_succeeded: true,
        };
        let e = evaluate(&cfg, &st(2, 18, 0), true, Some(1.0), &ok);
        assert_eq!(e.breaker, Breaker::Healthy);
        assert!(matches!(e.transition, Some(Transition::Recover(_))));
        // the rolling window alone: 20% failures is at the recovery threshold
        let e = evaluate(&cfg, &st(16, 4, 0), true, Some(1.0), &Since::default());
        assert!(matches!(e.transition, Some(Transition::Recover(_))));
        // 30% is not good enough
        let e = evaluate(&cfg, &st(14, 6, 0), true, Some(1.0), &Since::default());
        assert!(e.transition.is_none());
    }

    #[test]
    fn turning_the_breaker_off_releases_a_degraded_provider() {
        let cfg = Config {
            breaker_enabled: false,
            ..Config::default()
        };
        let e = evaluate(&cfg, &st(0, 20, 0), true, Some(1.0), &Since::default());
        assert_eq!(e.breaker, Breaker::Healthy);
        assert!(matches!(e.transition, Some(Transition::Recover(_))));
        assert!(
            evaluate(&cfg, &st(0, 20, 0), false, None, &Since::default())
                .transition
                .is_none()
        );
    }

    #[test]
    fn settings_are_validated() {
        assert!(Config::default().validate().is_ok());
        let c = |f: fn(&mut Config)| {
            let mut c = Config::default();
            f(&mut c);
            c.validate().is_err()
        };
        assert!(c(|c| c.window_attempts = 2));
        assert!(c(|c| c.min_samples = 1));
        assert!(c(|c| c.recover_failure_pct = 60));
        assert!(c(|c| c.cooldown_seconds = 1));
        assert!(c(|c| c.degrade_failure_pct = 0));
    }
}
