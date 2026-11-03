//! Outgoing email. Mail is queued in the database and sent by the worker, so a mail server that is slow
//! or down never fails a request, and a failed send is retried with a growing delay.
//!
//! Without `ZYVOR_SMTP_URL` the mailer only logs what it would have sent, links included, so a
//! developer can finish the flows. That is for local use: nothing is delivered, and the sign-in check
//! that needs a verified email is switched off with it.

use std::time::Duration;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{
        MultiPart,
        header::{ContentType, Header, HeaderName, HeaderValue},
    },
};
use sqlx::PgPool;
use zyvor_core::new_id;

use crate::AppState;

/// `List-Unsubscribe: <url>`: the button mail programs show next to automatic mail.
#[derive(Clone)]
struct ListUnsubscribe(String);

impl Header for ListUnsubscribe {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("List-Unsubscribe")
    }
    fn parse(s: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self(s.to_string()))
    }
    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), self.0.clone())
    }
}

/// `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058): lets the program unsubscribe with one request.
#[derive(Clone)]
struct ListUnsubscribePost;

impl Header for ListUnsubscribePost {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("List-Unsubscribe-Post")
    }
    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self)
    }
    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), "List-Unsubscribe=One-Click".to_string())
    }
}

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
        self.send_as(None, to, subject, body, reply_to, html, None)
            .await
    }

    /// Like `send`, with the display name the payer sees on the From line (the address stays the configured one).
    #[allow(clippy::too_many_arguments)]
    pub async fn send_as(
        &self,
        from_name: Option<&str>,
        to: &str,
        subject: &str,
        body: &str,
        reply_to: Option<&str>,
        html: Option<&str>,
        unsubscribe: Option<&str>,
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
                let msg = build_message(&from, to, subject, body, reply_to, html, unsubscribe)?;
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
    unsubscribe: Option<&str>,
) -> Result<Message, String> {
    let mut b = Message::builder()
        .from(from.parse().map_err(|e| format!("from: {e}"))?)
        .to(to.parse().map_err(|e| format!("to: {e}"))?)
        .subject(subject);
    if let Some(u) = unsubscribe {
        b = b
            .header(ListUnsubscribe(format!("<{u}>")))
            .header(ListUnsubscribePost);
    }
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
    html_from_text_branded(body, None)
}

/// How a business's emails look: its name in the card's header, an accent colour for the button and the card's top
/// edge, and a logo beside the name.
pub struct Brand {
    pub name: String,
    /// `#rrggbb`.
    pub accent: Option<String>,
    /// An `https://` address.
    pub logo_url: Option<String>,
}

/// A `#rrggbb` colour as typed: trimmed, six hex digits, lower-cased.
pub fn clean_accent(raw: &str) -> Option<String> {
    let r = raw.trim();
    (r.len() == 7 && r.starts_with('#') && r[1..].chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| r.to_ascii_lowercase())
}

