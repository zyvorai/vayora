//! Outbound SMS, kept deliberately small: a queue and one hook. Zyvor ships no SMS provider integration.
//!
//! `ZYVOR_SMS_MODE` chooses what happens to a message:
//! - `off` (default): nothing is queued; asking to send an SMS is a 409 (`sms_not_configured`).
//! - `sandbox`: the message is recorded as `logged` and the log line shows only the last four digits of the number.
//!   Nothing leaves the server.
//! - `hook`: the message is queued and the worker posts it, signed like a webhook (`Zyvor-Signature`), to
//!   `ZYVOR_SMS_HOOK_URL` (an address the operator sets, so it is not checked as a merchant-supplied address). The
//!   hook is where a business connects its own SMS provider and its own sender and template registration; Zyvor
//!   neither holds provider credentials nor registers templates.
//!
//! A message is sent at most once per `dedupe_key`. Sent text is blanked from the queue.

use serde_json::json;
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{AppState, tenant};

#[derive(Debug, Clone, Default)]
pub enum Mode {
    #[default]
    Off,
    Sandbox,
    Hook {
        url: String,
        secret: Vec<u8>,
    },
}

impl Mode {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string());
        match get("ZYVOR_SMS_MODE").as_deref() {
            Some("sandbox") => Mode::Sandbox,
            Some("hook") => match (get("ZYVOR_SMS_HOOK_URL"), get("ZYVOR_SMS_HOOK_SECRET")) {
                (Some(url), Some(secret)) if url.starts_with("http") && secret.len() >= 16 => {
                    Mode::Hook {
                        url,
                        secret: secret.into_bytes(),
                    }
                }
                _ => {
                    tracing::warn!(
                        "ZYVOR_SMS_MODE=hook needs ZYVOR_SMS_HOOK_URL and a ZYVOR_SMS_HOOK_SECRET of 16+ characters; SMS stays off"
                    );
                    Mode::Off
                }
            },
            _ => Mode::Off,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Sandbox => "sandbox",
            Mode::Hook { .. } => "hook",
        }
    }
    pub fn is_on(&self) -> bool {
        !matches!(self, Mode::Off)
    }
}

/// An E.164-style number from what a person typed: spaces, dashes and brackets are dropped; a bare 10-digit Indian
/// mobile number (starting 6-9) gets +91; anything else must carry its country code (with or without +) and have 8-15
/// digits. `None` when it cannot be read as a number.
pub fn normalise_phone(raw: &str) -> Option<String> {
    let t = raw.trim();
    let plus = t.starts_with('+');
    let digits: String = t
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '.' | '+'))
        .collect();
    if digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
        || t.chars()
            .any(|c| !(c.is_ascii_digit() || " -().+".contains(c)))
        || t.matches('+').count() > usize::from(plus)
        || (t.contains('+') && !plus)
    {
        return None;
    }
    let n = digits.len();
    if !plus && n == 10 && matches!(digits.as_bytes()[0], b'6'..=b'9') {
        return Some(format!("+91{digits}"));
    }
    if !plus && n == 11 && digits.starts_with('0') && matches!(digits.as_bytes()[1], b'6'..=b'9') {
        return Some(format!("+91{}", &digits[1..]));
    }
    if (8..=15).contains(&n) && (plus || n >= 11) && !digits.starts_with('0') {
        Some(format!("+{digits}"))
    } else {
        None
    }
}

