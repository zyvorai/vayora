// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Provider and bank downtime alerts.
//!
//! The worker (and `POST /v1/routing/alerts/evaluate`) reads the recent attempts of a business and looks at three
//! kinds of group: a provider for one method (`provider_method`), a bank within it where attempts carry one
//! (`bank`), and a UPI handle within it (`upi_handle`, from the UPI ID the payer typed). A group whose failures reach
//! `open_failure_pct` of at least `min_samples` resolved attempts opens ONE alert: a reconciliation case, a
//! `provider.alert_opened` event for the webhook outbox, an audit entry and, when an address is configured, an email.
//! This sits beside the circuit breaker (`health.rs`), which steers routing for a provider and method; alerts also
//! cover banks and UPI handles, and tell people.
//!
//! What counts is the same as for routing health: successes are captured or authorised; failures are provider or bank
//! side declines and declines of unknown cause; an unknown outcome counts against the group; payer declines and
//! merchant cancellations are left out.
//!
//! De-duplication and flap control, so a flaky provider does not become a flood:
//! - at most one open alert per provider, method, kind and value (a partial unique index), however often it is swept;
//! - hysteresis: it opens at `open_failure_pct` and resolves only at or below `resolve_failure_pct`, with at least
//!   `min_samples` attempts as evidence, and not before it has been open for `hold_seconds`;
//! - after it resolves, the failures that caused it are not counted again: only attempts made since then can reopen it;
//! - a recurrence within `flap_window_seconds` of the resolution REOPENS the same alert silently (`flap_count`
//!   goes up, the case is reopened, no new event or email), and only a later one raises a new alert;
//! - a provider-level alert also resolves when the circuit breaker has recovered, and any alert resolves after 24 hours
//!   with no attempts at all, because a skipped provider gets no traffic to prove it is back.
//!
//! The worker runs this with one advisory lock per business, so several replicas raise each alert once.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection};
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, cases::open_case, tenant};

const STALE_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Config {
    pub enabled: bool,
    pub min_samples: i32,
    pub open_failure_pct: i32,
    pub resolve_failure_pct: i32,
    pub window_minutes: i32,
    pub hold_seconds: i32,
    pub flap_window_seconds: i32,
    pub email_to: Option<String>,
    /// Set for one sweep while the routing kill switch is on: alerts are recorded (row and audit entry) but no case,
    /// event or email is made. Not stored.
    #[sqlx(skip)]
    #[serde(skip)]
    pub quiet: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // opt-in: a business that never asked for downtime alerts gets none, and no new event type
            enabled: false,
            min_samples: 20,
            open_failure_pct: 50,
            resolve_failure_pct: 20,
            window_minutes: 30,
            hold_seconds: 300,
            flap_window_seconds: 900,
            email_to: None,
            quiet: false,
        }
    }
}

pub async fn load_config(conn: &mut PgConnection) -> Result<Config, ApiError> {
    Ok(sqlx::query_as(
        "SELECT enabled, min_samples, open_failure_pct, resolve_failure_pct, window_minutes, hold_seconds, flap_window_seconds, email_to FROM provider_alert_config",
    )
    .fetch_optional(conn)
    .await?
    .unwrap_or_default())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub successes: i64,
    pub failures: i64,
    pub unresolved: i64,
}

impl Counts {
    pub fn samples(&self) -> i64 {
        self.successes + self.failures + self.unresolved
    }
    /// Failed or unresolved, as a whole percent (rounded down).
    pub fn failure_pct(&self) -> i64 {
        let n = self.samples();
        if n == 0 {
            0
        } else {
            (self.failures + self.unresolved) * 100 / n
        }
    }
}

/// What the numbers say about a group. Pure.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Reading {
    /// Too few attempts to say anything.
    Thin,
    Bad,
    /// Between the two thresholds: not bad enough to open, not good enough to resolve.
    Middling,
    Good,
}

