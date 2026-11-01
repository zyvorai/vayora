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

/// When live access needs two different reviewers to approve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwoPerson {
    /// Only when a reviewer escalates it.
    Never,
    /// When a rule below fires, or a reviewer escalates it.
    Flagged,
    /// Always.
    Always,
}

impl TwoPerson {
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_lowercase().as_str() {
            "never" => Some(Self::Never),
            "flagged" => Some(Self::Flagged),
            "always" => Some(Self::Always),
            _ => None,
        }
    }
}

/// Why this review needs a second reviewer; empty when one is enough. A business with a check that did not pass, or
/// with no check at all (nothing but a person's eyes stands behind it), or that has been turned back twice already,
/// is the unusual case a single reviewer should not wave through alone.
pub fn second_reviewer_reasons(
    policy: TwoPerson,
    checks_total: i64,
    checks_not_passed: i64,
    round: i32,
    escalated: bool,
) -> Vec<String> {
    let mut why = Vec::new();
    if escalated {
        why.push("a reviewer asked for a second opinion".to_string());
    }
    match policy {
        TwoPerson::Never => {}
        TwoPerson::Always => why.push("every approval needs two reviewers".to_string()),
        TwoPerson::Flagged => {
            if checks_total == 0 {
                why.push("no automatic check ran".to_string());
            }
            if checks_not_passed > 0 {
                why.push(format!("{checks_not_passed} check(s) did not pass"));
            }
            if round >= 3 {
                why.push(format!("the business has asked {round} times"));
            }
        }
    }
    why
}

