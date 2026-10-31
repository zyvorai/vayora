//! Merchant onboarding: a profile, document references and bank accounts that a different person
//! reviews before the merchant is approved. Zyvor records the decision; it does not perform KYC.
//! Only references and last-four digits are kept. A tenant with no profile is unaffected; once a
//! profile exists, payouts wait for approval.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, gst, kyc, tenant};

async fn record_check(
    conn: &mut PgConnection,
    tenant_id: &str,
    kind: &str,
    subject: &str,
    c: &kyc::Check,
    provider: Option<&kyc::Kyc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO kyc_checks (id, tenant_id, kind, subject, outcome, detail, provider) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(new_id("kyc")).bind(tenant_id).bind(kind).bind(subject).bind(c.outcome.as_str()).bind(&c.detail)
        .bind(provider.map_or("none", |k| k.name()))
        .execute(conn).await?;
    Ok(())
}

/// A paid check provider is only called this many times a day per business. Signup is public, so without a cap
/// anyone could use it to run up the platform's bill. Past the cap the checks come back "a person will review".
const MAX_PAID_CHECKS_PER_DAY: i64 = 12;

/// Count `calls` paid provider calls for this business, or say there is no room left today. Free providers
/// (the simulator) are never counted.
async fn paid_checks_allowed(
    s: &AppState,
    tenant_id: &str,
    k: &kyc::Kyc,
    calls: i64,
) -> Result<bool, sqlx::Error> {
    if !k.is_metered() {
        return Ok(true);
    }
    let used: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kyc_usage WHERE tenant_id = $1 AND at > now() - interval '24 hours'",
    )
    .bind(tenant_id)
    .fetch_one(&s.pool)
    .await?;
    if used + calls > MAX_PAID_CHECKS_PER_DAY {
        return Ok(false);
    }
    for _ in 0..calls {
        sqlx::query("INSERT INTO kyc_usage (tenant_id, kind) VALUES ($1, $2)")
            .bind(tenant_id)
            .bind(k.name())
            .execute(&s.pool)
            .await?;
    }
    Ok(true)
}

fn capped() -> kyc::Check {
    kyc::Check {
        outcome: kyc::Outcome::Inconclusive,
        detail: "too many automatic checks for this business today; a person will review this"
            .into(),
        registered_name: None,
    }
}

const BUSINESS_TYPES: [&str; 6] = [
    "proprietorship",
    "partnership",
    "llp",
    "private_limited",
    "trust_or_society",
    "other",
];
const DOC_KINDS: [&str; 6] = [
    "pan",
    "gst_certificate",
    "incorporation",
    "address_proof",
    "cancelled_cheque",
    "other",
];

/// `AAAAA9999A`
pub fn valid_pan(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() == 10
        && b[..5].iter().all(u8::is_ascii_uppercase)
        && b[5..9].iter().all(u8::is_ascii_digit)
        && b[9].is_ascii_uppercase()
}

/// `HDFC0001234`: four letters, a zero, six letters or digits.
pub fn valid_ifsc(i: &str) -> bool {
    let b = i.as_bytes();
    b.len() == 11
        && b[..4].iter().all(u8::is_ascii_uppercase)
        && b[4] == b'0'
        && b[5..].iter().all(u8::is_ascii_alphanumeric)
}

/// Payouts wait for approval once onboarding has been started.
pub async fn require_approved(conn: &mut PgConnection) -> Result<(), ApiError> {
    let st: Option<String> = sqlx::query_scalar("SELECT status FROM merchant_profiles")
        .fetch_optional(&mut *conn)
        .await?;
    match st.as_deref() {
        None | Some("approved") => Ok(()),
        Some(s) => Err(ApiError::conflict(
            "onboarding_incomplete",
            format!("payouts open once onboarding is approved (now {s})"),
        )),
    }
}

#[derive(Deserialize)]
pub struct ProfileIn {
    legal_name: String,
    business_type: String,
    pan: String,
    gstin: Option<String>,
    address: String,
}

