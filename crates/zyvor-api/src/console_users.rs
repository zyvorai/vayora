//! Console users: named accounts with roles (admin, finance, viewer, cashier), Argon2id passwords, optional
//! TOTP second factor with single-use recovery codes, and lockout after repeated failures.
//! The console authenticates a person by calling `POST /v1/console/authenticate` with its own admin
//! key; the API owns the credentials, the console only keeps the signed session.

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, tenant};

const MIN_PASSWORD: usize = 12;
const LOCK_AFTER: i32 = 5;
const LOCK_MINUTES: i32 = 15;
const ROLES: [&str; 4] = ["admin", "finance", "viewer", "cashier"];

fn hash_password(pw: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(pw.as_bytes())
        .map(|h| h.to_string())
        .map_err(|_| ApiError::internal())
}

fn verify_password(pw: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| Argon2::default().verify_password(pw.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

// ---- TOTP (RFC 6238: HMAC-SHA1, 30 s steps, 6 digits) ----

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(data: &[u8]) -> String {
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=' && *c != b' ') {
        let v = B32.iter().position(|x| *x == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

pub fn totp_at(secret: &[u8], step: u64) -> String {
    let mut m = <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("any key length");
    m.update(&step.to_be_bytes());
    let h = m.finalize().into_bytes();
    let o = (h[19] & 0xf) as usize;
    let code = u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]]) % 1_000_000;
    format!("{code:06}")
}

/// The matching step within ±1 of `now`, if the code is right. Callers store it to refuse replays.
pub fn totp_verify(secret: &[u8], code: &str, now_secs: u64) -> Option<u64> {
    let step = now_secs / 30;
    (step.saturating_sub(1)..=step + 1).find(|s| {
        let want = totp_at(secret, *s);
        code.len() == 6 && bool::from(want.as_bytes().ct_eq(code.as_bytes()))
    })
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(16) {
        let u = uuid::Uuid::new_v4();
        chunk.copy_from_slice(&u.as_bytes()[..chunk.len()]);
    }
    out
}

fn recovery_code() -> String {
    let b: [u8; 5] = random_bytes();
    let s = base32_encode(&b).to_lowercase();
    format!("{}-{}", &s[..4], &s[4..8])
}

fn hash_recovery(c: &str) -> String {
    hex::encode(Sha256::digest(c.trim().to_lowercase().as_bytes()))
}

// ---- management (admin key) ----

#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    password: String,
    role: String,
    /// Only for a cashier: the one branch they work in.
    #[serde(default)]
    branch_id: Option<String>,
}

fn valid_username(u: &str) -> bool {
    (3..=64).contains(&u.len())
        && u.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
}

fn check_password(p: &str) -> Result<(), ApiError> {
    if p.chars().count() < MIN_PASSWORD {
        return Err(ApiError::invalid(
            "weak_password",
            format!("password must be at least {MIN_PASSWORD} characters"),
        ));
    }
    Ok(())
}

pub async fn create_user(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewUser>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let username = b.username.trim().to_lowercase();
    if !valid_username(&username) {
        return Err(ApiError::invalid(
            "invalid_username",
            "3-64 characters: letters, digits and . _ - @",
        ));
    }
    if !ROLES.contains(&b.role.as_str()) {
        return Err(ApiError::invalid(
            "invalid_role",
            "role must be admin, finance, viewer or cashier",
        ));
    }
    check_password(&b.password)?;
    if b.branch_id.is_some() && b.role != "cashier" {
        return Err(ApiError::invalid(
            "branch_only_for_cashier",
            "only a cashier can be tied to a branch",
        ));
    }
    let (id, hash) = (new_id("usr"), hash_password(&b.password)?);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // foreign keys skip row-level security, so check the branch is this tenant's
    if let Some(branch) = &b.branch_id {
        let known: Option<(String,)> = sqlx::query_as("SELECT id FROM branches WHERE id = $1")
            .bind(branch)
            .fetch_optional(&mut *tx)
            .await?;
        if known.is_none() {
            return Err(ApiError::invalid("unknown_branch", "no such branch"));
        }
    }
    sqlx::query("INSERT INTO console_users (id, tenant_id, username, password_hash, role, created_by, branch_id) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(&id).bind(&auth.tenant_id).bind(&username).bind(&hash).bind(&b.role).bind(&auth.actor).bind(&b.branch_id)
        .execute(&mut *tx).await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => ApiError::conflict("username_taken", "that username already exists"),
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "console_user.created",
        Some(&id),
        json!({ "username": username, "role": b.role }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "username": username, "role": b.role, "branch_id": b.branch_id })),
    ))
}