pub fn read(c: &Counts, cfg: &Config) -> Reading {
    if c.samples() < i64::from(cfg.min_samples) {
        Reading::Thin
    } else if c.failure_pct() >= i64::from(cfg.open_failure_pct) {
        Reading::Bad
    } else if c.failure_pct() <= i64::from(cfg.resolve_failure_pct) {
        Reading::Good
    } else {
        Reading::Middling
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    provider: String,
    method: String,
    dimension: &'static str,
    value: String,
}

const NOT_CANCELLED: &str = "NOT (status = 'failed' AND (failure_class IS NOT DISTINCT FROM 'cancelled' OR failure_reason IS NOT DISTINCT FROM 'canceled_by_merchant'))";
const NOT_PAYER: &str = "failure_class IS DISTINCT FROM 'payer_declined'";

async fn groups(
    conn: &mut PgConnection,
    tenant_id: &str,
    window_min: i32,
    dimension: &'static str,
) -> Result<Vec<(Key, Counts)>, ApiError> {
    let (expr, cond) = match dimension {
        "provider_method" => ("''::text", "TRUE"),
        "bank" => ("instrument->>'bank'", "instrument->>'bank' IS NOT NULL"),
        _ => (
            "instrument->>'upi_handle'",
            "instrument->>'upi_handle' IS NOT NULL",
        ),
    };
    let rows: Vec<(String, String, String, i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT provider, method, {expr}, COUNT(*) FILTER (WHERE status IN ('captured', 'authorized')),
                COUNT(*) FILTER (WHERE status = 'failed'), COUNT(*) FILTER (WHERE status = 'unknown')
         FROM payment_attempts
         WHERE tenant_id = $1 AND created_at > now() - make_interval(mins => $2)
           AND status IN ('captured', 'authorized', 'failed', 'unknown') AND {NOT_CANCELLED} AND {NOT_PAYER} AND {cond}
         GROUP BY 1, 2, 3 ORDER BY 1, 2, 3"
    )))
    .bind(tenant_id)
    .bind(window_min)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(provider, method, value, s, f, u)| {
            (
                Key {
                    provider,
                    method,
                    dimension,
                    value,
                },
                Counts {
                    successes: s,
                    failures: f,
                    unresolved: u,
                },
            )
        })
        .collect())
}

/// The same count, only for attempts made after `since` (so a failure that already raised and closed an alert is
/// not counted against the group again).
async fn counts_since(
    conn: &mut PgConnection,
    tenant_id: &str,
    k: &Key,
    since: DateTime<Utc>,
) -> Result<Counts, ApiError> {
    let filter = match k.dimension {
        "bank" => "AND instrument->>'bank' = $5",
        "upi_handle" => "AND instrument->>'upi_handle' = $5",
        _ => "AND $5::text = ''",
    };
    let (s, f, u): (i64, i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FILTER (WHERE status IN ('captured', 'authorized')), COUNT(*) FILTER (WHERE status = 'failed'), COUNT(*) FILTER (WHERE status = 'unknown')
         FROM payment_attempts WHERE tenant_id = $1 AND provider = $2 AND method = $3 AND created_at > $4 {filter}
           AND status IN ('captured', 'authorized', 'failed', 'unknown') AND {NOT_CANCELLED} AND {NOT_PAYER}"
    )))
    .bind(tenant_id)
    .bind(&k.provider)
    .bind(&k.method)
    .bind(since)
    .bind(&k.value)
    .fetch_one(conn)
    .await?;
    Ok(Counts {
        successes: s,
        failures: f,
        unresolved: u,
    })
}

