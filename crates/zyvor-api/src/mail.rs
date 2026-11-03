//! Outgoing email. Mail is queued in the database and sent by the worker, so a mail server that is slow
//! or down never fails a request, and a failed send is retried with a growing delay.
//!
//! Without `ZYVOR_SMTP_URL` the mailer only logs what it would have sent, links included, so a
//! developer can finish the flows. That is for local use: nothing is delivered, and the sign-in check
//! that needs a verified email is switched off with it.

use std::time::Duration;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{MultiPart, header::ContentType},
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

    pub async fn send(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        reply_to: Option<&str>,
        html: Option<&str>,
    ) -> Result<(), String> {
        self.send_as(None, to, subject, body, reply_to, html).await
    }

    /// Like `send`, with the display name the payer sees on the From line (the address stays the configured one).
    pub async fn send_as(
        &self,
        from_name: Option<&str>,
        to: &str,
        subject: &str,
        body: &str,
        reply_to: Option<&str>,
        html: Option<&str>,
    ) -> Result<(), String> {
        match self {
            Mailer::Log => {
                tracing::warn!(%to, %subject, ?reply_to, "email NOT sent (log-only mailer, set ZYVOR_SMTP_URL):\n{body}");
                Ok(())
            }
            Mailer::Null => Ok(()),
            Mailer::Smtp(s) => {
                let from = match from_name {
                    Some(n) => with_display_name(&s.from, n)?,
                    None => s.from.clone(),
                };
                let msg = build_message(&from, to, subject, body, reply_to, html)?;
                s.transport
                    .send(msg)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }
    }
}

/// The message as the SMTP mailer sends it. A reply-to that does not parse is an error, so a bad address is never
/// silently dropped.
fn build_message(
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
    reply_to: Option<&str>,
    html: Option<&str>,
) -> Result<Message, String> {
    let mut b = Message::builder()
        .from(from.parse().map_err(|e| format!("from: {e}"))?)
        .to(to.parse().map_err(|e| format!("to: {e}"))?)
        .subject(subject);
    if let Some(r) = reply_to {
        b = b.reply_to(r.parse().map_err(|e| format!("reply-to: {e}"))?);
    }
    match html {
        Some(h) => b.multipart(MultiPart::alternative_plain_html(
            body.to_string(),
            h.to_string(),
        )),
        None => b.header(ContentType::TEXT_PLAIN).body(body.to_string()),
    }
    .map_err(|e| e.to_string())
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// An HTML version of a payer-facing plain-text email, made from the text itself so the wording lives in one
/// place. Paragraphs follow the blank lines; a `Pay here: <https url>` line becomes a button (any other kind of
/// address stays text); the last paragraph is the small print. Everything is escaped, and there is no image,
/// script or tracking pixel.
pub fn html_from_text(body: &str) -> String {
    let blocks: Vec<&str> = body
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .collect();
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"></head><body style=\"margin:0;padding:24px;background:#f5f5f7;font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;color:#1d1d1f\"><div style=\"max-width:560px;margin:0 auto;background:#fff;border-radius:16px;padding:28px;font-size:16px;line-height:1.5\">",
    );
    for (i, block) in blocks.iter().enumerate() {
        let small = i + 1 == blocks.len() && blocks.len() > 1;
        let style = if small {
            "margin:24px 0 0;font-size:13px;color:#6e6e73"
        } else {
            "margin:0 0 16px"
        };
        let mut lines = Vec::new();
        let mut button = None;
        for line in block.lines() {
            match line.strip_prefix("Pay here: ") {
                Some(u)
                    if (u.starts_with("https://") || u.starts_with("http://"))
                        && !u.contains(char::is_whitespace) =>
                {
                    button = Some(u.to_string());
                }
                _ => lines.push(esc(line)),
            }
        }
        if !lines.is_empty() {
            out.push_str(&format!("<p style=\"{style}\">{}</p>", lines.join("<br>")));
        }
        if let Some(u) = button {
            let u = esc(&u);
            out.push_str(&format!(
                "<p style=\"margin:0 0 16px\"><a href=\"{u}\" style=\"display:inline-block;background:#0071e3;color:#fff;text-decoration:none;padding:12px 22px;border-radius:980px;font-weight:600\">Pay now</a></p><p style=\"margin:0 0 16px;font-size:13px;color:#6e6e73;word-break:break-all\">{u}</p>"
            ));
        }
    }
    out.push_str("</div></body></html>");
    out
}

