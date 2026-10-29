//! Signed outbound webhooks over a transactional outbox.
//!
//! Producers call [`enqueue`] inside the same DB transaction as the business
//! change, so an event exists if and only if the change committed. A
//! [`Dispatcher`] then delivers with retry and exponential backoff.
//!
//! Signature header: `Vayora-Signature: t=<unix>,v1=<hex hmac-sha256("{t}.{body}")>`.

use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::{PgConnection, PgPool};

type HmacSha256 = Hmac<Sha256>;

pub const SIGNATURE_HEADER: &str = "Vayora-Signature";
/// Attempts before an event is parked as `dead`.
pub const MAX_ATTEMPTS: i32 = 10;

fn mac(secret: &[u8], timestamp: i64, body: &[u8]) -> HmacSha256 {
    let mut m = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    m.update(timestamp.to_string().as_bytes());
    m.update(b".");
    m.update(body);
    m
}

pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let tag = mac(secret, timestamp, body).finalize().into_bytes();
    format!("t={timestamp},v1={}", hex::encode(tag))
}

/// Verify a signature header; rejects stale timestamps (replay) beyond `tolerance_secs`.
pub fn verify(secret: &[u8], header: &str, body: &[u8], now: i64, tolerance_secs: i64) -> bool {
    let (mut t, mut v1) = (None, None);
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => v1 = hex::decode(v).ok(),
            _ => {}
        }
    }
    let (Some(t), Some(v1)) = (t, v1) else {
        return false;
    };
    if (now - t).abs() > tolerance_secs {
        return false;
    }
    mac(secret, t, body).verify_slice(&v1).is_ok()
}

/// Queue an event. Call inside the business transaction.
pub async fn enqueue(
    conn: &mut PgConnection,
    event_type: &str,
    data: serde_json::Value,
) -> Result<String, sqlx::Error> {
    let id = vayora_core::new_id("evt");
    let payload = serde_json::json!({
        "id": id,
        "type": event_type,
        "created_at": chrono::Utc::now(),
        "data": data,
    });
    sqlx::query("INSERT INTO webhook_events (id, type, payload) VALUES ($1, $2, $3)")
        .bind(&id)
        .bind(event_type)
        .bind(&payload)
        .execute(conn)
        .await?;
    Ok(id)
}

fn backoff(attempts: i32) -> i64 {
    // 5s, 10s, 20s ... capped at 1h.
    (5i64.saturating_mul(1i64 << attempts.clamp(0, 20))).min(3600)
}

pub struct Dispatcher {
    pool: PgPool,
    client: reqwest::Client,
    url: String,
    secret: Vec<u8>,
}

impl Dispatcher {
    pub fn new(pool: PgPool, url: impl Into<String>, secret: impl Into<Vec<u8>>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Self {
            pool,
            client,
            url: url.into(),
            secret: secret.into(),
        }
    }

    /// Deliver every due event once. Returns how many were delivered.
    /// Rows stay locked (SKIP LOCKED) while in flight, so several dispatchers can run safely.
    pub async fn run_once(&self) -> Result<usize, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let due: Vec<(String, serde_json::Value, i32)> = sqlx::query_as(
            "SELECT id, payload, attempts FROM webhook_events
             WHERE status = 'pending' AND next_attempt_at <= now()
             ORDER BY created_at LIMIT 20 FOR UPDATE SKIP LOCKED",
        )
        .fetch_all(&mut *tx)
        .await?;

        let mut delivered = 0;
        for (id, payload, attempts) in due {
            let body = serde_json::to_vec(&payload).expect("json value serialises");
            let ts = chrono::Utc::now().timestamp();
            let result = self
                .client
                .post(&self.url)
                .header("content-type", "application/json")
                .header(SIGNATURE_HEADER, sign(&self.secret, ts, &body))
                .body(body)
                .send()
                .await
                .and_then(|r| r.error_for_status());

            match result {
                Ok(_) => {
                    delivered += 1;
                    sqlx::query(
                        "UPDATE webhook_events SET status = 'delivered', delivered_at = now(),
                         attempts = attempts + 1, last_error = NULL WHERE id = $1",
                    )
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                }
                Err(e) => {
                    tracing::warn!(event = %id, attempt = attempts + 1, error = %e, "webhook delivery failed");
                    let dead = attempts + 1 >= MAX_ATTEMPTS;
                    sqlx::query(
                        "UPDATE webhook_events SET attempts = attempts + 1, last_error = $2,
                         status = CASE WHEN $3 THEN 'dead' ELSE 'pending' END,
                         next_attempt_at = now() + make_interval(secs => $4::float8)
                         WHERE id = $1",
                    )
                    .bind(&id)
                    .bind(e.to_string())
                    .bind(dead)
                    .bind(backoff(attempts) as f64)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        tx.commit().await?;
        Ok(delivered)
    }

    pub async fn run_forever(self) {
        loop {
            if let Err(e) = self.run_once().await {
                tracing::error!(error = %e, "webhook dispatcher error");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip() {
        let h = sign(b"whsec", 1_000, b"{\"a\":1}");
        assert!(verify(b"whsec", &h, b"{\"a\":1}", 1_000, 300));
        assert!(verify(b"whsec", &h, b"{\"a\":1}", 1_299, 300));
    }

    #[test]
    fn rejects_tamper_wrong_key_and_stale() {
        let h = sign(b"whsec", 1_000, b"body");
        assert!(!verify(b"whsec", &h, b"body!", 1_000, 300));
        assert!(!verify(b"other", &h, b"body", 1_000, 300));
        assert!(!verify(b"whsec", &h, b"body", 2_000, 300));
        assert!(!verify(b"whsec", "garbage", b"body", 1_000, 300));
    }

    #[test]
    fn backoff_caps() {
        assert_eq!(backoff(0), 5);
        assert_eq!(backoff(3), 40);
        assert_eq!(backoff(30), 3600);
    }
}
