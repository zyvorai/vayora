//! Letting a customer stop the automatic emails a business sends them (invoices raised by billing, payment
//! reminders). Such mail carries an unsubscribe link and the `List-Unsubscribe` headers mail programs show as a
//! button; following it adds the address to the business's suppression list, and automatic mail to a suppressed
//! address is skipped. Mail a person sends by hand, and receipts, are not affected.
//!
//! The link is stateless: `/unsubscribe/<token>` where the token is the tenant and address signed with
//! `ZYVOR_UNSUBSCRIBE_SECRET`. It needs `ZYVOR_PUBLIC_API_URL` (where payers reach this API) as well; without both
//! no link is added and nothing else changes.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Clone)]
pub struct Config {
    /// Where payers reach this API, without a trailing slash.
    pub base: String,
    pub secret: Vec<u8>,
}

fn mac(secret: &[u8], tenant: &str, email: &str) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    m.update(b"unsubscribe-v1\n");
    m.update(tenant.as_bytes());
    m.update(b"\n");
    m.update(email.as_bytes());
    m
}

impl Config {
    /// The address that stops a business's automatic mail to `email`.
    pub fn url(&self, tenant: &str, email: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(format!("{tenant}\n{email}"));
        let sig = hex::encode(mac(&self.secret, tenant, email).finalize().into_bytes());
        format!("{}/unsubscribe/{payload}.{sig}", self.base)
    }

    /// The tenant and address a token was made for, when its signature is right.
    pub fn verify(&self, token: &str) -> Option<(String, String)> {
        let (payload, sig) = token.split_once('.')?;
        let text = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
        let (tenant, email) = text.split_once('\n')?;
        let sig = hex::decode(sig).ok()?;
        mac(&self.secret, tenant, email).verify_slice(&sig).ok()?;
        Some((tenant.to_string(), email.to_string()))
    }
}

pub async fn is_suppressed(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    email: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM mail_suppressions WHERE tenant_id = $1 AND lower(email) = lower($2))",
    )
    .bind(tenant_id)
    .bind(email)
    .fetch_one(conn)
    .await
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><meta name=\"robots\" content=\"noindex\"><title>{t}</title></head><body style=\"margin:0;padding:24px;background:#f5f5f7;font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;color:#1d1d1f\"><main style=\"max-width:480px;margin:40px auto;background:#fff;border-radius:16px;padding:28px;line-height:1.5\"><h1 style=\"font-size:20px;margin:0 0 12px\">{t}</h1>{body}</main></body></html>",
        t = esc(title)
    ))
}

async fn merchant(s: &AppState, tenant_id: &str) -> String {
    sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(tenant_id)
        .fetch_optional(&s.pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "this business".into())
}

fn invalid() -> (StatusCode, Html<String>) {
    (
        StatusCode::NOT_FOUND,
        page(
            "This link is not valid",
            "<p>It may have been copied wrongly. Use the link in the email you received.</p>",
        ),
    )
}

/// `GET /unsubscribe/{token}`: asks first, so a mail scanner opening the link does not unsubscribe anyone.
pub async fn confirm_page(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> (StatusCode, Html<String>) {
    let Some(cfg) = &s.unsubscribe else {
        return invalid();
    };
    let Some((tenant_id, email)) = cfg.verify(&token) else {
        return invalid();
    };
    let name = merchant(&s, &tenant_id).await;
    (
        StatusCode::OK,
        page(
            &format!("Stop emails from {name}?"),
            &format!(
                "<p>You will no longer get automatic emails from {n} at {e}: invoices and payment reminders. Receipts for payments you make still come, and {n} can still send you something by hand.</p><form method=\"post\"><button style=\"background:#0071e3;color:#fff;border:0;padding:12px 22px;border-radius:980px;font-size:16px;font-weight:600;cursor:pointer\">Stop these emails</button></form>",
                n = esc(&name),
                e = esc(&email)
            ),
        ),
    )
}

/// `POST /unsubscribe/{token}`: the button on the page, and the one-click request a mail program sends for
/// the `List-Unsubscribe-Post` header. Idempotent.
pub async fn unsubscribe(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> Result<(StatusCode, Html<String>), ApiError> {
    let Some(cfg) = &s.unsubscribe else {
        return Ok(invalid());
    };
    let Some((tenant_id, email)) = cfg.verify(&token) else {
        return Ok(invalid());
    };
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    sqlx::query(
        "INSERT INTO mail_suppressions (tenant_id, email) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(&tenant_id)
    .bind(email.to_lowercase())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let name = merchant(&s, &tenant_id).await;
    Ok((
        StatusCode::OK,
        page(
            "You are unsubscribed",
            &format!(
                "<p>{n} will no longer send you automatic emails at {e}.</p>",
                n = esc(&name),
                e = esc(&email)
            ),
        ),
    ))
}

/// `GET /v1/mail_suppressions`: the addresses that asked to stop automatic mail.
pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT email, source, created_at FROM mail_suppressions ORDER BY created_at DESC LIMIT 500",
    )
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({
        "data": rows.into_iter().map(|(email, source, at)| json!({ "email": email, "source": source, "created_at": at })).collect::<Vec<_>>()
    })))
}

/// `DELETE /v1/mail_suppressions/{email}`: start sending automatic mail to the address again (for example after
/// it was unsubscribed by mistake).
pub async fn remove(
    State(s): State<AppState>,
    auth: Auth,
    Path(email): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("DELETE FROM mail_suppressions WHERE lower(email) = lower($1)")
        .bind(&email)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("suppressed address"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "mail_suppression.removed",
        None,
        json!({ "email": email }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "email": email, "removed": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            base: "https://api.example.com".into(),
            secret: b"a-long-enough-test-secret".to_vec(),
        }
    }

    #[test]
    fn a_link_names_its_tenant_and_address_and_cannot_be_forged() {
        let c = cfg();
        let url = c.url("ten_1", "asha@example.com");
        let token = url
            .strip_prefix("https://api.example.com/unsubscribe/")
            .unwrap();
        assert_eq!(
            c.verify(token),
            Some(("ten_1".into(), "asha@example.com".into()))
        );
        // a different secret, a changed signature and a swapped payload are all refused
        let other = Config {
            secret: b"another-long-enough-secret".to_vec(),
            ..cfg()
        };
        assert_eq!(other.verify(token), None);
        let (payload, sig) = token.split_once('.').unwrap();
        assert_eq!(
            c.verify(&format!("{payload}.{}", "0".repeat(sig.len()))),
            None
        );
        let forged = URL_SAFE_NO_PAD.encode("ten_2\nasha@example.com");
        assert_eq!(c.verify(&format!("{forged}.{sig}")), None);
        assert_eq!(c.verify("garbage"), None);
        assert_eq!(c.verify(""), None);
    }
}
