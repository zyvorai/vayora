//! Payment link analytics that keep nothing about the visitor.
//!
//! What is counted, and how:
//! - `opened` and `started` are counters per link per UTC day. A visit counts once per *coarse visitor token* per
//!   day, not once per request: the token is a keyed hash of (day, link, the address rounded to /24 or /48, device
//!   class, system, browser). The hash key is derived from the server's master key and the day is part of the input,
//!   so a token cannot be linked across days or looked up from an address. Only the hash is stored, only for the
//!   day, and the worker deletes it after two days. No address, user-agent text or cookie is ever stored.
//! - Requests from clients that say they are not people (crawlers, link-preview fetchers, command-line tools, no
//!   user-agent at all) are not counted. At most `DAILY_CAP` distinct visitors are counted per link per day, so a
//!   flood cannot grow the table without bound.
//! - `paid`, `failed` and `expired` are not counters: they are read from the payments themselves (captured payments
//!   through the link, payments through it that had a failed attempt, links that reached their expiry unpaid), so
//!   nothing a bot does can raise them.
//!
//! Limits, stated plainly: a token is a coarse guess at "one visitor". People behind one network address with the
//! same kind of phone and browser (a school, an office) count once; one person on a changing mobile address counts
//! more than once; a bot that looks like a browser from many networks is counted. Treat opened and started as
//! estimates of reach, and paid as exact. Visits are only counted when the checkout app passes the visitor's address
//! through (`ZYVOR_TRUST_PROXY_HOPS`); when the API only ever sees the checkout server's address, all visitors with
//! the same device kind collapse into one, which undercounts.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, request::Parts},
};
use chrono::{Duration, NaiveDate, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{collections::BTreeMap, net::IpAddr};

use crate::{ApiError, AppState, auth::Auth, tenant};

/// Distinct visitors counted per link, per event, per day.
pub const DAILY_CAP: i32 = 5_000;
/// Longest window the analytics endpoints answer for.
pub const MAX_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Opened,
    Started,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Opened => "opened",
            Event::Started => "started",
        }
    }
}

/// Clients that announce themselves as machines, plus an absent or implausibly short user-agent.
pub fn is_bot(ua: &str) -> bool {
    let l = ua.trim().to_ascii_lowercase();
    if l.len() < 10 {
        return true;
    }
    const MARKERS: &[&str] = &[
        "bot",
        "crawl",
        "spider",
        "slurp",
        "preview",
        "fetch",
        "facebookexternalhit",
        "whatsapp",
        "telegram",
        "slack",
        "discord",
        "linkedin",
        "twitter",
        "skype",
        "curl/",
        "wget/",
        "python",
        "go-http-client",
        "headless",
        "lighthouse",
        "pingdom",
        "uptime",
        "monitor",
        "scanner",
        "libwww",
        "httpclient",
        "axios",
        "java/",
        "okhttp",
        "postman",
        "insomnia",
        "phantom",
        "selenium",
    ];
    MARKERS.iter().any(|m| l.contains(m))
}

/// The address rounded so it names a network, not a person: /24 for IPv4, /48 for IPv6.
pub fn coarse_ip(ip: Option<IpAddr>) -> String {
    match ip {
        Some(IpAddr::V4(v)) => {
            let o = v.octets();
            format!("{}.{}.{}.0/24", o[0], o[1], o[2])
        }
        Some(IpAddr::V6(v)) => {
            let s = v.segments();
            format!("{:x}:{:x}:{:x}::/48", s[0], s[1], s[2])
        }
        None => "unknown".into(),
    }
}

/// 32 hex characters: the day-scoped keyed hash of the coarse visitor.
pub fn visitor_token(
    key: &[u8],
    day: NaiveDate,
    link_id: &str,
    ip: Option<IpAddr>,
    headers: &HeaderMap,
) -> String {
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let env = crate::clientenv::from_user_agent(ua);
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(
        format!(
            "visitor|{day}|{link_id}|{}|{}|{}|{}",
            coarse_ip(ip),
            env.device,
            env.os,
            env.browser
        )
        .as_bytes(),
    );
    hex::encode(&m.finalize().into_bytes()[..16])
}

