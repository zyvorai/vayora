//! Self-serve signup: a public call that creates an organisation with test and live tenants and a
//! first admin console user, then an emailed link that proves the address. The test key is returned
//! once. Live use stays closed until onboarding is approved (see `onboarding`), as for any tenant.
//! The verification link is emailed through the queue in `mail`; with no SMTP configured it is only logged.

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

pub(crate) fn valid_email(e: &str) -> bool {
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

pub(crate) fn token_hash(t: &str) -> String {
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

    // One organisation per email. The lock serialises two signups for the same address, and the check covers
    // addresses that already belong to an organisation through an invitation or the People page, so nothing
    // is created (and no orphan left) when the address is taken.
    let mut guard = s.pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(&email)
        .execute(&mut *guard)
        .await?;
    let taken: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM signups WHERE email = $1 UNION ALL SELECT 1 FROM login_directory WHERE email = $1 LIMIT 1",
    )
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
    // identity lives in the test tenant; live is reached through the environment switch
    {
        let tid = &org.test_tenant;
        let mut tx = tenant::begin(&s.pool, tid).await?;
        sqlx::query("INSERT INTO console_users (id, tenant_id, username, password_hash, role, created_by) VALUES ($1, $2, $3, $4, 'admin', 'signup')")
            .bind(new_id("usr")).bind(tid).bind(&email).bind(&hash)
            .execute(&mut *tx).await?;
        tx.commit().await?;
    }
    sqlx::query("INSERT INTO login_directory (email, organisation_id) VALUES ($1, $2)")
        .bind(&email)
        .bind(&org.organisation_id)
        .execute(&s.pool)
        .await?;
    sqlx::query("INSERT INTO signups (id, email, organisation_id, token_hash, token_expires_at) VALUES ($1, $2, $3, $4, now() + make_interval(hours => $5))")
        .bind(new_id("sgn")).bind(&email).bind(&org.organisation_id).bind(token_hash(&token)).bind(TOKEN_HOURS as i32)
        .execute(&s.pool).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("email_taken", "an account already exists for that email"),
            e => e.into(),
        })?;
    send_verification(&s, &email, name, &token).await?;
    guard.commit().await?;
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

/// `POST /v1/console/login`: public. Finds the organisation from the email, checks the password
/// (and second factor) and returns the person plus a session API key that expires in 12 hours.
pub async fn console_login(
    State(s): State<AppState>,
    Json(b): Json<LoginIn>,
) -> Result<Json<Value>, ApiError> {
    let email = b.email.trim().to_lowercase();
    let env = "test"; // people sign in to the test tenant; live is reached through `switch_env`
    let tenant_id: Option<String> = sqlx::query_scalar(
        "SELECT t.id FROM login_directory d JOIN tenants t ON t.organisation_id = d.organisation_id WHERE d.email = $1 AND t.env = 'test'",
    )
    .bind(&email)
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
            username: email.clone(),
            password: b.password,
            otp: b.otp,
        },
    )
    .await?;
    if s.mailer.delivers() {
        let unverified: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM signups WHERE email = $1 AND verified_at IS NULL)",
        )
        .bind(&email)
        .fetch_one(&s.pool)
        .await?;
        if unverified {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "email_not_verified",
                "confirm your email first; the link was sent when you signed up",
            ));
        }
    }
    let role = out["user"]["role"].as_str().unwrap_or("viewer").to_string();
    let branch = out["user"]["branch_id"].as_str().map(str::to_string);
    let (key, prefix, hash) = mint_key(env);
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes, expires_at, branch_id, session_username) VALUES ($1, $2, 'console session', $3, $4, $5, now() + make_interval(hours => $6), $7, $8)")
        .bind(new_id("key")).bind(&tenant_id).bind(prefix).bind(hash).bind(session_scopes(&role)).bind(SESSION_HOURS).bind(branch).bind(&email)
        .execute(&s.pool).await?;
    out["session_key"] = json!(key);
    out["tenant_id"] = json!(tenant_id);
    out["env"] = json!(env);
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct SwitchIn {
    env: String,
}

