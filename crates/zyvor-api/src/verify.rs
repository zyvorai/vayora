// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Verification primitives: is this UPI id (VPA) live, and does this bank account exist and belong to this name
//! (a "penny drop")?
//!
//! Both go through the same provider abstraction as onboarding (`kyc::Kyc`), so a real provider is a new arm there,
//! not a new code path here. Today:
//! - the **simulator** answers from fixed patterns and verifies nothing real; every response says `simulated: true`;
//! - **Cashfree Secure ID** does the bank-account check (as in onboarding); it is not wired for UPI ids and says so
//!   (an inconclusive answer, never a pass). It has only been run against its documented sandbox inputs.
//! - with no provider configured, test-mode businesses get the labelled simulator and live-mode businesses get a
//!   refusal. A simulated answer is never offered for live money.
//!
//! What is kept: the masked subject (last four digits of an account, the first two characters and the handle of a
//! UPI id), the IFSC, the outcome, the name-match result and the provider. Never the full account number, the full
//! UPI id, or the name the bank returned. A keyed hash of the request lets a repeat within ten minutes reuse the
//! answer instead of calling the provider (and paying for it) again.
//!
//! Limits: the calls are rate limited per business (a token bucket in memory, plus an hourly cap counted from the
//! stored results), and no response ever contains the registered name, so the endpoints cannot be used to look up
//! who holds an account. A name comparison can still be used as a yes/no oracle, which is why the rate limit
//! exists.
//!
//! **Names are compared conservatively** (`compare_names`): `match` only when the words are the same (order and
//! case aside, titles and company suffixes ignored, a single initial allowed for a full word if at least one full
//! word agrees). One extra or one misspelt word is `review`, meaning a person must decide. Anything else, including
//! a single shared word, is `mismatch`. Nothing automatic acts on `review` or `mismatch`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;

use crate::{
    ApiError, AppState,
    auth::Auth,
    kyc::{Check, Kyc, Outcome},
    ratelimit::Bucket,
    tenant,
};

/// How many provider calls one business may make per hour (a paid provider gets the lower number).
pub const HOURLY_CAP: i64 = 100;
pub const HOURLY_CAP_PAID: i64 = 30;
/// A repeat of the same request inside this many minutes reuses the stored answer.
pub const REUSE_MINUTES: i64 = 10;
const BUCKET: Bucket = Bucket {
    burst: 10.0,
    per_sec: 0.25,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMatch {
    Match,
    Review,
    Mismatch,
    NotChecked,
}

impl NameMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            NameMatch::Match => "match",
            NameMatch::Review => "review",
            NameMatch::Mismatch => "mismatch",
            NameMatch::NotChecked => "not_checked",
        }
    }
}

const TITLES: [&str; 11] = [
    "MR", "MRS", "MS", "MISS", "DR", "SHRI", "SMT", "SRI", "SHRIMATI", "LATE", "LT",
];
const SUFFIXES: [&str; 5] = ["PVT", "LTD", "LIMITED", "PRIVATE", "LLP"];

pub fn name_tokens(s: &str) -> Vec<String> {
    s.to_uppercase()
        .replace("M/S", " ")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !TITLES.contains(w) && !SUFFIXES.contains(w))
        .map(str::to_string)
        .collect()
}

fn one_edit_apart(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 || a.len().min(b.len()) < 4 {
        return false;
    }
    if a.len() == b.len() {
        return a.iter().zip(&b).filter(|(x, y)| x != y).count() == 1;
    }
    let (short, long) = if a.len() < b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    (0..long.len()).any(|skip| {
        let rest: Vec<char> = long
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, c)| *c)
            .collect();
        &rest == short
    })
}