fn label(k: &Key) -> String {
    match k.dimension {
        "bank" => format!(
            "{} {} payments through bank {}",
            k.provider, k.method, k.value
        ),
        "upi_handle" => format!("{} UPI payments to handle {}", k.provider, k.value),
        _ => format!("{} {} payments", k.provider, k.method),
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Sweep {
    pub checked: usize,
    pub opened: usize,
    pub reopened: usize,
    pub resolved: usize,
    pub skipped_busy: bool,
}

struct OpenAlert {
    id: String,
    opened_at: DateTime<Utc>,
}

struct Resolved {
    id: String,
    resolved_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
async fn notify(
    conn: &mut PgConnection,
    tenant_id: &str,
    cfg: &Config,
    event: &str,
    dedupe: &str,
    subject: &str,
    body: &str,
    data: Value,
) -> Result<(), ApiError> {
    if cfg.quiet {
        return Ok(());
    }
    tenant::emit(conn, tenant_id, event, data).await?;
    if let Some(to) = &cfg.email_to {
        crate::mail::enqueue_once_as(conn, tenant_id, dedupe, to, subject, body, true).await?;
    }
    Ok(())
}

pub async fn sweep_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<Sweep, ApiError> {
    let mut tx = tenant::begin(pool, tenant_id).await?;
    let got: bool = sqlx::query_scalar(
        "SELECT pg_try_advisory_xact_lock(hashtext('zyvor.alert_sweep'), hashtext($1))",
    )
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;
    if !got {
        return Ok(Sweep {
            skipped_busy: true,
            ..Sweep::default()
        });
    }
    let out = sweep(&mut tx, tenant_id).await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn sweep(conn: &mut PgConnection, tenant_id: &str) -> Result<Sweep, ApiError> {
    let mut cfg = load_config(conn).await?;
    let mut out = Sweep::default();
    if !cfg.enabled {
        return Ok(out);
    }
    // the routing kill switch: record what is seen, take no automated action
    cfg.quiet = crate::routing_auto::load(conn, tenant_id).await?.paused;
    let now: DateTime<Utc> = sqlx::query_scalar("SELECT now()")
        .fetch_one(&mut *conn)
        .await?;
    let window_start = now - chrono::Duration::minutes(i64::from(cfg.window_minutes));
    let hold = chrono::Duration::seconds(i64::from(cfg.hold_seconds));
    let flap = chrono::Duration::seconds(i64::from(cfg.flap_window_seconds));

    let open_rows: Vec<(String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, provider, method, dimension, dimension_value, opened_at FROM provider_alerts WHERE status = 'open'",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut open: HashMap<Key, OpenAlert> = HashMap::new();
    for (id, provider, method, dim, value, opened_at) in open_rows {
        let dimension = match dim.as_str() {
            "bank" => "bank",
            "upi_handle" => "upi_handle",
            _ => "provider_method",
        };
        open.insert(
            Key {
                provider,
                method,
                dimension,
                value,
            },
            OpenAlert { id, opened_at },
        );
    }
    let resolved_rows: Vec<(String, String, String, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT DISTINCT ON (provider, method, dimension, dimension_value) id, provider, method, dimension, dimension_value, resolved_at
         FROM provider_alerts WHERE status = 'resolved' AND resolved_at IS NOT NULL ORDER BY provider, method, dimension, dimension_value, resolved_at DESC",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut recent: HashMap<Key, Resolved> = HashMap::new();
    for (id, provider, method, dim, value, resolved_at) in resolved_rows {
        let dimension = match dim.as_str() {
            "bank" => "bank",
            "upi_handle" => "upi_handle",
            _ => "provider_method",
        };
        recent.insert(
            Key {
                provider,
                method,
                dimension,
                value,
            },
            Resolved { id, resolved_at },
        );
    }

    let mut seen: Vec<Key> = Vec::new();
    let mut closed: Vec<String> = Vec::new();
    for dimension in ["provider_method", "bank", "upi_handle"] {
        for (k, c) in groups(conn, tenant_id, cfg.window_minutes, dimension).await? {
            out.checked += 1;
            seen.push(k.clone());
            if let Some(a) = open.get(&k) {
                sqlx::query("UPDATE provider_alerts SET samples = $2, failures = $3, failure_pct = $4, last_seen_at = now() WHERE id = $1")
                    .bind(&a.id)
                    .bind(c.samples() as i32)
                    .bind((c.failures + c.unresolved) as i32)
                    .bind(c.failure_pct() as i32)
                    .execute(&mut *conn)
                    .await?;
                if read(&c, &cfg) == Reading::Good && now - a.opened_at >= hold {
                    resolve(
                        conn,
                        tenant_id,
                        &cfg,
                        &a.id,
                        &k,
                        &c,
                        &format!(
                            "failure rate fell to {}% over {} attempts",
                            c.failure_pct(),
                            c.samples()
                        ),
                    )
                    .await?;
                    closed.push(a.id.clone());
                    out.resolved += 1;
                }
                continue;
            }
            if read(&c, &cfg) != Reading::Bad {
                continue;
            }
            // failures an earlier alert already covered do not count again
            let last = recent.get(&k);
            let c = match last {
                Some(r) if r.resolved_at > window_start => {
                    counts_since(conn, tenant_id, &k, r.resolved_at).await?
                }
                _ => c,
            };
            if read(&c, &cfg) != Reading::Bad {
                continue;
            }
            match last {
                Some(r) if now - r.resolved_at <= flap => {
                    reopen(conn, tenant_id, &r.id, &k, &c).await?;
                    out.reopened += 1;
                }
                _ => {
                    raise(conn, tenant_id, &cfg, &k, &c).await?;
                    out.opened += 1;
                }
            }
        }
    }
    // an open alert that the numbers did not settle
    for (k, a) in &open {
        if closed.contains(&a.id) {
            continue;
        }
        if now - a.opened_at < hold {
            continue;
        }
        if k.dimension == "provider_method" {
            let recovered: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM provider_health_state WHERE provider = $1 AND method = $2 AND state = 'healthy' AND changed_at > $3)",
            )
            .bind(&k.provider)
            .bind(&k.method)
            .bind(a.opened_at)
            .fetch_one(&mut *conn)
            .await?;
            if recovered {
                resolve(
                    conn,
                    tenant_id,
                    &cfg,
                    &a.id,
                    k,
                    &Counts::default(),
                    "the circuit breaker recovered this provider",
                )
                .await?;
                out.resolved += 1;
                continue;
            }
        }
        if !seen.contains(k) {
            let quiet: bool = sqlx::query_scalar("SELECT last_seen_at < now() - make_interval(hours => $2) FROM provider_alerts WHERE id = $1")
                .bind(&a.id)
                .bind(STALE_HOURS as i32)
                .fetch_one(&mut *conn)
                .await?;
            if quiet {
                resolve(
                    conn,
                    tenant_id,
                    &cfg,
                    &a.id,
                    k,
                    &Counts::default(),
                    &format!("no attempts for {STALE_HOURS} hours, so no evidence either way"),
                )
                .await?;
                out.resolved += 1;
            }
        }
    }
    Ok(out)
}

async fn raise(
    conn: &mut PgConnection,
    tenant_id: &str,
    cfg: &Config,
    k: &Key,
    c: &Counts,
) -> Result<(), ApiError> {
    let id = new_id("pal");
    let reason = format!(
        "{} of the last {} {} failed or are unresolved ({}%)",
        c.failures + c.unresolved,
        c.samples(),
        label(k),
        c.failure_pct()
    );
    let inserted = sqlx::query(
        "INSERT INTO provider_alerts (id, tenant_id, provider, method, dimension, dimension_value, samples, failures, failure_pct, reason)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT DO NOTHING",
    )
    .bind(&id)
    .bind(tenant_id)
    .bind(&k.provider)
    .bind(&k.method)
    .bind(k.dimension)
    .bind(&k.value)
    .bind(c.samples() as i32)
    .bind((c.failures + c.unresolved) as i32)
    .bind(c.failure_pct() as i32)
    .bind(&reason)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(()); // another worker opened it first
    }
    tenant::audit(conn, tenant_id, "system", "provider_alert.opened", Some(&id), json!({ "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value, "failure_pct": c.failure_pct(), "samples": c.samples(), "notifications_suppressed": cfg.quiet })).await?;
    if !cfg.quiet {
        open_case(conn, tenant_id, "provider_alert", "provider_alert", &id, None, None, &format!("Possible downtime: {reason}. Routing health and the circuit breaker show what routing is doing about it."), json!({ "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value })).await?;
    }
    let subject = format!("Possible downtime: {}", label(k));
    let body = format!(
        "{reason}.\n\nThis alert opened because at least {}% of the last {} minutes' payments failed (at least {} attempts were needed). It resolves on its own when the failure rate falls to {}% or less.\n\nSee Routing in the console for what routing is doing about it.",
        cfg.open_failure_pct, cfg.window_minutes, cfg.min_samples, cfg.resolve_failure_pct
    );
    notify(conn, tenant_id, cfg, "provider.alert_opened", &format!("palert:{id}:opened"), &subject, &body, json!({ "alert_id": id, "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value, "failure_pct": c.failure_pct(), "samples": c.samples(), "reason": reason })).await
}

async fn reopen(
    conn: &mut PgConnection,
    tenant_id: &str,
    id: &str,
    k: &Key,
    c: &Counts,
) -> Result<(), ApiError> {
    // the unique index allows one open alert per key; another worker may have just reopened it
    let n = sqlx::query("UPDATE provider_alerts SET status = 'open', resolved_at = NULL, resolve_reason = NULL, opened_at = now(), last_seen_at = now(), flap_count = flap_count + 1, samples = $2, failures = $3, failure_pct = $4, acknowledged_by = NULL, acknowledged_at = NULL WHERE id = $1 AND status = 'resolved'")
        .bind(id)
        .bind(c.samples() as i32)
        .bind((c.failures + c.unresolved) as i32)
        .bind(c.failure_pct() as i32)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if n == 0 {
        return Ok(());
    }
    sqlx::query("UPDATE recon_cases SET status = 'open', resolution = NULL, resolved_at = NULL WHERE kind = 'provider_alert' AND subject_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    tenant::audit(conn, tenant_id, "system", "provider_alert.reopened", Some(id), json!({ "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value, "failure_pct": c.failure_pct(), "note": "recurred soon after it resolved: reopened, no new notification" })).await?;
    Ok(())
}

async fn resolve(
    conn: &mut PgConnection,
    tenant_id: &str,
    cfg: &Config,
    id: &str,
    k: &Key,
    c: &Counts,
    why: &str,
) -> Result<(), ApiError> {
    let row: Option<i32> = sqlx::query_scalar("UPDATE provider_alerts SET status = 'resolved', resolved_at = now(), resolve_reason = $2 WHERE id = $1 AND status = 'open' RETURNING flap_count")
        .bind(id)
        .bind(why)
        .fetch_optional(&mut *conn)
        .await?;
    let Some(flaps) = row else {
        return Ok(());
    };
    sqlx::query("UPDATE recon_cases SET status = 'resolved', resolution = $2, resolved_at = now() WHERE kind = 'provider_alert' AND subject_id = $1 AND status <> 'resolved'")
        .bind(id)
        .bind(why)
        .execute(&mut *conn)
        .await?;
    tenant::audit(conn, tenant_id, "system", "provider_alert.resolved", Some(id), json!({ "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value, "reason": why })).await?;
    let subject = format!("Recovered: {}", label(k));
    let body = format!("The downtime alert for {} resolved: {why}.", label(k));
    notify(conn, tenant_id, cfg, "provider.alert_resolved", &format!("palert:{id}:resolved:{flaps}"), &subject, &body, json!({ "alert_id": id, "provider": k.provider, "method": k.method, "dimension": k.dimension, "value": k.value, "reason": why, "samples": c.samples() })).await
}

// ------------------------------------------------------------------ API

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    if q.status
        .as_deref()
        .is_some_and(|x| !matches!(x, "open" | "resolved"))
    {
        return Err(ApiError::invalid(
            "invalid_status",
            "status is open or resolved",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(a) - 'tenant_id' FROM provider_alerts a WHERE ($1::text IS NULL OR status = $1) ORDER BY opened_at DESC, id LIMIT 200",
    )
    .bind(q.status)
    .fetch_all(&mut *tx)
    .await?;
    let open: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_alerts WHERE status = 'open'")
            .fetch_one(&mut *tx)
            .await?;
    Ok(Json(json!({ "open": open, "data": rows })))
}

pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<Value> =
        sqlx::query_scalar("SELECT to_jsonb(a) - 'tenant_id' FROM provider_alerts a WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let alert = row.ok_or_else(|| ApiError::not_found("alert"))?;
    let case: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'status', status, 'assignee', assignee) FROM recon_cases WHERE kind = 'provider_alert' AND subject_id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(Json(json!({ "alert": alert, "case": case })))
}

pub async fn acknowledge(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE provider_alerts SET acknowledged_by = $2, acknowledged_at = now() WHERE id = $1 AND status = 'open'")
        .bind(&id)
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("open alert"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "provider_alert.acknowledged",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "acknowledged_by": auth.actor })))
}

pub async fn get_settings(State(s): State<AppState>, auth: Auth) -> Result<Json<Config>, ApiError> {
    auth.require("routing:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(load_config(&mut tx).await?))
}

#[derive(Deserialize)]
pub struct AlertSettingsIn {
    enabled: Option<bool>,
    min_samples: Option<i32>,
    open_failure_pct: Option<i32>,
    resolve_failure_pct: Option<i32>,
    window_minutes: Option<i32>,
    hold_seconds: Option<i32>,
    flap_window_seconds: Option<i32>,
    /// An address to email when an alert opens or resolves. An empty string turns email off.
    email_to: Option<String>,
}

pub async fn put_settings(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<AlertSettingsIn>,
) -> Result<Json<Config>, ApiError> {
    auth.require("routing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mut c = load_config(&mut tx).await?;
    macro_rules! set {
        ($f:ident) => {
            if let Some(v) = b.$f {
                c.$f = v;
            }
        };
    }
    set!(enabled);
    set!(min_samples);
    set!(open_failure_pct);
    set!(resolve_failure_pct);
    set!(window_minutes);
    set!(hold_seconds);
    set!(flap_window_seconds);
    if let Some(e) = &b.email_to {
        let e = e.trim();
        if e.is_empty() {
            c.email_to = None;
        } else if e.len() > 254
            || e.contains(char::is_whitespace)
            || e.split_once('@')
                .is_none_or(|(l, d)| l.is_empty() || !d.contains('.'))
        {
            return Err(ApiError::invalid(
                "invalid_settings",
                "email_to is not an email address",
            ));
        } else {
            c.email_to = Some(e.to_string());
        }
    }
    let ok = (5..=10_000).contains(&c.min_samples)
        && (1..=100).contains(&c.open_failure_pct)
        && (0..=99).contains(&c.resolve_failure_pct)
        && c.resolve_failure_pct < c.open_failure_pct
        && (5..=1440).contains(&c.window_minutes)
        && (0..=86_400).contains(&c.hold_seconds)
        && (0..=86_400).contains(&c.flap_window_seconds);
    if !ok {
        return Err(ApiError::invalid(
            "invalid_settings",
            "a value is out of range, or resolve_failure_pct is not below open_failure_pct",
        ));
    }
    sqlx::query(
        "INSERT INTO provider_alert_config (tenant_id, enabled, min_samples, open_failure_pct, resolve_failure_pct, window_minutes, hold_seconds, flap_window_seconds, email_to, updated_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (tenant_id) DO UPDATE SET enabled = $2, min_samples = $3, open_failure_pct = $4, resolve_failure_pct = $5, window_minutes = $6, hold_seconds = $7, flap_window_seconds = $8, email_to = $9, updated_by = $10, updated_at = now()",
    )
    .bind(&auth.tenant_id)
    .bind(c.enabled)
    .bind(c.min_samples)
    .bind(c.open_failure_pct)
    .bind(c.resolve_failure_pct)
    .bind(c.window_minutes)
    .bind(c.hold_seconds)
    .bind(c.flap_window_seconds)
    .bind(&c.email_to)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(&mut tx, &auth.tenant_id, &auth.actor, "provider_alert.settings_updated", None, json!({ "enabled": c.enabled, "min_samples": c.min_samples, "open_failure_pct": c.open_failure_pct, "resolve_failure_pct": c.resolve_failure_pct, "email_set": c.email_to.is_some() })).await?;
    tx.commit().await?;
    Ok(Json(c))
}

/// Run the check now (the worker does the same on its cycle).
pub async fn evaluate(State(s): State<AppState>, auth: Auth) -> Result<Json<Sweep>, ApiError> {
    auth.require("routing:write")?;
    Ok(Json(sweep_tenant(&s.pool, &auth.tenant_id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: i64, f: i64, u: i64) -> Counts {
        Counts {
            successes: s,
            failures: f,
            unresolved: u,
        }
    }

    #[test]
    fn thresholds_have_a_gap_so_a_flaky_provider_does_not_flap() {
        let cfg = Config::default(); // open at 50%, resolve at 20%, 20 samples
        assert_eq!(read(&c(5, 10, 0), &cfg), Reading::Thin);
        assert_eq!(read(&c(5, 15, 0), &cfg), Reading::Bad); // 75%
        assert_eq!(read(&c(10, 10, 0), &cfg), Reading::Bad); // exactly 50%
        assert_eq!(read(&c(12, 8, 0), &cfg), Reading::Middling); // 40%: neither opens nor resolves
        assert_eq!(read(&c(16, 4, 0), &cfg), Reading::Good); // exactly 20%
        // unresolved attempts count against the group
        assert_eq!(read(&c(5, 0, 15), &cfg), Reading::Bad);
    }

    #[test]
    fn failure_pct_rounds_down_and_handles_no_data() {
        assert_eq!(c(0, 0, 0).failure_pct(), 0);
        assert_eq!(c(2, 1, 0).failure_pct(), 33);
    }
}
