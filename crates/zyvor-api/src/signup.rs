//! Self-serve signup: a public call that creates an organisation with test and live tenants and a
//! first admin console user, then an emailed link that proves the address. The test key is returned
//! once. Live use stays closed until onboarding is approved (see `onboarding`), as for any tenant.
//! There is no mailer yet: the verification link token is only written to the log, so email is not
//! actually delivered until an SMTP adapter exists.

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    admin::bootstrap_org,
    auth::mint_key,
    console_users::{Login, authenticate_in, check_password, hash_password, valid_username},
    tenant,
};

const TOKEN_HOURS: i64 = 48;

#[derive(Deserialize)]
pub struct SignupIn {
    organisation_name: String,
    email: String,
    password: String,
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    e.len() <= 254
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.chars().any(|c| c.is_whitespace())
}

fn token_hash(t: &str) -> String {
    hex::encode(Sha256::digest(t.as_bytes()))
}

/// `POST /v1/signup`
pub async fn signup(
    State(s): State<AppState>,
    Json(b): Json<SignupIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let email = b.email.trim().to_lowercase();
    let name = b.organisation_name.trim();
    if !valid_email(&email) || !valid_username(&email) {
        return Err(ApiError::invalid(
            "invalid_email",
            "enter a valid email address",
        ));
    }
    if !(2..=120).contains(&name.chars().count()) {
        return Err(ApiError::invalid(
            "invalid_organisation_name",
            "organisation_name must be 2-120 characters",
        ));
    }
    check_password(&b.password)?;
    let hash = hash_password(&b.password)?;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );

    // one organisation per email; checked before creating anything
    let taken: Option<String> = sqlx::query_scalar("SELECT id FROM signups WHERE email = $1")
        .bind(&email)
        .fetch_optional(&s.pool)
        .await?;
    if taken.is_some() {
        return Err(ApiError::conflict(
            "email_taken",
            "an account already exists for that email",
        ));
    }
    let org = bootstrap_org(&s.pool, name).await?;
    for tid in [&org.test_tenant, &org.live_tenant] {
        let mut tx = tenant::begin(&s.pool, tid).await?;
        sqlx::query("INSERT INTO console_users (id, tenant_id, username, password_hash, role, created_by) VALUES ($1, $2, $3, $4, 'admin', 'signup')")
            .bind(new_id("usr")).bind(tid).bind(&email).bind(&hash)
            .execute(&mut *tx).await?;
        tx.commit().await?;
    }
    sqlx::query("INSERT INTO signups (id, email, organisation_id, token_hash, token_expires_at) VALUES ($1, $2, $3, $4, now() + make_interval(hours => $5))")
        .bind(new_id("sgn")).bind(&email).bind(&org.organisation_id).bind(token_hash(&token)).bind(TOKEN_HOURS as i32)
        .execute(&s.pool).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("email_taken", "an account already exists for that email"),
            e => e.into(),
        })?;
    tracing::info!(%email, link_token = %token, "signup verification link (log-only mailer)");
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "organisation_id": org.organisation_id,
            "test_tenant": org.test_tenant,
            "test_key": org.test_key,
            "email": email,
            "verification": "sent",
        })),
    ))
}

#[derive(Deserialize)]
pub struct VerifyIn {
    token: String,
}

/// `POST /v1/signup/verify`
pub async fn verify(
    State(s): State<AppState>,
    Json(b): Json<VerifyIn>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT organisation_id, verified_at, token_expires_at FROM signups WHERE token_hash = $1",
    )
    .bind(token_hash(b.token.trim()))
    .fetch_optional(&s.pool)
    .await?;
    let bad = || ApiError::invalid("invalid_token", "this link is invalid or has expired");
    let (org, verified, expires) = row.ok_or_else(bad)?;
    if verified.is_none() {
        if expires < chrono::Utc::now() {
            return Err(bad());
        }
        sqlx::query("UPDATE signups SET verified_at = now() WHERE organisation_id = $1")
            .bind(&org)
            .execute(&s.pool)
            .await?;
    }
    Ok(Json(json!({ "organisation_id": org, "verified": true })))
}

const SESSION_HOURS: i32 = 12;

#[derive(Deserialize)]
pub struct LoginIn {
    email: String,
    password: String,
    otp: Option<String>,
    /// `test` (default) or `live`.
    env: Option<String>,
}

/// What a signed-in person's session key may do. The console still enforces roles on its own pages;
/// this limits what a leaked session key could do against the API directly.
fn session_scopes(role: &str) -> Vec<String> {
    let s: &[&str] = match role {
        "admin" => &["admin"],
        "finance" => &[
            "payments:read",
            "payments:write",
            "refunds:write",
            "refunds:approve",
            "recon:read",
            "recon:write",
            "invoices:write",
            "customers:write",
            "collections:write",
            "billing:write",
            "disputes:write",
            "payouts:write",
            "payouts:approve",
            "closings:approve",
            "onboarding:write",
        ],
        "cashier" => &["payments:read", "payments:write"],
        _ => &["payments:read", "recon:read"],
    };
    s.iter().map(|x| x.to_string()).collect()
}

/// `POST /v1/console/login`: public. Finds the organisation from the signup email, checks the password
/// (and second factor) and returns the person plus a session API key that expires in 12 hours.
pub async fn console_login(
    State(s): State<AppState>,
    Json(b): Json<LoginIn>,
) -> Result<Json<Value>, ApiError> {
    let email = b.email.trim().to_lowercase();
    let env = match b.env.as_deref() {
        None | Some("test") => "test",
        Some("live") => "live",
        _ => return Err(ApiError::invalid("invalid_env", "env is test or live")),
    };
    let tenant_id: Option<String> = sqlx::query_scalar(
        "SELECT t.id FROM signups g JOIN tenants t ON t.organisation_id = g.organisation_id WHERE g.email = $1 AND t.env = $2",
    )
    .bind(&email)
    .bind(env)
    .fetch_optional(&s.pool)
    .await?;
    let Some(tenant_id) = tenant_id else {
        // same time and same answer as a wrong password, so emails cannot be probed
        let _ = hash_password(&b.password);
        return Err(ApiError::unauthorized(
            "incorrect username, password or code",
        ));
    };
    let mut out = authenticate_in(
        &s,
        &tenant_id,
        Login {
            username: email,
            password: b.password,
            otp: b.otp,
        },
    )
    .await?;
    let role = out["user"]["role"].as_str().unwrap_or("viewer").to_string();
    let branch = out["user"]["branch_id"].as_str().map(str::to_string);
    let (key, prefix, hash) = mint_key(env);
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes, expires_at, branch_id) VALUES ($1, $2, 'console session', $3, $4, $5, now() + make_interval(hours => $6), $7)")
        .bind(new_id("key")).bind(&tenant_id).bind(prefix).bind(hash).bind(session_scopes(&role)).bind(SESSION_HOURS).bind(branch)
        .execute(&s.pool).await?;
    out["session_key"] = json!(key);
    out["tenant_id"] = json!(tenant_id);
    out["env"] = json!(env);
    Ok(Json(out))
}
