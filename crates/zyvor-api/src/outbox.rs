//! Delivery of merchant-facing events. At-least-once with exponential backoff, dead-lettered after
//! `MAX_ATTEMPTS`. A row only changes after the HTTP outcome is known, so a worker that crashes
//! mid-batch leaves every event pending for the next run.

use std::time::Duration;

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
        let res = client
            .post(&url)
            .header("content-type", "application/json")
            .header(
                SIGNATURE_HEADER,
                sign(&secret, chrono::Utc::now().timestamp(), &body),
            )
            .body(body)
            .send()
            .await
            .and_then(|r| r.error_for_status());
        match res {
            Ok(_) => {
                delivered += 1;
                sqlx::query("UPDATE outbox_events SET status = 'delivered', delivered_at = now(), attempts = attempts + 1, last_error = NULL WHERE id = $1").bind(&id).execute(&mut *tx).await?;
            }
            Err(e) => {
                tracing::warn!(event = %id, error = %e, "outbound webhook failed");
                let dead = attempts + 1 >= MAX_ATTEMPTS;
                sqlx::query(
                    "UPDATE outbox_events SET attempts = attempts + 1, last_error = $2, status = CASE WHEN $3 THEN 'dead' ELSE 'pending' END,
                     next_attempt_at = now() + make_interval(secs => $4::float8) WHERE id = $1",
                )
                .bind(&id)
                .bind(e.to_string())
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
}