pub async fn put_profile(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<ProfileIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")?;
    let bad = |m: &str| ApiError::invalid("invalid_profile", m.to_string());
    if b.legal_name.trim().len() < 2 || b.address.trim().len() < 5 {
        return Err(bad("legal_name and address are required"));
    }
    if !BUSINESS_TYPES.contains(&b.business_type.as_str()) {
        return Err(bad(&format!("business_type is one of {BUSINESS_TYPES:?}")));
    }
    let pan = b.pan.trim().to_uppercase();
    if !valid_pan(&pan) {
        return Err(bad("pan must look like ABCDE1234F"));
    }
    let gstin = b
        .gstin
        .as_deref()
        .map(|g| g.trim().to_uppercase())
        .filter(|g| !g.is_empty());
    if gstin.as_deref().is_some_and(|g| !gst::valid_gstin(g)) {
        return Err(bad("gstin failed its checksum"));
    }
    // identity checks run now, while the full PAN is in hand; only their outcome is kept
    let mut checks: Vec<(&str, kyc::Check)> = Vec::new();
    if let Some(k) = s.kyc.as_ref() {
        let calls = 1 + i64::from(gstin.is_some());
        if !paid_checks_allowed(&s, &auth.tenant_id, k, calls).await? {
            checks.push(("pan", capped()));
            if gstin.is_some() {
                checks.push(("gstin", capped()));
            }
        } else {
            let mut c = k.verify_pan(&pan, b.legal_name.trim()).await;
            if c.outcome == kyc::Outcome::Pass && !kyc::pan_fits_business(&pan, &b.business_type) {
                c = kyc::Check {
                    outcome: kyc::Outcome::Inconclusive,
                    detail: "PAN holder type does not fit the business type".to_string(),
                    registered_name: None,
                };
            }
            checks.push(("pan", c));
            if let Some(g) = &gstin {
                checks.push(("gstin", k.verify_gstin(g, &pan, b.legal_name.trim()).await));
            }
        }
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st: Option<String> = sqlx::query_scalar("SELECT status FROM merchant_profiles FOR UPDATE")
        .fetch_optional(&mut *tx)
        .await?;
    if matches!(st.as_deref(), Some("in_review" | "approved")) {
        return Err(ApiError::conflict(
            "profile_locked",
            "the profile cannot change while in review or after approval",
        ));
    }
    sqlx::query("INSERT INTO merchant_profiles (tenant_id, legal_name, business_type, pan_last4, gstin, address) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (tenant_id) DO UPDATE SET legal_name = $2, business_type = $3, pan_last4 = $4, gstin = $5, address = $6, status = 'draft', updated_at = now()")
        .bind(&auth.tenant_id).bind(b.legal_name.trim()).bind(&b.business_type).bind(&pan[6..]).bind(&gstin).bind(b.address.trim())
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kyc_checks WHERE subject = 'profile'")
        .execute(&mut *tx)
        .await?;
    for (kind, c) in &checks {
        record_check(&mut tx, &auth.tenant_id, kind, "profile", c, s.kyc.as_ref()).await?;
    }
    sqlx::query("UPDATE merchant_profiles SET review_reasons = '{}'")
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.profile_saved",
        None,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "status": "draft" })))
}

#[derive(Deserialize)]
pub struct DocIn {
    kind: String,
    /// Where the file lives (your document store); Zyvor keeps only this reference.
    reference: String,
}

pub async fn add_document(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<DocIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("onboarding:write")?;
    if !DOC_KINDS.contains(&b.kind.as_str())
        || b.reference.trim().is_empty()
        || b.reference.len() > 500
    {
        return Err(ApiError::invalid(
            "invalid_document",
            format!("kind is one of {DOC_KINDS:?} and reference is required"),
        ));
    }
    let id = new_id("odoc");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO onboarding_documents (id, tenant_id, kind, reference, added_by) VALUES ($1,$2,$3,$4,$5)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.kind).bind(b.reference.trim()).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.document_added",
        Some(&id),
        json!({ "kind": b.kind }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

#[derive(Deserialize)]
pub struct BankIn {
    holder_name: String,
    ifsc: String,
    account_number: String,
}

pub async fn add_bank_account(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<BankIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("onboarding:write")?;
    let ifsc = b.ifsc.trim().to_uppercase();
    let acct = b.account_number.trim();
    if b.holder_name.trim().len() < 2
        || !valid_ifsc(&ifsc)
        || !(9..=18).contains(&acct.len())
        || !acct.chars().all(|c| c.is_ascii_digit())
    {
        return Err(ApiError::invalid(
            "invalid_bank_account",
            "holder_name, an IFSC like HDFC0001234 and a 9-18 digit account number are required",
        ));
    }
    let id = new_id("mba");
    let penny = match s.kyc.as_ref() {
        Some(k) if !paid_checks_allowed(&s, &auth.tenant_id, k, 1).await? => Some(capped()),
        Some(k) => {
            let mut c = k.verify_bank(&ifsc, acct, b.holder_name.trim()).await;
            // the bank's name for the account must be the merchant's, not just any live account
            if c.outcome == kyc::Outcome::Pass
                && let Some(n) = c.registered_name.as_deref()
                && !kyc::names_match(b.holder_name.trim(), n)
            {
                c.outcome = kyc::Outcome::Inconclusive;
                c.detail = "the account holder name does not match the name given".into();
            }
            Some(c)
        }
        None => None,
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO merchant_bank_accounts (id, tenant_id, holder_name, ifsc, account_last4, added_by) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&id).bind(&auth.tenant_id).bind(b.holder_name.trim()).bind(&ifsc).bind(&acct[acct.len() - 4..]).bind(&auth.actor)
        .execute(&mut *tx).await?;
    if let Some(c) = &penny {
        record_check(&mut tx, &auth.tenant_id, "bank", &id, c, s.kyc.as_ref()).await?;
        // A passed penny drop stands in for the second person who would otherwise verify the account, but only
        // while onboarding is still open. Once a business is submitted or approved, a new payout account is
        // checked and then verified by a different person, as without KYC.
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM merchant_profiles")
            .fetch_optional(&mut *tx)
            .await?;
        let open = !matches!(status.as_deref(), Some("in_review" | "approved"));
        if c.outcome == kyc::Outcome::Pass && open {
            sqlx::query("UPDATE merchant_bank_accounts SET verified_by = 'system:kyc', verified_at = now() WHERE id = $1")
                .bind(&id).execute(&mut *tx).await?;
        }
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.bank_account_added",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "account_last4": &acct[acct.len() - 4..], "check": penny.map(|c| json!({ "outcome": c.outcome.as_str(), "detail": c.detail })) }),
        ),
    ))
}