/// `POST /v1/console/switch_env`: exchange a console session key for the same person's key in the
/// organisation's other environment. Only session keys can do this. Live opens once onboarding is
/// approved, the same rule that already holds payouts back.
pub async fn switch_env(
    State(s): State<AppState>,
    auth: crate::auth::Auth,
    Json(b): Json<SwitchIn>,
) -> Result<Json<Value>, ApiError> {
    let env = match b.env.as_str() {
        "test" | "live" => b.env.as_str(),
        _ => return Err(ApiError::invalid("invalid_env", "env is test or live")),
    };
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT k.session_username, t.organisation_id FROM api_keys k JOIN tenants t ON t.id = k.tenant_id WHERE k.id = $1",
    )
    .bind(&auth.key_id)
    .fetch_optional(&s.pool)
    .await?;
    let (Some((Some(username), org)), true) = (row, auth.branch_id.is_none()) else {
        return Err(ApiError::forbidden(
            "only a signed-in console session can switch environments",
        ));
    };
    let target: String =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1 AND env = $2")
            .bind(&org)
            .bind(env)
            .fetch_one(&s.pool)
            .await?;
    if env == "live" {
        open_live(&s, &org, &target).await?;
    }
    // who they are is decided in the test tenant, where identity lives
    let home: String =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1 AND env = 'test'")
            .bind(&org)
            .fetch_one(&s.pool)
            .await?;
    let mut tx = tenant::begin(&s.pool, &home).await?;
    let user: Option<(
        String,
        String,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, role, branch_id, disabled_at FROM console_users WHERE username = $1",
    )
    .bind(&username)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    let Some((uid, role, branch, None)) = user else {
        return Err(ApiError::forbidden("your account is not active"));
    };
    if env == "live" && role == "cashier" {
        return Err(ApiError::forbidden(
            "cashier sessions stay in test mode; use a branch-limited API key at the till",
        ));
    }
    let branch = if env == "live" { None } else { branch };
    let (key, prefix, hash) = mint_key(env);
    sqlx::query("INSERT INTO api_keys (id, tenant_id, name, prefix, key_hash, scopes, expires_at, branch_id, session_username) VALUES ($1, $2, 'console session', $3, $4, $5, now() + make_interval(hours => $6), $7, $8)")
        .bind(new_id("key")).bind(&target).bind(prefix).bind(hash).bind(session_scopes(&role)).bind(SESSION_HOURS).bind(&branch).bind(&username)
        .execute(&s.pool).await?;
    Ok(Json(json!({
        "user": { "id": uid, "username": username, "role": role, "branch_id": branch },
        "session_key": key, "tenant_id": target, "env": env,
    })))
}

/// Live opens only for an organisation whose business is approved (in either environment). The approval
/// is copied into the live tenant, so its own payout gate and onboarding page agree with it.
async fn open_live(s: &AppState, org: &str, live: &str) -> Result<(), ApiError> {
    let tenants: Vec<String> =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1")
            .bind(org)
            .fetch_all(&s.pool)
            .await?;
    type Profile = (
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    );
    let mut approved: Option<Profile> = None;
    let mut live_has_profile = false;
    // merchant_profiles is per tenant (row-level security), so ask each of the organisation's tenants
    for tid in &tenants {
        let mut tx = tenant::begin(&s.pool, tid).await?;
        let row: Option<(String, String, String, String, Option<String>, String, Option<String>)> = sqlx::query_as(
            "SELECT status, legal_name, business_type, pan_last4, gstin, address, reviewed_by FROM merchant_profiles",
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        if let Some((status, name, bt, pan, gstin, addr, by)) = row {
            live_has_profile |= tid == live;
            if status == "approved" && approved.is_none() {
                approved = Some((name, bt, pan, gstin, addr, by));
            }
        }
    }
    let Some((name, bt, pan, gstin, addr, by)) = approved else {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "live mode opens once your business details are approved",
        ));
    };
    if !live_has_profile {
        let mut tx = tenant::begin(&s.pool, live).await?;
        sqlx::query("INSERT INTO merchant_profiles (tenant_id, legal_name, business_type, pan_last4, gstin, address, status, reviewed_by, reviewed_at, review_note) VALUES ($1,$2,$3,$4,$5,$6,'approved',$7, now(), 'Carried over from the approval of the test environment')")
            .bind(live).bind(name).bind(bt).bind(pan).bind(gstin).bind(addr).bind(by)
            .execute(&mut *tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn send_verification(
    s: &AppState,
    email: &str,
    org: &str,
    token: &str,
) -> Result<(), ApiError> {
    crate::mail::enqueue(
        &s.pool,
        email,
        "Confirm your email for Zyvor",
        &format!(
            "Welcome to Zyvor.\n\nConfirm that {email} is yours to finish setting up {org}:\n\n{}/verify?token={token}\n\nThe link works for {TOKEN_HOURS} hours. If you did not sign up, ignore this email.\n",
            s.console_url
        ),
    )
    .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ResendIn {
    email: String,
}

/// `POST /v1/signup/resend`: a fresh verification link. Always answers the same, so addresses cannot be probed.
pub async fn resend(
    State(s): State<AppState>,
    Json(b): Json<ResendIn>,
) -> Result<Json<Value>, ApiError> {
    let email = b.email.trim().to_lowercase();
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT o.name FROM signups g JOIN organisations o ON o.id = g.organisation_id WHERE g.email = $1 AND g.verified_at IS NULL",
    )
    .bind(&email)
    .fetch_optional(&s.pool)
    .await?;
    if let Some((org,)) = row {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        sqlx::query("UPDATE signups SET token_hash = $2, token_expires_at = now() + make_interval(hours => $3) WHERE email = $1")
            .bind(&email).bind(token_hash(&token)).bind(TOKEN_HOURS as i32)
            .execute(&s.pool).await?;
        send_verification(&s, &email, &org, &token).await?;
    }
    Ok(Json(json!({ "ok": true })))
}
