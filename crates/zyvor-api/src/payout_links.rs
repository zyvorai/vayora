// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

//! Payout links: the merchant fixes an amount and sends a link; the recipient opens it, types their own bank
//! account or UPI id, and the payout then goes through the existing payout flow.
//!
//! **Steps.**
//! 1. A person with `payouts:write` creates the link: amount, purpose, expiry (up to 30 days, 72 hours by default),
//!    optionally the name the account should be held in (`expected_name`). The token is shown once and stored only as
//!    a hash. The amount cannot be changed by the recipient: the public endpoints take no amount.
//! 2. The recipient claims it: a name (when the merchant gave none) and one of `bank_account` or `upi`. The details are
//!    **verified first** through `verify::run` (penny drop for a bank account, UPI id check for a VPA): the account
//!    must exist and its holder name must `match` the expected name (the conservative comparison in `verify`).
//!    Anything else (not found, a different name, only a partial match, or no answer) creates nothing. A wrong or
//!    mismatching answer counts against the link: five failures lock it. A provider that gives no answer does not
//!    count against it.
//! 3. On a verified claim, a vendor is created (full details encrypted exactly as for any vendor, masks in lists) and
//!    a payout is requested for the fixed amount **through `payouts::create_one`**, with the link's creator as the
//!    requester. So the normal rules apply unchanged: the payout waits for a second person (the creator cannot approve
//!    it) unless it is under the business's approval threshold, then it is paid and marked paid as any payout is.
//!    The link is single use: once claimed it cannot be claimed again, whatever happens to the payout.
//!
//! **What the recipient learns:** the business name, the amount, the purpose, the expiry and the state of their
//! payout. Never the name the bank holds, the creator, or anything about other links. Responses are the same whether a
//! token is wrong or belongs to another business.
//!
//! **Limits:** with the simulator as the verification provider (always the case today in test mode, and the only one
//! with a UPI id check), "verified" means nothing about a real account; every claim response says so. Live mode needs
//! a real provider (see `verify`). This feature moves no money itself: the payout still needs a person to record or
//! send it.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;
use zyvor_core::{Currency, new_id};

use crate::{
    ApiError, AppState,
    auth::{Auth, hash_key},
    ratelimit::Bucket,
    tenant,
    verify::{self, Subject},
};

/// Wrong or mismatching answers a link takes before it locks.
pub const MAX_FAILED_ATTEMPTS: i32 = 5;
const DEFAULT_HOURS: i64 = 72;
const MAX_DAYS: i64 = 30;
/// One link takes at most this many claim attempts in quick succession.
const CLAIM_BUCKET: Bucket = Bucket {
    burst: 5.0,
    per_sec: 0.05,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Claimed,
    Paid,
    PayoutEnded,
    Cancelled,
    Expired,
    Locked,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Active => "active",
            Status::Claimed => "claimed",
            Status::Paid => "paid",
            Status::PayoutEnded => "payout_ended",
            Status::Cancelled => "cancelled",
            Status::Expired => "expired",
            Status::Locked => "locked",
        }
    }
}

/// The link's state from what is stored. A claimed link follows its payout: `claimed` while it is pending or approved,
/// `paid` when paid, `payout_ended` when it was rejected or cancelled (the link stays used; make a new one).
pub fn status(
    cancelled: bool,
    expires_at: DateTime<Utc>,
    payout: Option<&str>,
    failed_attempts: i32,
    now: DateTime<Utc>,
) -> Status {
    match payout {
        Some("paid") => Status::Paid,
        Some("rejected" | "canceled") => Status::PayoutEnded,
        Some(_) => Status::Claimed,
        None if cancelled => Status::Cancelled,
        None if expires_at <= now => Status::Expired,
        None if failed_attempts >= MAX_FAILED_ATTEMPTS => Status::Locked,
        None => Status::Active,
    }
}

#[derive(Deserialize)]
pub struct NewPayoutLink {
    /// What the recipient will be paid, in minor units. Fixed; the recipient cannot change it.
    amount_minor: i64,
    currency: Option<String>,
    /// Shown to the recipient and recorded on the payout.
    purpose: String,
    /// The name the account must be held in. Without it the recipient's typed name is used (a weaker check).
    expected_name: Option<String>,
    /// Your reference; repeating the request with it returns the same link (without the token, which is shown once).
    external_ref: Option<String>,
    /// Hours the link stays open (default 72, at most 720).
    expires_in_hours: Option<i64>,
}