/// A person confirms the account really belongs to the merchant (penny-drop, cancelled cheque, or a
/// partner check). Someone other than the person who added it.
pub async fn verify_bank_account(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:review")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT added_by, verified_by FROM merchant_bank_accounts WHERE id = $1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (added_by, verified) = row.ok_or_else(|| ApiError::not_found("bank account"))?;
    if added_by == auth.actor {
        return Err(ApiError::forbidden(
            "a different person must verify a bank account".to_string(),
        ));
    }
    if verified.is_some() {
        return Err(ApiError::conflict("already_verified", "already verified"));
    }
    sqlx::query(
        "UPDATE merchant_bank_accounts SET verified_by = $2, verified_at = now() WHERE id = $1",
    )
    .bind(&id)
    .bind(&auth.actor)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.bank_account_verified",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "verified": true })))
}

pub async fn get_onboarding(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")
        .or_else(|_| auth.require("onboarding:review"))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let p: Option<(String, String, String, Option<String>, String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT legal_name, business_type, pan_last4, gstin, address, status, submitted_by, reviewed_by, review_note FROM merchant_profiles",
    ).fetch_optional(&mut *tx).await?;
    let docs: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT id, kind, reference, status FROM onboarding_documents ORDER BY created_at",
    )
    .fetch_all(&mut *tx)
    .await?;
    let banks: Vec<(String, String, String, String, bool)> = sqlx::query_as("SELECT id, holder_name, ifsc, account_last4, verified_at IS NOT NULL FROM merchant_bank_accounts ORDER BY created_at").fetch_all(&mut *tx).await?;
    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT unnest(review_reasons) FROM merchant_profiles")
            .fetch_all(&mut *tx)
            .await?;
    let checks: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT kind, subject, outcome, detail FROM kyc_checks ORDER BY created_at")
            .fetch_all(&mut *tx)
            .await?;
    Ok(Json(json!({
        "automated_checks": s.kyc.is_some(),
        "review_reasons": reasons,
        "checks": checks.into_iter().map(|c| json!({ "kind": c.0, "subject": c.1, "outcome": c.2, "detail": c.3 })).collect::<Vec<_>>(),
        "status": p.as_ref().map_or("not_started", |p| p.5.as_str()),
        "profile": p.as_ref().map(|p| json!({ "legal_name": p.0, "business_type": p.1, "pan": format!("XXXXXX{}", &p.2[..]), "gstin": p.3, "address": p.4, "submitted_by": p.6, "reviewed_by": p.7, "review_note": p.8 })),
        "documents": docs.into_iter().map(|d| json!({ "id": d.0, "kind": d.1, "reference": d.2, "status": d.3 })).collect::<Vec<_>>(),
        "bank_accounts": banks.into_iter().map(|b| json!({ "id": b.0, "holder_name": b.1, "ifsc": b.2, "account_last4": b.3, "verified": b.4 })).collect::<Vec<_>>(),
    })))
}

