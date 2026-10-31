//! Zyvor-operated review before live mode. A merchant whose business details are approved asks for live access;
//! a named Zyvor reviewer looks at what they submitted and the checks that ran, and approves or rejects with a
//! note. Only then can the merchant's console switch to live. Reviewers use their own keys (`zp_...`, made with
//! `zyvor-api platform-reviewer add`); a merchant's key, session key or second admin can never stand in for one.

use axum::{
    Json,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, auth::hash_key, tenant};

/// A Zyvor reviewer, identified by a key that is not a merchant API key.
pub struct PlatformAuth {
    pub id: String,
    pub name: String,
}

impl FromRequestParts<AppState> for PlatformAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let key = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|k| k.starts_with("zp_"))
            .ok_or_else(|| ApiError::unauthorized("a Zyvor reviewer key is required"))?;
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT id, name FROM platform_reviewers WHERE key_hash = $1 AND disabled_at IS NULL",
        )
        .bind(hash_key(key))
        .fetch_optional(&state.pool)
        .await?;
        let (id, name) =
            row.ok_or_else(|| ApiError::unauthorized("invalid or disabled reviewer key"))?;
        Ok(PlatformAuth { id, name })
    }
}

/// Make a reviewer. Returns `(id, key)`; the key is shown once and only its hash is kept.
pub async fn create_reviewer(pool: &PgPool, name: &str) -> Result<(String, String), sqlx::Error> {
    let id = new_id("rev");
    let key = format!(
        "zp_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    sqlx::query("INSERT INTO platform_reviewers (id, name, key_hash) VALUES ($1, $2, $3)")
        .bind(&id)
        .bind(name)
        .bind(hash_key(&key))
        .execute(pool)
        .await?;
    Ok((id, key))
}

pub async fn list_reviewers(pool: &PgPool) -> Result<Vec<(String, String, bool)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, disabled_at IS NOT NULL FROM platform_reviewers ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
}

pub async fn disable_reviewer(pool: &PgPool, id: &str) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE platform_reviewers SET disabled_at = now() WHERE id = $1 AND disabled_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

async fn org_of(s: &AppState, tenant_id: &str) -> Result<(String, String), ApiError> {
    Ok(sqlx::query_as(
        "SELECT o.id, o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&s.pool)
    .await?)
}

async fn test_tenant_of(s: &AppState, org: &str) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1 AND env = 'test'")
        .bind(org)
        .fetch_optional(&s.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("organisation"))
}

/// The merchant's own onboarding is approved (in either of its tenants). That is the entry ticket to a review,
/// not the permission to go live.
pub async fn onboarding_approved(s: &AppState, org: &str) -> Result<bool, ApiError> {
    let tenants: Vec<String> =
        sqlx::query_scalar("SELECT id FROM tenants WHERE organisation_id = $1")
            .bind(org)
            .fetch_all(&s.pool)
            .await?;
    for tid in &tenants {
        let mut tx = tenant::begin(&s.pool, tid).await?;
        let st: Option<String> = sqlx::query_scalar("SELECT status FROM merchant_profiles")
            .fetch_optional(&mut *tx)
            .await?;
        tx.commit().await?;
        if st.as_deref() == Some("approved") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The gate `switch_env` uses: live opens only when a reviewer approved this organisation. Returns the
/// reviewer's name for the record that is copied into the live tenant.
pub async fn live_gate(s: &AppState, org: &str) -> Result<String, ApiError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT l.status, r.name FROM live_access l LEFT JOIN platform_reviewers r ON r.id = l.reviewed_by WHERE l.organisation_id = $1",
    )
    .bind(org)
    .fetch_optional(&s.pool)
    .await?;
    match row {
        Some((st, Some(name))) if st == "approved" => Ok(name),
        Some((st, _)) if st == "revoked" => Err(ApiError::conflict(
            "live_access_revoked",
            "Zyvor has taken live mode away from this business; see the note on the Onboarding page or contact Zyvor",
        )),
        Some((st, _)) if st == "pending" => Err(ApiError::conflict(
            "live_access_pending",
            "Zyvor is reviewing your business; live mode opens when that is approved",
        )),
        _ => Err(ApiError::conflict(
            "live_access_required",
            "Zyvor reviews every business before live mode opens; request live access from the Onboarding page",
        )),
    }
}

// ---------- the merchant's side ----------