/// The From name for mail sent for a business: "Acme Stores via Zyvor". The business's own name alone is never
/// used, so a business cannot present itself as someone else (a bank, say) from Zyvor's address. Characters that
/// could break a header or confuse an address are dropped, and a long name is cut.
pub fn sender_name(business: &str) -> Option<String> {
    let clean: String = business
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | '"' | '@' | ',' | ';' | ':' | '\\'))
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.chars().take(60).collect();
    let clean = clean.trim();
    (!clean.is_empty()).then(|| format!("{clean} via Zyvor"))
}

/// `from` (as configured, with or without a name) with its display name replaced by `name`.
fn with_display_name(from: &str, name: &str) -> Result<String, String> {
    let m: lettre::message::Mailbox = from.parse().map_err(|e| format!("from: {e}"))?;
    Ok(lettre::message::Mailbox::new(Some(name.to_string()), m.email).to_string())
}

/// A business's reply-to address as typed: trimmed, and it must be one plain address (no display name).
pub fn clean_reply_to(raw: &str) -> Option<String> {
    let r = raw.trim();
    (r.len() <= 200 && r.parse::<lettre::Address>().is_ok()).then(|| r.to_string())
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

/// Queue a message inside the caller's transaction, once per `dedupe_key`: a repeat with the same key
/// queues nothing. Returns whether a message was queued.
pub async fn enqueue_once(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    dedupe_key: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<bool, sqlx::Error> {
    let reply_to: Option<String> =
        sqlx::query_scalar("SELECT reply_to_email FROM tenant_settings WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let from_name: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?
        .and_then(|n: String| sender_name(&n));
    let n = sqlx::query("INSERT INTO mail_queue (id, to_email, subject, body, dedupe_key, reply_to, html, from_name) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (dedupe_key) DO NOTHING")
        .bind(new_id("mail"))
        .bind(to)
        .bind(subject)
        .bind(body)
        .bind(dedupe_key)
        .bind(reply_to)
        .bind(html_from_text(body))
        .bind(from_name)
        .execute(conn)
        .await?
        .rows_affected();
    Ok(n == 1)
}

/// The words of a payment receipt. Plain text, only facts the payer already knows or the merchant recorded.
pub fn receipt(
    merchant: &str,
    payer_name: &str,
    amount: &str,
    paid_on: chrono::NaiveDate,
    payment_id: &str,
    reference: Option<&str>,
    invoice_number: Option<&str>,
) -> (String, String) {
    let subject = format!("Receipt: {amount} paid to {merchant}");
    let mut lines = vec![
        format!("Hello {payer_name},"),
        String::new(),
        format!("{merchant} received your payment of {amount} on {paid_on}."),
    ];
    if let Some(n) = invoice_number {
        lines.push(format!("Invoice: {n}"));
    }
    if let Some(r) = reference {
        lines.push(format!("Reference: {r}"));
    }
    lines.push(format!("Payment id: {payment_id}"));
    lines.push(String::new());
    lines.push("Keep this email as your receipt. If anything looks wrong, reply to the business that sent your bill.".into());
    (subject, lines.join("\n"))
}

/// The words of a refund receipt. The reason is the merchant's own note, so it is left out: it may not be meant
/// for the payer.
pub fn refund_receipt(
    merchant: &str,
    payer_name: &str,
    amount: &str,
    refunded_on: chrono::NaiveDate,
    refund_id: &str,
    payment_id: &str,
    reference: Option<&str>,
) -> (String, String) {
    let subject = format!("Refund: {amount} from {merchant}");
    let mut lines = vec![
        format!("Hello {payer_name},"),
        String::new(),
        format!("{merchant} refunded {amount} on {refunded_on}."),
    ];
    if let Some(r) = reference {
        lines.push(format!("Reference: {r}"));
    }
    lines.push(format!("Refund id: {refund_id}"));
    lines.push(format!("Original payment id: {payment_id}"));
    lines.push(String::new());
    lines.push("The money goes back the way you paid. Your bank can take a few days to show it. If anything looks wrong, reply to the business that sent your bill.".into());
    (subject, lines.join("\n"))
}

/// The words of an email that sends a payment link. `amount` is None for a link whose amount the payer types or
/// that follows an invoice.
pub fn payment_link(
    merchant: &str,
    payer_name: &str,
    description: Option<&str>,
    amount: Option<&str>,
    url: &str,
    expires_on: Option<chrono::NaiveDate>,
) -> (String, String) {
    let subject = match amount {
        Some(a) => format!("{merchant} asks you to pay {a}"),
        None => format!("{merchant} sent you a payment link"),
    };
    let mut lines = vec![format!("Hello {payer_name},"), String::new()];
    lines.push(match amount {
        Some(a) => format!("{merchant} has asked you to pay {a}."),
        None => format!("{merchant} has sent you a link to pay."),
    });
    if let Some(d) = description {
        lines.push(format!("For: {d}"));
    }
    lines.push(String::new());
    lines.push(format!("Pay here: {url}"));
    if let Some(e) = expires_on {
        lines.push(format!("The link works until {e}."));
    }
    lines.push(String::new());
    lines.push("Only pay through the link above if you were expecting it from this business. Zyvor never asks for a card number, PIN or password by email.".into());
    (subject, lines.join("\n"))
}

/// The words of the one reminder a payment link sends before it expires.
pub fn payment_link_expiring(
    merchant: &str,
    payer_name: &str,
    description: Option<&str>,
    amount: Option<&str>,
    url: &str,
    expires_on: chrono::NaiveDate,
) -> (String, String) {
    let subject = format!("Reminder: your payment link to {merchant} expires on {expires_on}");
    let mut lines = vec![format!("Hello {payer_name},"), String::new()];
    lines.push(match amount {
        Some(a) => format!("{merchant} is still waiting for your payment of {a}."),
        None => format!("{merchant} is still waiting for your payment."),
    });
    if let Some(d) = description {
        lines.push(format!("For: {d}"));
    }
    lines.push(String::new());
    lines.push(format!("Pay here: {url}"));
    lines.push(format!(
        "The link stops working after {expires_on}. This is the only reminder we will send."
    ));
    lines.push(String::new());
    lines.push("Ignore this email if you have already paid. Only pay through the link above if you were expecting it from this business.".into());
    (subject, lines.join("\n"))
}

/// The words of a payment reminder email. `wording` is what a person wrote or approved (or the standard text).
pub fn reminder(
    merchant: &str,
    payer_name: &str,
    wording: &str,
    invoice_number: &str,
    outstanding: &str,
) -> (String, String) {
    let subject = format!("Payment reminder: invoice {invoice_number} from {merchant}");
    let mut lines = Vec::new();
    if !wording.trim_start().to_lowercase().starts_with("hello") {
        lines.push(format!("Hello {payer_name},"));
        lines.push(String::new());
    }
    lines.push(wording.trim().to_string());
    lines.push(String::new());
    lines.push(format!("Invoice: {invoice_number}"));
    lines.push(format!("Amount due: {outstanding}"));
    lines.push(String::new());
    lines.push("If you have already paid, please ignore this email. If anything looks wrong, reply to the business that sent your bill.".into());
    (subject, lines.join("\n"))
}

/// Seconds to wait after the nth failed send: 30s, 1m, 2m ... capped at an hour.
pub fn backoff_secs(attempts: i32) -> i64 {
    (30i64 << attempts.clamp(0, 7).min(7)).min(3600)
}

const MAX_ATTEMPTS: i32 = 8;

/// Send what is due. Returns how many were sent. A message that keeps failing is marked dead after
/// eight tries, and the failure stays in `last_error`.
/// Messages waiting to go out, and messages given up on (`dead`), for the metrics page. -1 when the count fails.
pub async fn counts(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as("SELECT COUNT(*) FILTER (WHERE status = 'pending'), COUNT(*) FILTER (WHERE status = 'dead') FROM mail_queue")
        .fetch_one(pool)
        .await
        .unwrap_or((-1, -1))
}

pub async fn drain(s: &AppState) -> Result<usize, sqlx::Error> {
    let mut sent = 0;
    for _ in 0..10 {
        let mut tx = s.pool.begin().await?;
        let row: Option<(String, String, String, String, i32, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, to_email, subject, body, attempts, reply_to, html, from_name FROM mail_queue WHERE status = 'pending' AND next_attempt_at <= now()
             ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((id, to, subject, body, attempts, reply_to, html, from_name)) = row else {
            break;
        };
        match s
            .mailer
            .send_as(
                from_name.as_deref(),
                &to,
                &subject,
                &body,
                reply_to.as_deref(),
                html.as_deref(),
            )
            .await
        {
            Ok(()) => {
                sqlx::query("UPDATE mail_queue SET status = 'sent', sent_at = now(), body = '', html = NULL, last_error = NULL WHERE id = $1")
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
    fn the_from_name_says_who_it_is_for_and_cannot_pose_as_someone_else() {
        assert_eq!(
            sender_name("Acme Stores").as_deref(),
            Some("Acme Stores via Zyvor")
        );
        // characters that could break a header or look like an address are dropped; space is tidied
        assert_eq!(
            sender_name("  Acme\r\nBcc: x@evil.example <boss@bank.example>  ").as_deref(),
            Some("AcmeBcc xevil.example bossbank.example via Zyvor")
        );
        assert_eq!(sender_name(" <>@ ").as_deref(), None);
        assert_eq!(sender_name("").as_deref(), None);
        assert!(sender_name(&"x".repeat(500)).unwrap().len() <= 70);
        let from =
            with_display_name("Zyvor <no-reply@example.com>", "Acme Stores via Zyvor").unwrap();
        assert_eq!(from, "Acme Stores via Zyvor <no-reply@example.com>");
        let bare = with_display_name("no-reply@example.com", "Acme via Zyvor").unwrap();
        assert!(bare.ends_with("<no-reply@example.com>"), "{bare}");
    }

    #[test]
    fn the_html_version_escapes_everything_and_only_buttons_a_web_address() {
        let h = html_from_text(
            "Hello <b>Asha</b> & co,\n\nPay here: https://pay.example.com/pay/link/t/x?a=1&b=2\n\nSmall print.",
        );
        assert!(h.contains("Hello &lt;b&gt;Asha&lt;/b&gt; &amp; co,"), "{h}");
        assert!(
            h.contains(">Pay now</a>")
                && h.contains("href=\"https://pay.example.com/pay/link/t/x?a=1&amp;b=2\""),
            "{h}"
        );
        assert!(
            h.contains("font-size:13px") && h.contains("Small print."),
            "{h}"
        );
        assert!(
            !h.contains("<b>") && !h.contains("<script") && !h.contains("<img"),
            "{h}"
        );
        assert!(h.contains("<meta charset=\"utf-8\">"), "{h}");
        // not a web address: stays visible text, no link
        let t = html_from_text("Hi,\n\nPay here: javascript:alert(1)\n\nBye.");
        assert!(
            !t.contains("href=") && t.contains("Pay here: javascript:alert(1)"),
            "{t}"
        );
    }

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