pub async fn list_users(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    type Row = (
        String,
        String,
        String,
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as("SELECT id, username, role, totp_confirmed, last_login_at, locked_until, disabled_at, branch_id FROM console_users ORDER BY username")
        .fetch_all(&mut *tx)
        .await?;
    let now = chrono::Utc::now();
    let data: Vec<Value> = rows
        .into_iter()
        .map(|(id, u, role, mfa, last, locked, disabled, branch)| {
            json!({
                "id": id, "username": u, "role": role, "mfa_enrolled": mfa, "last_login_at": last,
                "locked": locked.is_some_and(|l| l > now), "disabled": disabled.is_some(), "branch_id": branch,
            })
        })
        .collect();
    Ok(Json(json!({ "data": data })))
}

async fn user_action(
    s: &AppState,
    auth: &Auth,
    id: &str,
    sql: &'static str,
    action: &str,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(sql)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("console user"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        action,
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "ok": true })))
}

pub async fn disable_user(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    user_action(
        &s,
        &auth,
        &id,
        "UPDATE console_users SET disabled_at = now() WHERE id = $1 AND disabled_at IS NULL",
        "console_user.disabled",
    )
    .await
}

pub async fn enable_user(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    user_action(&s, &auth, &id, "UPDATE console_users SET disabled_at = NULL, failed_attempts = 0, locked_until = NULL WHERE id = $1", "console_user.enabled").await
}

pub async fn reset_mfa(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    user_action(&s, &auth, &id, "UPDATE console_users SET totp_secret_enc = NULL, totp_confirmed = false, totp_last_step = 0, recovery_hashes = '{}' WHERE id = $1", "console_user.mfa_reset").await
}

#[derive(Deserialize)]
pub struct NewPassword {
    password: String,
}

pub async fn set_password(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<NewPassword>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    check_password(&b.password)?;
    let hash = hash_password(&b.password)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE console_users SET password_hash = $2, failed_attempts = 0, locked_until = NULL WHERE id = $1")
        .bind(&id).bind(&hash).execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("console user"));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "console_user.password_set",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "ok": true })))
}

// ---- MFA enrolment ----

/// Start enrolment: returns the secret and an `otpauth://` URI for an authenticator app. Not active until confirmed.
pub async fn mfa_begin(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let secret: [u8; 20] = random_bytes();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, bool)> = sqlx::query_as("SELECT username, totp_confirmed FROM console_users WHERE id = $1 AND disabled_at IS NULL FOR UPDATE")
        .bind(&id).fetch_optional(&mut *tx).await?;
    let (username, confirmed) = row.ok_or_else(|| ApiError::not_found("console user"))?;
    if confirmed {
        return Err(ApiError::conflict(
            "mfa_already_enrolled",
            "reset MFA first to enrol a new device",
        ));
    }
    sqlx::query("UPDATE console_users SET totp_secret_enc = $2 WHERE id = $1")
        .bind(&id)
        .bind(s.crypto.encrypt(&secret))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let b32 = base32_encode(&secret);
    Ok(Json(
        json!({ "secret": b32, "otpauth_uri": format!("otpauth://totp/Zyvor:{username}?secret={b32}&issuer=Zyvor&digits=6&period=30") }),
    ))
}

#[derive(Deserialize)]
pub struct Otp {
    otp: String,
}