/// Count one visit if this coarse visitor has not been counted today. Never fails the request it is called from.
pub async fn record(s: &AppState, tenant_id: &str, link_id: &str, event: Event, parts: &Parts) {
    if let Err(e) = try_record(s, tenant_id, link_id, event, parts).await {
        tracing::warn!(error = %e, "link analytics not recorded");
    }
}

async fn try_record(
    s: &AppState,
    tenant_id: &str,
    link_id: &str,
    event: Event,
    parts: &Parts,
) -> Result<(), sqlx::Error> {
    let ua = parts
        .headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if is_bot(ua) {
        return Ok(());
    }
    let ip = crate::ratelimit::client_ip(&parts.headers, &parts.extensions, s.trusted_hops);
    let day = Utc::now().date_naive();
    let token = visitor_token(
        &s.crypto.derive("link-analytics"),
        day,
        link_id,
        ip,
        &parts.headers,
    );
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let so_far: i32 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT n FROM link_events_daily WHERE link_id = $1 AND day = $2 AND event = $3), 0)",
    )
    .bind(link_id)
    .bind(day)
    .bind(event.as_str())
    .fetch_one(&mut *tx)
    .await?;
    if so_far >= DAILY_CAP {
        return Ok(());
    }
    let fresh = sqlx::query("INSERT INTO link_visitors_seen (tenant_id, link_id, day, event, visitor) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
        .bind(tenant_id)
        .bind(link_id)
        .bind(day)
        .bind(event.as_str())
        .bind(&token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if fresh == 1 {
        sqlx::query("INSERT INTO link_events_daily (tenant_id, link_id, day, event, n) VALUES ($1, $2, $3, $4, 1) ON CONFLICT (link_id, day, event) DO UPDATE SET n = link_events_daily.n + 1")
            .bind(tenant_id)
            .bind(link_id)
            .bind(day)
            .bind(event.as_str())
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

/// The worker's housekeeping for one tenant: forget the visitor hashes of earlier days. Counters stay.
pub async fn purge_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<u64, sqlx::Error> {
    let mut tx = tenant::begin(pool, tenant_id).await?;
    let n = sqlx::query(
        "DELETE FROM link_visitors_seen WHERE day < (now() AT TIME ZONE 'UTC')::date - 1",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(n)
}

#[derive(Deserialize, Default)]
pub struct DaysQuery {
    /// How many days back to report, 1-90 (default 30). Days are UTC.
    days: Option<i64>,
}

#[derive(Default, Clone, Copy)]
struct Day {
    opened: i64,
    started: i64,
    paid: i64,
    paid_minor: i64,
    failed: i64,
    expired: i64,
}

fn rate(num: i64, den: i64) -> Value {
    if den <= 0 {
        Value::Null
    } else {
        // visitors and payments are counted differently, so a ratio can pass 100%; show at most 1
        json!((((num as f64 / den as f64).min(1.0)) * 10_000.0).round() / 10_000.0)
    }
}

async fn report(
    tx: &mut sqlx::PgConnection,
    from: NaiveDate,
    to: NaiveDate,
    link: Option<&str>,
) -> Result<(BTreeMap<NaiveDate, Day>, Day), ApiError> {
    let mut days: BTreeMap<NaiveDate, Day> = BTreeMap::new();
    let mut d = from;
    while d <= to {
        days.insert(d, Day::default());
        d += Duration::days(1);
    }
    let counters: Vec<(NaiveDate, String, i64)> = sqlx::query_as(
        "SELECT day, event, SUM(n)::BIGINT FROM link_events_daily WHERE day BETWEEN $1 AND $2 AND ($3::text IS NULL OR link_id = $3) GROUP BY day, event",
    )
    .bind(from)
    .bind(to)
    .bind(link)
    .fetch_all(&mut *tx)
    .await?;
    for (day, ev, n) in counters {
        if let Some(r) = days.get_mut(&day) {
            match ev.as_str() {
                "opened" => r.opened = n,
                "started" => r.started = n,
                _ => {}
            }
        }
    }
    let paid: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(
        "SELECT (i.updated_at AT TIME ZONE 'UTC')::date, count(*)::BIGINT, COALESCE(SUM((i.metadata->>'base_minor')::bigint), 0)::BIGINT
           FROM payment_intents i
          WHERE i.status = 'succeeded' AND i.metadata->>'payment_link' IS NOT NULL AND ($3::text IS NULL OR i.metadata->>'payment_link' = $3)
            AND (i.updated_at AT TIME ZONE 'UTC')::date BETWEEN $1 AND $2
          GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .bind(link)
    .fetch_all(&mut *tx)
    .await?;
    for (day, n, minor) in paid {
        if let Some(r) = days.get_mut(&day) {
            r.paid = n;
            r.paid_minor = minor;
        }
    }
    let failed: Vec<(NaiveDate, i64)> = sqlx::query_as(
        "SELECT (a.updated_at AT TIME ZONE 'UTC')::date, count(DISTINCT a.intent_id)::BIGINT
           FROM payment_attempts a JOIN payment_intents i ON i.id = a.intent_id
          WHERE a.status = 'failed' AND i.metadata->>'payment_link' IS NOT NULL AND ($3::text IS NULL OR i.metadata->>'payment_link' = $3)
            AND (a.updated_at AT TIME ZONE 'UTC')::date BETWEEN $1 AND $2
          GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .bind(link)
    .fetch_all(&mut *tx)
    .await?;
    for (day, n) in failed {
        if let Some(r) = days.get_mut(&day) {
            r.failed = n;
        }
    }
    // a link that reached its expiry without being paid (a part-paid link: with a balance still owing)
    let expired: Vec<(NaiveDate, i64)> = sqlx::query_as(
        "SELECT (l.expires_at AT TIME ZONE 'UTC')::date, count(*)::BIGINT FROM payment_links l
          WHERE l.expires_at < now() AND l.canceled_at IS NULL AND ($3::text IS NULL OR l.id = $3)
            AND (l.expires_at AT TIME ZONE 'UTC')::date BETWEEN $1 AND $2
            AND CASE WHEN l.allow_partial THEN
                  (CASE WHEN l.invoice_id IS NOT NULL THEN invoice_settled(l.invoice_id) < (SELECT amount_minor FROM invoices WHERE id = l.invoice_id)
                        ELSE COALESCE((SELECT SUM((i.metadata->>'base_minor')::bigint) FROM payment_intents i WHERE i.metadata->>'payment_link' = l.id AND i.status = 'succeeded'), 0) < l.amount_minor END)
                ELSE NOT EXISTS (SELECT 1 FROM payment_intents i WHERE i.metadata->>'payment_link' = l.id AND i.status = 'succeeded') END
          GROUP BY 1",
    )
    .bind(from)
    .bind(to)
    .bind(link)
    .fetch_all(&mut *tx)
    .await?;
    for (day, n) in expired {
        if let Some(r) = days.get_mut(&day) {
            r.expired = n;
        }
    }
    let mut t = Day::default();
    for r in days.values() {
        t.opened += r.opened;
        t.started += r.started;
        t.paid += r.paid;
        t.paid_minor += r.paid_minor;
        t.failed += r.failed;
        t.expired += r.expired;
    }
    Ok((days, t))
}

fn day_json(d: NaiveDate, r: &Day) -> Value {
    json!({ "day": d, "opened": r.opened, "started": r.started, "paid": r.paid, "paid_minor": r.paid_minor, "failed": r.failed, "expired": r.expired })
}

fn body(from: NaiveDate, to: NaiveDate, days: &BTreeMap<NaiveDate, Day>, t: &Day) -> Value {
    json!({
        "from": from, "to": to,
        "totals": { "opened": t.opened, "started": t.started, "paid": t.paid, "paid_minor": t.paid_minor, "failed": t.failed, "expired": t.expired },
        "daily": days.iter().map(|(d, r)| day_json(*d, r)).collect::<Vec<_>>(),
        "funnel": {
            "opened": t.opened, "started": t.started, "paid": t.paid,
            "started_of_opened": rate(t.started, t.opened),
            "paid_of_started": rate(t.paid, t.started),
            "paid_of_opened": rate(t.paid, t.opened),
        },
        "notes": [
            "opened and started count coarse visitor tokens once per day (estimates of reach, not people); crawlers and link previews are not counted",
            "paid, failed and expired are read from the payments and links themselves: captured payments through the link, payments with a failed attempt, links that reached their expiry unpaid",
            "days are UTC; rates are capped at 1 because visitors and payments are counted differently; null when there is nothing to divide by",
        ],
    })
}

fn window(q: &DaysQuery) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let n = q.days.unwrap_or(30);
    if !(1..=MAX_DAYS).contains(&n) {
        return Err(ApiError::invalid(
            "invalid_days",
            "days is between 1 and 90",
        ));
    }
    let to = Utc::now().date_naive();
    Ok((to - Duration::days(n - 1), to))
}

/// `GET /v1/payment_links/analytics`: every link of the business together, per day, with the conversion funnel.
pub async fn summary(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<DaysQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let (from, to) = window(&q)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (days, t) = report(&mut tx, from, to, None).await?;
    let mut out = body(from, to, &days, &t);
    let (created, active): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE (created_at AT TIME ZONE 'UTC')::date BETWEEN $1 AND $2)::BIGINT,
                count(*) FILTER (WHERE canceled_at IS NULL AND (expires_at IS NULL OR expires_at > now()) AND (max_uses IS NULL OR uses < max_uses))::BIGINT
           FROM payment_links",
    )
    .bind(from)
    .bind(to)
    .fetch_one(&mut *tx)
    .await?;
    out["links"] = json!({ "created_in_window": created, "currently_usable": active });
    Ok(Json(out))
}

/// `GET /v1/payment_links/{id}/analytics`: one link, per day, with its funnel and (for a link paid in parts) its balance.
pub async fn link_analytics(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<DaysQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let (from, to) = window(&q)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(bool, Option<i64>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT allow_partial, amount_minor, invoice_id, min_part_minor FROM payment_links WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (partial, amount, invoice, min_part) =
        row.ok_or_else(|| ApiError::not_found("payment link"))?;
    let (days, t) = report(&mut tx, from, to, Some(&id)).await?;
    let mut out = body(from, to, &days, &t);
    out["link_id"] = json!(id);
    if partial {
        let b = crate::payment_links::balance(&mut tx, &id, amount, invoice.as_deref()).await?;
        out["balance"] = json!({ "total_minor": b.total, "paid_minor": b.paid, "owed_minor": b.owed(), "held_minor": b.held, "overpaid_minor": b.overpaid(), "min_part_minor": min_part });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_are_not_counted() {
        assert!(is_bot(""));
        assert!(is_bot("curl/8.4.0"));
        assert!(is_bot(
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"
        ));
        assert!(is_bot("WhatsApp/2.23.20.0 A"));
        assert!(is_bot("Slackbot-LinkExpanding 1.0"));
        assert!(is_bot("python-requests/2.31"));
        assert!(!is_bot(
            "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Mobile Safari/537.36"
        ));
        assert!(!is_bot(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1"
        ));
    }

    #[test]
    fn addresses_are_rounded_and_tokens_are_day_scoped() {
        assert_eq!(
            coarse_ip(Some("203.0.113.77".parse().unwrap())),
            "203.0.113.0/24"
        );
        assert_eq!(
            coarse_ip(Some("2001:db8:abcd:1234::1".parse().unwrap())),
            "2001:db8:abcd::/48"
        );
        assert_eq!(coarse_ip(None), "unknown");
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::USER_AGENT,
            "Mozilla/5.0 (Linux; Android 14) Chrome/124.0 Mobile Safari/537.36"
                .parse()
                .unwrap(),
        );
        let k = b"key-for-tests-only-0123456789abc";
        let d1 = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let a = visitor_token(k, d1, "plk_1", "203.0.113.5".parse().ok(), &h);
        let same_net = visitor_token(k, d1, "plk_1", "203.0.113.200".parse().ok(), &h);
        let other_net = visitor_token(k, d1, "plk_1", "203.0.114.5".parse().ok(), &h);
        let next_day = visitor_token(k, d2, "plk_1", "203.0.113.5".parse().ok(), &h);
        let other_link = visitor_token(k, d1, "plk_2", "203.0.113.5".parse().ok(), &h);
        assert_eq!(a.len(), 32);
        assert_eq!(a, same_net);
        assert_ne!(a, other_net);
        assert_ne!(a, next_day);
        assert_ne!(a, other_link);
        assert!(!a.contains("203"), "the token must not carry the address");
    }

    #[test]
    fn rates_are_capped_and_empty_when_there_is_nothing_to_divide() {
        assert_eq!(rate(1, 0), Value::Null);
        assert_eq!(rate(1, 4), json!(0.25));
        assert_eq!(rate(9, 3), json!(1.0));
    }
}