/// Compare the name given with the name the bank holds. See the module docs for what each result means. Never
/// returns `Match` on a single shared word, on two bare initials, or when one name merely contains the other.
pub fn compare_names(given: &str, registered: &str) -> NameMatch {
    let (a, b) = (name_tokens(given), name_tokens(registered));
    if a.is_empty() || b.is_empty() {
        return NameMatch::NotChecked;
    }
    // words that agree exactly
    let (mut ra, mut rb): (Vec<String>, Vec<String>) = (Vec::new(), b.clone());
    let mut exact = 0;
    for t in a {
        if let Some(i) = rb.iter().position(|x| *x == t) {
            rb.remove(i);
            exact += 1;
        } else {
            ra.push(t);
        }
    }
    if ra.is_empty() && rb.is_empty() {
        return NameMatch::Match;
    }
    if exact == 0 {
        return NameMatch::Mismatch;
    }
    if ra.is_empty() || rb.is_empty() {
        // one name is the other plus extra words: only a person can say whether it is the same party
        return if exact >= 2 {
            NameMatch::Review
        } else {
            NameMatch::Mismatch
        };
    }
    if ra.len() != rb.len() {
        return NameMatch::Mismatch;
    }
    let mut typo = false;
    for x in &ra {
        let initial = |i: &str, full: &str| i.chars().count() == 1 && full.starts_with(i);
        if let Some(p) = rb.iter().position(|y| initial(x, y) || initial(y, x)) {
            rb.remove(p);
        } else if let Some(p) = rb.iter().position(|y| one_edit_apart(x, y)) {
            rb.remove(p);
            typo = true;
        } else {
            return NameMatch::Mismatch;
        }
    }
    if typo {
        NameMatch::Review
    } else {
        NameMatch::Match
    }
}