pub async fn submit(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st: String = sqlx::query_scalar("SELECT status FROM merchant_profiles FOR UPDATE")
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::conflict("no_profile", "save the profile first"))?;
    if !matches!(st.as_str(), "draft" | "rejected") {
        return Err(ApiError::conflict(
            "not_submittable",
            format!("onboarding is {st}"),
        ));
    }
    let docs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM onboarding_documents WHERE kind = 'pan'")
            .fetch_one(&mut *tx)
            .await?;
    let banks: i64 = sqlx::query_scalar("SELECT count(*) FROM merchant_bank_accounts")
        .fetch_one(&mut *tx)
        .await?;
    if docs == 0 || banks == 0 {
        return Err(ApiError::conflict(
            "onboarding_incomplete",
            "add a PAN document and a bank account before submitting",
        ));
    }
    let decision = if s.kyc.is_some() {
        let outcome = |kind: &str, v: String| match (kind, v.as_str()) {
            (_, "pass") => kyc::Outcome::Pass,
            (_, "fail") => kyc::Outcome::Fail,
            _ => kyc::Outcome::Inconclusive,
        };
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT kind, outcome FROM kyc_checks")
            .fetch_all(&mut *tx)
            .await?;
        let pick = |k: &str| {
            rows.iter()
                .find(|r| r.0 == k)
                .map(|r| outcome(k, r.1.clone()))
        };
        let banks: Vec<kyc::Outcome> = rows
            .iter()
            .filter(|r| r.0 == "bank")
            .map(|r| outcome("bank", r.1.clone()))
            .collect();
        let bt: String = sqlx::query_scalar("SELECT business_type FROM merchant_profiles")
            .fetch_one(&mut *tx)
            .await?;
        Some(kyc::decide(pick("pan"), pick("gstin"), &banks, &bt))
    } else {
        None
    };
    if let Some(kyc::Decision::Approve) = decision {
        sqlx::query("UPDATE merchant_profiles SET status = 'approved', submitted_by = $1, submitted_at = now(), reviewed_by = 'system:auto', reviewed_at = now(), review_note = 'Approved automatically: every identity and bank check passed', review_reasons = '{}'")
            .bind(&auth.actor).execute(&mut *tx).await?;
        tenant::audit(
            &mut tx,
            &auth.tenant_id,
            "system:auto",
            "onboarding.auto_approved",
            None,
            json!({ "submitted_by": auth.actor }),
        )
        .await?;
        tenant::emit(
            &mut tx,
            &auth.tenant_id,
            "onboarding.approved",
            json!({ "reviewed_by": "system:auto", "automatic": true }),
        )
        .await?;
        tx.commit().await?;
        return Ok(Json(
            json!({ "status": "approved", "automatic": true, "reasons": [] }),
        ));
    }
    let reasons = match decision {
        Some(kyc::Decision::Review(r)) => r,
        _ => Vec::new(),
    };
    sqlx::query("UPDATE merchant_profiles SET status = 'in_review', submitted_by = $1, submitted_at = now(), reviewed_by = NULL, review_note = NULL, review_reasons = $2")
        .bind(&auth.actor).bind(&reasons).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "onboarding.submitted",
        None,
        json!({}),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "onboarding.submitted",
        json!({ "submitted_by": auth.actor }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "status": "in_review", "automatic": false, "reasons": reasons }),
    ))
}

#[derive(Deserialize)]
pub struct Review {
    /// `approve` or `reject`.
    decision: String,
    note: Option<String>,
}

pub async fn review(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Review>,
) -> Result<Json<Value>, ApiError> {
    auth.require("onboarding:review")?;
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
    if !approve && b.note.as_deref().is_none_or(|n| n.trim().is_empty()) {
        return Err(ApiError::invalid(
            "note_required",
            "say why it was rejected",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let (st, by): (String, Option<String>) =
        sqlx::query_as("SELECT status, submitted_by FROM merchant_profiles FOR UPDATE")
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::not_found("onboarding"))?;
    if st != "in_review" {
        return Err(ApiError::conflict(
            "not_in_review",
            format!("onboarding is {st}"),
        ));
    }
    if by.as_deref() == Some(auth.actor.as_str()) {
        return Err(ApiError::forbidden(
            "a different person must review the submission".to_string(),
        ));
    }
    if approve {
        let verified: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM merchant_bank_accounts WHERE verified_at IS NOT NULL",
        )
        .fetch_one(&mut *tx)
        .await?;
        if verified == 0 {
            return Err(ApiError::conflict(
                "bank_unverified",
                "verify a bank account first",
            ));
        }
    }
    let new = if approve { "approved" } else { "rejected" };
    sqlx::query("UPDATE merchant_profiles SET status = $1, reviewed_by = $2, reviewed_at = now(), review_note = $3")
        .bind(new).bind(&auth.actor).bind(&b.note).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        &format!("onboarding.{new}"),
        None,
        json!({ "note": b.note }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        &format!("onboarding.{new}"),
        json!({ "reviewed_by": auth.actor }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "status": new })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pan_and_ifsc_shapes() {
        assert!(valid_pan("ABCDE1234F"));
        assert!(!valid_pan("abcde1234f") && !valid_pan("ABCDE12345") && !valid_pan("ABCD1234FF"));
        assert!(valid_ifsc("HDFC0001234") && valid_ifsc("SBIN0A12345"));
        assert!(!valid_ifsc("HDFC1001234") && !valid_ifsc("HDFC000123"));
    }
}