/// Why this change needs a second reviewer; empty when one is enough. `flagged` is what was unclear when the
/// business made it. A reviewer's escalation counts under every policy, `never` included.
pub fn change_second_reviewer_reasons(
    policy: TwoPerson,
    flagged: &[String],
    escalated: bool,
) -> Vec<String> {
    let mut why = Vec::new();
    if escalated {
        why.push("a reviewer asked for a second opinion".to_string());
    }
    match policy {
        TwoPerson::Never => {}
        TwoPerson::Always => why.push("every approval needs two reviewers".to_string()),
        TwoPerson::Flagged => why.extend(flagged.iter().cloned()),
    }
    why
}

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
    let done = sqlx::query(
        "UPDATE platform_reviewers SET disabled_at = now() WHERE id = $1 AND disabled_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected()
        == 1;
    if done {
        // whatever they were holding goes back to the queue
        sqlx::query(
            "UPDATE live_access SET assigned_to = NULL, assigned_at = NULL WHERE assigned_to = $1",
        )
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(done)
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
    // changes made after approval that Zyvor has not settled yet (and the last few it has)
    let changes: Vec<(String, String, String, String, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, kind, summary, status, note, requested_at FROM live_changes WHERE organisation_id = $1 ORDER BY requested_at DESC LIMIT 5",
    )
    .bind(&org)
    .fetch_all(&s.pool)
    .await?;
    let changes: Vec<Value> = changes
        .into_iter()
        .map(|c| json!({ "id": c.0, "kind": c.1, "summary": c.2, "status": c.3, "note": c.4, "expected_by": (c.3 == "pending").then(|| c.5 + chrono::Duration::hours(s.change_sla_hours)) }))
        .collect();
    Ok(Json(match row {
        Some((st, note, at, rev, round)) => {
            json!({ "status": st, "note": note, "requested_at": at, "reviewed_at": rev, "round": round, "onboarding_approved": ready, "changes": changes,
                "expected_by": (st == "pending").then(|| at + chrono::Duration::hours(s.review_sla_hours)), "sla_hours": s.review_sla_hours })
        }
        None => {
            json!({ "status": "not_requested", "note": null, "onboarding_approved": ready, "changes": changes })
        }
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
            sqlx::query("UPDATE live_access SET status = 'pending', requested_by = $2, requested_at = now(), reviewed_by = NULL, reviewed_at = NULL, note = NULL, round = round + 1, first_approved_by = NULL, first_approved_at = NULL, first_note = NULL, escalated_by = NULL, escalated_note = NULL, sla_alerted_at = NULL, assigned_to = NULL, assigned_at = NULL WHERE organisation_id = $1")
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
    /// `me`, `none` (nobody has it) or a reviewer id.
    assigned: Option<String>,
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
    p: PlatformAuth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    // `none` is "nobody"; anything else names a reviewer (`me` is the caller)
    let (want_none, want) = match q.assigned.as_deref() {
        None | Some("") => (false, None),
        Some("none") => (true, None),
        Some("me") => (false, Some(p.id.clone())),
        Some(id) => (false, Some(id.to_string())),
    };
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, chrono::DateTime<chrono::Utc>, i32, bool, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT l.organisation_id, o.name, l.status, l.requested_by, l.requested_at, l.round, l.escalated_by IS NOT NULL, f.name, l.assigned_to, a.name
         FROM live_access l JOIN organisations o ON o.id = l.organisation_id LEFT JOIN platform_reviewers f ON f.id = l.first_approved_by
         LEFT JOIN platform_reviewers a ON a.id = l.assigned_to
         WHERE ($1::text IS NULL OR l.status = $1)
           AND (NOT $2 OR l.assigned_to IS NULL)
           AND ($3::text IS NULL OR l.assigned_to = $3)
         ORDER BY l.requested_at LIMIT 100",
    )
    .bind(&q.status)
    .bind(want_none)
    .bind(&want)
    .fetch_all(&s.pool)
    .await?;
    let mut out = Vec::new();
    for (org, name, status, by, at, round, escalated, first, assignee, assignee_name) in rows {
        let d = detail(&s, &org).await?;
        let checks = d["checks"].as_array().cloned().unwrap_or_default();
        let failed = checks.iter().filter(|x| x["outcome"] != "pass").count();
        let why = second_reviewer_reasons(
            s.review_two_person,
            checks.len() as i64,
            failed as i64,
            round,
            escalated,
        );
        let (due, overdue) = sla(at, s.review_sla_hours, status == "pending");
        out.push(json!({ "organisation_id": org, "organisation": name, "status": status, "requested_by": by, "requested_at": at, "round": round, "legal_name": d["profile"]["legal_name"], "checks_failed": failed,
            "stage": if status == "pending" && first.is_some() { "awaiting_second" } else { "new" }, "first_approved_by": first, "needs_second_because": why,
            "assigned_to": assignee, "assigned_to_name": assignee_name, "assigned_to_me": assignee.as_deref() == Some(p.id.as_str()),
            "due_at": due, "overdue": overdue, "sla_hours": s.review_sla_hours }));
    }
    Ok(Json(json!({ "data": out })))
}

/// `GET /v1/platform/reviews/{organisation_id}`
pub async fn show_review(
    State(s): State<AppState>,
    p: PlatformAuth,
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
    #[allow(clippy::type_complexity)]
    let (escalated, first, esc_note, assignee, assignee_name, assigned_at): (bool, Option<String>, Option<String>, Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT l.escalated_by IS NOT NULL, f.name, l.escalated_note, l.assigned_to, a.name, l.assigned_at FROM live_access l LEFT JOIN platform_reviewers f ON f.id = l.first_approved_by LEFT JOIN platform_reviewers a ON a.id = l.assigned_to WHERE l.organisation_id = $1",
    )
    .bind(&org)
    .fetch_one(&s.pool)
    .await?;
    let checks = d["checks"].as_array().cloned().unwrap_or_default();
    let failed = checks.iter().filter(|x| x["outcome"] != "pass").count();
    let (due, overdue) = sla(at, s.review_sla_hours, status == "pending");
    d["needs_second_because"] = json!(second_reviewer_reasons(
        s.review_two_person,
        checks.len() as i64,
        failed as i64,
        round,
        escalated
    ));
    d["stage"] = json!(if status == "pending" && first.is_some() {
        "awaiting_second"
    } else {
        "new"
    });
    d["first_approved_by"] = json!(first);
    d["escalation_note"] = json!(esc_note);
    d["restore_needs_other"] =
        json!(status == "revoked" && restore_needs_other(s.review_two_person));
    d["reviewed_by_name"] = json!(
        sqlx::query_scalar::<_, String>("SELECT name FROM platform_reviewers WHERE id = $1")
            .bind(&reviewer)
            .fetch_optional(&s.pool)
            .await?
    );
    d["assigned_to_me"] = json!(assignee.as_deref() == Some(p.id.as_str()));
    d["assigned_to"] = json!(assignee);
    d["assigned_to_name"] = json!(assignee_name);
    d["assigned_at"] = json!(assigned_at);
    d["due_at"] = json!(due);
    d["overdue"] = json!(overdue);
    d["sla_hours"] = json!(s.review_sla_hours);
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
    #[allow(clippy::type_complexity)]
    let cur: Option<(String, String, i32, bool, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT status, requested_by, round, escalated_by IS NOT NULL, first_approved_by, assigned_to FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(&org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, requested_by, round, escalated, first_by, assignee) =
        cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this review is already {status}"),
        ));
    }
    if let Some(a) = assignee.as_deref().filter(|a| *a != p.id) {
        let who: Option<String> =
            sqlx::query_scalar("SELECT name FROM platform_reviewers WHERE id = $1")
                .bind(a)
                .fetch_optional(&mut *tx)
                .await?;
        return Err(ApiError::conflict(
            "assigned_to_other",
            format!(
                "{} has this review; ask them, or take it over first",
                who.unwrap_or_else(|| a.to_string())
            ),
        ));
    }
    if approve && !onboarding_approved(&s, &org).await? {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "the business details are no longer approved",
        ));
    }
    // Some approvals need two different reviewers. The first is recorded and the review stays pending.
    let mut first_name: Option<String> = None;
    if approve {
        let (total, not_passed): (i64, i64) = sqlx::query_as(
            "SELECT count(*), count(*) FILTER (WHERE outcome <> 'pass') FROM kyc_checks",
        )
        .fetch_one(&mut *tx)
        .await?;
        let why = second_reviewer_reasons(s.review_two_person, total, not_passed, round, escalated);
        if !why.is_empty() {
            match first_by.as_deref() {
                None => {
                    sqlx::query("UPDATE live_access SET first_approved_by = $2, first_approved_at = now(), first_note = $3, assigned_to = NULL, assigned_at = NULL WHERE organisation_id = $1")
                        .bind(&org)
                        .bind(&p.id)
                        .bind(note)
                        .execute(&mut *tx)
                        .await?;
                    tenant::audit(
                        &mut tx,
                        &tid,
                        &format!("zyvor:{}", p.name),
                        "live_access.first_approval",
                        None,
                        json!({ "note": note, "needs_second_because": why }),
                    )
                    .await?;
                    tx.commit().await?;
                    return Ok(Json(json!({
                        "organisation_id": org, "status": "pending", "stage": "awaiting_second", "needs_second_because": why,
                    })));
                }
                Some(f) if f == p.id => {
                    return Err(ApiError::conflict(
                        "second_reviewer_required",
                        format!(
                            "this approval needs a second, different reviewer ({})",
                            why.join("; ")
                        ),
                    ));
                }
                Some(f) => {
                    first_name =
                        sqlx::query_scalar("SELECT name FROM platform_reviewers WHERE id = $1")
                            .bind(f)
                            .fetch_optional(&mut *tx)
                            .await?;
                }
            }
        }
    }
    let new = if approve { "approved" } else { "rejected" };
    sqlx::query("UPDATE live_access SET status = $2, reviewed_by = $3, reviewed_at = now(), note = $4, assigned_to = NULL, assigned_at = NULL WHERE organisation_id = $1")
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
        json!({ "note": note, "first_approved_by": first_name }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &tid,
        &format!("live_access.{new}"),
        json!({ "reviewed_by": actor, "note": note, "first_approved_by": first_name }),
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

#[derive(Deserialize)]
pub struct ChangeDecision {
    decision: String,
    note: Option<String>,
}

/// `GET /v1/platform/changes?status=pending`: changes a business made after Zyvor approved it, waiting for a reviewer.
pub async fn list_changes(
    State(s): State<AppState>,
    _p: PlatformAuth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, String, String, String, String, chrono::DateTime<chrono::Utc>, Option<String>, Vec<String>, Option<String>, bool, Option<String>)> = sqlx::query_as(
        "SELECT c.id, c.organisation_id, o.name, c.kind, c.summary, c.status, c.requested_by, c.requested_at, c.note, c.flagged_because, f.name, c.escalated_by IS NOT NULL, c.escalated_note
         FROM live_changes c JOIN organisations o ON o.id = c.organisation_id LEFT JOIN platform_reviewers f ON f.id = c.first_approved_by
         WHERE ($1::text IS NULL OR c.status = $1) ORDER BY c.requested_at LIMIT 100",
    )
    .bind(&q.status)
    .fetch_all(&s.pool)
    .await?;
    Ok(Json(json!({ "data": rows.into_iter().map(|r| {
        let (due, overdue) = sla(r.7, s.change_sla_hours, r.5 == "pending");
        json!({
            "id": r.0, "organisation_id": r.1, "organisation": r.2, "kind": r.3, "summary": r.4,
            "status": r.5, "requested_by": r.6, "requested_at": r.7, "note": r.8,
            "due_at": due, "overdue": overdue, "sla_hours": s.change_sla_hours,
            "stage": if r.5 == "pending" && r.10.is_some() { "awaiting_second" } else { "new" },
            "first_approved_by": r.10,
            "needs_second_because": change_second_reviewer_reasons(s.review_two_person, &r.9, r.11),
            "escalation_note": r.12,
        })
    }).collect::<Vec<_>>() })))
}