pub fn valid_ifsc(i: &str) -> bool {
    i.len() == 11
        && i[..4].chars().all(|c| c.is_ascii_uppercase())
        && i.as_bytes()[4] == b'0'
        && i[5..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub fn valid_account(a: &str) -> bool {
    (6..=20).contains(&a.len()) && a.chars().all(|c| c.is_ascii_digit())
}

pub fn valid_vpa(v: &str) -> bool {
    let Some((local, handle)) = v.split_once('@') else {
        return false;
    };
    (2..=60).contains(&local.len())
        && (2..=30).contains(&handle.len())
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.')
}

pub fn mask_vpa(v: &str) -> String {
    match v.split_once('@') {
        Some((l, h)) => format!("{}***@{h}", l.chars().take(2).collect::<String>()),
        None => "***".into(),
    }
}

pub fn mask_account(a: &str) -> String {
    let n = a.chars().count();
    format!(
        "******{}",
        a.chars().skip(n.saturating_sub(4)).collect::<String>()
    )
}

/// What a check produced, ready to store and return.
#[derive(Debug, Clone)]
pub struct Verified {
    pub id: String,
    pub outcome: &'static str,
    pub name_match: NameMatch,
    pub provider: &'static str,
    pub simulated: bool,
    pub detail: String,
    pub reused: bool,
}

impl Verified {
    /// Valid and the name agrees. Nothing less is enough to pay someone automatically.
    pub fn safe_to_pay(&self) -> bool {
        self.outcome == "valid" && self.name_match == NameMatch::Match
    }
    fn json(&self, kind: &str, subject: &str, ifsc: Option<&str>) -> Value {
        json!({
            "id": self.id, "kind": kind, "subject": subject, "ifsc": ifsc,
            "outcome": self.outcome, "name_match": self.name_match.as_str(),
            "safe_to_pay": self.safe_to_pay(),
            "provider": self.provider, "simulated": self.simulated,
            "detail": self.detail, "reused": self.reused,
            "note": if self.simulated { "SIMULATED: no bank or payment network was asked. Never rely on this for real money." } else { "Checked with the provider named above." },
        })
    }
}

pub enum Subject<'a> {
    Vpa(&'a str),
    Bank { account: &'a str, ifsc: &'a str },
}

/// Run one verification: rate limit, reuse, provider call, store. Used by the endpoints and by payout links.
pub async fn run(
    s: &AppState,
    tenant_id: &str,
    env: &str,
    actor: &str,
    subject: Subject<'_>,
    name: &str,
) -> Result<Verified, ApiError> {
    let provider = match (&s.kyc, env) {
        (Some(k), "live") if k.is_simulated() => {
            return Err(ApiError::conflict(
                "verification_unavailable",
                "live mode needs a real verification provider; the simulator is configured, and its answers would not be real",
            ));
        }
        (Some(k), _) => k.clone(),
        (None, "live") => {
            return Err(ApiError::conflict(
                "verification_unavailable",
                "no verification provider is configured (ZYVOR_KYC_PROVIDER)",
            ));
        }
        (None, _) => Kyc::Simulator,
    };
    let (kind, masked, ifsc, raw) = match &subject {
        Subject::Vpa(v) => ("vpa", mask_vpa(v), None, v.to_string()),
        Subject::Bank { account, ifsc } => (
            "bank_account",
            mask_account(account),
            Some(ifsc.to_string()),
            format!("{account}|{ifsc}"),
        ),
    };
    if let Err(retry) = s
        .limiter
        .check(format!("verify:{tenant_id}"), BUCKET, Instant::now())
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            format!("too many verification requests; retry in {retry}s"),
        ));
    }
    // reuse a recent identical answer; otherwise check the hourly cap
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let key = crate::keyed::tenant_key(&mut tx, &s.crypto, tenant_id).await?;
    let request_hash = crate::keyed::hash(
        &key,
        kind,
        &format!("{raw}|{}", name_tokens(name).join(" ")),
    );
    let recent: Option<(String, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, outcome, name_match, provider, detail, simulated FROM verifications
          WHERE request_hash = $1 AND created_at > now() - make_interval(mins => $2::int) ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&request_hash)
    .bind(REUSE_MINUTES as i32)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, outcome, nm, prov, detail, simulated)) = recent {
        return Ok(Verified {
            id,
            outcome: static_outcome(&outcome),
            name_match: parse_match(&nm),
            provider: static_provider(&prov),
            simulated,
            detail,
            reused: true,
        });
    }
    let cap = if provider.is_metered() {
        HOURLY_CAP_PAID
    } else {
        HOURLY_CAP
    };
    let used: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM verifications WHERE created_at > now() - interval '1 hour'",
    )
    .fetch_one(&mut *tx)
    .await?;
    if used >= cap {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "verification_cap",
            format!("this business has used its {cap} verifications for the hour; try again later"),
        ));
    }
    // commits the business's hashing key if this call just made it
    tx.commit().await?;

    let check: Check = match &subject {
        Subject::Vpa(v) => provider.verify_vpa(v, name).await,
        Subject::Bank { account, ifsc } => provider.verify_bank(ifsc, account, name).await,
    };
    let outcome = match check.outcome {
        Outcome::Pass => "valid",
        Outcome::Fail => "invalid",
        Outcome::Inconclusive => "inconclusive",
    };
    let name_match = match (&check.registered_name, check.outcome) {
        (Some(reg), Outcome::Pass) if !name.trim().is_empty() => compare_names(name, reg),
        _ => NameMatch::NotChecked,
    };
    let id = zyvor_core::new_id("ver");
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    sqlx::query("INSERT INTO verifications (id, tenant_id, kind, subject_masked, ifsc, request_hash, outcome, name_match, provider, simulated, detail, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(&id).bind(tenant_id).bind(kind).bind(&masked).bind(&ifsc).bind(&request_hash)
        .bind(outcome).bind(name_match.as_str()).bind(provider.name()).bind(provider.is_simulated())
        .bind(&check.detail).bind(actor)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        tenant_id,
        actor,
        "verification.run",
        Some(&id),
        json!({ "kind": kind, "subject": masked, "outcome": outcome, "name_match": name_match.as_str(), "provider": provider.name() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Verified {
        id,
        outcome,
        name_match,
        provider: provider.name(),
        simulated: provider.is_simulated(),
        detail: check.detail,
        reused: false,
    })
}

fn static_outcome(s: &str) -> &'static str {
    match s {
        "valid" => "valid",
        "invalid" => "invalid",
        _ => "inconclusive",
    }
}
fn parse_match(s: &str) -> NameMatch {
    match s {
        "match" => NameMatch::Match,
        "review" => NameMatch::Review,
        "mismatch" => NameMatch::Mismatch,
        _ => NameMatch::NotChecked,
    }
}
fn static_provider(s: &str) -> &'static str {
    match s {
        "cashfree_secure_id" => "cashfree_secure_id",
        _ => "simulator",
    }
}