/// `+91******3210`: enough to recognise, not enough to call.
pub fn mask(phone: &str) -> String {
    let n = phone.chars().count();
    if n <= 4 {
        return "*".repeat(n);
    }
    let tail: String = phone.chars().skip(n - 4).collect();
    format!("{}{tail}", "*".repeat(n - 4))
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Queued {
    /// Waiting for the worker to post it to the hook.
    Queued,
    /// Sandbox: recorded, not sent.
    Logged,
    /// This `dedupe_key` was already used; nothing new was queued.
    Duplicate,
}

/// Queue a text inside the caller's transaction. `Mode::Off` queues nothing and returns `None`.
pub async fn enqueue(
    conn: &mut PgConnection,
    mode: &Mode,
    tenant_id: &str,
    to_phone: &str,
    body: &str,
    dedupe_key: &str,
) -> Result<Option<Queued>, sqlx::Error> {
    let status = match mode {
        Mode::Off => return Ok(None),
        Mode::Sandbox => "logged",
        Mode::Hook { .. } => "queued",
    };
    let n = sqlx::query("INSERT INTO sms_queue (id, tenant_id, to_phone, body, dedupe_key, status) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (tenant_id, dedupe_key) DO NOTHING")
        .bind(new_id("sms"))
        .bind(tenant_id)
        .bind(to_phone)
        .bind(body)
        .bind(dedupe_key)
        .bind(status)
        .execute(conn)
        .await?
        .rows_affected();
    if n == 0 {
        return Ok(Some(Queued::Duplicate));
    }
    if status == "logged" {
        tracing::info!(to = %mask(to_phone), chars = body.chars().count(), "sandbox SMS logged, not sent");
        Ok(Some(Queued::Logged))
    } else {
        Ok(Some(Queued::Queued))
    }
}

const MAX_ATTEMPTS: i32 = 6;

/// The worker's pass over one tenant: post what is due to the hook. Returns how many were delivered. Does nothing
/// unless the mode is `hook`. Also blanks the numbers and text of old finished rows.
pub async fn drain_tenant(
    s: &AppState,
    client: &reqwest::Client,
    tenant_id: &str,
) -> Result<usize, sqlx::Error> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    sqlx::query("UPDATE sms_queue SET to_phone = '', body = '' WHERE status IN ('logged', 'dead') AND created_at < now() - interval '30 days' AND to_phone <> ''")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let crate::sms::Mode::Hook { url, secret } = &s.sms else {
        return Ok(0);
    };
    let mut sent = 0;
    for _ in 0..20 {
        let mut tx = tenant::begin(&s.pool, tenant_id).await?;
        let row: Option<(String, String, String, i32)> = sqlx::query_as(
            "SELECT id, to_phone, body, attempts FROM sms_queue WHERE status = 'queued' AND next_attempt_at <= now()
             ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, to, body, attempts)) = row else {
            break;
        };
        let payload = json!({ "id": id, "to": to, "body": body }).to_string();
        let sig = crate::outbox::sign(secret, chrono::Utc::now().timestamp(), payload.as_bytes());
        let res = client
            .post(url)
            .header("content-type", "application/json")
            .header(crate::outbox::SIGNATURE_HEADER, sig)
            .body(payload)
            .send()
            .await;
        match res {
            Ok(r) if r.status().is_success() => {
                sqlx::query("UPDATE sms_queue SET status = 'sent', sent_at = now(), to_phone = '', body = '', last_error = NULL WHERE id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                sent += 1;
            }
            other => {
                let err = match other {
                    Ok(r) => format!("hook answered {}", r.status()),
                    Err(e) => e.without_url().to_string(),
                };
                let n = attempts + 1;
                tracing::warn!(%id, error = %err, attempts = n, "SMS hook delivery failed");
                sqlx::query("UPDATE sms_queue SET attempts = $2, last_error = $3, status = CASE WHEN $2 >= $4 THEN 'dead' ELSE 'queued' END, next_attempt_at = now() + make_interval(secs => $5::float8) WHERE id = $1")
                    .bind(&id)
                    .bind(n)
                    .bind(err.chars().take(300).collect::<String>())
                    .bind(MAX_ATTEMPTS)
                    .bind(crate::mail::backoff_secs(n) as f64)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
    }
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_read_strictly() {
        assert_eq!(
            normalise_phone("98765 43210").as_deref(),
            Some("+919876543210")
        );
        assert_eq!(
            normalise_phone("09876543210").as_deref(),
            Some("+919876543210")
        );
        assert_eq!(
            normalise_phone("+91 (98765) 43-210").as_deref(),
            Some("+919876543210")
        );
        assert_eq!(
            normalise_phone("919876543210").as_deref(),
            Some("+919876543210")
        );
        assert_eq!(
            normalise_phone("+44 20 7946 0958").as_deref(),
            Some("+442079460958")
        );
        for bad in [
            "",
            "12345",
            "5876543210",
            "98765x3210",
            "++919876543210",
            "9876+543210",
            "+0123456789",
            "1234567890123456",
            "call me",
        ] {
            assert_eq!(normalise_phone(bad), None, "{bad}");
        }
    }

    #[test]
    fn masking_keeps_four_digits() {
        assert_eq!(mask("+919876543210"), "*********3210");
        assert_eq!(mask("123"), "***");
    }
}