/// `POST /v1/platform/changes/{id}/escalate`: ask for a second, different reviewer on a pending change that no rule
/// flagged. The note says why; the second reviewer reads it. Payouts stay held either way.
pub async fn escalate_change(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(id): Path<String>,
    Json(b): Json<Escalation>,
) -> Result<Json<Value>, ApiError> {
    let Some(note) = b.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) else {
        return Err(ApiError::invalid(
            "note_required",
            "say why a second reviewer should look; they will read it",
        ));
    };
    let tid: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM live_changes WHERE id = $1")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await?;
    let tid = tid.ok_or_else(|| ApiError::not_found("change"))?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    let cur: Option<(String, bool, String)> = sqlx::query_as(
        "SELECT status, escalated_by IS NOT NULL, ref_id FROM live_changes WHERE id = $1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, already, ref_id) = cur.ok_or_else(|| ApiError::not_found("change"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this change is already {status}"),
        ));
    }
    if !already {
        sqlx::query("UPDATE live_changes SET escalated_by = $2, escalated_note = $3 WHERE id = $1")
            .bind(&id)
            .bind(&p.id)
            .bind(note)
            .execute(&mut *tx)
            .await?;
        tenant::audit(
            &mut tx,
            &tid,
            &format!("zyvor:{}", p.name),
            "live_change.escalated",
            Some(&ref_id),
            json!({ "note": note }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "status": "pending", "escalated": true }),
    ))
}