/// Prove the device works; activates MFA and returns recovery codes, shown once.
pub async fn mfa_confirm(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<Otp>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(Option<Vec<u8>>, bool)> = sqlx::query_as("SELECT totp_secret_enc, totp_confirmed FROM console_users WHERE id = $1 AND disabled_at IS NULL FOR UPDATE")
        .bind(&id).fetch_optional(&mut *tx).await?;
    let (enc, confirmed) = row.ok_or_else(|| ApiError::not_found("console user"))?;
    let secret = enc
        .and_then(|e| s.crypto.decrypt(&e))
        .ok_or_else(|| ApiError::conflict("mfa_not_started", "begin enrolment first"))?;
    if confirmed {
        return Err(ApiError::conflict(
            "mfa_already_enrolled",
            "already enrolled",
        ));
    }
    let step = totp_verify(&secret, b.otp.trim(), chrono::Utc::now().timestamp() as u64)
        .ok_or_else(|| {
            ApiError::invalid(
                "invalid_otp",
                "that code is not right; check the device clock",
            )
        })?;
    let codes: Vec<String> = (0..8).map(|_| recovery_code()).collect();
    let hashes: Vec<String> = codes.iter().map(|c| hash_recovery(c)).collect();
    sqlx::query("UPDATE console_users SET totp_confirmed = true, totp_last_step = $2, recovery_hashes = $3 WHERE id = $1")
        .bind(&id).bind(step as i64).bind(&hashes).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "console_user.mfa_enrolled",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "recovery_codes": codes, "note": "store these now; each works once and they cannot be shown again" }),
    ))
}

// ---- sign-in ----

#[derive(Deserialize)]
pub struct Login {
    username: String,
    password: String,
    otp: Option<String>,
}

enum Outcome {
    Ok(Value),
    Bad,
    OtpRequired,
    Locked,
}

