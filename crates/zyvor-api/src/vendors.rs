//! Vendor onboarding workflow and payout settings.
//!
//! A vendor you invite moves invited -> details_submitted -> bank_added -> verified -> active, and can be suspended
//! and reactivated. Two people are involved: whoever adds the bank account, and a different person with
//! `payouts:approve` who verifies it. The verifier sees the outcome of the automatic bank check when a provider
//! is configured (`ZYVOR_KYC_PROVIDER`; the simulator verifies nothing real) and whether the account holder's name
//! matches the vendor's legal name (the same loose comparison onboarding uses). A name that does not match needs a
//! written reason from the verifier. Nothing here approves anybody automatically.
//!
//! Only an `active` vendor can be given a share of a payment or be paid. Vendors made the older way
//! (`POST /v1/vendors` with bank details) are `active` already, so nothing that worked before stops working.
//! Changing the bank account of an active vendor puts it back to `bank_added` until a person verifies it again.
//!
//! Documents are optional: PDF, PNG or JPEG files kept encrypted, like onboarding uploads (same size limit and file
//! check; Aadhaar documents are refused, not stored).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::Auth,
    docfacts,
    docread::FileKind,
    kyc, onboarding, tenant,
    uploads::{MAX_BYTES, download_response, safe_name},
};

const DOC_KINDS: [&str; 6] = [
    "pan",
    "gst_certificate",
    "address_proof",
    "cancelled_cheque",
    "agreement",
    "other",
];
const MAX_DOCS: i64 = 12;

fn ok_email(e: &str) -> bool {
    e.len() <= 120
        && e.matches('@').count() == 1
        && !e.contains(char::is_whitespace)
        && e.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.'))
}

fn clean(v: &Option<String>, max: usize) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(max).collect())
}

/// The whole vendor, for the API. Never carries the full account number (see `GET /v1/vendors/{id}/details`).
pub async fn vendor_json(conn: &mut PgConnection, id: &str) -> Result<Value, ApiError> {
    let r: Option<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'id', id, 'name', name, 'external_ref', external_ref, 'status', status, 'archived', archived_at IS NOT NULL,
            'legal_name', legal_name, 'email', email, 'phone', phone, 'address', address,
            'bank', jsonb_build_object('holder_name', bank_holder_name, 'ifsc', ifsc, 'account_last4', account_last4,
                'vpa', vpa_masked, 'added_by', bank_added_by, 'added_at', bank_added_at, 'check', bank_check,
                'verified_by', verified_by, 'verified_at', verified_at, 'name_override_reason', name_override_reason),
            'workflow', jsonb_build_object('invited_by', invited_by, 'invited_at', invited_at,
                'details_submitted_at', details_submitted_at, 'activated_at', activated_at,
                'suspended_at', suspended_at, 'suspended_reason', suspended_reason),
            'payout_settings', jsonb_build_object('hold_days', hold_days, 'min_payout_minor', min_payout_minor,
                'default_rule_id', default_rule_id),
            'created_at', created_at)
         FROM vendors WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    r.ok_or_else(|| ApiError::not_found("vendor"))
}

#[derive(Deserialize)]
pub struct Invite {
    name: String,
    external_ref: Option<String>,
    email: Option<String>,
    phone: Option<String>,
}

/// `POST /v1/vendors/invite`: record a vendor you are about to onboard. Nothing is sent to them.
pub async fn invite(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Invite>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    let name = b.name.trim();
    if name.len() < 2 || name.len() > 120 {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "name is required (2 to 120 characters)",
        ));
    }
    let email = clean(&b.email, 120);
    if email.as_deref().is_some_and(|e| !ok_email(e)) {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "email does not look right",
        ));
    }
    let id = new_id("ven");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO vendors (id, tenant_id, name, external_ref, created_by, status, email, phone, invited_by, invited_at) VALUES ($1,$2,$3,$4,$5,'invited',$6,$7,$5,now())")
        .bind(&id).bind(&auth.tenant_id).bind(name).bind(clean(&b.external_ref, 80)).bind(&auth.actor)
        .bind(&email).bind(clean(&b.phone, 20))
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.invited",
        Some(&id),
        json!({ "name": name }),
    )
    .await?;
    let out = vendor_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// `GET /v1/vendors/{id}`
pub async fn get_vendor(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(vendor_json(&mut tx, &id).await?))
}

async fn status_of(conn: &mut PgConnection, id: &str) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT status FROM vendors WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::not_found("vendor"))
}

