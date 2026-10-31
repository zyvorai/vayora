//! People and email: inviting someone to an organisation, and resetting a forgotten password. Both use a
//! one-time link sent by email (see `mail`); only a hash of the token is kept, and each link works once.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::Auth,
    console_users::{check_password, hash_password, home, revoke_sessions},
    signup::{token_hash, valid_email},
    tenant,
};

const INVITE_DAYS: i32 = 7;
const RESET_MINUTES: i32 = 60;
const ROLES: [&str; 4] = ["admin", "finance", "viewer", "cashier"];

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn bad_link() -> ApiError {
    ApiError::invalid("invalid_token", "this link is invalid or has expired")
}

#[derive(Deserialize)]
pub struct InviteIn {
    email: String,
    role: String,
    /// Only for a cashier: the one branch they work in.
    branch_id: Option<String>,
}

/// `POST /v1/console/invitations`: email someone a link to join this organisation.
pub async fn invite(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<InviteIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let auth = home(&s, auth).await?;
    let email = b.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(ApiError::invalid(
            "invalid_email",
            "enter a valid email address",
        ));
    }
    if !ROLES.contains(&b.role.as_str()) {
        return Err(ApiError::invalid(
            "invalid_role",
            "role must be admin, finance, viewer or cashier",
        ));
    }
    if b.branch_id.is_some() && b.role != "cashier" {
        return Err(ApiError::invalid(
            "branch_only_for_cashier",
            "only a cashier can be tied to a branch",
        ));
    }
    let (org, org_name): (String, String) = sqlx::query_as(
        "SELECT o.id, o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1",
    )
    .bind(&auth.tenant_id)
    .fetch_one(&s.pool)
    .await?;
    let known: Option<String> =
        sqlx::query_scalar("SELECT organisation_id FROM login_directory WHERE email = $1")
            .bind(&email)
            .fetch_optional(&s.pool)
            .await?;
    match known {
        Some(o) if o == org => {
            return Err(ApiError::conflict(
                "already_member",
                "that person is already in this organisation",
            ));
        }
        Some(_) => {
            return Err(ApiError::conflict(
                "email_taken",
                "that email already belongs to another organisation",
            ));
        }
        None => {}
    }
    if let Some(branch) = &b.branch_id {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let found: Option<(String,)> = sqlx::query_as("SELECT id FROM branches WHERE id = $1")
            .bind(branch)
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        if found.is_none() {
            return Err(ApiError::invalid("unknown_branch", "no such branch"));
        }
    }
    let (id, token) = (new_id("inv"), new_token());
    let mut tx = s.pool.begin().await?;
    // a new invitation replaces any earlier unused one for the same person
    sqlx::query("UPDATE email_tokens SET revoked_at = now() WHERE organisation_id = $1 AND email = $2 AND purpose = 'invite' AND used_at IS NULL AND revoked_at IS NULL")
        .bind(&org).bind(&email).execute(&mut *tx).await?;
    let expires: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("INSERT INTO email_tokens (id, purpose, email, organisation_id, token_hash, payload, expires_at, created_by) VALUES ($1, 'invite', $2, $3, $4, $5, now() + make_interval(days => $6), $7) RETURNING expires_at")
        .bind(&id).bind(&email).bind(&org).bind(token_hash(&token))
        .bind(json!({ "role": b.role, "branch_id": b.branch_id })).bind(INVITE_DAYS).bind(&auth.actor)
        .fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO mail_queue (id, to_email, subject, body) VALUES ($1, $2, $3, $4)")
        .bind(new_id("mail"))
        .bind(&email)
        .bind(format!("You are invited to {org_name} on Zyvor"))
        .bind(format!(
            "You have been invited to join {org_name} on Zyvor as {}.\n\nAccept and choose a password:\n\n{}/accept?token={token}\n\nThe link works once and expires in {INVITE_DAYS} days. If you were not expecting this, ignore this email.\n",
            b.role, s.console_url
        ))
        .execute(&mut *tx).await?;
    tx.commit().await?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "console_user.invited",
        Some(&id),
        json!({ "email": email, "role": b.role }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "email": email, "role": b.role, "expires_at": expires })),
    ))
}

