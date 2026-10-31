//! Outgoing email. Mail is queued in the database and sent by the worker, so a mail server that is slow
//! or down never fails a request, and a failed send is retried with a growing delay.
//!
//! Without `ZYVOR_SMTP_URL` the mailer only logs what it would have sent, links included, so a
//! developer can finish the flows. That is for local use: nothing is delivered, and the sign-in check
//! that needs a verified email is switched off with it.

use std::time::Duration;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::header::ContentType,
};
use sqlx::PgPool;
use zyvor_core::new_id;

use crate::AppState;

#[derive(Clone)]
pub enum Mailer {
    /// Logs the message instead of sending it.
    Log,
    /// Accepts and drops every message, and counts as "delivering". For tests of what depends on that.
    Null,
    Smtp(Box<Smtp>),
}

#[derive(Clone)]
pub struct Smtp {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl Mailer {
    /// `ZYVOR_SMTP_URL` (`smtps://user:pass@host:465` or `smtp://user:pass@host:587?tls=required`) and
    /// `ZYVOR_MAIL_FROM` (`Zyvor <no-reply@example.com>`). Neither set: the log-only mailer.
    pub fn from_env() -> Result<Self, String> {
        let Ok(url) = std::env::var("ZYVOR_SMTP_URL") else {
            return Ok(Mailer::Log);
        };
        let from = std::env::var("ZYVOR_MAIL_FROM")
            .map_err(|_| "ZYVOR_MAIL_FROM is required with ZYVOR_SMTP_URL".to_string())?;
        Self::smtp(&url, &from)
    }

    pub fn smtp(url: &str, from: &str) -> Result<Self, String> {
        from.parse::<lettre::message::Mailbox>()
            .map_err(|e| format!("ZYVOR_MAIL_FROM is not a valid address: {e}"))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|e| format!("ZYVOR_SMTP_URL is not valid: {e}"))?
            .timeout(Some(Duration::from_secs(15)))
            .build();
        Ok(Mailer::Smtp(Box::new(Smtp {
            transport,
            from: from.to_string(),
        })))
    }

    /// Whether messages really reach people. Checks that depend on someone reading their email are only
    /// enforced when this is true.
    pub fn delivers(&self) -> bool {
        !matches!(self, Mailer::Log)
    }

    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), String> {
        match self {
            Mailer::Log => {
                tracing::warn!(%to, %subject, "email NOT sent (log-only mailer, set ZYVOR_SMTP_URL):\n{body}");
                Ok(())
            }
            Mailer::Null => Ok(()),
            Mailer::Smtp(s) => {
                let msg = Message::builder()
                    .from(s.from.parse().map_err(|e| format!("from: {e}"))?)
                    .to(to.parse().map_err(|e| format!("to: {e}"))?)
                    .subject(subject)
                    .header(ContentType::TEXT_PLAIN)
                    .body(body.to_string())
                    .map_err(|e| e.to_string())?;
                s.transport
                    .send(msg)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }
    }
}

/// Queue a message. Cheap and local; the worker sends it.
pub async fn enqueue(
    pool: &PgPool,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO mail_queue (id, to_email, subject, body) VALUES ($1, $2, $3, $4)")
        .bind(new_id("mail"))
        .bind(to)
        .bind(subject)
        .bind(body)
        .execute(pool)
        .await?;
    Ok(())
}

/// Seconds to wait after the nth failed send: 30s, 1m, 2m ... capped at an hour.
pub fn backoff_secs(attempts: i32) -> i64 {
    (30i64 << attempts.clamp(0, 7).min(7)).min(3600)
}

const MAX_ATTEMPTS: i32 = 8;

/// Send what is due. Returns how many were sent. A message that keeps failing is marked dead after
/// eight tries, and the failure stays in `last_error`.
pub async fn drain(s: &AppState) -> Result<usize, sqlx::Error> {
    let mut sent = 0;
    for _ in 0..10 {
        let mut tx = s.pool.begin().await?;
        let row: Option<(String, String, String, String, i32)> = sqlx::query_as(
            "SELECT id, to_email, subject, body, attempts FROM mail_queue WHERE status = 'pending' AND next_attempt_at <= now()
             ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, to, subject, body, attempts)) = row else {
            break;
        };
        match s.mailer.send(&to, &subject, &body).await {
            Ok(()) => {
                sqlx::query("UPDATE mail_queue SET status = 'sent', sent_at = now(), body = '', last_error = NULL WHERE id = $1")
                    .bind(&id).execute(&mut *tx).await?;
                sent += 1;
            }
            Err(e) => {
                let n = attempts + 1;
                tracing::warn!(%id, error = %e, attempts = n, "email send failed");
                sqlx::query("UPDATE mail_queue SET attempts = $2, last_error = $3, status = CASE WHEN $2 >= $4 THEN 'dead' ELSE 'pending' END, next_attempt_at = now() + make_interval(secs => $5::float8) WHERE id = $1")
                    .bind(&id).bind(n).bind(e.chars().take(300).collect::<String>()).bind(MAX_ATTEMPTS).bind(backoff_secs(n) as f64)
                    .execute(&mut *tx).await?;
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
    fn retries_back_off_and_cap() {
        assert_eq!(backoff_secs(0), 30);
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(3), 240);
        assert_eq!(backoff_secs(7), 3600);
        assert_eq!(backoff_secs(50), 3600);
    }

    #[test]
    fn only_a_real_mailer_counts_as_delivering() {
        assert!(!Mailer::Log.delivers());
        assert!(Mailer::Null.delivers());
    }
}