/// `POST /v1/console/authenticate`. Wrong username and wrong password are indistinguishable; the
/// second factor is only asked for after the password is right.
pub async fn authenticate(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Login>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let username = b.username.trim().to_lowercase();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    type Row = (
        String,
        String,
        String,
        Option<Vec<u8>>,
        bool,
        i64,
        Vec<String>,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, password_hash, role, totp_secret_enc, totp_confirmed, totp_last_step, recovery_hashes, failed_attempts, locked_until, disabled_at
         FROM console_users WHERE username = $1 FOR UPDATE",
    )
    .bind(&username)
    .fetch_optional(&mut *tx)
    .await?;
    let now = chrono::Utc::now();
    let outcome = match row {
        // spend the same time hashing as a real check so timing does not reveal which usernames exist
        None => {
            static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            let dummy =
                DUMMY.get_or_init(|| hash_password("not-a-real-password").unwrap_or_default());
            let _ = verify_password(&b.password, dummy);
            Outcome::Bad
        }
        Some((id, hash, role, _, _, _, _, _, locked, disabled))
            if disabled.is_some() || locked.is_some_and(|l| l > now) =>
        {
            let _ = (id, hash, role);
            if disabled.is_some() {
                Outcome::Bad
            } else {
                Outcome::Locked
            }
        }
        Some((id, hash, role, enc, mfa, last_step, recovery, failed, _, _)) => {
            let mut result = if verify_password(&b.password, &hash) {
                None
            } else {
                Some(Outcome::Bad)
            };
            let mut new_step = None;
            let mut new_recovery = None;
            if result.is_none() && mfa {
                match b.otp.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
                    None => result = Some(Outcome::OtpRequired),
                    Some(code) => {
                        let secret = enc.and_then(|e| s.crypto.decrypt(&e));
                        let step = secret
                            .and_then(|sec| totp_verify(&sec, code, now.timestamp() as u64))
                            .filter(|st| *st as i64 > last_step);
                        if let Some(st) = step {
                            new_step = Some(st as i64);
                        } else {
                            let h = hash_recovery(code);
                            if recovery.contains(&h) {
                                new_recovery = Some(
                                    recovery
                                        .iter()
                                        .filter(|x| **x != h)
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                );
                            } else {
                                result = Some(Outcome::Bad);
                            }
                        }
                    }
                }
            }
            match result {
                Some(Outcome::OtpRequired) => Outcome::OtpRequired,
                Some(_) => {
                    let locked = failed + 1 >= LOCK_AFTER;
                    sqlx::query("UPDATE console_users SET failed_attempts = CASE WHEN $2 THEN 0 ELSE failed_attempts + 1 END, locked_until = CASE WHEN $2 THEN now() + make_interval(mins => $3) ELSE locked_until END WHERE id = $1")
                        .bind(&id).bind(locked).bind(LOCK_MINUTES).execute(&mut *tx).await?;
                    tenant::audit(
                        &mut tx,
                        &auth.tenant_id,
                        "system:console",
                        if locked {
                            "console_user.locked"
                        } else {
                            "console_user.login_failed"
                        },
                        Some(&id),
                        json!({}),
                    )
                    .await?;
                    Outcome::Bad
                }
                None => {
                    sqlx::query("UPDATE console_users SET failed_attempts = 0, locked_until = NULL, last_login_at = now(), totp_last_step = COALESCE($2, totp_last_step), recovery_hashes = COALESCE($3, recovery_hashes) WHERE id = $1")
                        .bind(&id).bind(new_step).bind(&new_recovery).execute(&mut *tx).await?;
                    tenant::audit(
                        &mut tx,
                        &auth.tenant_id,
                        "system:console",
                        "console_user.login",
                        Some(&id),
                        json!({ "recovery_code_used": new_recovery.is_some() }),
                    )
                    .await?;
                    let branch: Option<(Option<String>,)> =
                        sqlx::query_as("SELECT branch_id FROM console_users WHERE id = $1")
                            .bind(&id)
                            .fetch_optional(&mut *tx)
                            .await?;
                    let branch = branch.and_then(|b| b.0);
                    Outcome::Ok(
                        json!({ "user": { "id": id, "username": username, "role": role, "branch_id": branch }, "mfa_enrolled": mfa }),
                    )
                }
            }
        }
    };
    tx.commit().await?;
    match outcome {
        Outcome::Ok(v) => Ok(Json(v)),
        Outcome::Bad => Err(ApiError::unauthorized(
            "incorrect username, password or code",
        )),
        Outcome::OtpRequired => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "otp_required",
            "enter the code from your authenticator app",
        )),
        Outcome::Locked => Err(ApiError::new(
            StatusCode::LOCKED,
            "account_locked",
            format!("too many failed attempts; try again in {LOCK_MINUTES} minutes"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_the_rfc_6238_vectors() {
        // RFC 6238 appendix B, SHA-1, secret "12345678901234567890"; the RFC lists 8 digits, we use the last 6
        let secret = b"12345678901234567890";
        for (t, want) in [
            (59u64, "287082"),
            (1_111_111_109, "081804"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
        ] {
            assert_eq!(totp_at(secret, t / 30), want, "t={t}");
        }
        assert_eq!(totp_verify(secret, "287082", 59), Some(1));
        assert_eq!(
            totp_verify(secret, "287082", 59 + 30),
            Some(1),
            "one step of clock drift is tolerated"
        );
        assert_eq!(totp_verify(secret, "287082", 59 + 90), None);
        assert_eq!(totp_verify(secret, "28708", 59), None);
    }

    #[test]
    fn base32_round_trips() {
        for len in 0..24usize {
            let data: Vec<u8> = (0..len as u8).map(|i| i.wrapping_mul(37)).collect();
            assert_eq!(base32_decode(&base32_encode(&data)).unwrap(), data);
        }
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert!(base32_decode("not base32!").is_none());
    }

    #[test]
    fn passwords_hash_and_verify() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("correct horse batterz", &h));
        assert!(!verify_password("x", "not a hash"));
    }
}