/// `GET /v1/live_access`
pub async fn live_access_status(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")?;
    let (org, _) = org_of(&s, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let row: Option<(String, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>, i32)> = sqlx::query_as(
        "SELECT status, note, requested_at, reviewed_at, round FROM live_access WHERE organisation_id = $1",
    )
    .bind(&org)
    .fetch_optional(&s.pool)
    .await?;
    let ready = onboarding_approved(&s, &org).await?;
    Ok(Json(match row {
        Some((st, note, at, rev, round)) => {
            json!({ "status": st, "note": note, "requested_at": at, "reviewed_at": rev, "round": round, "onboarding_approved": ready })
        }
        None => json!({ "status": "not_requested", "note": null, "onboarding_approved": ready }),
    }))
}

/// `POST /v1/live_access`: ask Zyvor to review the business. Safe to repeat: a pending or approved request is
/// returned as it is, and a rejected one becomes pending again.
pub async fn request_live_access(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("onboarding:write")?;
    let (org, _) = org_of(&s, &auth.tenant_id).await?;
    if !onboarding_approved(&s, &org).await? {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "get your business details approved first (Onboarding), then ask for live access",
        ));
    }
    let mut tx = s.pool.begin().await?;
    let cur: Option<String> =
        sqlx::query_scalar("SELECT status FROM live_access WHERE organisation_id = $1 FOR UPDATE")
            .bind(&org)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, created) = match cur.as_deref() {
        None => {
            sqlx::query("INSERT INTO live_access (organisation_id, status, requested_by) VALUES ($1, 'pending', $2)")
                .bind(&org)
                .bind(&auth.actor)
                .execute(&mut *tx)
                .await?;
            ("pending", true)
        }
        Some("rejected") => {
            sqlx::query("UPDATE live_access SET status = 'pending', requested_by = $2, requested_at = now(), reviewed_by = NULL, reviewed_at = NULL, note = NULL, round = round + 1 WHERE organisation_id = $1")
                .bind(&org)
                .bind(&auth.actor)
                .execute(&mut *tx)
                .await?;
            ("pending", true)
        }
        Some("approved") => ("approved", false),
        // only a reviewer can give live mode back; asking again would hide that it was taken away
        Some("revoked") => {
            return Err(ApiError::conflict(
                "live_access_revoked",
                "Zyvor has taken live mode away from this business; contact Zyvor to have it reviewed again",
            ));
        }
        Some(_) => ("pending", false),
    };
    tx.commit().await?;
    if created {
        let mut t = tenant::begin(&s.pool, &auth.tenant_id).await?;
        tenant::audit(
            &mut t,
            &auth.tenant_id,
            &auth.actor,
            "live_access.requested",
            None,
            json!({}),
        )
        .await?;
        t.commit().await?;
        tracing::info!(organisation = %org, "live access requested; waiting for a Zyvor reviewer");
    }
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({ "status": status })),
    ))
}

// ---------- the reviewer's side ----------

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
}

/// What a reviewer needs to decide, read from the merchant's test tenant. No secrets, and never a full PAN or
/// account number (only the last four digits are kept anywhere).
async fn detail(s: &AppState, org: &str) -> Result<Value, ApiError> {
    let tid = test_tenant_of(s, org).await?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    #[allow(clippy::type_complexity)]
    let p: Option<(String, String, String, Option<String>, String, String, Option<String>, Option<String>, Option<String>, Vec<String>)> = sqlx::query_as(
        "SELECT legal_name, business_type, pan_last4, gstin, address, status, submitted_by, reviewed_by, review_note, review_reasons FROM merchant_profiles",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let docs: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT kind, reference, status FROM onboarding_documents ORDER BY created_at",
    )
    .fetch_all(&mut *tx)
    .await?;
    let banks: Vec<(String, String, String, bool)> = sqlx::query_as("SELECT holder_name, ifsc, account_last4, verified_at IS NOT NULL FROM merchant_bank_accounts ORDER BY created_at").fetch_all(&mut *tx).await?;
    let checks: Vec<(String, String, String)> =
        sqlx::query_as("SELECT kind, outcome, detail FROM kyc_checks ORDER BY created_at")
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(json!({
        "profile": p.map(|p| json!({ "legal_name": p.0, "business_type": p.1, "pan": format!("XXXXXX{}", p.2), "gstin": p.3, "address": p.4, "status": p.5, "submitted_by": p.6, "reviewed_by": p.7, "review_note": p.8, "review_reasons": p.9 })),
        "documents": docs.into_iter().map(|d| json!({ "kind": d.0, "reference": d.1, "status": d.2 })).collect::<Vec<_>>(),
        "bank_accounts": banks.into_iter().map(|b| json!({ "holder_name": b.0, "ifsc": b.1, "account_last4": b.2, "verified": b.3 })).collect::<Vec<_>>(),
        "checks": checks.into_iter().map(|c| json!({ "kind": c.0, "outcome": c.1, "detail": c.2 })).collect::<Vec<_>>(),
    }))
}