#[derive(Deserialize)]
pub struct VerifyVpa {
    /// The UPI id to check, like `name@bank`.
    vpa: String,
    /// The name you expect it to be held in. Optional; without it only existence is checked.
    name: Option<String>,
}

/// `POST /v1/verify/vpa`
pub async fn verify_vpa(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<VerifyVpa>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verify:write")?;
    let vpa = b.vpa.trim();
    if !valid_vpa(vpa) {
        return Err(ApiError::invalid(
            "invalid_vpa",
            "vpa must look like name@bank",
        ));
    }
    let name = b.name.as_deref().unwrap_or("").trim();
    if name.len() > 100 {
        return Err(ApiError::invalid("invalid_name", "name is too long"));
    }
    let v = run(
        &s,
        &auth.tenant_id,
        &auth.env,
        &auth.actor,
        Subject::Vpa(vpa),
        name,
    )
    .await?;
    Ok(Json(v.json("vpa", &mask_vpa(vpa), None)))
}

#[derive(Deserialize)]
pub struct VerifyBank {
    account_number: String,
    ifsc: String,
    /// The name the account should be held in. Without it only existence is checked.
    name: Option<String>,
}

/// `POST /v1/verify/bank_account`: a penny-drop style check through the configured provider.
pub async fn verify_bank(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<VerifyBank>,
) -> Result<Json<Value>, ApiError> {
    auth.require("verify:write")?;
    let (account, ifsc) = (b.account_number.trim(), b.ifsc.trim().to_uppercase());
    if !valid_account(account) || !valid_ifsc(&ifsc) {
        return Err(ApiError::invalid(
            "invalid_bank_account",
            "account_number must be 6-20 digits and ifsc 11 characters like HDFC0001234",
        ));
    }
    let name = b.name.as_deref().unwrap_or("").trim();
    if name.len() > 100 {
        return Err(ApiError::invalid("invalid_name", "name is too long"));
    }
    let v = run(
        &s,
        &auth.tenant_id,
        &auth.env,
        &auth.actor,
        Subject::Bank {
            account,
            ifsc: &ifsc,
        },
        name,
    )
    .await?;
    Ok(Json(v.json(
        "bank_account",
        &mask_account(account),
        Some(&ifsc),
    )))
}

#[derive(Deserialize, Default)]
pub struct ListQuery {
    kind: Option<String>,
    limit: Option<i64>,
}

/// `GET /v1/verifications`: what was checked, masked, newest first.
pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, Option<String>, String, String, String, bool, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT id, kind, subject_masked, ifsc, outcome, name_match, provider, simulated, created_by, created_at
           FROM verifications WHERE ($1::text IS NULL OR kind = $1) ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(&q.kind)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&mut *tx)
    .await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, kind, subject, ifsc, outcome, nm, provider, simulated, by, at)| json!({
        "id": id, "kind": kind, "subject": subject, "ifsc": ifsc, "outcome": outcome, "name_match": nm,
        "provider": provider, "simulated": simulated, "created_by": by, "created_at": at })).collect::<Vec<_>>() }),
    ))
}