#[derive(Deserialize)]
pub struct VendorProfileIn {
    legal_name: String,
    email: Option<String>,
    phone: Option<String>,
    address: Option<String>,
}

/// `PUT /v1/vendors/{id}/profile`: the details the vendor gave. Open until the bank account is verified.
pub async fn put_profile(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<VendorProfileIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let legal = b.legal_name.trim();
    if legal.len() < 2 || legal.len() > 160 {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "legal_name is required (2 to 160 characters)",
        ));
    }
    let email = clean(&b.email, 120);
    if email.as_deref().is_some_and(|e| !ok_email(e)) {
        return Err(ApiError::invalid(
            "invalid_vendor",
            "email does not look right",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st = status_of(&mut tx, &id).await?;
    if !matches!(st.as_str(), "invited" | "details_submitted" | "bank_added") {
        return Err(ApiError::conflict(
            "vendor_locked",
            format!("a {st} vendor's details can no longer be changed this way"),
        ));
    }
    sqlx::query("UPDATE vendors SET legal_name = $2, email = COALESCE($3, email), phone = COALESCE($4, phone), address = COALESCE($5, address),
                 status = CASE WHEN status = 'invited' THEN 'details_submitted' ELSE status END,
                 details_submitted_at = COALESCE(details_submitted_at, now()) WHERE id = $1")
        .bind(&id).bind(legal).bind(&email).bind(clean(&b.phone, 20)).bind(clean(&b.address, 300))
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.details_submitted",
        Some(&id),
        json!({}),
    )
    .await?;
    let out = vendor_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct VendorBankIn {
    holder_name: String,
    ifsc: Option<String>,
    account_number: Option<String>,
    vpa: Option<String>,
}

/// `POST /v1/vendors/{id}/bank_account`: the account to pay. Encrypted like any vendor bank detail; only the last
/// four digits are shown. On an active vendor this puts it back to `bank_added` until a person verifies it again.
pub async fn add_bank_account(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<VendorBankIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let holder = b.holder_name.trim().to_string();
    let ifsc = b.ifsc.as_deref().map(|i| i.trim().to_uppercase());
    let acct = b.account_number.as_deref().map(str::trim);
    let vpa = b.vpa.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let bank_ok = match (acct, &ifsc) {
        (Some(a), Some(i)) => {
            (6..=20).contains(&a.len())
                && a.chars().all(|c| c.is_ascii_digit())
                && onboarding::valid_ifsc(i)
        }
        (None, None) => true,
        _ => false,
    };
    if holder.len() < 2
        || holder.len() > 120
        || !bank_ok
        || (acct.is_none() && vpa.is_none())
        || vpa.is_some_and(|v| v.matches('@').count() != 1 || v.len() > 80)
    {
        return Err(ApiError::invalid(
            "invalid_bank_account",
            "holder_name, and either account_number with an IFSC like HDFC0001234 or a vpa like name@bank, are required",
        ));
    }
    // read the vendor first: its legal name is what the account holder is compared with
    let (st, legal, name): (String, Option<String>, String) = {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        let r: Option<(String, Option<String>, String)> =
            sqlx::query_as("SELECT status, legal_name, name FROM vendors WHERE id = $1")
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?;
        r.ok_or_else(|| ApiError::not_found("vendor"))?
    };
    if st == "invited" {
        return Err(ApiError::conflict(
            "details_first",
            "record the vendor's details (PUT /v1/vendors/{id}/profile) before the bank account",
        ));
    }
    if st == "suspended" {
        return Err(ApiError::conflict(
            "vendor_suspended",
            "reactivate the vendor before changing its bank account",
        ));
    }
    let compare_to = legal.unwrap_or(name);
    let holder_matches = kyc::names_match(&holder, &compare_to);
    // the automatic bank check, when a provider is configured; a paid provider is capped per day
    let mut check = json!({
        "provider": s.kyc.as_ref().map(|k| k.name()),
        "outcome": null, "detail": "no automatic bank check ran",
        "holder_matches_legal_name": holder_matches,
    });
    if let (Some(k), Some(a), Some(i)) = (s.kyc.as_ref(), acct, &ifsc) {
        let c = if !onboarding::paid_checks_allowed(&s, &auth.tenant_id, k, 1).await? {
            onboarding::capped()
        } else {
            let mut c = k.verify_bank(i, a, &holder).await;
            if c.outcome == kyc::Outcome::Pass
                && let Some(n) = c.registered_name.as_deref()
                && !kyc::names_match(&holder, n)
            {
                c.outcome = kyc::Outcome::Inconclusive;
                c.detail = "the account holder name does not match the name given".into();
            }
            c
        };
        check["outcome"] = json!(c.outcome.as_str());
        check["detail"] = json!(c.detail);
    }
    let details = json!({ "account_number": acct, "ifsc": ifsc, "vpa": vpa });
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let now_st = status_of(&mut tx, &id).await?;
    if now_st != st {
        return Err(ApiError::conflict(
            "vendor_changed",
            "the vendor changed while this was being checked; try again",
        ));
    }
    sqlx::query("UPDATE vendors SET bank_holder_name = $2, account_last4 = $3, ifsc = $4, vpa_masked = $5, details_enc = $6,
                 bank_added_by = $7, bank_added_at = now(), bank_check = $8,
                 verified_by = NULL, verified_at = NULL, name_override_reason = NULL, activated_at = NULL,
                 status = 'bank_added' WHERE id = $1")
        .bind(&id).bind(&holder)
        .bind(acct.map(|a| a[a.len().saturating_sub(4)..].to_string()))
        .bind(&ifsc)
        .bind(vpa.map(|v| format!("***{}", v.split_once('@').map(|(_, h)| format!("@{h}")).unwrap_or_default())))
        .bind(s.crypto.encrypt(details.to_string().as_bytes()))
        .bind(&auth.actor).bind(&check)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.bank_account_added",
        Some(&id),
        json!({ "previous_status": st, "holder_matches_legal_name": holder_matches }),
    )
    .await?;
    let out = vendor_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
pub struct VendorVerifyIn {
    /// Needed when the account holder's name does not match the vendor's legal name: why it is still right.
    override_reason: Option<String>,
}

/// `POST /v1/vendors/{id}/verify`: a person other than the one who added the account confirms it is the vendor's.
pub async fn verify(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Option<Json<VendorVerifyIn>>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    let b = body.map(|j| j.0).unwrap_or_default();
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, Option<String>, Option<Value>, Option<String>, Option<String>, String)> =
        sqlx::query_as("SELECT status, bank_added_by, bank_check, bank_holder_name, legal_name, name FROM vendors WHERE id = $1 FOR UPDATE")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let (st, added_by, check, holder, legal, name) =
        row.ok_or_else(|| ApiError::not_found("vendor"))?;
    if st != "bank_added" {
        return Err(ApiError::conflict(
            "not_awaiting_verification",
            format!("vendor is {st}; only a vendor with a bank account added can be verified"),
        ));
    }
    if added_by.as_deref() == Some(auth.actor.as_str()) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "separation_of_duties",
            "a different person must verify a vendor's bank account",
        ));
    }
    if check.as_ref().and_then(|c| c["outcome"].as_str()) == Some("fail") {
        return Err(ApiError::conflict(
            "bank_check_failed",
            "the automatic bank check failed; add the correct account instead",
        ));
    }
    // the comparison is made again here, from what is stored, so a changed legal name is not missed
    let matches = holder
        .as_deref()
        .is_some_and(|h| kyc::names_match(h, legal.as_deref().unwrap_or(&name)));
    let reason = clean(&b.override_reason, 300);
    if !matches && reason.as_deref().is_none_or(|r| r.len() < 5) {
        return Err(ApiError::conflict(
            "name_mismatch",
            "the account holder's name does not match the vendor's legal name; say why it is still right in override_reason",
        ));
    }
    sqlx::query("UPDATE vendors SET status = 'verified', verified_by = $2, verified_at = now(), name_override_reason = $3 WHERE id = $1")
        .bind(&id)
        .bind(&auth.actor)
        .bind(if matches { None } else { reason.clone() })
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.bank_account_verified",
        Some(&id),
        json!({ "name_matched": matches, "override_reason": if matches { None } else { reason } }),
    )
    .await?;
    let out = vendor_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