/// `GET /v1/platform/reviews?status=pending`
pub async fn list_reviews(
    State(s): State<AppState>,
    _p: PlatformAuth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, chrono::DateTime<chrono::Utc>, i32)> = sqlx::query_as(
        "SELECT l.organisation_id, o.name, l.status, l.requested_by, l.requested_at, l.round FROM live_access l JOIN organisations o ON o.id = l.organisation_id
         WHERE ($1::text IS NULL OR l.status = $1) ORDER BY l.requested_at LIMIT 100",
    )
    .bind(&q.status)
    .fetch_all(&s.pool)
    .await?;
    let mut out = Vec::new();
    for (org, name, status, by, at, round) in rows {
        let d = detail(&s, &org).await?;
        out.push(json!({ "organisation_id": org, "organisation": name, "status": status, "requested_by": by, "requested_at": at, "round": round, "legal_name": d["profile"]["legal_name"], "checks_failed": d["checks"].as_array().map_or(0, |c| c.iter().filter(|x| x["outcome"] != "pass").count()) }));
    }
    Ok(Json(json!({ "data": out })))
}

/// `GET /v1/platform/reviews/{organisation_id}`
pub async fn show_review(
    State(s): State<AppState>,
    _p: PlatformAuth,
    Path(org): Path<String>,
) -> Result<Json<Value>, ApiError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(String, String, String, String, chrono::DateTime<chrono::Utc>, Option<String>, Option<String>, i32)> = sqlx::query_as(
        "SELECT o.name, l.status, l.requested_by, l.organisation_id, l.requested_at, l.note, l.reviewed_by, l.round FROM live_access l JOIN organisations o ON o.id = l.organisation_id WHERE l.organisation_id = $1",
    )
    .bind(&org)
    .fetch_optional(&s.pool)
    .await?;
    let (name, status, by, _, at, note, reviewer, round) =
        row.ok_or_else(|| ApiError::not_found("review"))?;
    let mut d = detail(&s, &org).await?;
    d["organisation_id"] = json!(org);
    d["organisation"] = json!(name);
    d["status"] = json!(status);
    d["requested_by"] = json!(by);
    d["requested_at"] = json!(at);
    d["note"] = json!(note);
    d["reviewed_by"] = json!(reviewer);
    d["round"] = json!(round);
    Ok(Json(d))
}

#[derive(Deserialize)]
pub struct Decision {
    /// `approve` or `reject`.
    decision: String,
    note: Option<String>,
}

/// `POST /v1/platform/reviews/{organisation_id}/decision`
pub async fn decide(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(org): Path<String>,
    Json(b): Json<Decision>,
) -> Result<Json<Value>, ApiError> {
    let approve = match b.decision.as_str() {
        "approve" => true,
        "reject" => false,
        _ => {
            return Err(ApiError::invalid(
                "invalid_decision",
                "decision is approve or reject",
            ));
        }
    };
    let note = b.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if !approve && note.is_none() {
        return Err(ApiError::invalid(
            "note_required",
            "say why it was rejected; the merchant is shown this",
        ));
    }
    let tid = test_tenant_of(&s, &org).await?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    // the review row is not tenant data; lock it in this same transaction
    let cur: Option<(String, String)> = sqlx::query_as(
        "SELECT status, requested_by FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(&org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, requested_by) = cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this review is already {status}"),
        ));
    }
    if approve && !onboarding_approved(&s, &org).await? {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "the business details are no longer approved",
        ));
    }
    let new = if approve { "approved" } else { "rejected" };
    sqlx::query("UPDATE live_access SET status = $2, reviewed_by = $3, reviewed_at = now(), note = $4 WHERE organisation_id = $1")
        .bind(&org)
        .bind(new)
        .bind(&p.id)
        .bind(note)
        .execute(&mut *tx)
        .await?;
    let actor = format!("zyvor:{}", p.name);
    tenant::audit(
        &mut tx,
        &tid,
        &actor,
        &format!("live_access.{new}"),
        None,
        json!({ "note": note }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &tid,
        &format!("live_access.{new}"),
        json!({ "reviewed_by": actor, "note": note }),
    )
    .await?;
    tx.commit().await?;
    let (subject, body) = if approve {
        (
            "Live mode is open for your business".to_string(),
            format!(
                "Zyvor has reviewed your business and opened live mode.\n\nSign in and use the Test / Live switch at the top of the console: {}\n",
                s.console_url
            ),
        )
    } else {
        (
            "Zyvor could not open live mode yet".to_string(),
            format!(
                "Zyvor reviewed your request and did not open live mode yet.\n\nWhat to fix: {}\n\nYou can correct it and ask again from the Onboarding page: {}/onboarding\n",
                note.unwrap_or(""),
                s.console_url
            ),
        )
    };
    if requested_by.contains('@') || requested_by.starts_with("user:") {
        let to = requested_by.trim_start_matches("user:");
        if to.contains('@') {
            crate::mail::enqueue(&s.pool, to, &subject, &body).await?;
        }
    }
    Ok(Json(json!({ "organisation_id": org, "status": new })))
}