/// A logo address as typed: trimmed, `https://`, plain and short.
pub fn clean_logo_url(raw: &str) -> Option<String> {
    let r = raw.trim();
    (r.len() <= 300
        && r.starts_with("https://")
        && r.len() > 8
        && !r.contains(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"')))
    .then(|| r.to_string())
}

/// White or near-black, whichever reads better on `#rrggbb`.
fn text_on(accent: &str) -> &'static str {
    let c = |i: usize| f64::from(u8::from_str_radix(&accent[i..i + 2], 16).unwrap_or(0)) / 255.0;
    let lin = |v: f64| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * lin(c(1)) + 0.7152 * lin(c(3)) + 0.0722 * lin(c(5));
    if l > 0.4 { "#1d1d1f" } else { "#ffffff" }
}

/// `html_from_text` with a business's look. With no brand it is the plain version.
pub fn html_from_text_branded(body: &str, brand: Option<&Brand>) -> String {
    let accent = brand
        .and_then(|b| b.accent.as_deref())
        .filter(|a| clean_accent(a).is_some())
        .unwrap_or("#0071e3");
    let on_accent = text_on(accent);
    let blocks: Vec<&str> = body
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .collect();
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"></head><body style=\"margin:0;padding:24px;background:#f5f5f7;font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;color:#1d1d1f\"><div style=\"max-width:560px;margin:0 auto;background:#fff;border-radius:16px;border-top:4px solid {accent};padding:28px;font-size:16px;line-height:1.5\">",
    );
    out = out.replace("{accent}", accent);
    if let Some(b) = brand {
        let logo = b
            .logo_url
            .as_deref()
            .and_then(clean_logo_url)
            .map(|u| {
                format!(
                    "<img src=\"{}\" alt=\"\" height=\"32\" style=\"border:0;vertical-align:middle;margin-right:10px\">",
                    esc(&u)
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<div style=\"margin:0 0 20px\">{logo}<strong style=\"font-size:15px;vertical-align:middle\">{}</strong></div>",
            esc(&b.name)
        ));
    }
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
                "<p style=\"margin:0 0 16px\"><a href=\"{u}\" style=\"display:inline-block;background:{accent};color:{on_accent};text-decoration:none;padding:12px 22px;border-radius:980px;font-weight:600\">Pay now</a></p><p style=\"margin:0 0 16px;font-size:13px;color:#6e6e73;word-break:break-all\">{u}</p>"
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
    enqueue_once_as(conn, tenant_id, dedupe_key, to, subject, body, false).await
}

/// Like `enqueue_once`. `automatic` mail (raised by the system, not sent by a person or in answer to a payment) is
/// skipped for an address that unsubscribed and carries an unsubscribe link when one is configured.
pub async fn enqueue_once_as(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    dedupe_key: &str,
    to: &str,
    subject: &str,
    body: &str,
    automatic: bool,
) -> Result<bool, sqlx::Error> {
    let (reply_to, accent, logo_url): (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT reply_to_email, email_accent, email_logo_url FROM tenant_settings WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or((None, None, None));
    let business: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&mut *conn)
        .await?;
    let from_name = business.as_deref().and_then(sender_name);
    let brand = business.map(|name| Brand {
        name,
        accent,
        logo_url,
    });
    let n = sqlx::query("INSERT INTO mail_queue (id, to_email, subject, body, dedupe_key, reply_to, html, from_name, tenant_id, automatic) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (dedupe_key) DO NOTHING")
        .bind(new_id("mail"))
        .bind(to)
        .bind(subject)
        .bind(body)
        .bind(dedupe_key)
        .bind(reply_to)
        .bind(html_from_text_branded(body, brand.as_ref()))
        .bind(from_name)
        .bind(tenant_id)
        .bind(automatic)
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

/// The words of an invoice sent to its customer: what it is made of (when it says), what is still owed and by when.
/// With a checkout address it carries a pay link; without one it says how to pay in words the business can answer.
#[allow(clippy::too_many_arguments)]
pub fn invoice_notice(
    merchant: &str,
    payer_name: &str,
    number: &str,
    lines: &[String],
    total: &str,
    settled: Option<&str>,
    outstanding: &str,
    due: Option<chrono::NaiveDate>,
    pay_url: Option<&str>,
) -> (String, String) {
    let subject = match due {
        Some(d) => format!("Invoice {number} from {merchant}: {outstanding} due {d}"),
        None => format!("Invoice {number} from {merchant}: {outstanding}"),
    };
    let mut out = vec![
        format!("Hello {payer_name},"),
        String::new(),
        format!("{merchant} has sent you invoice {number}."),
        String::new(),
    ];
    out.extend(lines.iter().map(|l| format!("- {l}")));
    if !lines.is_empty() {
        out.push(String::new());
    }
    out.push(format!("Total: {total}"));
    if let Some(s) = settled {
        out.push(format!("Already paid or credited: {s}"));
    }
    out.push(format!("Amount due: {outstanding}"));
    if let Some(d) = due {
        out.push(format!("Due by: {d}"));
    }
    out.push(String::new());
    match pay_url {
        Some(u) => out.push(format!("Pay here: {u}")),
        None => out.push(format!(
            "To pay, use the payment details {merchant} has given you, or reply to this email."
        )),
    }
    out.push(String::new());
    out.push("Only pay to details you already know belong to this business. Zyvor never asks for a card number, PIN or password by email.".into());
    (subject, out.join("\n"))
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

/// The HTML email with a "stop these emails" line before the end of its card.
fn with_unsubscribe_footer(html: &str, url: &str) -> String {
    let line = format!(
        "<p style=\"margin:16px 0 0;font-size:12px;color:#6e6e73\"><a href=\"{}\" style=\"color:#6e6e73\">Stop these emails</a></p>",
        esc(url)
    );
    match html.rfind("</div></body></html>") {
        Some(i) => format!("{}{line}{}", &html[..i], &html[i..]),
        None => html.to_string(),
    }
}

pub async fn drain(s: &AppState) -> Result<usize, sqlx::Error> {
    let mut sent = 0;
    for _ in 0..10 {
        let mut tx = s.pool.begin().await?;
        #[allow(clippy::type_complexity)]
        let row: Option<(String, String, String, String, i32, Option<String>, Option<String>, Option<String>, Option<String>, bool)> = sqlx::query_as(
            "SELECT id, to_email, subject, body, attempts, reply_to, html, from_name, tenant_id, automatic FROM mail_queue WHERE status = 'pending' AND next_attempt_at <= now()
             ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some((
            id,
            to,
            subject,
            mut body,
            attempts,
            reply_to,
            mut html,
            from_name,
            tenant_id,
            automatic,
        )) = row
        else {
            break;
        };
        // automatic mail: skipped for an address that unsubscribed, and given the unsubscribe link and headers
        let mut unsubscribe = None;
        if automatic && let Some(tid) = &tenant_id {
            sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
                .bind(tid)
                .execute(&mut *tx)
                .await?;
            if crate::unsubscribe::is_suppressed(&mut tx, tid, &to).await? {
                sqlx::query("UPDATE mail_queue SET status = 'suppressed', body = '', html = NULL WHERE id = $1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                continue;
            }
            if let Some(cfg) = &s.unsubscribe {
                let url = cfg.url(tid, &to.to_lowercase());
                body.push_str(&format!("\n\nTo stop these emails: {url}"));
                html = html.map(|h| with_unsubscribe_footer(&h, &url));
                unsubscribe = Some(url);
            }
        }
        match s
            .mailer
            .send_as(
                from_name.as_deref(),
                &to,
                &subject,
                &body,
                reply_to.as_deref(),
                html.as_deref(),
                unsubscribe.as_deref(),
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
    fn a_business_look_is_checked_and_the_button_stays_readable() {
        assert_eq!(clean_accent(" #0B7285 ").as_deref(), Some("#0b7285"));
        for bad in [
            "0b7285",
            "#0b728",
            "#0b72855",
            "#gggggg",
            "red",
            "#0b7285;x",
            "",
        ] {
            assert_eq!(clean_accent(bad), None, "{bad}");
        }
        assert_eq!(
            clean_logo_url(" https://acme.example/logo.png ").as_deref(),
            Some("https://acme.example/logo.png")
        );
        for bad in [
            "http://acme.example/l.png",
            "https://",
            "https://a b",
            "https://a\"onload=x",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(clean_logo_url(bad), None, "{bad}");
        }
        assert_eq!(text_on("#ffffff"), "#1d1d1f");
        assert_eq!(text_on("#0b7285"), "#ffffff");
        assert_eq!(text_on("#ffe066"), "#1d1d1f");
        let brand = Brand {
            name: "Acme <Stores> & Co".into(),
            accent: Some("#0b7285".into()),
            logo_url: Some("https://acme.example/logo.png".into()),
        };
        let h = html_from_text_branded(
            "Hi,\n\nPay here: https://pay.example.com/x\n\nSmall print.",
            Some(&brand),
        );
        assert!(h.contains("border-top:4px solid #0b7285"), "{h}");
        assert!(h.contains("background:#0b7285;color:#ffffff"), "{h}");
        assert!(
            h.contains("<img src=\"https://acme.example/logo.png\""),
            "{h}"
        );
        assert!(h.contains("Acme &lt;Stores&gt; &amp; Co"), "{h}");
        assert!(!h.contains("Acme <Stores>"), "{h}");
        // a bad colour or logo in a stored brand is ignored, never written into the page
        let odd = Brand {
            name: "Acme".into(),
            accent: Some("red;}</style>".into()),
            logo_url: Some("javascript:alert(1)".into()),
        };
        let h = html_from_text_branded("Hi,\n\nBye.", Some(&odd));
        assert!(
            h.contains("border-top:4px solid #0071e3")
                && !h.contains("<img")
                && !h.contains("javascript"),
            "{h}"
        );
        // no brand: the plain look, no header
        let plain = html_from_text("Hi,\n\nBye.");
        assert!(
            !plain.contains("<strong") && plain.contains("#0071e3"),
            "{plain}"
        );
    }

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