/// `GET /v1/console/invitations`: invitations nobody has accepted yet.
pub async fn list_invitations(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let auth = home(&s, auth).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Value, chrono::DateTime<chrono::Utc>, Option<String>)> = sqlx::query_as(
        "SELECT e.id, e.email, e.payload, e.expires_at, e.created_by FROM email_tokens e JOIN tenants t ON t.organisation_id = e.organisation_id
         WHERE t.id = $1 AND e.purpose = 'invite' AND e.used_at IS NULL AND e.revoked_at IS NULL AND e.expires_at > now() ORDER BY e.created_at DESC",
    )
    .bind(&auth.tenant_id)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, email, p, exp, by)| json!({ "id": id, "email": email, "role": p["role"], "expires_at": exp, "invited_by": by })).collect::<Vec<_>>() }),
    ))
}

/// `POST /v1/console/invitations/{id}/revoke`
pub async fn revoke_invitation(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let auth = home(&s, auth).await?;
    let n = sqlx::query("UPDATE email_tokens SET revoked_at = now() WHERE id = $1 AND purpose = 'invite' AND used_at IS NULL AND revoked_at IS NULL AND organisation_id = (SELECT organisation_id FROM tenants WHERE id = $2)")
        .bind(&id).bind(&auth.tenant_id).execute(&s.pool).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("invitation"));
    }
    Ok(Json(json!({ "id": id, "revoked": true })))
}

#[derive(Deserialize)]
pub struct TokenIn {
    token: String,
}

/// `POST /v1/invitations/lookup`: who the link is for, so the page can greet them. Public.
pub async fn lookup_invitation(
    State(s): State<AppState>,
    Json(b): Json<TokenIn>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, String, Value)> = sqlx::query_as(
        "SELECT e.email, o.name, e.payload FROM email_tokens e JOIN organisations o ON o.id = e.organisation_id
         WHERE e.token_hash = $1 AND e.purpose = 'invite' AND e.used_at IS NULL AND e.revoked_at IS NULL AND e.expires_at > now()",
    )
    .bind(token_hash(b.token.trim()))
    .fetch_optional(&s.pool)
    .await?;
    let (email, org, p) = row.ok_or_else(bad_link)?;
    Ok(Json(
        json!({ "email": email, "organisation": org, "role": p["role"] }),
    ))
}

#[derive(Deserialize)]
pub struct AcceptIn {
    token: String,
    password: String,
}

/// `POST /v1/invitations/accept`: choose a password and join. Public; the emailed token is the proof.
pub async fn accept_invitation(
    State(s): State<AppState>,
    Json(b): Json<AcceptIn>,
) -> Result<Json<Value>, ApiError> {
    check_password(&b.password)?;
    let hash = hash_password(&b.password)?;
    let mut tx = s.pool.begin().await?;
    // spending the token and creating the person are one step, so a second click finds nothing
    let row: Option<(String, String, String, Value)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now() WHERE token_hash = $1 AND purpose = 'invite' AND used_at IS NULL AND revoked_at IS NULL AND expires_at > now()
         RETURNING email, organisation_id, id, payload",
    )
    .bind(token_hash(b.token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let (email, org, inv_id, p) = row.ok_or_else(bad_link)?;
    let home_tenant: String =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1 AND env = 'test'")
            .bind(&org)
            .fetch_one(&mut *tx)
            .await?;
    let taken = sqlx::query("INSERT INTO login_directory (email, organisation_id) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING")
        .bind(&email).bind(&org).execute(&mut *tx).await?.rows_affected() == 0;
    if taken {
        // someone with this email joined (or an account was made) after the invitation went out
        let same: bool =
            sqlx::query_scalar("SELECT organisation_id = $2 FROM login_directory WHERE email = $1")
                .bind(&email)
                .bind(&org)
                .fetch_one(&mut *tx)
                .await?;
        if !same {
            return Err(ApiError::conflict(
                "email_taken",
                "that email already belongs to another organisation",
            ));
        }
    }
    let uid = new_id("usr");
    // row-level security needs the tenant set on this connection
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(&home_tenant)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO console_users (id, tenant_id, username, password_hash, role, created_by, branch_id) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&uid).bind(&home_tenant).bind(&email).bind(&hash)
        .bind(p["role"].as_str().unwrap_or("viewer")).bind(format!("invitation:{inv_id}")).bind(p["branch_id"].as_str())
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("already_member", "that person is already in this organisation"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &home_tenant,
        &format!("invitation:{inv_id}"),
        "console_user.joined",
        Some(&uid),
        json!({ "email": email, "role": p["role"] }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "email": email, "joined": true })))
}