async fn move_status(
    s: &AppState,
    auth: &Auth,
    id: &str,
    from: &[&str],
    to: &str,
    action: &str,
    reason: Option<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st = status_of(&mut tx, id).await?;
    if !from.contains(&st.as_str()) {
        return Err(ApiError::conflict(
            "invalid_transition",
            format!("a {st} vendor cannot be {action}"),
        ));
    }
    sqlx::query(
        "UPDATE vendors SET status = $2,
                 activated_at = CASE WHEN $2 = 'active' THEN now() ELSE activated_at END,
                 suspended_at = CASE WHEN $2 = 'suspended' THEN now() ELSE NULL END,
                 suspended_reason = CASE WHEN $2 = 'suspended' THEN $3 ELSE NULL END WHERE id = $1",
    )
    .bind(id)
    .bind(to)
    .bind(&reason)
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        &format!("vendor.{action}"),
        Some(id),
        json!({ "from": st, "reason": reason }),
    )
    .await?;
    let out = vendor_json(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// `POST /v1/vendors/{id}/activate`: a verified vendor can now be given shares and paid.
pub async fn activate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    move_status(&s, &auth, &id, &["verified"], "active", "activated", None).await
}

#[derive(Deserialize)]
pub struct VendorSuspendIn {
    reason: String,
}