fn new_token() -> String {
    format!(
        "pol_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// `POST /v1/payout_links`
pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewPayoutLink>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    let ccy: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let purpose = b.purpose.trim();
    if b.amount_minor <= 0
        || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR
        || purpose.is_empty()
        || purpose.len() > 200
    {
        return Err(ApiError::invalid(
            "invalid_payout_link",
            "amount_minor must be positive and purpose is 1-200 characters",
        ));
    }
    let expected = b
        .expected_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    if expected.is_some_and(|n| n.len() > 100 || verify::name_tokens(n).is_empty()) {
        return Err(ApiError::invalid(
            "invalid_payout_link",
            "expected_name is 2-100 characters with at least one name",
        ));
    }
    if b.external_ref
        .as_deref()
        .is_some_and(|r| r.is_empty() || r.len() > 64)
    {
        return Err(ApiError::invalid(
            "invalid_payout_link",
            "external_ref is 1-64 characters",
        ));
    }
    let hours = b.expires_in_hours.unwrap_or(DEFAULT_HOURS);
    if !(1..=MAX_DAYS * 24).contains(&hours) {
        return Err(ApiError::invalid(
            "invalid_expiry",
            "expires_in_hours is 1 to 720",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    crate::onboarding::require_approved(&mut tx).await?;
    if let Some(r) = &b.external_ref {
        let existing: Option<(String, DateTime<Utc>)> =
            sqlx::query_as("SELECT id, expires_at FROM payout_links WHERE external_ref = $1")
                .bind(r)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((id, exp)) = existing {
            return Ok((
                StatusCode::OK,
                Json(
                    json!({ "id": id, "existing": true, "token": null, "expires_at": exp, "note": "the token is shown once, when the link is created" }),
                ),
            ));
        }
    }
    let (id, token) = (new_id("pol"), new_token());
    let expires_at = Utc::now() + Duration::hours(hours);
    sqlx::query("INSERT INTO payout_links (id, tenant_id, token_hash, amount_minor, currency, purpose, expected_name, external_ref, expires_at, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(hash_key(&token)).bind(b.amount_minor).bind(ccy.code())
        .bind(purpose).bind(expected).bind(&b.external_ref).bind(expires_at).bind(&auth.actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_link.created",
        Some(&id),
        json!({ "amount_minor": b.amount_minor, "expires_at": expires_at }),
    )
    .await?;
    tx.commit().await?;
    let path = format!("/payout/link/{}/{token}", auth.tenant_id);
    let url = s
        .checkout_public_url
        .as_deref()
        .map(|b| format!("{}{path}", b.trim_end_matches('/')));
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "token": token, "path": path, "url": url, "amount_minor": b.amount_minor, "currency": ccy.code(), "expires_at": expires_at, "status": "active" }),
        ),
    ))
}

type Row = (
    String,
    i64,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    i32,
    Option<String>,
    Option<String>,
    String,
    DateTime<Utc>,
);

const COLS: &str = "l.id, l.amount_minor, l.currency, l.purpose, l.external_ref, l.expires_at, l.cancelled_at, l.failed_attempts, l.payout_id, (SELECT p.status FROM payouts p WHERE p.id = l.payout_id), l.created_by, l.created_at";

fn row_json(r: Row) -> Value {
    let (id, amount, currency, purpose, ext, exp, canc, fails, payout_id, payout_status, by, at) =
        r;
    let st = status(
        canc.is_some(),
        exp,
        payout_status.as_deref(),
        fails,
        Utc::now(),
    );
    json!({ "id": id, "amount_minor": amount, "currency": currency, "purpose": purpose, "external_ref": ext,
            "status": st.as_str(), "expires_at": exp, "failed_attempts": fails, "payout_id": payout_id,
            "payout_status": payout_status, "created_by": by, "created_at": at })
}