/// `POST /v1/platform/changes/{id}/decision`: approve, or reject with a note (the account is then removed and the
/// merchant is told why). Either way the hold on live payouts ends once nothing else is waiting.
pub async fn decide_change(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(id): Path<String>,
    Json(b): Json<ChangeDecision>,
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
    let tid: Option<String> =
        sqlx::query_scalar("SELECT tenant_id FROM live_changes WHERE id = $1")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await?;
    let tid = tid.ok_or_else(|| ApiError::not_found("change"))?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    #[allow(clippy::type_complexity)]
    let cur: Option<(String, String, String, String, String, Vec<String>, Option<String>, bool)> = sqlx::query_as(
        "SELECT status, kind, ref_id, requested_by, summary, flagged_because, first_approved_by, escalated_by IS NOT NULL FROM live_changes WHERE id = $1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, kind, ref_id, requested_by, summary, flagged, first_by, escalated) =
        cur.ok_or_else(|| ApiError::not_found("change"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this change is already {status}"),
        ));
    }
    // An approval that warrants it needs two different reviewers: the first is recorded and the change (and the hold
    // on live payouts) stays as it is. Rejecting never needs two.
    if approve {
        let why = change_second_reviewer_reasons(s.review_two_person, &flagged, escalated);
        if !why.is_empty() {
            match first_by.as_deref() {
                None => {
                    sqlx::query("UPDATE live_changes SET first_approved_by = $2, first_approved_at = now(), first_note = $3 WHERE id = $1")
                        .bind(&id)
                        .bind(&p.id)
                        .bind(note)
                        .execute(&mut *tx)
                        .await?;
                    tenant::audit(
                        &mut tx,
                        &tid,
                        &format!("zyvor:{}", p.name),
                        "live_change.first_approval",
                        Some(&ref_id),
                        json!({ "kind": kind, "note": note, "needs_second_because": why }),
                    )
                    .await?;
                    tx.commit().await?;
                    return Ok(Json(json!({
                        "id": id, "status": "pending", "stage": "awaiting_second", "needs_second_because": why,
                    })));
                }
                Some(f) if f == p.id => {
                    return Err(ApiError::conflict(
                        "second_reviewer_required",
                        format!(
                            "this approval needs a second, different reviewer ({})",
                            why.join("; ")
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    let new = if approve { "approved" } else { "rejected" };
    sqlx::query("UPDATE live_changes SET status = $2, reviewed_by = $3, reviewed_at = now(), note = $4 WHERE id = $1")
        .bind(&id)
        .bind(new)
        .bind(&p.id)
        .bind(note)
        .execute(&mut *tx)
        .await?;
    if !approve && kind == "bank_account" {
        // the account Zyvor did not accept is removed; the review record keeps what it was
        sqlx::query("DELETE FROM merchant_bank_accounts WHERE id = $1")
            .bind(&ref_id)
            .execute(&mut *tx)
            .await?;
    }
    if approve && kind == "profile" {
        // the details Zyvor accepted replace the old ones in every tenant of the business (test and live each keep a
        // copy); each is row-level secured, so switch to it for its own update
        let tenants: Vec<String> = sqlx::query_scalar(
            "SELECT t.id FROM tenants t JOIN live_changes c ON c.organisation_id = t.organisation_id WHERE c.id = $1",
        )
        .bind(&id)
        .fetch_all(&mut *tx)
        .await?;
        for t in &tenants {
            sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
                .bind(t)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE merchant_profiles m SET legal_name = c.legal_name, business_type = c.business_type, pan_last4 = c.pan_last4, gstin = c.gstin, address = c.address, updated_at = now() FROM profile_changes c WHERE c.change_id = $1 AND m.tenant_id = $2")
                .bind(&id)
                .bind(t)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
            .bind(&tid)
            .execute(&mut *tx)
            .await?;
    }
    let actor = format!("zyvor:{}", p.name);
    let event = format!("live_change.{new}");
    tenant::audit(
        &mut tx,
        &tid,
        &actor,
        &event,
        Some(&ref_id),
        json!({ "kind": kind, "note": note }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &tid,
        &event,
        json!({ "id": id, "kind": kind, "summary": summary, "reviewed_by": actor, "note": note }),
    )
    .await?;
    tx.commit().await?;
    let (subject, body) = if approve {
        (
            "Zyvor approved the change to your business".to_string(),
            format!(
                "Zyvor reviewed this change and approved it: {summary}.\n\n{}Payouts are no longer held for it: {}\n",
                if kind == "profile" {
                    "Your business details are now the new ones.\n\n"
                } else {
                    ""
                },
                s.console_url
            ),
        )
    } else {
        (
            "Zyvor did not approve a change to your business".to_string(),
            format!(
                "Zyvor reviewed this change and did not approve it: {summary}.\n{}\nWhy: {}\n\nYou can try again from the Onboarding page: {}/onboarding\n",
                if kind == "profile" {
                    "Your business details stay as they were."
                } else {
                    "The account was removed."
                },
                note.unwrap_or(""),
                s.console_url
            ),
        )
    };
    let to_addr = requested_by.strip_prefix("user:").unwrap_or(&requested_by);
    if to_addr.contains('@') {
        crate::mail::enqueue(&s.pool, to_addr, &subject, &body).await?;
    }
    Ok(Json(json!({ "id": id, "status": new })))
}

/// When a request is due and whether it is already late; only a pending one can be late.
fn sla(
    asked: chrono::DateTime<chrono::Utc>,
    hours: i64,
    pending: bool,
) -> (chrono::DateTime<chrono::Utc>, bool) {
    let due = asked + chrono::Duration::hours(hours);
    (due, pending && chrono::Utc::now() > due)
}

#[derive(Deserialize)]
pub struct Assignment {
    /// `me`, a reviewer id, or absent / `none` to put it back in the queue.
    to: Option<String>,
    /// Say so on purpose to touch a review another reviewer holds.
    #[serde(default)]
    take_over: bool,
}

/// `POST /v1/platform/reviews/{organisation_id}/assign`: give a pending review to one reviewer (`to: "me"` claims
/// it), or hand it back. Only the holder decides it while it is held; taking one from someone else is explicit and
/// audited. Nothing is held by a reviewer who has been disabled.
pub async fn assign(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(org): Path<String>,
    Json(b): Json<Assignment>,
) -> Result<Json<Value>, ApiError> {
    let target: Option<String> = match b.to.as_deref().map(str::trim) {
        None | Some("") | Some("none") => None,
        Some("me") => Some(p.id.clone()),
        Some(id) => Some(id.to_string()),
    };
    let tid = test_tenant_of(&s, &org).await?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    let cur: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT status, assigned_to, first_approved_by FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(&org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, holder, first_by) = cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this review is already {status}"),
        ));
    }
    if holder.as_deref().is_some_and(|h| h != p.id) && !b.take_over {
        let who: Option<String> =
            sqlx::query_scalar("SELECT name FROM platform_reviewers WHERE id = $1")
                .bind(holder.as_deref())
                .fetch_optional(&mut *tx)
                .await?;
        return Err(ApiError::conflict(
            "assigned_to_other",
            format!(
                "{} has this review; say take_over to change that",
                who.unwrap_or_default()
            ),
        ));
    }
    let mut to_name: Option<String> = None;
    if let Some(t) = &target {
        to_name = sqlx::query_scalar(
            "SELECT name FROM platform_reviewers WHERE id = $1 AND disabled_at IS NULL",
        )
        .bind(t)
        .fetch_optional(&mut *tx)
        .await?;
        if to_name.is_none() {
            return Err(ApiError::invalid(
                "unknown_reviewer",
                "that reviewer does not exist or is disabled",
            ));
        }
        if first_by.as_deref() == Some(t.as_str()) {
            return Err(ApiError::conflict(
                "first_reviewer_cannot_second",
                "that reviewer gave the first approval; the second has to be someone else",
            ));
        }
    }
    sqlx::query("UPDATE live_access SET assigned_to = $2, assigned_at = CASE WHEN $2::text IS NULL THEN NULL ELSE now() END WHERE organisation_id = $1")
        .bind(&org)
        .bind(&target)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &tid,
        &format!("zyvor:{}", p.name),
        "live_access.assigned",
        None,
        json!({ "to": target, "to_name": to_name, "from": holder, "took_over": holder.as_deref().is_some_and(|h| h != p.id) }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "organisation_id": org, "assigned_to": target, "assigned_to_name": to_name }),
    ))
}

#[derive(Deserialize)]
pub struct Escalation {
    note: Option<String>,
}

/// `POST /v1/platform/reviews/{organisation_id}/escalate`: ask for a second reviewer on a pending review that no
/// rule flagged (a feeling is a reason). The note says why; the second reviewer sees it.
pub async fn escalate(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(org): Path<String>,
    Json(b): Json<Escalation>,
) -> Result<Json<Value>, ApiError> {
    let note = b.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let Some(note) = note else {
        return Err(ApiError::invalid(
            "note_required",
            "say why a second reviewer should look; they will read it",
        ));
    };
    let tid = test_tenant_of(&s, &org).await?;
    let mut tx = tenant::begin(&s.pool, &tid).await?;
    let cur: Option<(String, bool)> = sqlx::query_as(
        "SELECT status, escalated_by IS NOT NULL FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(&org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, already) = cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != "pending" {
        return Err(ApiError::conflict(
            "not_pending",
            format!("this review is already {status}"),
        ));
    }
    if !already {
        sqlx::query("UPDATE live_access SET escalated_by = $2, escalated_note = $3 WHERE organisation_id = $1")
            .bind(&org)
            .bind(&p.id)
            .bind(note)
            .execute(&mut *tx)
            .await?;
        tenant::audit(
            &mut tx,
            &tid,
            &format!("zyvor:{}", p.name),
            "live_access.escalated",
            None,
            json!({ "note": note }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({ "organisation_id": org, "status": "pending", "escalated": true }),
    ))
}

/// How many requests and changes are past their target, and the age in seconds of the oldest thing waiting. Any
/// count is `-1` if the database could not be asked.
pub async fn overdue(s: &AppState) -> (i64, i64, i64) {
    let access = sqlx::query_scalar("SELECT count(*) FROM live_access WHERE status = 'pending' AND requested_at < now() - make_interval(hours => $1::int)")
        .bind(s.review_sla_hours as i32)
        .fetch_one(&s.pool)
        .await
        .unwrap_or(-1);
    let changes = sqlx::query_scalar("SELECT count(*) FROM live_changes WHERE status = 'pending' AND requested_at < now() - make_interval(hours => $1::int)")
        .bind(s.change_sla_hours as i32)
        .fetch_one(&s.pool)
        .await
        .unwrap_or(-1);
    let oldest: Option<f64> = sqlx::query_scalar("SELECT extract(epoch FROM now() - min(t))::float8 FROM (SELECT requested_at AS t FROM live_access WHERE status = 'pending' UNION ALL SELECT requested_at FROM live_changes WHERE status = 'pending') w")
        .fetch_one(&s.pool)
        .await
        .unwrap_or(None);
    (access, changes, oldest.map_or(0, |v| v.max(0.0) as i64))
}

/// Email the operator (`ZYVOR_REVIEW_ALERT_EMAIL`) once for each request or change that has waited past its
/// target. Returns how many were reported; without an address nothing is sent and nothing is marked, so setting
/// one later still reports what is already late.
pub async fn notify_overdue(s: &AppState) -> Result<usize, ApiError> {
    let Some(to) = s.review_alert_email.as_deref() else {
        return Ok(0);
    };
    let mut sent = 0;
    let mut tx = s.pool.begin().await?;
    let late: Vec<(String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT l.organisation_id, o.name, l.requested_at FROM live_access l JOIN organisations o ON o.id = l.organisation_id
         WHERE l.status = 'pending' AND l.sla_alerted_at IS NULL AND l.requested_at < now() - make_interval(hours => $1::int)
         ORDER BY l.requested_at FOR UPDATE OF l SKIP LOCKED LIMIT 20",
    )
    .bind(s.review_sla_hours as i32)
    .fetch_all(&mut *tx)
    .await?;
    for (org, name, at) in late {
        sqlx::query("UPDATE live_access SET sla_alerted_at = now() WHERE organisation_id = $1")
            .bind(&org)
            .execute(&mut *tx)
            .await?;
        crate::mail::enqueue(
            &s.pool,
            to,
            &format!("Live access request waiting past {} hours: {name}", s.review_sla_hours),
            &format!("{name} ({org}) asked for live access on {} and has not had an answer within the {}-hour target.\n\nReviewer page: {}/reviewer\n", at.format("%d %b %Y %H:%M UTC"), s.review_sla_hours, s.console_url),
        )
        .await?;
        sent += 1;
    }
    let late: Vec<(String, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT c.id, o.name, c.summary, c.requested_at FROM live_changes c JOIN organisations o ON o.id = c.organisation_id
         WHERE c.status = 'pending' AND c.sla_alerted_at IS NULL AND c.requested_at < now() - make_interval(hours => $1::int)
         ORDER BY c.requested_at FOR UPDATE OF c SKIP LOCKED LIMIT 20",
    )
    .bind(s.change_sla_hours as i32)
    .fetch_all(&mut *tx)
    .await?;
    for (id, name, summary, at) in late {
        sqlx::query("UPDATE live_changes SET sla_alerted_at = now() WHERE id = $1")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        crate::mail::enqueue(
            &s.pool,
            to,
            &format!("Change waiting past {} hours, payouts held: {name}", s.change_sla_hours),
            &format!("{name} added {summary} on {} and its live payouts are held until a reviewer decides. The {}-hour target has passed.\n\nReviewer page: {}/reviewer\n", at.format("%d %b %Y %H:%M UTC"), s.change_sla_hours, s.console_url),
        )
        .await?;
        sent += 1;
    }
    tx.commit().await?;
    Ok(sent)
}

/// Changes waiting for a reviewer (for the `/metrics` gauge and an alert).
pub async fn pending_changes(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM live_changes WHERE status = 'pending'")
        .fetch_one(pool)
        .await
        .unwrap_or(-1)
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

/// Whether giving live mode back must be done by someone other than the reviewer who took it away. Only `never`
/// lets one reviewer undo their own revoke.
pub fn restore_needs_other(policy: TwoPerson) -> bool {
    !matches!(policy, TwoPerson::Never)
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
    let cur: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT status, requested_by, reviewed_by FROM live_access WHERE organisation_id = $1 FOR UPDATE",
    )
    .bind(org)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, requested_by, last_by) = cur.ok_or_else(|| ApiError::not_found("review"))?;
    if status != from {
        return Err(ApiError::conflict(
            "wrong_state",
            format!("this review is {status}, not {from}"),
        ));
    }
    // Taking live mode away is a protective act, so one reviewer does it at once. Giving it back reopens the risk,
    // so a different reviewer than the one who took it away has to agree.
    if to == "approved"
        && restore_needs_other(s.review_two_person)
        && last_by.as_deref() == Some(p.id.as_str())
    {
        return Err(ApiError::conflict(
            "second_reviewer_required",
            "you took live mode away from this business; a different reviewer has to give it back",
        ));
    }
    sqlx::query("UPDATE live_access SET status = $2, reviewed_by = $3, reviewed_at = now(), note = $4, assigned_to = NULL, assigned_at = NULL WHERE organisation_id = $1")
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
    tenant::audit(
        &mut tx,
        &tid,
        &actor,
        event,
        None,
        json!({ "note": note, "revoked_by": if to == "approved" { last_by.clone() } else { None } }),
    )
    .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_first_request_needs_one_reviewer_and_anything_odd_needs_two() {
        let why = second_reviewer_reasons;
        // everything passed, first time
        assert!(why(TwoPerson::Flagged, 3, 0, 1, false).is_empty());
        // a check did not pass, no check ran, asked three times
        assert_eq!(why(TwoPerson::Flagged, 3, 1, 1, false).len(), 1);
        assert_eq!(why(TwoPerson::Flagged, 0, 0, 1, false).len(), 1);
        assert_eq!(why(TwoPerson::Flagged, 3, 0, 3, false).len(), 1);
        assert_eq!(why(TwoPerson::Flagged, 0, 0, 3, false).len(), 2);
        // always, never, and an escalation that beats never
        assert_eq!(why(TwoPerson::Always, 3, 0, 1, false).len(), 1);
        assert!(why(TwoPerson::Never, 0, 2, 5, false).is_empty());
        assert_eq!(why(TwoPerson::Never, 3, 0, 1, true).len(), 1);
    }

    #[test]
    fn the_policy_names_parse_and_nothing_else_does() {
        assert_eq!(TwoPerson::parse(" Always "), Some(TwoPerson::Always));
        assert_eq!(TwoPerson::parse("flagged"), Some(TwoPerson::Flagged));
        assert_eq!(TwoPerson::parse("NEVER"), Some(TwoPerson::Never));
        assert_eq!(TwoPerson::parse("sometimes"), None);
    }

    #[test]
    fn only_a_pending_request_can_be_late() {
        let long_ago = chrono::Utc::now() - chrono::Duration::hours(30);
        assert!(sla(long_ago, 24, true).1);
        assert!(!sla(long_ago, 24, false).1);
        assert!(!sla(chrono::Utc::now() - chrono::Duration::hours(2), 24, true).1);
    }
}