/// `POST /v1/vendors/{id}/suspend`: no new shares and no payouts until reactivated. Payouts already proposed are
/// checked again before approval and again before sending, and are refused.
pub async fn suspend(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<VendorSuspendIn>,
) -> Result<Json<Value>, ApiError> {
    let reason = b.reason.trim().to_string();
    if reason.len() < 3 || reason.len() > 300 {
        return Err(ApiError::invalid(
            "invalid_reason",
            "reason is required (3 to 300 characters)",
        ));
    }
    move_status(
        &s,
        &auth,
        &id,
        &["active", "verified"],
        "suspended",
        "suspended",
        Some(reason),
    )
    .await
}

/// `POST /v1/vendors/{id}/reactivate`
pub async fn reactivate(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    // a vendor onboarded through the workflow comes back only if a person verified its account; vendors made the
    // older way (no invitation) were active without that step and may come back
    let row: Option<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> = {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        sqlx::query_as("SELECT verified_at, invited_at FROM vendors WHERE id = $1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?
    };
    let (verified_at, invited_at) = row.ok_or_else(|| ApiError::not_found("vendor"))?;
    if verified_at.is_none() && invited_at.is_some() {
        return Err(ApiError::conflict(
            "not_verified",
            "this vendor's bank account was never verified",
        ));
    }
    move_status(
        &s,
        &auth,
        &id,
        &["suspended"],
        "active",
        "reactivated",
        None,
    )
    .await
}

#[derive(Deserialize)]
pub struct VendorSettingsIn {
    hold_days: Option<i32>,
    min_payout_minor: Option<i64>,
    /// A commission rule id, or an empty string to clear it.
    default_rule_id: Option<String>,
}

/// `PUT /v1/vendors/{id}/payout_settings`: hold days (money from a payment waits this long before it can be paid
/// out of the vendor's balance), a minimum payout, and the commission rule used when no more specific one applies.
pub async fn put_settings(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<VendorSettingsIn>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:approve")?;
    if b.hold_days.is_some_and(|d| !(0..=365).contains(&d))
        || b.min_payout_minor
            .is_some_and(|m| !(0..=zyvor_core::MAX_AMOUNT_MINOR).contains(&m))
    {
        return Err(ApiError::invalid(
            "invalid_settings",
            "hold_days is 0 to 365 and min_payout_minor is whole paise, zero or more",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    status_of(&mut tx, &id).await?;
    let rule = b.default_rule_id.as_deref().map(str::trim);
    if let Some(r) = rule.filter(|r| !r.is_empty()) {
        // the rule must be this tenant's, active, and either shared or this vendor's own
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM commission_rules WHERE id = $1 AND status = 'active' AND (vendor_id IS NULL OR vendor_id = $2))",
        )
        .bind(r)
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        if !ok {
            return Err(ApiError::not_found("commission rule"));
        }
    }
    sqlx::query("UPDATE vendors SET hold_days = COALESCE($2, hold_days), min_payout_minor = COALESCE($3, min_payout_minor),
                 default_rule_id = CASE WHEN $4::text IS NULL THEN default_rule_id WHEN $4 = '' THEN NULL ELSE $4 END WHERE id = $1")
        .bind(&id)
        .bind(b.hold_days)
        .bind(b.min_payout_minor)
        .bind(rule)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.payout_settings_changed",
        Some(&id),
        json!({ "hold_days": b.hold_days, "min_payout_minor": b.min_payout_minor, "default_rule_id": b.default_rule_id }),
    )
    .await?;
    let out = vendor_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

// ---------- documents ----------

#[derive(Deserialize)]
pub struct VendorDocIn {
    kind: String,
    filename: String,
    /// The file, standard base64.
    data_base64: String,
}

/// `POST /v1/vendors/{id}/documents`
pub async fn upload_document(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<VendorDocIn>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    if !DOC_KINDS.contains(&b.kind.as_str()) {
        return Err(ApiError::invalid(
            "invalid_document",
            format!("kind is one of {DOC_KINDS:?}"),
        ));
    }
    if b.data_base64.len() > MAX_BYTES * 4 / 3 + 16 {
        return Err(ApiError::invalid(
            "file_too_large",
            "files are limited to 5 MB",
        ));
    }
    let bytes = STANDARD
        .decode(b.data_base64.trim())
        .map_err(|_| ApiError::invalid("invalid_file", "data_base64 is not valid base64"))?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(ApiError::invalid(
            "file_too_large",
            "files are limited to 5 MB",
        ));
    }
    let Some(kind) = FileKind::sniff(&bytes) else {
        return Err(ApiError::invalid(
            "unsupported_file",
            "send a PDF, PNG or JPEG file",
        ));
    };
    let sha = hex::encode(Sha256::digest(&bytes));
    {
        let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
        status_of(&mut tx, &id).await?;
        let n: i64 =
            sqlx::query_scalar("SELECT count(*) FROM vendor_documents WHERE vendor_id = $1")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        if n >= MAX_DOCS {
            return Err(ApiError::conflict(
                "too_many_files",
                format!("at most {MAX_DOCS} files per vendor"),
            ));
        }
    }
    // the slow reading holds no transaction; an Aadhaar document is refused, not kept
    let reading = s.doc_reader.read(&bytes, kind).await;
    if docfacts::looks_like_aadhaar(&reading.text) {
        return Err(ApiError::invalid(
            "aadhaar_not_accepted",
            "Aadhaar cards and letters cannot be uploaded. Use the PAN card, or the certificates of the business",
        ));
    }
    let doc_id = new_id("vdoc");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO vendor_documents (id, tenant_id, vendor_id, kind, filename, content_type, size_bytes, sha256, data_enc, uploaded_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&doc_id).bind(&auth.tenant_id).bind(&id).bind(&b.kind).bind(safe_name(&b.filename))
        .bind(kind.mime()).bind(bytes.len() as i32).bind(&sha).bind(s.crypto.encrypt(&bytes)).bind(&auth.actor)
        .execute(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                ApiError::conflict("already_uploaded", "this file is already uploaded for this vendor")
            }
            e => e.into(),
        })?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.document_added",
        Some(&id),
        json!({ "document_id": doc_id, "kind": b.kind, "size_bytes": bytes.len() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": doc_id, "vendor_id": id, "kind": b.kind, "filename": safe_name(&b.filename), "content_type": kind.mime(), "size_bytes": bytes.len(), "sha256": sha }),
        ),
    ))
}