/// `GET /v1/payout_links`
pub async fn list(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM payout_links l ORDER BY l.created_at DESC, l.id LIMIT 200"
    )))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(row_json).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/payout_links/{id}`
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM payout_links l WHERE l.id = $1"
    )))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(Json(row_json(
        row.ok_or_else(|| ApiError::not_found("payout link"))?,
    )))
}

/// `POST /v1/payout_links/{id}/cancel`: only while nobody has claimed it.
pub async fn cancel(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE payout_links SET cancelled_at = now() WHERE id = $1 AND payout_id IS NULL AND cancelled_at IS NULL")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::conflict(
            "cannot_cancel",
            "this payout link was not found, is already cancelled, or has been claimed (cancel its payout instead)",
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_link.cancelled",
        Some(&id),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "cancelled" })))
}

struct Loaded {
    id: String,
    amount_minor: i64,
    currency: String,
    purpose: String,
    expected_name: Option<String>,
    expires_at: DateTime<Utc>,
    cancelled: bool,
    failed_attempts: i32,
    payout_status: Option<String>,
    claimed: bool,
    created_by: String,
}

async fn load(conn: &mut sqlx::PgConnection, token: &str, lock: bool) -> Result<Loaded, ApiError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(String, i64, String, String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>, i32, Option<String>, Option<String>, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT l.id, l.amount_minor, l.currency, l.purpose, l.expected_name, l.expires_at, l.cancelled_at, l.failed_attempts, l.payout_id,
                (SELECT p.status FROM payouts p WHERE p.id = l.payout_id), l.created_by
           FROM payout_links l WHERE l.token_hash = $1{}",
        if lock { " FOR UPDATE OF l" } else { "" }
    )))
    .bind(hash_key(token))
    .fetch_optional(&mut *conn)
    .await?;
    let (
        id,
        amount_minor,
        currency,
        purpose,
        expected_name,
        expires_at,
        canc,
        failed_attempts,
        payout_id,
        payout_status,
        created_by,
    ) = row.ok_or_else(|| ApiError::not_found("payout link"))?;
    Ok(Loaded {
        id,
        amount_minor,
        currency,
        purpose,
        expected_name,
        expires_at,
        cancelled: canc.is_some(),
        failed_attempts,
        claimed: payout_id.is_some(),
        payout_status,
        created_by,
    })
}

impl Loaded {
    fn status(&self) -> Status {
        status(
            self.cancelled,
            self.expires_at,
            self.payout_status
                .as_deref()
                .or(self.claimed.then_some("pending_approval")),
            self.failed_attempts,
            Utc::now(),
        )
    }
}

/// `GET /v1/checkout/payout_links/{tenant_id}/{token}`: public; what the recipient needs to see.
pub async fn info(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let l = load(&mut tx, &token, false).await?;
    let merchant: Option<String> = sqlx::query_scalar("SELECT o.name FROM tenants t JOIN organisations o ON o.id = t.organisation_id WHERE t.id = $1")
        .bind(&tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(Json(json!({
        "merchant": merchant, "amount_minor": l.amount_minor, "currency": l.currency, "purpose": l.purpose,
        "status": l.status().as_str(), "expires_at": l.expires_at,
        "name_required": l.expected_name.is_none(),
        "methods": ["bank_account", "upi"],
    })))
}

#[derive(Deserialize)]
pub struct BankIn {
    account_number: String,
    ifsc: String,
}

#[derive(Deserialize)]
pub struct UpiIn {
    vpa: String,
}

#[derive(Deserialize)]
pub struct Claim {
    /// The recipient's name; needed only when the link has no `expected_name`.
    name: Option<String>,
    /// Give exactly one of `bank_account` or `upi`.
    bank_account: Option<BankIn>,
    upi: Option<UpiIn>,
}

fn unavailable() -> ApiError {
    ApiError::conflict(
        "link_unavailable",
        "this payout link cannot be used (it may be used, expired, cancelled or locked)",
    )
}

/// `POST /v1/checkout/payout_links/{tenant_id}/{token}/claim`: public. The amount is not an input.
pub async fn claim(
    State(s): State<AppState>,
    Path((tenant_id, token)): Path<(String, String)>,
    Json(b): Json<Claim>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // shape checks, before anything is looked up or called
    let (kind_bank, details): (bool, (String, Option<String>, Option<String>)) = match (
        &b.bank_account,
        &b.upi,
    ) {
        (Some(bank), None) => {
            let (acc, ifsc) = (bank.account_number.trim(), bank.ifsc.trim().to_uppercase());
            if !verify::valid_account(acc) || !verify::valid_ifsc(&ifsc) {
                return Err(ApiError::invalid(
                    "invalid_bank_account",
                    "account_number must be 6-20 digits and ifsc 11 characters like HDFC0001234",
                ));
            }
            (true, (acc.to_string(), Some(ifsc), None))
        }
        (None, Some(u)) => {
            let v = u.vpa.trim();
            if !verify::valid_vpa(v) {
                return Err(ApiError::invalid(
                    "invalid_vpa",
                    "vpa must look like name@bank",
                ));
            }
            (false, (String::new(), None, Some(v.to_string())))
        }
        _ => {
            return Err(ApiError::invalid(
                "invalid_claim",
                "give exactly one of bank_account or upi",
            ));
        }
    };
    let typed = b.name.as_deref().map(str::trim).unwrap_or("");
    if typed.len() > 100 {
        return Err(ApiError::invalid("invalid_name", "name is too long"));
    }

    // the link, as it is now (no lock is held while a provider is called)
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let l = load(&mut tx, &token, false).await?;
    drop(tx);
    if l.status() != Status::Active {
        return Err(unavailable());
    }
    let name = l.expected_name.clone().unwrap_or_else(|| typed.to_string());
    if verify::name_tokens(&name).is_empty() {
        return Err(ApiError::invalid(
            "name_required",
            "enter the name the account is held in",
        ));
    }
    if let Err(retry) = s
        .limiter
        .check(format!("plink:{}", l.id), CLAIM_BUCKET, Instant::now())
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("too many tries on this link; retry in {retry}s"),
        ));
    }
    let actor = format!("payout_link:{}", l.id);
    let env: String = {
        let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
        sqlx::query_scalar("SELECT env FROM tenants WHERE id = $1")
            .bind(&tenant_id)
            .fetch_one(&mut *tx)
            .await?
    };
    let subject = if kind_bank {
        Subject::Bank {
            account: &details.0,
            ifsc: details.1.as_deref().unwrap_or(""),
        }
    } else {
        Subject::Vpa(details.2.as_deref().unwrap_or(""))
    };
    let v = verify::run(&s, &tenant_id, &env, &actor, subject, &name)
        .await
        .map_err(|e| match e.code {
            // do not tell a stranger how verification is configured
            "verification_unavailable" => ApiError::conflict(
                "try_later",
                "this cannot be completed right now; try again later",
            ),
            _ => e,
        })?;
    if v.outcome == "inconclusive" {
        return Err(ApiError::conflict(
            "try_later",
            "the bank did not answer; try again in a few minutes",
        ));
    }
    if !v.safe_to_pay() {
        let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
        let fails: i32 = sqlx::query_scalar("UPDATE payout_links SET failed_attempts = failed_attempts + 1 WHERE id = $1 AND payout_id IS NULL RETURNING failed_attempts")
            .bind(&l.id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(MAX_FAILED_ATTEMPTS);
        tenant::audit(&mut tx, &tenant_id, &actor, "payout_link.claim_refused", Some(&l.id), json!({ "outcome": v.outcome, "name_match": v.name_match.as_str(), "failed_attempts": fails })).await?;
        tx.commit().await?;
        let left = (MAX_FAILED_ATTEMPTS - fails).max(0);
        // the same words whether the account is missing or held by someone else
        return Err(ApiError::invalid(
            "details_not_confirmed",
            format!(
                "we could not confirm that account is held in the expected name; {left} tries left on this link"
            ),
        ));
    }

    // create the vendor and the payout, once
    let mut tx = tenant::begin(&s.pool, &tenant_id).await?;
    let locked = load(&mut tx, &token, true).await?;
    if locked.status() != Status::Active {
        return Err(unavailable());
    }
    let vendor_id = new_id("ven");
    let vendor_details = json!({ "account_number": kind_bank.then(|| details.0.clone()), "ifsc": details.1, "vpa": details.2 });
    let (last4, vpa_masked) = if kind_bank {
        (
            Some(verify::mask_account(&details.0)[6..].to_string()),
            None,
        )
    } else {
        let handle = details
            .2
            .as_deref()
            .and_then(|v| v.split_once('@'))
            .map(|(_, h)| format!("***@{h}"));
        (None, handle)
    };
    sqlx::query("INSERT INTO vendors (id, tenant_id, name, external_ref, account_last4, ifsc, vpa_masked, details_enc, created_by) VALUES ($1,$2,$3,NULL,$4,$5,$6,$7,$8)")
        .bind(&vendor_id).bind(&tenant_id).bind(&name).bind(&last4).bind(&details.1).bind(&vpa_masked)
        .bind(s.crypto.encrypt(vendor_details.to_string().as_bytes()))
        .bind(&actor)
        .execute(&mut *tx).await?;
    // The link's creator is the requester, so a second person must approve and the creator cannot.
    let maker = Auth {
        tenant_id: tenant_id.clone(),
        key_id: actor.clone(),
        env: env.clone(),
        scopes: vec!["payouts:write".into()],
        actor: locked.created_by.clone(),
        branch_id: None,
    };
    let request: crate::payouts::NewPayout = serde_json::from_value(json!({
        "vendor_id": vendor_id, "amount_minor": locked.amount_minor, "currency": locked.currency,
        "purpose": locked.purpose, "external_ref": format!("plink:{}", locked.id),
    }))
    .map_err(|_| ApiError::internal())?;
    // the payout rules of the business (vendor status and limits, blocked payees, daily caps) apply as to any payout; a
    // refusal rolls the vendor back too, and the recipient is told only that it cannot be done now
    let (_, payout) = crate::payouts::create_one(&mut tx, &s.crypto, &maker, &request)
        .await
        .map_err(|e| {
            if e.status == StatusCode::CONFLICT || e.status == StatusCode::UNPROCESSABLE_ENTITY {
                tracing::info!(link = %locked.id, code = e.code, "payout link claim refused by payout rules");
                ApiError::conflict("try_later", "this payment cannot be set up right now; ask the sender")
            } else {
                e
            }
        })?;
    let payout_id = payout["id"].as_str().unwrap_or_default().to_string();
    sqlx::query("UPDATE payout_links SET vendor_id = $2, payout_id = $3, verification_id = $4, claimed_at = now() WHERE id = $1 AND payout_id IS NULL")
        .bind(&locked.id).bind(&vendor_id).bind(&payout_id).bind(&v.id)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &tenant_id,
        &actor,
        "payout_link.claimed",
        Some(&locked.id),
        json!({ "payout_id": payout_id, "verification_id": v.id, "simulated": v.simulated }),
    )
    .await?;
    tenant::emit(&mut tx, &tenant_id, "payout_link.claimed", json!({ "payout_link_id": locked.id, "payout_id": payout_id, "amount_minor": locked.amount_minor })).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "status": "claimed",
            "payout_status": payout["status"],
            "amount_minor": locked.amount_minor,
            "currency": locked.currency,
            "simulated": v.simulated,
            "message": if v.simulated { "SIMULATED: your details were accepted by a test checker. No money moves in this sandbox." } else { "Your details were confirmed. The payment is now being processed." },
        })),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::NameMatch;

    fn at(mins: i64) -> DateTime<Utc> {
        Utc::now() + Duration::minutes(mins)
    }

    #[test]
    fn status_follows_the_payout_once_claimed() {
        let now = Utc::now();
        assert_eq!(status(false, at(10), None, 0, now), Status::Active);
        assert_eq!(status(false, at(-1), None, 0, now), Status::Expired);
        assert_eq!(status(true, at(10), None, 0, now), Status::Cancelled);
        assert_eq!(
            status(false, at(10), None, MAX_FAILED_ATTEMPTS, now),
            Status::Locked
        );
        assert_eq!(
            status(false, at(10), Some("pending_approval"), 0, now),
            Status::Claimed
        );
        assert_eq!(
            status(false, at(10), Some("approved"), 0, now),
            Status::Claimed
        );
        assert_eq!(
            status(false, at(-100), Some("approved"), 0, now),
            Status::Claimed,
            "expiry does not undo a claim"
        );
        assert_eq!(status(false, at(10), Some("paid"), 0, now), Status::Paid);
        assert_eq!(
            status(false, at(10), Some("rejected"), 0, now),
            Status::PayoutEnded
        );
        assert_eq!(
            status(false, at(10), Some("canceled"), 0, now),
            Status::PayoutEnded
        );
    }

    #[test]
    fn tokens_are_long_and_prefixed() {
        let t = new_token();
        assert!(t.starts_with("pol_") && t.len() > 60);
        assert_ne!(t, new_token());
        assert_ne!(hash_key(&t), t);
    }

    #[test]
    fn a_match_is_the_only_name_result_that_pays() {
        assert_eq!(
            verify::compare_names("Ravi Kumar", "RAVI KUMAR"),
            NameMatch::Match
        );
        assert_ne!(
            verify::compare_names("Ravi Kumar", "RAVI KUMAR SHARMA"),
            NameMatch::Match
        );
    }
}
