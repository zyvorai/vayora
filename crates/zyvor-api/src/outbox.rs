// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Delivery of merchant-facing events. At-least-once with exponential backoff, dead-lettered after
//! `MAX_ATTEMPTS`. A row only changes after the HTTP outcome is known, so a worker that crashes
//! mid-batch leaves every event pending for the next run. Each try is also written to
//! `outbox_deliveries` for the inspector.

use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use crate::{Crypto, tenant};

pub const SIGNATURE_HEADER: &str = "Zyvor-Signature";
pub const MAX_ATTEMPTS: i32 = 10;

fn mac(secret: &[u8], t: i64, body: &[u8]) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    m.update(t.to_string().as_bytes());
    m.update(b".");
    m.update(body);
    m
}

/// `t=<unix>,v1=<hex hmac-sha256("{t}.{body}")>`
pub fn sign(secret: &[u8], t: i64, body: &[u8]) -> String {
    format!(
        "t={t},v1={}",
        hex::encode(mac(secret, t, body).finalize().into_bytes())
    )
}

pub fn verify(secret: &[u8], header: &str, body: &[u8], now: i64, tolerance: i64) -> bool {
    let (mut t, mut v1) = (None, None);
    for p in header.split(',') {
        match p.split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => v1 = hex::decode(v).ok(),
            _ => {}
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else {
        return false;
    };
    (now - t).abs() <= tolerance && mac(secret, t, body).verify_slice(&v1).is_ok()
}

fn backoff(attempts: i32) -> f64 {
    (5i64.saturating_mul(1i64 << attempts.clamp(0, 20))).min(3600) as f64
}

#[allow(clippy::too_many_arguments)]
async fn record_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    event_id: &str,
    attempt_no: i32,
    ok: bool,
    http_status: Option<i32>,
    error: Option<&str>,
    latency_ms: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO outbox_deliveries (tenant_id, event_id, attempt_no, ok, http_status, error, latency_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(tenant_id)
    .bind(event_id)
    .bind(attempt_no)
    .bind(ok)
    .bind(http_status)
    .bind(error)
    .bind(latency_ms)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deliver due events for one tenant. Returns the number delivered.
pub async fn deliver_tenant(
    pool: &PgPool,
    crypto: &Crypto,
    client: &reqwest::Client,
    tenant_id: &str,
    allow_private: bool,
) -> Result<usize, sqlx::Error> {
    let mut tx = tenant::begin(pool, tenant_id).await?;
    let cfg: Option<(Option<String>, Option<Vec<u8>>)> = sqlx::query_as("SELECT outbound_webhook_url, outbound_webhook_secret_enc FROM tenant_settings WHERE tenant_id = $1").bind(tenant_id).fetch_optional(&mut *tx).await?;
    let Some((Some(url), Some(secret_enc))) = cfg else {
        return Ok(0);
    };
    let Some(secret) = crypto.decrypt(&secret_enc) else {
        return Ok(0);
    };

    // Connect only to addresses vetted just now, so a name that starts resolving to an internal
    // address after it was saved cannot be used to reach our network.
    let pinned;
    let client = if allow_private {
        client
    } else {
        let checked = match crate::ssrf::check_url(&url, false) {
            Ok(u) => crate::ssrf::resolve_public(&u).await.map(|a| (u, a)),
            Err(e) => Err(e),
        };
        match checked {
            Ok((u, addrs)) => {
                pinned = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(u.host_str().unwrap_or_default(), &addrs)
                    .build()
                    .expect("http client");
                &pinned
            }
            Err(e) => {
                tracing::warn!(tenant = tenant_id, error = %e, "outbound webhook target refused");
                return Ok(0);
            }
        }
    };

    let due: Vec<(String, serde_json::Value, i32)> = sqlx::query_as(
        "SELECT id, payload, attempts FROM outbox_events WHERE status = 'pending' AND next_attempt_at <= now() ORDER BY created_at LIMIT 50 FOR UPDATE SKIP LOCKED",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut delivered = 0;
    for (id, payload, attempts) in due {
        let body = serde_json::to_vec(&payload).expect("json serialises");
        let started = Instant::now();
        let attempt_no = attempts + 1;
        let send = client
            .post(&url)
            .header("content-type", "application/json")
            .header(
                SIGNATURE_HEADER,
                sign(&secret, chrono::Utc::now().timestamp(), &body),
            )
            .body(body)
            .send()
            .await;
        let latency_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        match send {
            Ok(res) => {
                let status = res.status().as_u16() as i32;
                if res.status().is_success() {
                    delivered += 1;
                    record_attempt(
                        &mut tx,
                        tenant_id,
                        &id,
                        attempt_no,
                        true,
                        Some(status),
                        None,
                        latency_ms,
                    )
                    .await?;
                    sqlx::query("UPDATE outbox_events SET status = 'delivered', delivered_at = now(), attempts = attempts + 1, last_error = NULL WHERE id = $1").bind(&id).execute(&mut *tx).await?;
                } else {
                    let err = format!("HTTP {status}");
                    tracing::warn!(event = %id, status, "outbound webhook failed");
                    let dead = attempt_no >= MAX_ATTEMPTS;
                    record_attempt(
                        &mut tx,
                        tenant_id,
                        &id,
                        attempt_no,
                        false,
                        Some(status),
                        Some(&err),
                        latency_ms,
                    )
                    .await?;
                    sqlx::query(
                        "UPDATE outbox_events SET attempts = attempts + 1, last_error = $2, status = CASE WHEN $3 THEN 'dead' ELSE 'pending' END,
                         next_attempt_at = now() + make_interval(secs => $4::float8) WHERE id = $1",
                    )
                    .bind(&id)
                    .bind(&err)
                    .bind(dead)
                    .bind(backoff(attempts))
                    .execute(&mut *tx)
                    .await?;
                }
            }
            Err(e) => {
                tracing::warn!(event = %id, error = %e, "outbound webhook failed");
                let err = e.to_string();
                let dead = attempt_no >= MAX_ATTEMPTS;
                record_attempt(
                    &mut tx,
                    tenant_id,
                    &id,
                    attempt_no,
                    false,
                    None,
                    Some(&err),
                    latency_ms,
                )
                .await?;
                sqlx::query(
                    "UPDATE outbox_events SET attempts = attempts + 1, last_error = $2, status = CASE WHEN $3 THEN 'dead' ELSE 'pending' END,
                     next_attempt_at = now() + make_interval(secs => $4::float8) WHERE id = $1",
                )
                .bind(&id)
                .bind(&err)
                .bind(dead)
                .bind(backoff(attempts))
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(delivered)
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_replay_window() {
        let h = sign(b"s", 1_000, b"{}");
        assert!(verify(b"s", &h, b"{}", 1_100, 300));
        assert!(!verify(b"s", &h, b"{ }", 1_100, 300));
        assert!(!verify(b"x", &h, b"{}", 1_100, 300));
        assert!(!verify(b"s", &h, b"{}", 2_000, 300));
        assert_eq!(backoff(0), 5.0);
        assert_eq!(backoff(40), 3600.0);
    }
    /// The signature scheme is part of the public contract: this vector is also asserted by the Node SDK tests
    /// (`packages/zyvor-node/src/webhooks.test.ts`), computed there with Node'"'"'s own HMAC, so both sides agree.
    #[test]
    fn signature_vector_is_pinned_and_shared_with_the_sdk() {
        let body = br#"{"id":"evt_test_1","type":"payment.captured","amount_minor":50000}"#;
        let h = sign(b"whsec_test_vector", 1_700_000_000, body);
        assert_eq!(
            h,
            "t=1700000000,v1=b7d274b460eaf81fcd9b75301d5db186d1062bbd2402f6606d0e144a56709030"
        );
        assert!(verify(b"whsec_test_vector", &h, body, 1_700_000_100, 300));
        // replay window is symmetric: a timestamp from the future is as stale as one from the past
        assert!(!verify(b"whsec_test_vector", &h, body, 1_699_999_000, 300));
        assert!(!verify(b"whsec_test_vector", &h, body, 1_700_000_301, 300));
        assert!(verify(b"whsec_test_vector", &h, body, 1_700_000_300, 300));
    }

    /// Pins today'"'"'s behaviour for a header with two `v1` entries: the LAST one is the one checked. The SDK helper
    /// `verifyWebhookDetailed` accepts any entry (for secret rotation); the server only ever sends one.
    #[test]
    fn with_two_v1_entries_the_last_one_counts() {
        let good = sign(b"s", 1_000, b"{}");
        let v1 = good.split_once(",").unwrap().1;
        let bad = "v1=00";
        assert!(verify(
            b"s",
            &format!("t=1000,{bad},{v1}"),
            b"{}",
            1_000,
            300
        ));
        assert!(!verify(
            b"s",
            &format!("t=1000,{v1},{bad}"),
            b"{}",
            1_000,
            300
        ));
    }

    #[test]
    fn hostile_headers_do_not_verify_or_panic() {
        for h in [
            "",
            "t=",
            "t=1,v1=",
            "t=x,v1=00",
            "v1=00",
            "t=1,v1=zz",
            ",,,",
            "t=99999999999999999999,v1=00",
        ] {
            assert!(!verify(b"s", h, b"{}", 1, 300), "{h}");
        }
    }
}