/// Is this tenant a live one whose organisation has had live mode taken away? Such a tenant can read and
/// refund, but cannot take new payments, move money out, or change anything else (see `Auth`).
pub async fn live_frozen(pool: &PgPool, tenant_id: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM tenants t JOIN live_access l ON l.organisation_id = t.organisation_id
         WHERE t.id = $1 AND t.env = 'live' AND l.status = 'revoked')",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
}

#[derive(Deserialize)]
pub struct Revocation {
    note: Option<String>,
}

/// `POST /v1/platform/reviews/{organisation_id}/revoke`: take live mode away from an approved business. The note is
/// required: the merchant is shown it.
pub async fn revoke(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(org): Path<String>,
    Json(b): Json<Revocation>,
) -> Result<Json<Value>, ApiError> {
    let note = b.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let Some(note) = note else {
        return Err(ApiError::invalid(
            "note_required",
            "say why live mode is being taken away; the merchant is shown this",
        ));
    };
    change_live_access(&s, &p, &org, "approved", "revoked", Some(note)).await
}

/// `POST /v1/platform/reviews/{organisation_id}/restore`: give live mode back to a revoked business.
pub async fn restore(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(org): Path<String>,
    Json(b): Json<Revocation>,
) -> Result<Json<Value>, ApiError> {
    let note = b.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    if !onboarding_approved(&s, &org).await? {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "the business details are no longer approved; a person must approve them again first",
        ));
    }
    change_live_access(&s, &p, &org, "revoked", "approved", note).await
}

/// Move a review between two states, with the record, the event and the email that go with it.
async fn change_live_access(
    s: &AppState,
    p: &PlatformAuth,
    org: &str,
    from: &str,
    to: &str,
    note: Option<&str>,
) -> Result<Json<Value>, ApiError> {
    let tid = test_tenant_of(s, org).await?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    let cur: Option<(String, String)> = sqlx::query_as(
        "SELECT status, requested_by FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, requested_by) = cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != from {
        return Err(ApiError::conflict(
            "wrong_state",
            format!("this review is {status}, not {from}"),
        ));
    }
    sqlx::query("UPDATE live_access SET status = $2, reviewed_by = $3, reviewed_at = now(), note = $4 WHERE organisation_id = $1")
        .bind(org)
        .bind(to)
        .bind(&p.id)
        .bind(note)
        .execute(&mut *tx)
        .await?;
    let actor = format!("zyvor:{}", p.name);
    let event = if to == "revoked" {
        "live_access.revoked"
    } else {
        "live_access.restored"
    };
    tenant::audit(&mut tx, &tid, &actor, event, None, json!({ "note": note })).await?;
    tenant::emit(
        &mut tx,
        &tid,
        event,
        json!({ "reviewed_by": actor, "note": note }),
    )
    .await?;
    tx.commit().await?;
    let (subject, body) = if to == "revoked" {
        (
            "Zyvor has paused live mode for your business".to_string(),
            format!(
                "Zyvor has taken live mode away from your business, so new payments and payouts are stopped. Your records stay readable and refunds still work.\n\nWhy: {}\n\nContact Zyvor to have it reviewed again. Details are on the Onboarding page: {}/onboarding\n",
                note.unwrap_or(""),
                s.console_url
            ),
        )
    } else {
        (
            "Live mode is open again for your business".to_string(),
            format!(
                "Zyvor has reviewed your business again and opened live mode.\n\nSign in: {}\n",
                s.console_url
            ),
        )
    };
    if let Some(to_addr) = requested_by.strip_prefix("user:").or(Some(&requested_by))
        && to_addr.contains('@')
    {
        crate::mail::enqueue(&s.pool, to_addr, &subject, &body).await?;
    }
    Ok(Json(json!({ "organisation_id": org, "status": to })))
}

/// How many requests are waiting for a reviewer (for the `/metrics` gauge and an alert).
pub async fn pending_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM live_access WHERE status = 'pending'")
        .fetch_one(pool)
        .await
        .unwrap_or(-1)
}