#[derive(Deserialize)]
pub struct ResetRequestIn {
    email: String,
}

/// `POST /v1/password_reset/request`: email a reset link if the address belongs to an active person.
/// Always answers the same, so addresses cannot be probed.
pub async fn request_reset(
    State(s): State<AppState>,
    Json(b): Json<ResetRequestIn>,
) -> Result<Json<Value>, ApiError> {
    let email = b.email.trim().to_lowercase();
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT d.organisation_id, h.id FROM login_directory d JOIN tenants h ON h.organisation_id = d.organisation_id AND h.env = 'test' WHERE d.email = $1",
    )
    .bind(&email)
    .fetch_optional(&s.pool)
    .await?;
    if let Some((org, home_tenant)) = row {
        let mut tx = tenant::begin(&s.pool, &home_tenant).await?;
        let active: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM console_users WHERE username = $1 AND disabled_at IS NULL",
        )
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        if active.is_some() {
            let token = new_token();
            let mut tx = s.pool.begin().await?;
            sqlx::query("UPDATE email_tokens SET revoked_at = now() WHERE organisation_id = $1 AND email = $2 AND purpose = 'reset' AND used_at IS NULL AND revoked_at IS NULL")
                .bind(&org).bind(&email).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO email_tokens (id, purpose, email, organisation_id, token_hash, expires_at) VALUES ($1, 'reset', $2, $3, $4, now() + make_interval(mins => $5))")
                .bind(new_id("rst")).bind(&email).bind(&org).bind(token_hash(&token)).bind(RESET_MINUTES)
                .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO mail_queue (id, to_email, subject, body) VALUES ($1, $2, $3, $4)")
                .bind(new_id("mail")).bind(&email).bind("Reset your Zyvor password")
                .bind(format!("Someone asked to reset the password for {email}.\n\nChoose a new one:\n\n{}/reset?token={token}\n\nThe link works once and expires in {RESET_MINUTES} minutes. If it was not you, ignore this email; your password has not changed.\n", s.console_url))
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ResetConfirmIn {
    token: String,
    password: String,
}

/// `POST /v1/password_reset/confirm`: set the new password, end every session and clear any lockout.
pub async fn confirm_reset(
    State(s): State<AppState>,
    Json(b): Json<ResetConfirmIn>,
) -> Result<Json<Value>, ApiError> {
    check_password(&b.password)?;
    let hash = hash_password(&b.password)?;
    let mut tx = s.pool.begin().await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE email_tokens SET used_at = now() WHERE token_hash = $1 AND purpose = 'reset' AND used_at IS NULL AND revoked_at IS NULL AND expires_at > now()
         RETURNING email, organisation_id",
    )
    .bind(token_hash(b.token.trim()))
    .fetch_optional(&mut *tx)
    .await?;
    let (email, org) = row.ok_or_else(bad_link)?;
    let home_tenant: String =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1 AND env = 'test'")
            .bind(&org)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(&home_tenant)
        .execute(&mut *tx)
        .await?;
    let n = sqlx::query("UPDATE console_users SET password_hash = $2, failed_attempts = 0, locked_until = NULL WHERE username = $1 AND disabled_at IS NULL")
        .bind(&email).bind(&hash).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(bad_link());
    }
    tenant::audit(
        &mut tx,
        &home_tenant,
        "system:password_reset",
        "console_user.password_reset",
        None,
        json!({ "email": email }),
    )
    .await?;
    tx.commit().await?;
    revoke_sessions(&s, &home_tenant, &email).await?;
    Ok(Json(json!({ "ok": true })))
}