/// `GET /v1/vendors/{id}/documents`
pub async fn list_documents(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    status_of(&mut tx, &id).await?;
    let data: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'kind', kind, 'filename', filename, 'content_type', content_type,
                'size_bytes', size_bytes, 'sha256', sha256, 'uploaded_by', uploaded_by, 'created_at', created_at)
         FROM vendor_documents WHERE vendor_id = $1 ORDER BY created_at, id",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(json!({ "data": data })))
}

/// `GET /v1/vendors/{id}/documents/{doc}/file`: the stored file, as a download. Each download is audited.
pub async fn document_file(
    State(s): State<AppState>,
    auth: Auth,
    Path((id, doc)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    auth.require("payouts:approve")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, Vec<u8>)> = sqlx::query_as(
        "SELECT filename, content_type, data_enc FROM vendor_documents WHERE id = $1 AND vendor_id = $2",
    )
    .bind(&doc)
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (name, mime, enc) = row.ok_or_else(|| ApiError::not_found("document"))?;
    let bytes = s.crypto.decrypt(&enc).ok_or_else(ApiError::internal)?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "vendor.document_downloaded",
        Some(&id),
        json!({ "document_id": doc }),
    )
    .await?;
    tx.commit().await?;
    download_response(&name, &mime, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_checked_loosely() {
        assert!(ok_email("a@b.in") && ok_email("first.last@shop.example.com"));
        assert!(
            !ok_email("a@b") && !ok_email("a b@c.in") && !ok_email("@c.in") && !ok_email("a@@c.in")
        );
    }
}