/// `GET /v1/verifications/{id}`
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, Option<String>, String, String, String, bool, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT kind, subject_masked, ifsc, outcome, name_match, provider, simulated, detail, created_at FROM verifications WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, subject, ifsc, outcome, nm, provider, simulated, detail, at) =
        row.ok_or_else(|| ApiError::not_found("verification"))?;
    Ok(Json(json!({
        "id": id, "kind": kind, "subject": subject, "ifsc": ifsc, "outcome": outcome, "name_match": nm,
        "safe_to_pay": outcome == "valid" && nm == "match", "provider": provider, "simulated": simulated,
        "detail": detail, "created_at": at,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use NameMatch::*;

    #[test]
    fn identical_names_match_whatever_the_case_order_or_titles() {
        assert_eq!(compare_names("Ravi Kumar", "RAVI KUMAR"), Match);
        assert_eq!(compare_names("KUMAR RAVI", "Ravi Kumar"), Match);
        assert_eq!(compare_names("Mr. Ravi Kumar", "RAVI KUMAR"), Match);
        assert_eq!(
            compare_names("Kiran Traders Pvt. Ltd.", "KIRAN TRADERS PRIVATE LIMITED"),
            Match
        );
        assert_eq!(compare_names("M/s Kiran Traders", "KIRAN TRADERS"), Match);
    }

    #[test]
    fn one_initial_for_one_word_is_a_match_when_a_full_word_agrees() {
        assert_eq!(compare_names("R Kumar", "RAVI KUMAR"), Match);
        assert_eq!(compare_names("Ravi K", "RAVI KUMAR"), Match);
        // two bare initials prove nothing
        assert_eq!(compare_names("R K", "RAVI KUMAR"), Mismatch);
        // an initial that is the wrong letter
        assert_eq!(compare_names("S Kumar", "RAVI KUMAR"), Mismatch);
    }

    #[test]
    fn extra_or_misspelt_words_need_a_person() {
        assert_eq!(compare_names("Ravi Kumar", "RAVI KUMAR SHARMA"), Review);
        assert_eq!(compare_names("Ravi Kumar Sharma", "RAVI KUMAR"), Review);
        assert_eq!(compare_names("Ravi Kumar", "RAVII KUMAR"), Review);
        assert_eq!(compare_names("Priya Sharma", "PRIYA SHARMA"), Match);
    }

    #[test]
    fn a_shared_word_is_not_enough() {
        // the failure a lenient "any word in common" rule makes
        assert_eq!(compare_names("Ravi Kumar", "AMIT KUMAR"), Mismatch);
        assert_eq!(compare_names("Kumar", "RAVI KUMAR"), Mismatch);
        assert_eq!(compare_names("Ravi", "RAVI KUMAR"), Mismatch);
        assert_eq!(
            compare_names("Ravi Kumar", "SOMEONE ELSE ENTIRELY"),
            Mismatch
        );
        // short words are never "a typo" of each other
        assert_eq!(compare_names("Ravi Rao", "RAVI RAM"), Mismatch);
        // one letter apart in a longer word is "maybe the same person": a person decides, nothing is automatic
        assert_eq!(compare_names("Ravi Kumar", "RAVI KUMARI"), Review);
    }

    #[test]
    fn empty_names_are_not_checked() {
        assert_eq!(compare_names("", "RAVI KUMAR"), NotChecked);
        assert_eq!(compare_names("Ravi", "   "), NotChecked);
        assert_eq!(compare_names("Mr.", "Mrs."), NotChecked);
    }

    #[test]
    fn masks_hide_the_number_and_the_name_part() {
        assert_eq!(mask_account("026291800001191"), "******1191");
        assert_eq!(mask_account("123456"), "******3456");
        assert_eq!(mask_vpa("ravi.kumar@okhdfcbank"), "ra***@okhdfcbank");
        assert!(!mask_account("026291800001191").contains("0262"));
    }

    #[test]
    fn formats_are_checked_before_any_provider_is_called() {
        assert!(valid_ifsc("HDFC0001234") && valid_ifsc("YESB0000262"));
        assert!(
            !valid_ifsc("hdfc0001234") && !valid_ifsc("HDFC1001234") && !valid_ifsc("HDFC00012")
        );
        assert!(
            valid_account("026291800001191") && !valid_account("12ab56") && !valid_account("12345")
        );
        assert!(
            valid_vpa("ravi.k@okhdfcbank")
                && !valid_vpa("ravi")
                && !valid_vpa("a@b")
                && !valid_vpa("ra vi@ok")
        );
        assert!(!valid_vpa("x@@y") && !valid_vpa("ravi@ok.hdfc/x"));
    }

    #[test]
    fn only_a_valid_account_with_a_matching_name_is_safe_to_pay() {
        let v = |outcome, nm| Verified {
            id: "ver_1".into(),
            outcome,
            name_match: nm,
            provider: "simulator",
            simulated: true,
            detail: String::new(),
            reused: false,
        };
        assert!(v("valid", Match).safe_to_pay());
        assert!(!v("valid", Review).safe_to_pay());
        assert!(!v("valid", NotChecked).safe_to_pay());
        assert!(!v("inconclusive", Match).safe_to_pay());
        assert!(!v("invalid", Match).safe_to_pay());
    }
}
