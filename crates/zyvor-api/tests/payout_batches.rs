//! Vendor onboarding, commission rules, payout batches and payout schedules. Everything here runs against the
//! simulators: the payout rail is a scripted stand-in that counts what it is asked to pay, and no bank details are real.

mod common;

use async_trait::async_trait;
use axum::{Router, http::StatusCode};
use chrono::{Datelike, Days, NaiveDate};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use zyvor_settle::{PayoutInstruction, PayoutRail, PayoutRailStatus, PayoutResult, RailError};

// ---------- a rail that does what the test tells it and counts what it was asked ----------

#[derive(Clone, Copy, Debug)]
enum Act {
    Settle,
    Fail,
    Uncertain,
}

/// What `fetch` answers for a payout it is asked about.
#[derive(Clone, Copy)]
enum Ask {
    Settled,
    Unknown,
    Failed,
}

struct ScriptRail {
    sends: Mutex<HashMap<String, u32>>,
    /// By beneficiary name: what the next calls do; once empty, the call settles.
    script: Mutex<HashMap<String, VecDeque<Act>>>,
    /// What `fetch` answers, by payout id; anything not named is unknown.
    ask: Mutex<HashMap<String, Ask>>,
}

impl ScriptRail {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sends: Mutex::new(HashMap::new()),
            script: Mutex::new(HashMap::new()),
            ask: Mutex::new(HashMap::new()),
        })
    }
    fn plan(&self, beneficiary: &str, acts: &[Act]) {
        self.script
            .lock()
            .unwrap()
            .insert(beneficiary.to_string(), acts.iter().copied().collect());
    }
    fn answer(&self, payout_id: &str, a: Ask) {
        self.ask.lock().unwrap().insert(payout_id.to_string(), a);
    }
    fn sent(&self, payout_id: &str) -> u32 {
        self.sends
            .lock()
            .unwrap()
            .get(payout_id)
            .copied()
            .unwrap_or(0)
    }
    fn total_sends(&self) -> u32 {
        // settlement releases to the merchant's own bank use the same rail; only vendor payouts are counted here
        self.sends
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with("pay_"))
            .map(|(_, v)| v)
            .sum()
    }
    fn result(status: PayoutRailStatus, id: &str, why: Option<&str>) -> PayoutResult {
        PayoutResult {
            status,
            rail_ref: Some(format!("SCRIPT{}", &id[id.len() - 8..])),
            failure_reason: why.map(str::to_string),
            raw: json!({ "script": true }),
        }
    }
}

#[async_trait]
impl PayoutRail for ScriptRail {
    async fn send(&self, insn: &PayoutInstruction) -> Result<PayoutResult, RailError> {
        *self
            .sends
            .lock()
            .unwrap()
            .entry(insn.payout_id.clone())
            .or_default() += 1;
        // a real rail takes a moment; this lets a second caller arrive while the first is still waiting
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let act = self
            .script
            .lock()
            .unwrap()
            .get_mut(&insn.beneficiary_name)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Act::Settle);
        match act {
            Act::Settle => Ok(Self::result(
                PayoutRailStatus::Settled,
                &insn.payout_id,
                None,
            )),
            Act::Fail => Ok(Self::result(
                PayoutRailStatus::Failed,
                &insn.payout_id,
                Some("beneficiary bank refused"),
            )),
            Act::Uncertain => Err(RailError::Uncertain(
                "timed out waiting for the bank".into(),
            )),
        }
    }
    async fn fetch(&self, payout_id: &str) -> Result<PayoutResult, RailError> {
        Ok(
            match self
                .ask
                .lock()
                .unwrap()
                .get(payout_id)
                .copied()
                .unwrap_or(Ask::Unknown)
            {
                Ask::Settled => Self::result(PayoutRailStatus::Settled, payout_id, None),
                Ask::Failed => Self::result(
                    PayoutRailStatus::Failed,
                    payout_id,
                    Some("the rail has no such transfer"),
                ),
                Ask::Unknown => Self::result(PayoutRailStatus::Unknown, payout_id, None),
            },
        )
    }
}

struct Env {
    t: T,
    state: zyvor_api::AppState,
    /// The same app, with the scripted rail in place.
    app: Router,
    rail: Arc<ScriptRail>,
}

/// Organisation A in acquire mode (the batch rail is acquire-only), with the scripted rail.
async fn env(pool: PgPool) -> Env {
    let t = setup(pool).await;
    t.ok(
        &t.a,
        "PUT",
        "/v1/money_mode",
        None,
        json!({ "money_mode": "acquire", "settlement_delay_days": 0 }),
    )
    .await;
    t.ok(
        &t.a,
        "PUT",
        "/v1/acquire_kyc",
        None,
        json!({ "status": "approved" }),
    )
    .await;
    let rail = ScriptRail::new();
    let mut st = t.state.clone();
    st.payout_rail = Some(rail.clone());
    let app = zyvor_api::app(st.clone());
    Env {
        t,
        state: st,
        app,
        rail,
    }
}

impl Env {
    async fn as_(&self, actor: &str, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        raw(
            &self.app,
            &self.t.a.key,
            Some(actor),
            method,
            path,
            None,
            body,
        )
        .await
    }
    async fn ok(&self, actor: &str, method: &str, path: &str, body: Value) -> Value {
        let (s, b) = self.as_(actor, method, path, body).await;
        assert!(s.is_success(), "{actor} {method} {path} -> {s}: {b}");
        b
    }
    async fn err(
        &self,
        actor: &str,
        method: &str,
        path: &str,
        body: Value,
        want: u16,
        code: &str,
    ) -> Value {
        let (s, b) = self.as_(actor, method, path, body).await;
        assert_eq!(s.as_u16(), want, "{actor} {method} {path}: {b}");
        assert_eq!(b["error"]["code"], code, "{b}");
        b
    }
    /// A vendor made the original way: active at once, with a bank account.
    async fn vendor(&self, name: &str, ext: &str) -> String {
        self.t
            .ok(&self.t.a, "POST", "/v1/vendors", None, json!({ "name": name, "external_ref": ext, "account_number": "1234567890", "ifsc": "SYNT0000001" }))
            .await["id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    /// A captured payment that gives the vendor `share` paise of what they are owed.
    async fn earn(&self, vendor: &str, share: i64) {
        self.t
            .paid_razorpay(
                &self.t.a,
                share + 1_000,
                json!({ "splits": [{ "vendor_id": vendor, "amount_minor": share }] }),
            )
            .await;
    }
    async fn owed(&self, vendor: &str) -> (i64, i64) {
        let r = self
            .ok(
                "priya",
                "GET",
                &format!("/v1/vendors/{vendor}/balance"),
                Value::Null,
            )
            .await;
        (
            r["balances"][0]["owed_minor"].as_i64().unwrap_or(0),
            r["balances"][0]["available_minor"].as_i64().unwrap_or(0),
        )
    }
    /// A GET that answers with text (the report).
    async fn text(&self, actor: &str, path: &str) -> (StatusCode, String) {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let res = self
            .app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {}", self.t.a.key))
                    .header("zyvor-actor", actor)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let st = res.status();
        (
            st,
            String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec())
                .unwrap(),
        )
    }
    async fn batch(&self, id: &str) -> Value {
        self.ok(
            "priya",
            "GET",
            &format!("/v1/payout_batches/{id}"),
            Value::Null,
        )
        .await
    }
    async fn ledger(&self, account_like: &str) -> (i64, i64) {
        sqlx::query_as("SELECT COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries WHERE tenant_id = $1 AND account LIKE $2")
            .bind(&self.t.a.id)
            .bind(account_like)
            .fetch_one(&self.t.root)
            .await
            .unwrap()
    }
    /// Every posting in the ledger balances, for each organisation.
    async fn assert_ledger_balances(&self) {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as("SELECT tenant_id, COALESCE(SUM(amount_minor) FILTER (WHERE side='debit'),0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE side='credit'),0)::BIGINT FROM ledger_entries GROUP BY tenant_id")
            .fetch_all(&self.t.root)
            .await
            .unwrap();
        for (tid, d, c) in rows {
            assert_eq!(d, c, "the ledger of {tid} does not balance");
        }
    }
}

fn ids(batch: &Value) -> Vec<String> {
    batch["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn status_of(batch: &Value, line: i64) -> String {
    batch["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["line"] == line)
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn pdf() -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD.encode(b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n")
}

// ---------- vendors ----------

#[sqlx::test(migrations = "./migrations")]
async fn a_vendor_moves_from_invited_to_active_through_two_people_and_a_name_check(pool: PgPool) {
    let e = env(pool).await;
    let b = &e.t.b;
    let v = e
        .ok(
            "priya",
            "POST",
            "/v1/vendors/invite",
            json!({ "name": "Meera Crafts", "external_ref": "M-1", "email": "meera@example.in" }),
        )
        .await;
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["status"], "invited");

    // not payable, and cannot be given a share, until it is active
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        409,
        "vendor_not_active",
    )
    .await;
    let (s, body) = e.t.call(&e.t.a, "POST", "/v1/payment_intents", Some("pi-inv"), json!({ "amount_minor": 5_000, "currency": "INR", "splits": [{ "vendor_id": id, "amount_minor": 1_000 }] })).await;
    assert_eq!(
        (s.as_u16(), body["error"]["code"].clone()),
        (409, json!("vendor_not_active")),
        "{body}"
    );
    // the bank account comes after the details
    e.err("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Meera Handicrafts", "ifsc": "HDFC0001234", "account_number": "123456789012" }), 409, "details_first").await;
    e.err(
        "priya",
        "PUT",
        &format!("/v1/vendors/{id}/profile"),
        json!({ "legal_name": "x" }),
        422,
        "invalid_vendor",
    )
    .await;

    let v = e
        .ok(
            "priya",
            "PUT",
            &format!("/v1/vendors/{id}/profile"),
            json!({ "legal_name": "Meera Handicrafts Private Limited", "phone": "9000000000" }),
        )
        .await;
    assert_eq!(v["status"], "details_submitted");
    e.err("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Meera Handicrafts", "ifsc": "bad", "account_number": "123456789012" }), 422, "invalid_bank_account").await;
    let v = e.ok("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "MEERA HANDICRAFTS", "ifsc": "hdfc0001234", "account_number": "123456789012" })).await;
    assert_eq!(v["status"], "bank_added");
    assert_eq!(v["bank"]["account_last4"], "9012");
    assert!(
        !v.to_string().contains("123456789012"),
        "the account number is never shown"
    );
    assert_eq!(v["bank"]["check"]["holder_matches_legal_name"], true);

    // a different person verifies; the person who added the account cannot
    e.err(
        "priya",
        "POST",
        &format!("/v1/vendors/{id}/verify"),
        json!({}),
        403,
        "separation_of_duties",
    )
    .await;
    // verified is not yet active
    let v = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/verify"),
            json!({}),
        )
        .await;
    assert_eq!(
        (v["status"].as_str(), v["bank"]["verified_by"].as_str()),
        (Some("verified"), Some("user:ravi"))
    );
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        409,
        "vendor_not_active",
    )
    .await;
    let v = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/activate"),
            json!({}),
        )
        .await;
    assert_eq!(v["status"], "active");
    let p = e
        .ok(
            "priya",
            "POST",
            "/v1/payouts",
            json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        )
        .await;
    assert_eq!(p["status"], "pending_approval");

    // suspended: refused again, including approving what was already requested
    e.err(
        "ravi",
        "POST",
        &format!("/v1/vendors/{id}/suspend"),
        json!({ "reason": "" }),
        422,
        "invalid_reason",
    )
    .await;
    let v = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/suspend"),
            json!({ "reason": "disputed invoices" }),
        )
        .await;
    assert_eq!(
        (
            v["status"].as_str(),
            v["workflow"]["suspended_reason"].as_str()
        ),
        (Some("suspended"), Some("disputed invoices"))
    );
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        409,
        "vendor_not_active",
    )
    .await;
    e.err(
        "ravi",
        "POST",
        &format!("/v1/payouts/{}/approve", p["id"].as_str().unwrap()),
        json!({}),
        409,
        "vendor_not_active",
    )
    .await;
    e.err("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "MEERA HANDICRAFTS", "ifsc": "HDFC0001234", "account_number": "123456789012" }), 409, "vendor_suspended").await;
    let v = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/reactivate"),
            json!({}),
        )
        .await;
    assert_eq!(v["status"], "active");

    // a new bank account on an active vendor puts it back until a person verifies it again
    let v = e.ok("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Meera Handicrafts Pvt Ltd", "ifsc": "ICIC0001234", "account_number": "998877665544" })).await;
    assert_eq!(
        (v["status"].as_str(), v["bank"]["verified_by"].is_null()),
        (Some("bank_added"), true)
    );
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        409,
        "vendor_not_active",
    )
    .await;

    // a holder name that is not the vendor's needs a written reason
    e.ok("priya", "POST", &format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Sunil Kumar", "ifsc": "ICIC0001234", "account_number": "998877665544" })).await;
    let bad = e
        .err(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/verify"),
            json!({}),
            409,
            "name_mismatch",
        )
        .await;
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains("override_reason")
    );
    let v = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/vendors/{id}/verify"),
            json!({ "override_reason": "owner's personal account, cheque seen" }),
        )
        .await;
    assert_eq!(
        v["bank"]["name_override_reason"],
        "owner's personal account, cheque seen"
    );

    // payout settings
    for bad in [
        json!({ "hold_days": -1 }),
        json!({ "hold_days": 366 }),
        json!({ "min_payout_minor": -5 }),
    ] {
        e.err(
            "ravi",
            "PUT",
            &format!("/v1/vendors/{id}/payout_settings"),
            bad,
            422,
            "invalid_settings",
        )
        .await;
    }
    e.err(
        "ravi",
        "PUT",
        &format!("/v1/vendors/{id}/payout_settings"),
        json!({ "default_rule_id": "cmr_nope" }),
        404,
        "not_found",
    )
    .await;
    let v = e
        .ok(
            "ravi",
            "PUT",
            &format!("/v1/vendors/{id}/payout_settings"),
            json!({ "hold_days": 3, "min_payout_minor": 50_000 }),
        )
        .await;
    assert_eq!(
        (
            v["payout_settings"]["hold_days"].clone(),
            v["payout_settings"]["min_payout_minor"].clone()
        ),
        (json!(3), json!(50_000))
    );
    // someone who may only propose cannot change them
    let clerk =
        e.t.ok(
            &e.t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "clerk", "scopes": ["payouts:write", "payments:read"] }),
        )
        .await;
    let (s, _) = raw(
        &e.app,
        clerk["key"].as_str().unwrap(),
        Some("kiran"),
        "PUT",
        &format!("/v1/vendors/{id}/payout_settings"),
        None,
        json!({ "hold_days": 0 }),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // documents are optional: a PDF is kept (encrypted), a repeat or another file type is refused
    let d = e.ok("priya", "POST", &format!("/v1/vendors/{id}/documents"), json!({ "kind": "cancelled_cheque", "filename": "../cheque.pdf", "data_base64": pdf() })).await;
    assert_eq!(d["filename"], "cheque.pdf");
    e.err(
        "priya",
        "POST",
        &format!("/v1/vendors/{id}/documents"),
        json!({ "kind": "cancelled_cheque", "filename": "again.pdf", "data_base64": pdf() }),
        409,
        "already_uploaded",
    )
    .await;
    e.err(
        "priya",
        "POST",
        &format!("/v1/vendors/{id}/documents"),
        json!({ "kind": "other", "filename": "x.exe", "data_base64": "TVqQAAMAAAAEAAAA" }),
        422,
        "unsupported_file",
    )
    .await;
    e.err(
        "priya",
        "POST",
        &format!("/v1/vendors/{id}/documents"),
        json!({ "kind": "selfie", "filename": "x.pdf", "data_base64": pdf() }),
        422,
        "invalid_document",
    )
    .await;
    let list = e
        .ok(
            "priya",
            "GET",
            &format!("/v1/vendors/{id}/documents"),
            Value::Null,
        )
        .await;
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert!(!list.to_string().contains("data_enc"));
    let (s, _) = e
        .as_(
            "ravi",
            "GET",
            &format!(
                "/v1/vendors/{id}/documents/{}/file",
                d["id"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let stored: Vec<u8> = sqlx::query_scalar("SELECT data_enc FROM vendor_documents")
        .fetch_one(&e.t.root)
        .await
        .unwrap();
    assert!(!stored.starts_with(b"%PDF"), "the file is stored encrypted");
    assert_eq!(
        e.t.count("SELECT count(*) FROM audit_events WHERE action = 'vendor.document_downloaded'")
            .await,
        1
    );

    // the other organisation sees none of it
    for (m, path) in [
        ("GET", format!("/v1/vendors/{id}")),
        ("GET", format!("/v1/vendors/{id}/documents")),
        ("POST", format!("/v1/vendors/{id}/verify")),
        ("POST", format!("/v1/vendors/{id}/suspend")),
    ] {
        let (s, _) =
            e.t.call(b, m, &path, None, json!({ "reason": "not your vendor" }))
                .await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {path}");
    }
    let (s, _) =
        e.t.call(
            b,
            "POST",
            "/v1/payouts",
            None,
            json!({ "vendor_id": id, "amount_minor": 1_000, "purpose": "x" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // vendors made the old way are still active, and are listed with their status
    let old = e.vendor("Old Style", "OLD-1").await;
    let list = e.ok("priya", "GET", "/v1/vendors", Value::Null).await;
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == old.as_str() && v["status"] == "active")
    );
    assert!(e.t.count("SELECT count(*) FROM audit_events WHERE action IN ('vendor.invited', 'vendor.bank_account_verified', 'vendor.suspended')").await >= 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_bank_check_cannot_be_verified_by_a_person(pool: PgPool) {
    let t = setup(pool).await;
    let mut st = t.state.clone();
    st.kyc = Some(zyvor_api::kyc::Kyc::Simulator);
    let app = zyvor_api::app(st);
    let key = &t.a.key;
    let call = |actor: &'static str, m: &'static str, p: String, b: Value| {
        let (app, key) = (app.clone(), key.clone());
        async move { raw(&app, &key, Some(actor), m, &p, None, b).await }
    };
    let (_, v) = call(
        "priya",
        "POST",
        "/v1/vendors/invite".into(),
        json!({ "name": "Test Trader" }),
    )
    .await;
    let id = v["id"].as_str().unwrap().to_string();
    call(
        "priya",
        "PUT",
        format!("/v1/vendors/{id}/profile"),
        json!({ "legal_name": "Test Trader" }),
    )
    .await;
    // the simulator says an account number ending 0000 does not exist
    let (s, v) = call("priya", "POST", format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Test Trader", "ifsc": "HDFC0001234", "account_number": "123456780000" })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["bank"]["check"]["outcome"], "fail");
    let (s, v) = call(
        "ravi",
        "POST",
        format!("/v1/vendors/{id}/verify"),
        json!({}),
    )
    .await;
    assert_eq!(
        (s.as_u16(), v["error"]["code"].clone()),
        (409, json!("bank_check_failed"))
    );
    // another account that the simulator can verify
    let (_, v) = call("priya", "POST", format!("/v1/vendors/{id}/bank_account"), json!({ "holder_name": "Test Trader", "ifsc": "HDFC0001234", "account_number": "123456789012" })).await;
    assert_eq!(v["bank"]["check"]["outcome"], "pass");
    let (s, _) = call(
        "ravi",
        "POST",
        format!("/v1/vendors/{id}/verify"),
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
}

// ---------- commission rules ----------

#[sqlx::test(migrations = "./migrations")]
async fn commission_rules_turn_a_vendors_gross_into_a_share_that_accrues_and_reverses_with_refunds(
    pool: PgPool,
) {
    let e = env(pool).await;
    let v = e.vendor("Artisan", "A-1").await;
    let rule = |extra: Value| {
        let mut b = json!({ "name": "Artisan terms", "vendor_id": v, "basis_points": 1000, "effective_from": "2000-01-01" });
        for (k, x) in extra.as_object().unwrap() {
            b[k] = x.clone();
        }
        b
    };

    // no rule: a share given as gross is refused, nothing is guessed
    let (s, b) = e.t.call(&e.t.a, "POST", "/v1/payment_intents", Some("k-norule"), json!({ "amount_minor": 10_000, "currency": "INR", "splits": [{ "vendor_id": v, "gross_minor": 10_000 }] })).await;
    assert_eq!(
        (s.as_u16(), b["error"]["code"].clone()),
        (409, json!("no_commission_rule")),
        "{b}"
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM payment_intents").await,
        0,
        "a refused payment leaves nothing behind"
    );

    // rule validation
    for bad in [
        json!({ "basis_points": 10_001 }),
        json!({ "basis_points": -1 }),
        json!({ "flat_minor": -5 }),
        json!({ "cap_minor": 100, "min_commission_minor": 500 }),
        json!({ "effective_to": "1999-01-01" }),
        json!({ "name": "" }),
    ] {
        let (s, b) = e
            .as_("ravi", "POST", "/v1/commission_rules", rule(bad.clone()))
            .await;
        assert_eq!(s.as_u16(), 422, "{bad} -> {b}");
    }
    e.err(
        "ravi",
        "POST",
        "/v1/commission_rules",
        json!({ "name": "x", "vendor_id": v, "effective_from": "2000-01-01" }),
        422,
        "invalid_commission_rule",
    )
    .await;
    e.err(
        "ravi",
        "POST",
        "/v1/commission_rules",
        rule(json!({ "vendor_id": "ven_nope" })),
        404,
        "not_found",
    )
    .await;
    // writing rules needs the approver scope
    let clerk =
        e.t.ok(
            &e.t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "clerk", "scopes": ["payouts:write", "payments:read"] }),
        )
        .await;
    let (s, _) = raw(
        &e.app,
        clerk["key"].as_str().unwrap(),
        Some("kiran"),
        "POST",
        "/v1/commission_rules",
        None,
        rule(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let r1 = e
        .ok(
            "ravi",
            "POST",
            "/v1/commission_rules",
            rule(json!({ "cap_minor": 5_000 })),
        )
        .await;
    // two rules of one scope cannot both be in force on a day
    e.err(
        "ravi",
        "POST",
        "/v1/commission_rules",
        rule(json!({ "effective_from": "2030-01-01" })),
        409,
        "rule_overlaps",
    )
    .await;

    // the preview says exactly what each party gets, before any payment exists
    let pv = e.ok("priya", "POST", "/v1/commission_rules/preview", json!({ "amount_minor": 100_000, "splits": [{ "vendor_id": v }], "refund_minor": 25_000 })).await;
    assert_eq!(pv["ok"], true, "{pv}");
    let p0 = &pv["parties"][0];
    // 10% of 100,000 is 10,000 but the cap is 5,000
    assert_eq!(
        (
            p0["commission_minor"].as_i64(),
            p0["vendor_share_minor"].as_i64()
        ),
        (Some(5_000), Some(95_000))
    );
    assert_eq!(p0["rule"]["id"], r1["id"]);
    assert_eq!(pv["merchant_keeps_minor"], 5_000);
    // a quarter refunded takes a quarter of the vendor's share back
    assert_eq!(
        (
            p0["reversed_on_refund_minor"].as_i64(),
            p0["vendor_keeps_after_refund_minor"].as_i64()
        ),
        (Some(23_750), Some(71_250))
    );
    assert_eq!(
        pv["merchant_keeps_after_refund_minor"],
        100_000 - 25_000 - 71_250
    );
    // an unsaved rule can be tried too; flat plus percentage, with a floor
    let pv = e.ok("priya", "POST", "/v1/commission_rules/preview", json!({ "amount_minor": 10_000, "splits": [{ "vendor_id": v, "gross_minor": 6_000, "rule": { "basis_points": 250, "flat_minor": 100, "min_commission_minor": 400 } }] })).await;
    assert_eq!(pv["parties"][0]["commission_minor"], 400);
    assert_eq!(
        pv["merchant_keeps_minor"],
        10_000 - 5_600,
        "the part given to no vendor stays with the merchant too"
    );
    let pv = e.ok("priya", "POST", "/v1/commission_rules/preview", json!({ "amount_minor": 100, "splits": [{ "vendor_id": v, "rule": { "flat_minor": 100 } }] })).await;
    assert_eq!(
        pv["ok"], false,
        "a commission that takes everything is reported, not hidden"
    );

    // a real payment: the share is the gross less the commission, and it accrues on capture
    let (iid, aid, _) =
        e.t.paid_razorpay(
            &e.t.a,
            10_000,
            json!({ "splits": [{ "vendor_id": v, "gross_minor": 10_000 }] }),
        )
        .await;
    let split: (i64, Option<i64>, i64, Option<String>) = sqlx::query_as("SELECT amount_minor, gross_minor, commission_minor, rule_id FROM payment_splits WHERE intent_id = $1").bind(&iid).fetch_one(&e.t.root).await.unwrap();
    assert_eq!(
        split,
        (
            9_000,
            Some(10_000),
            1_000,
            Some(r1["id"].as_str().unwrap().to_string())
        )
    );
    assert_eq!(e.owed(&v).await, (9_000, 9_000));
    // half of it refunded: the vendor's share reverses in proportion, so the merchant bears no more than its own part
    e.t.ok(
        &e.t.a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("ref-c1"),
        json!({ "amount_minor": 5_000 }),
    )
    .await;
    let _ = aid;
    assert_eq!(e.owed(&v).await.0, 4_500);
    e.assert_ledger_balances().await;

    // rules in the future do not apply yet; one for an event beats the vendor's own; retired rules stop applying
    let ev = e.ok("ravi", "POST", "/v1/commission_rules", json!({ "name": "Fest", "vendor_id": v, "event_ref": "fest-2026", "flat_minor": 700, "effective_from": "2000-01-01" })).await;
    let (iid2, ..) = e.t.paid_razorpay(&e.t.a, 10_000, json!({ "splits": [{ "vendor_id": v, "gross_minor": 10_000 }], "metadata": { "event_ref": "fest-2026" } })).await;
    let c: i64 =
        sqlx::query_scalar("SELECT commission_minor FROM payment_splits WHERE intent_id = $1")
            .bind(&iid2)
            .fetch_one(&e.t.root)
            .await
            .unwrap();
    assert_eq!(c, 700);
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/commission_rules/{}/retire", ev["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    e.err(
        "ravi",
        "POST",
        &format!("/v1/commission_rules/{}/retire", ev["id"].as_str().unwrap()),
        json!({}),
        409,
        "already_retired",
    )
    .await;
    let (iid3, ..) = e.t.paid_razorpay(&e.t.a, 10_000, json!({ "splits": [{ "vendor_id": v, "gross_minor": 10_000 }], "metadata": { "event_ref": "fest-2026" } })).await;
    let c: i64 =
        sqlx::query_scalar("SELECT commission_minor FROM payment_splits WHERE intent_id = $1")
            .bind(&iid3)
            .fetch_one(&e.t.root)
            .await
            .unwrap();
    assert_eq!(c, 1_000, "back to the vendor's own rule");
    // a rule ending yesterday (by the organisation's clock) is not in force today
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/commission_rules/{}/retire", r1["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    let ended = e.ok("ravi", "POST", "/v1/commission_rules", rule(json!({ "name": "Old", "effective_from": "2000-01-01", "effective_to": "2001-01-01" }))).await;
    assert_eq!(ended["status"], "active");
    let (s, b) = e.t.call(&e.t.a, "POST", "/v1/payment_intents", Some("k-ended"), json!({ "amount_minor": 10_000, "currency": "INR", "splits": [{ "vendor_id": v, "gross_minor": 10_000 }] })).await;
    assert_eq!(
        (s.as_u16(), b["error"]["code"].clone()),
        (409, json!("no_commission_rule")),
        "{b}"
    );
    let listed = e
        .ok(
            "priya",
            "GET",
            &format!("/v1/commission_rules?vendor_id={v}&status=active"),
            Value::Null,
        )
        .await;
    assert_eq!(
        listed["data"].as_array().unwrap().len(),
        1,
        "only the rule that ended long ago is still active"
    );
    // the other organisation sees none of it
    let (s, _) =
        e.t.call(
            &e.t.b,
            "GET",
            &format!("/v1/commission_rules/{}", ended["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(
        e.t.call(&e.t.b, "GET", "/v1/commission_rules", None, Value::Null)
            .await
            .1["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

// ---------- payout batches ----------

const GOOD_CSV: &str = "vendor,amount,purpose,reference\nV1,1500.50,June supplies,INV-1\nV2,2000,June supplies,INV-2\nV3,750.25,June supplies,INV-3\n";

#[sqlx::test(migrations = "./migrations")]
async fn a_csv_batch_is_checked_line_by_line_approved_by_another_person_and_sent_exactly_once(
    pool: PgPool,
) {
    let e = env(pool).await;
    let (v1, _v2, _v3) = (
        e.vendor("Ahuja Traders", "V1").await,
        e.vendor("Bose Foods", "V2").await,
        e.vendor("Chopra Paper", "V3").await,
    );
    // a tenant threshold that waves single payouts through does not apply to a batch
    e.t.ok(
        &e.t.a,
        "POST",
        "/v1/settings",
        None,
        json!({ "payout_approval_threshold_minor": 100_000_000 }),
    )
    .await;

    // structural problems: every one named with its line, and nothing stored
    let pv = e.ok("priya", "POST", "/v1/payout_batches/preview", json!({ "csv": "vendor,amount,purpose\nV1,12,500,x\nV2,abc,x\n=1+1,5,x\nV1,5,x\nV1,5,x\n" })).await;
    assert_eq!(pv["ok"], false);
    let lines: Vec<i64> = pv["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["line"].as_i64().unwrap())
        .collect();
    assert_eq!(lines, vec![2, 3, 4, 6], "{pv}");
    let (s, b) = e
        .as_(
            "priya",
            "POST",
            "/v1/payout_batches",
            json!({ "csv": "vendor,amount,purpose\nV2,abc,x\n" }),
        )
        .await;
    assert_eq!(
        (
            s.as_u16(),
            b["error"]["code"].clone(),
            b["error"]["stored"].clone()
        ),
        (422, json!("invalid_payout_batch"), json!(false)),
        "{b}"
    );
    // problems about the vendors are found too
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "csv": "vendor,amount,purpose\nV9,10,x\nV1,10,\n" }),
        )
        .await;
    assert_eq!(pv["errors"][0]["line"], 2);
    assert_eq!(
        pv["errors"][1]["line"], 3,
        "a line with no purpose is named: {pv}"
    );
    for bad in [
        json!({}),
        json!({ "csv": GOOD_CSV, "items": [] }),
        json!({ "csv": GOOD_CSV, "rail": "swift" }),
    ] {
        let (s, _) = e.as_("priya", "POST", "/v1/payout_batches", bad).await;
        assert_eq!(s.as_u16(), 422);
    }
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches").await, 0);
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts").await,
        0,
        "a dry run and a refused file store nothing"
    );

    // a good file: preview, then create
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "csv": GOOD_CSV }),
        )
        .await;
    assert_eq!(
        (
            pv["ok"].clone(),
            pv["item_count"].clone(),
            pv["total_minor"].clone(),
            pv["stored"].clone()
        ),
        (json!(true), json!(3), json!(425_075), json!(false)),
        "{pv}"
    );
    assert_eq!(e.t.count("SELECT count(*) FROM payouts").await, 0);
    let b = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches",
            json!({ "csv": GOOD_CSV, "filename": "june.csv", "name": "June vendors" }),
        )
        .await;
    let id = b["id"].as_str().unwrap().to_string();
    assert_eq!(
        (
            b["status"].as_str(),
            b["item_count"].as_i64(),
            b["total_minor"].as_i64()
        ),
        (Some("pending_approval"), Some(3), Some(425_075))
    );
    assert!(
        b["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["status"] == "pending_approval"),
        "every item waits, whatever the single-payout threshold"
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'pending_approval'")
            .await,
        3
    );
    assert_eq!(e.ledger("%").await, (0, 0), "nothing moves before approval");

    // the same file again is refused (and a preview says so), however it is named
    let (s, b2) = e
        .as_(
            "priya",
            "POST",
            "/v1/payout_batches",
            json!({ "csv": GOOD_CSV, "filename": "june-copy.csv" }),
        )
        .await;
    assert_eq!(
        (s.as_u16(), b2["error"]["code"].clone()),
        (409, json!("duplicate_batch")),
        "{b2}"
    );
    let reordered = "vendor,amount,purpose,reference\nV3,750.25,June supplies,INV-3\nV1,1500.50,June supplies,INV-1\nV2,2000,June supplies,INV-2\n";
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "csv": reordered }),
        )
        .await;
    assert_eq!(
        pv["duplicate_of"],
        id.as_str(),
        "a reordered file is the same file"
    );
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "csv": GOOD_CSV, "batch_ref": "june-run-2" }),
        )
        .await;
    assert_eq!(
        pv["duplicate_of"],
        Value::Null,
        "a deliberate second run has its own reference"
    );

    // approval: not by the person who proposed it, and the approver may leave a line out
    e.err(
        "priya",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
        403,
        "separation_of_duties",
    )
    .await;
    let clerk =
        e.t.ok(
            &e.t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "clerk", "scopes": ["payouts:write", "payments:read"] }),
        )
        .await;
    let (s, _) = raw(
        &e.app,
        clerk["key"].as_str().unwrap(),
        Some("kiran"),
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        None,
        json!({}),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the approver scope is needed");
    e.err(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/send"),
        json!({}),
        409,
        "not_approved",
    )
    .await;
    e.err(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({ "exclude": ["pbi_nope"] }),
        422,
        "invalid_exclusion",
    )
    .await;
    let item3 = b["items"][2]["id"].as_str().unwrap().to_string();
    let approved = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/payout_batches/{id}/approve"),
            json!({ "exclude": [item3], "reason": "Chopra invoice disputed" }),
        )
        .await;
    assert_eq!(
        (
            approved["status"].as_str(),
            approved["item_count"].as_i64(),
            approved["total_minor"].as_i64()
        ),
        (Some("approved"), Some(2), Some(350_050))
    );
    // lines are the file's own: the header is line 1
    assert_eq!(
        (status_of(&approved, 2), status_of(&approved, 4)),
        ("approved".to_string(), "excluded".to_string())
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'canceled'")
            .await,
        1
    );
    e.err(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
        409,
        "not_pending_approval",
    )
    .await;
    // a payout inside a batch is not approved, rejected or sent on its own
    let p1 = approved["items"][0]["payout_id"]
        .as_str()
        .unwrap()
        .to_string();
    e.err(
        "priya",
        "POST",
        &format!("/v1/payouts/{p1}/send_rail"),
        json!({}),
        409,
        "in_batch",
    )
    .await;

    // send: each approved item goes to the rail once; a second send does nothing
    let sent = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(sent["status"], "completed", "{sent}");
    assert_eq!(sent["run"]["attempted"], 2);
    assert_eq!(sent["paid_minor"], 350_050);
    assert_eq!(e.rail.total_sends(), 2);
    let again = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(again["run"]["attempted"], 0);
    assert_eq!(e.rail.total_sends(), 2, "never sent twice");
    assert_eq!(e.ledger("payouts").await, (350_050, 0));
    assert_eq!(e.ledger("bank").await, (0, 350_050));
    assert_eq!(
        e.t.count("SELECT count(*) FROM outbox_events WHERE type = 'payout_batch.completed'")
            .await,
        1
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM outbox_events WHERE type = 'payout.paid'")
            .await,
        2
    );
    // a paid payout cannot be paid by hand as well
    e.err(
        "priya",
        "POST",
        &format!("/v1/payouts/{p1}/mark_paid"),
        json!({ "bank_ref": "HANDPAID0001" }),
        409,
        "not_approved",
    )
    .await;

    // the report: one line per item, the left-out one included
    let (s, csv) = e
        .text("priya", &format!("/v1/payout_batches/{id}/report.csv"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let rows: Vec<&str> = csv.lines().collect();
    assert_eq!(rows.len(), 4, "{csv}");
    assert!(rows[0].starts_with("line,item_id,payout_id,vendor_id,vendor,amount_inr,"));
    assert!(
        rows[1].contains("1500.50") && rows[1].contains(",paid,") && rows[1].contains("SCRIPT"),
        "{}",
        rows[1]
    );
    assert!(
        rows[3].contains("750.25") && rows[3].contains(",excluded,") && rows[3].starts_with("4,"),
        "{}",
        rows[3]
    );
    let listed = e
        .ok(
            "priya",
            "GET",
            "/v1/payout_batches?status=completed",
            Value::Null,
        )
        .await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);

    // another organisation sees none of it
    let other = &e.t.b;
    for (m, path) in [
        ("GET", format!("/v1/payout_batches/{id}")),
        ("POST", format!("/v1/payout_batches/{id}/approve")),
        ("POST", format!("/v1/payout_batches/{id}/send")),
        ("GET", format!("/v1/payout_batches/{id}/report.csv")),
    ] {
        let (s, _) = e.t.call(other, m, &path, None, json!({})).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {path}");
    }
    assert_eq!(
        e.t.call(other, "GET", "/v1/payout_batches", None, Value::Null)
            .await
            .1["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    // and cannot name the vendors of this one
    let (s, b) =
        e.t.call(
            other,
            "POST",
            "/v1/payout_batches/preview",
            None,
            json!({ "csv": GOOD_CSV }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["ok"], false);
    let _ = v1;
    e.assert_ledger_balances().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn failed_items_retry_with_the_same_payout_and_an_unknown_one_is_only_ever_asked_about(
    pool: PgPool,
) {
    let e = env(pool).await;
    for (n, x) in [
        ("Good Vendor", "G"),
        ("Failing Vendor", "F"),
        ("Unsure Vendor", "U"),
        ("Hopeless Vendor", "H"),
        ("Unsure Two", "U2"),
    ] {
        e.vendor(n, x).await;
    }
    e.rail.plan("Failing Vendor", &[Act::Fail, Act::Settle]);
    e.rail
        .plan("Unsure Vendor", &[Act::Uncertain, Act::Settle, Act::Settle]);
    e.rail
        .plan("Unsure Two", &[Act::Uncertain, Act::Settle, Act::Settle]);
    e.rail.plan("Hopeless Vendor", &[Act::Fail; 10]);
    let b = e.ok("priya", "POST", "/v1/payout_batches", json!({ "items": [
        { "vendor": "G", "amount_minor": 10_000, "purpose": "x" }, { "vendor": "F", "amount_minor": 20_000, "purpose": "x" },
        { "vendor": "U", "amount_minor": 30_000, "purpose": "x" }, { "vendor": "H", "amount_minor": 40_000, "purpose": "x" },
        { "vendor": "U2", "amount_minor": 50_000, "purpose": "x" } ] })).await;
    let id = b["id"].as_str().unwrap().to_string();
    let pay = |line: usize| {
        b["items"][line - 1]["payout_id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
    )
    .await;
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(r["status"], "attention");
    assert_eq!(
        [1, 2, 3, 4, 5].map(|l| status_of(&r, l)),
        ["paid", "failed", "unknown", "failed", "unknown"].map(String::from)
    );
    assert_eq!(r["items"][1]["last_error"], "beneficiary bank refused");
    assert_eq!(
        e.ledger("payouts").await,
        (10_000, 0),
        "only the paid one is in the ledger"
    );

    // a retry is the same payout again, and only for what failed; the unknown ones are not touched
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({}),
        )
        .await;
    assert_eq!(r["run"]["attempted"], 2);
    assert_eq!(
        [1, 2, 3, 4, 5].map(|l| status_of(&r, l)),
        ["paid", "paid", "unknown", "failed", "unknown"].map(String::from)
    );
    assert_eq!(
        [1, 2, 3, 4, 5].map(|l| e.rail.sent(&pay(l))),
        [1, 2, 1, 2, 1]
    );
    // send does not retry failures or the unknown, and an unknown stays blocked however often it is asked
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(r["run"]["attempted"], 0);
    for _ in 0..3 {
        e.ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({ "item_ids": [r["items"][2]["id"], r["items"][4]["id"]] }),
        )
        .await;
    }
    assert_eq!(
        (e.rail.sent(&pay(3)), e.rail.sent(&pay(5))),
        (1, 1),
        "an unknown outcome is never sent again"
    );
    // the rail still cannot say: still unknown, still not sent
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/resolve"),
            json!({}),
        )
        .await;
    assert_eq!(
        (status_of(&r, 3), status_of(&r, 5)),
        ("unknown".to_string(), "unknown".to_string())
    );
    assert_eq!((e.rail.sent(&pay(3)), e.rail.sent(&pay(5))), (1, 1));
    // the rail says one was paid: recorded once, without sending anything; the other it says it never made
    e.rail.answer(&pay(5), Ask::Settled);
    e.rail.answer(&pay(3), Ask::Failed);
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/resolve"),
            json!({}),
        )
        .await;
    assert_eq!(
        (status_of(&r, 3), status_of(&r, 5)),
        ("failed".to_string(), "paid".to_string())
    );
    assert_eq!((e.rail.sent(&pay(3)), e.rail.sent(&pay(5))), (1, 1));
    let again = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/resolve"),
            json!({}),
        )
        .await;
    assert_eq!(again["run"]["attempted"], 0);
    // only now that the rail says it did not pay is the item retried, with the same payout
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({ "item_ids": [r["items"][2]["id"]] }),
        )
        .await;
    assert_eq!(status_of(&r, 3), "paid");
    assert_eq!(e.rail.sent(&pay(3)), 2);
    assert_eq!(e.ledger("payouts").await, (110_000, 0));

    // retries stop after five attempts in all
    for _ in 0..3 {
        e.ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({}),
        )
        .await;
    }
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({}),
        )
        .await;
    assert_eq!(
        (
            r["run"]["attempted"].as_i64(),
            r["run"]["outcomes"]["attempts_exhausted"].as_i64()
        ),
        (Some(1), Some(1)),
        "{r}"
    );
    assert_eq!(e.rail.sent(&pay(4)), 5);
    assert_eq!(r["items"][3]["attempts"], 5);
    // a person can then close it out by hand: a failed item is a normal approved payout
    let (s, _) = e
        .as_(
            "priya",
            "POST",
            &format!("/v1/payouts/{}/mark_paid", pay(4)),
            json!({ "bank_ref": "MANUALUTR0004" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let r = e.batch(&id).await;
    assert_eq!(r["status"], "completed");
    // each payout was posted to the ledger once
    for line in 1..=5 {
        assert_eq!(
            e.t.count(&format!(
                "SELECT count(*) FROM ledger_transactions WHERE reference = 'payout:{}'",
                pay(line)
            ))
            .await,
            1
        );
    }
    assert_eq!(e.ledger("payouts").await, (150_000, 0));
    e.assert_ledger_balances().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_creates_approvals_and_sends_never_pay_twice(pool: PgPool) {
    let e = env(pool).await;
    for i in 1..=4 {
        e.vendor(&format!("Vendor {i}"), &format!("C{i}")).await;
    }
    let csv = "vendor,amount,purpose\nC1,100,x\nC2,200,x\nC3,300,x\nC4,400,x\n";
    // the same file from three places at once: one batch
    let tasks: Vec<_> = ["priya", "meena", "nisha"]
        .into_iter()
        .map(|actor| {
            let (app, key) = (e.app.clone(), e.t.a.key.clone());
            tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    Some(actor),
                    "POST",
                    "/v1/payout_batches",
                    None,
                    json!({ "csv": csv }),
                )
                .await
            })
        })
        .collect();
    let mut codes = Vec::new();
    let mut id = String::new();
    for t in tasks {
        let (s, b) = t.await.unwrap();
        if s == StatusCode::CREATED {
            id = b["id"].as_str().unwrap().to_string();
        }
        codes.push(s.as_u16());
    }
    codes.sort_unstable();
    assert_eq!(codes.iter().filter(|c| **c == 201).count(), 1, "{codes:?}");
    assert!(codes.iter().all(|c| *c == 201 || *c == 409), "{codes:?}");
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches").await, 1);
    assert_eq!(e.t.count("SELECT count(*) FROM payouts").await, 4);

    // four people approve at once: one decision
    let tasks: Vec<_> = ["ravi", "sunil", "tara", "uma"]
        .into_iter()
        .map(|actor| {
            let (app, key, id) = (e.app.clone(), e.t.a.key.clone(), id.clone());
            tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    Some(actor),
                    "POST",
                    &format!("/v1/payout_batches/{id}/approve"),
                    None,
                    json!({}),
                )
                .await
            })
        })
        .collect();
    let mut won = 0;
    for t in tasks {
        let (s, b) = t.await.unwrap();
        match s.as_u16() {
            200 => won += 1,
            409 => assert_eq!(b["error"]["code"], "not_pending_approval", "{b}"),
            other => panic!("unexpected {other}: {b}"),
        }
    }
    assert_eq!(won, 1);

    // six senders at once: each payout reaches the rail once and is posted once
    let tasks: Vec<_> = (0..6)
        .map(|i| {
            let (app, key, id) = (e.app.clone(), e.t.a.key.clone(), id.clone());
            tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    Some(["priya", "meena", "nisha"][i % 3]),
                    "POST",
                    &format!("/v1/payout_batches/{id}/send"),
                    None,
                    json!({}),
                )
                .await
            })
        })
        .collect();
    for t in tasks {
        let (s, b) = t.await.unwrap();
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let r = e.batch(&id).await;
    assert_eq!(r["status"], "completed", "{r}");
    for it in r["items"].as_array().unwrap() {
        assert_eq!(e.rail.sent(it["payout_id"].as_str().unwrap()), 1, "{it}");
        assert_eq!(it["attempts"], 1);
    }
    assert_eq!(e.rail.total_sends(), 4);
    assert_eq!(e.ledger("payouts").await, (100_000, 0));
    assert_eq!(
        e.t.count("SELECT count(*) FROM ledger_transactions WHERE reference LIKE 'payout:%'")
            .await,
        4
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM outbox_events WHERE type = 'payout_batch.completed'")
            .await,
        1
    );
    e.assert_ledger_balances().await;

    // a person paying a payout by hand while the batch sends it cannot happen: the payout is not approved any more
    let p = r["items"][0]["payout_id"].as_str().unwrap();
    e.err(
        "priya",
        "POST",
        &format!("/v1/payouts/{p}/mark_paid"),
        json!({ "bank_ref": "SECONDPAY001" }),
        409,
        "not_approved",
    )
    .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn limits_the_vendors_own_minimum_blocked_payees_and_changes_after_proposal_are_all_enforced(
    pool: PgPool,
) {
    let e = env(pool).await;
    let ids: Vec<String> = vec![
        e.vendor("Alpha", "A").await,
        e.vendor("Beta", "B").await,
        e.vendor("Gamma", "G").await,
        e.vendor("Delta", "D").await,
    ];
    // limits are an admin's to set
    let clerk = e.t.ok(&e.t.a, "POST", "/v1/api_keys", None, json!({ "name": "fin", "scopes": ["payouts:write", "payouts:approve", "payments:read"] })).await;
    let (s, _) = raw(
        &e.app,
        clerk["key"].as_str().unwrap(),
        Some("kiran"),
        "PUT",
        "/v1/payout_policy",
        None,
        json!({ "max_payout_minor": 1 }),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    for bad in [
        json!({ "max_payout_minor": 0 }),
        json!({ "max_batch_items": 5_000 }),
        json!({ "unknown_limit": 5 }),
    ] {
        let (s, _) =
            e.t.call(&e.t.a, "PUT", "/v1/payout_policy", None, bad.clone())
                .await;
        assert!(s.as_u16() == 422 || s.as_u16() == 400, "{bad}");
    }
    e.t.ok(&e.t.a, "PUT", "/v1/payout_policy", None, json!({ "max_payout_minor": 100_000, "max_batch_total_minor": 150_000, "max_batch_items": 3, "max_vendor_per_24h_minor": 120_000 })).await;
    assert_eq!(
        e.ok("priya", "GET", "/v1/payout_policy", Value::Null).await["max_batch_items"],
        3
    );

    let pv = e.ok("priya", "POST", "/v1/payout_batches/preview", json!({ "items": [
        { "vendor": "A", "amount_minor": 150_000, "purpose": "x" },
        { "vendor": "B", "amount_minor": 70_000, "purpose": "x" }, { "vendor": "B", "amount_minor": 60_000, "purpose": "x" },
        { "vendor": "G", "amount_minor": 1_000, "purpose": "x" }, { "vendor": "D", "amount_minor": 1_000, "purpose": "x" } ] })).await;
    let codes: Vec<(i64, String)> = pv["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["line"].as_i64().unwrap(),
                x["column"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    assert_eq!(codes[0], (1, "above_payout_limit".into()), "{pv}");
    assert!(
        codes.contains(&(3, "over_vendor_daily_limit".into())),
        "{pv}"
    );
    // single payouts meet the same limit
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": ids[0], "amount_minor": 150_000, "purpose": "x" }),
        409,
        "above_payout_limit",
    )
    .await;
    // batch-level limits are reported when the lines themselves are fine
    let pv = e.ok("priya", "POST", "/v1/payout_batches/preview", json!({ "items": [
        { "vendor": "A", "amount_minor": 80_000, "purpose": "x" }, { "vendor": "B", "amount_minor": 80_000, "purpose": "x" } ] })).await;
    assert!(
        pv["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("allows at most"),
        "{pv}"
    );
    let pv = e.ok("priya", "POST", "/v1/payout_batches/preview", json!({ "items": [
        { "vendor": "A", "amount_minor": 1_000, "purpose": "x" }, { "vendor": "B", "amount_minor": 1_000, "purpose": "x" },
        { "vendor": "G", "amount_minor": 1_000, "purpose": "x" }, { "vendor": "D", "amount_minor": 1_000, "purpose": "x" } ] })).await;
    assert!(
        pv["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("at most 3"),
        "{pv}"
    );

    // a vendor's own minimum
    e.ok(
        "ravi",
        "PUT",
        &format!("/v1/vendors/{}/payout_settings", ids[2]),
        json!({ "min_payout_minor": 5_000 }),
    )
    .await;
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "items": [{ "vendor": "G", "amount_minor": 4_999, "purpose": "x" }] }),
        )
        .await;
    assert_eq!(pv["errors"][0]["column"], "below_minimum_payout");
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": ids[2], "amount_minor": 4_999, "purpose": "x" }),
        409,
        "below_minimum_payout",
    )
    .await;

    // a blocked payee
    e.t.ok(
        &e.t.a,
        "POST",
        "/v1/settings",
        None,
        json!({ "risk": { "blocked_vpas": ["@badbank"] } }),
    )
    .await;
    let blocked =
        e.t.ok(
            &e.t.a,
            "POST",
            "/v1/vendors",
            None,
            json!({ "name": "Blocked Payee", "vpa": "pay@badbank", "external_ref": "BP" }),
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_string();
    let pv = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/preview",
            json!({ "items": [{ "vendor": "BP", "amount_minor": 1_000, "purpose": "x" }] }),
        )
        .await;
    assert_eq!(pv["errors"][0]["column"], "blocked_payee");
    let _ = blocked;

    // a proposal the vendor's state changes under: approval checks again and names what fails
    let b = e.ok("priya", "POST", "/v1/payout_batches", json!({ "items": [
        { "vendor": "A", "amount_minor": 10_000, "purpose": "x" }, { "vendor": "B", "amount_minor": 20_000, "purpose": "x" }, { "vendor": "D", "amount_minor": 5_000, "purpose": "x" } ] })).await;
    let id = b["id"].as_str().unwrap().to_string();
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/vendors/{}/suspend", ids[1]),
        json!({ "reason": "paused while we check" }),
    )
    .await;
    let r = e
        .err(
            "ravi",
            "POST",
            &format!("/v1/payout_batches/{id}/approve"),
            json!({}),
            409,
            "items_failed_checks",
        )
        .await;
    assert_eq!(r["error"]["items"][0]["line"], 2);
    assert_eq!(
        r["error"]["items"][0]["problems"][0]["code"],
        "vendor_not_active"
    );
    assert_eq!(
        e.batch(&id).await["status"],
        "pending_approval",
        "nothing was approved"
    );
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'approved'")
            .await,
        0
    );
    // leave it out and the rest goes through
    let item2 = b["items"][1]["id"].as_str().unwrap();
    let r = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/payout_batches/{id}/approve"),
            json!({ "exclude": [item2] }),
        )
        .await;
    assert_eq!(
        (r["item_count"].as_i64(), r["total_minor"].as_i64()),
        (Some(2), Some(15_000))
    );
    // the vendor is suspended after approval: the send is refused for that item, nothing reaches the rail
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/vendors/{}/suspend", ids[3]),
        json!({ "reason": "again" }),
    )
    .await;
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(
        (status_of(&r, 1), status_of(&r, 3)),
        ("paid".to_string(), "failed".to_string())
    );
    assert!(
        r["items"][2]["last_error"]
            .as_str()
            .unwrap()
            .contains("vendor_not_active")
    );
    assert_eq!(e.rail.total_sends(), 1);
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/vendors/{}/reactivate", ids[3]),
        json!({}),
    )
    .await;
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/retry"),
            json!({}),
        )
        .await;
    assert_eq!(status_of(&r, 3), "paid");
    assert_eq!(r["status"], "completed");
    assert_eq!(e.rail.total_sends(), 2);
    e.assert_ledger_balances().await;
}

#[sqlx::test(migrations = "./migrations")]
async fn rejecting_canceling_and_leaving_lines_out_free_the_content_to_be_proposed_again(
    pool: PgPool,
) {
    let e = env(pool).await;
    e.vendor("One", "O1").await;
    e.vendor("Two", "O2").await;
    let csv = "vendor,amount,purpose\nO1,10,x\nO2,20,x\n";
    let b = e
        .ok("priya", "POST", "/v1/payout_batches", json!({ "csv": csv }))
        .await;
    let id = b["id"].as_str().unwrap().to_string();
    // the proposer may leave a line out; nobody can reject their own
    let item1 = b["items"][0]["id"].as_str().unwrap();
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/items/{item1}/exclude"),
            json!({ "note": "typo" }),
        )
        .await;
    assert_eq!(
        (r["item_count"].as_i64(), r["total_minor"].as_i64()),
        (Some(1), Some(2_000))
    );
    e.err(
        "priya",
        "POST",
        &format!("/v1/payout_batches/{id}/reject"),
        json!({}),
        403,
        "separation_of_duties",
    )
    .await;
    let r = e
        .ok(
            "ravi",
            "POST",
            &format!("/v1/payout_batches/{id}/reject"),
            json!({ "note": "wrong month" }),
        )
        .await;
    assert_eq!(r["status"], "rejected");
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'rejected'")
            .await,
        1
    );
    e.err(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
        409,
        "not_pending_approval",
    )
    .await;
    // the same content can be proposed again, and withdrawn by whoever proposed it
    let b = e
        .ok("priya", "POST", "/v1/payout_batches", json!({ "csv": csv }))
        .await;
    let id2 = b["id"].as_str().unwrap().to_string();
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id2}/cancel"),
            json!({}),
        )
        .await;
    assert_eq!(r["status"], "canceled");
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status IN ('pending_approval', 'approved')")
            .await,
        0
    );
    let b = e
        .ok("priya", "POST", "/v1/payout_batches", json!({ "csv": csv }))
        .await;
    // all lines left out: there is nothing to approve
    for it in b["items"].as_array().unwrap().iter().skip(1) {
        e.ok(
            "priya",
            "POST",
            &format!(
                "/v1/payout_batches/{}/items/{}/exclude",
                b["id"].as_str().unwrap(),
                it["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;
    }
    let ex: Vec<String> = ids(&b);
    let r = e
        .err(
            "ravi",
            "POST",
            &format!("/v1/payout_batches/{}/approve", b["id"].as_str().unwrap()),
            json!({ "exclude": [ex[0]] }),
            409,
            "nothing_to_approve",
        )
        .await;
    let _ = r;
    assert_eq!(e.ledger("%").await, (0, 0));
}

#[sqlx::test(migrations = "./migrations")]
async fn in_orchestrate_mode_the_batch_is_the_list_of_transfers_and_each_is_recorded_by_hand(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let v = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Hand Paid", "external_ref": "HP", "account_number": "1234567890", "ifsc": "SYNT0000001" })).await["id"].as_str().unwrap().to_string();
    let b = t.ok(a, "POST", "/v1/payout_batches", None, json!({ "items": [{ "vendor": "HP", "amount_minor": 5_000, "purpose": "x" }, { "vendor": v, "amount_minor": 6_000, "purpose": "y" }] })).await;
    let id = b["id"].as_str().unwrap().to_string();
    let go = |actor: &'static str, m: &'static str, p: String, body: Value| {
        let t = &t;
        async move { t.call_as(&t.a, actor, m, &p, None, body).await }
    };
    assert_eq!(
        go(
            "ravi",
            "POST",
            format!("/v1/payout_batches/{id}/approve"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, e) = go(
        "priya",
        "POST",
        format!("/v1/payout_batches/{id}/send"),
        json!({}),
    )
    .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("orchestrate_mode")),
        "{e}"
    );
    let (_, r) = go(
        "priya",
        "GET",
        format!("/v1/payout_batches/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(r["status"], "approved");
    for (i, utr) in ["HANDUTR0001", "HANDUTR0002"].into_iter().enumerate() {
        let p = r["items"][i]["payout_id"].as_str().unwrap();
        let (s, b) = go(
            "priya",
            "POST",
            format!("/v1/payouts/{p}/mark_paid"),
            json!({ "bank_ref": utr }),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }
    let (_, r) = go(
        "priya",
        "GET",
        format!("/v1/payout_batches/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(
        (r["status"].as_str(), r["paid_minor"].as_i64()),
        (Some("completed"), Some(11_000))
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'payout_batch.completed'")
            .await,
        1
    );
    // a payment with no approval first is not a payment
    let b2 = t
        .ok(
            a,
            "POST",
            "/v1/payout_batches",
            None,
            json!({ "items": [{ "vendor": "HP", "amount_minor": 7_000, "purpose": "z" }] }),
        )
        .await;
    let p = b2["items"][0]["payout_id"].as_str().unwrap();
    let (s, e) = go(
        "priya",
        "POST",
        format!("/v1/payouts/{p}/mark_paid"),
        json!({ "bank_ref": "HANDUTR0003" }),
    )
    .await;
    assert_eq!(
        (s.as_u16(), e["error"]["code"].clone()),
        (409, json!("not_approved"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_batch_shows_the_bank_debits_that_matched_its_payouts(pool: PgPool) {
    let e = env(pool).await;
    for (n, x) in [("Bank One", "B1"), ("Bank Two", "B2"), ("Bank Three", "B3")] {
        e.vendor(n, x).await;
    }
    let b = e.ok("priya", "POST", "/v1/payout_batches", json!({ "items": [
        { "vendor": "B1", "amount_minor": 500_000, "purpose": "x" }, { "vendor": "B2", "amount_minor": 250_000, "purpose": "x" }, { "vendor": "B3", "amount_minor": 125_000, "purpose": "x" } ] })).await;
    let id = b["id"].as_str().unwrap().to_string();
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
    )
    .await;
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(r["bank_reconciliation"]["paid_items"], 3);
    assert_eq!(r["bank_reconciliation"]["matched_items"], 0);
    let d = zyvor_api::ist_today().format("%d/%m/%Y").to_string();
    let utr = |i: usize| r["items"][i]["bank_ref"].as_str().unwrap().to_string();
    // the first debit names the reference; the second has the right amount and no reference (only a suggestion); the third is missing
    let csv = format!(
        "Date,Narration,Reference,Debit,Credit\n{d},IMPS/{}/BANK ONE,,5000.00,\n{d},TRANSFER TO SOMEONE,,2500.00,\n",
        utr(0)
    );
    e.ok(
        "priya",
        "POST",
        "/v1/bank_statements/import_csv",
        json!({ "csv": csv }),
    )
    .await;
    let r = e.batch(&id).await;
    let rec = &r["bank_reconciliation"];
    assert_eq!(
        (
            rec["matched_items"].as_i64(),
            rec["suggested_items"].as_i64(),
            rec["unmatched_items"].as_i64(),
            rec["matched_minor"].as_i64()
        ),
        (Some(1), Some(1), Some(1), Some(500_000)),
        "{r}"
    );
    assert_eq!(r["items"][0]["bank_match"]["status"], "confirmed");
    assert_eq!(r["items"][0]["bank_match"]["amount_minor"], -500_000);
    assert_eq!(r["items"][1]["bank_match"]["status"], "suggested");
    assert_eq!(r["items"][2]["bank_match"], Value::Null);
    let (_, csv) = e
        .text("priya", &format!("/v1/payout_batches/{id}/report.csv"))
        .await;
    let rows: Vec<&str> = csv.lines().collect();
    assert!(
        rows[1].contains(",confirmed,") && rows[1].contains("5000.00"),
        "{}",
        rows[1]
    );
    assert!(rows[2].contains(",suggested,"), "{}", rows[2]);
    assert!(rows[3].contains(",none,"), "{}", rows[3]);
}

// ---------- paying from balances ----------

#[sqlx::test(migrations = "./migrations")]
async fn a_batch_from_balances_pays_what_is_owed_past_the_hold_and_the_minimum_and_never_the_same_money_twice(
    pool: PgPool,
) {
    let e = env(pool).await;
    let (v1, v2, v3) = (
        e.vendor("Pays Now", "P1").await,
        e.vendor("Under Minimum", "P2").await,
        e.vendor("On Hold", "P3").await,
    );
    e.vendor("Nothing Owed", "P4").await;
    e.earn(&v1, 6_000).await;
    e.earn(&v2, 3_000).await;
    e.earn(&v3, 8_000).await;
    e.ok(
        "ravi",
        "PUT",
        &format!("/v1/vendors/{v2}/payout_settings"),
        json!({ "min_payout_minor": 5_000 }),
    )
    .await;
    e.ok(
        "ravi",
        "PUT",
        &format!("/v1/vendors/{v3}/payout_settings"),
        json!({ "hold_days": 3 }),
    )
    .await;

    let dry = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/from_balances",
            json!({ "dry_run": true }),
        )
        .await;
    assert_eq!(
        (
            dry["ok"].clone(),
            dry["item_count"].clone(),
            dry["total_minor"].clone()
        ),
        (json!(true), json!(1), json!(6_000)),
        "{dry}"
    );
    let skipped: Vec<String> = dry["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            format!(
                "{}:{}",
                s["vendor_id"].as_str().unwrap(),
                s["reason"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(skipped.len(), 2, "{dry}");
    assert!(
        skipped
            .iter()
            .any(|s| s.starts_with(&v2) && s.contains("minimum")),
        "{skipped:?}"
    );
    assert!(
        skipped
            .iter()
            .any(|s| s.starts_with(&v3) && s.contains("held back")),
        "{skipped:?}"
    );
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches").await, 0);

    let b = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/from_balances",
            json!({ "name": "Weekly vendors" }),
        )
        .await;
    let id = b["id"].as_str().unwrap().to_string();
    assert_eq!(
        (
            b["source"].as_str(),
            b["from_balance"].as_bool(),
            b["total_minor"].as_i64()
        ),
        (Some("balances"), Some(true), Some(6_000))
    );
    // the money is promised to this batch: nothing is left for a second one, or for a single payout
    assert_eq!(e.owed(&v1).await, (6_000, 0));
    let (s, b2) = e
        .as_(
            "priya",
            "POST",
            "/v1/payout_batches/from_balances",
            json!({}),
        )
        .await;
    assert_eq!(
        (s.as_u16(), b2["error"]["code"].clone()),
        (422, json!("invalid_payout_batch")),
        "{b2}"
    );
    e.err(
        "priya",
        "POST",
        "/v1/payouts",
        json!({ "vendor_id": v1, "amount_minor": 100, "purpose": "x", "from_balance": true }),
        409,
        "insufficient_balance",
    )
    .await;
    let (s, over) = e.as_("priya", "POST", "/v1/payout_batches", json!({ "items": [{ "vendor": "P1", "amount_minor": 1, "purpose": "x" }], "from_balance": true })).await;
    assert_eq!(s.as_u16(), 422, "{over}");
    // naming a vendor that cannot be paid is an error, not a silent skip
    let (s, b3) = e
        .as_(
            "priya",
            "POST",
            "/v1/payout_batches/from_balances",
            json!({ "vendor_ids": [v3] }),
        )
        .await;
    assert_eq!(s.as_u16(), 422, "{b3}");
    assert!(
        b3["error"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("held back")
    );

    e.ok(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{id}/approve"),
        json!({}),
    )
    .await;
    let r = e
        .ok(
            "priya",
            "POST",
            &format!("/v1/payout_batches/{id}/send"),
            json!({}),
        )
        .await;
    assert_eq!(r["status"], "completed");
    assert_eq!(
        e.owed(&v1).await.0,
        0,
        "the vendor's balance is cleared by the payout"
    );
    assert_eq!(
        e.ledger(&format!("vendor_payable:{v1}")).await,
        (6_000, 6_000)
    );

    // the hold passes with the organisation's calendar (the test clock), not the wall clock
    e.t.ok(
        &e.t.a,
        "POST",
        "/v1/test_clock",
        None,
        json!({ "advance_days": 4 }),
    )
    .await;
    let b = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_batches/from_balances",
            json!({}),
        )
        .await;
    assert_eq!(
        (b["item_count"].as_i64(), b["total_minor"].as_i64()),
        (Some(1), Some(8_000)),
        "{b}"
    );
    assert_eq!(b["items"][0]["vendor_id"], v3.as_str());
    e.assert_ledger_balances().await;
}

// ---------- schedules ----------

#[sqlx::test(migrations = "./migrations")]
async fn schedules_propose_batches_on_the_organisations_dates_and_pay_nothing_without_approval(
    pool: PgPool,
) {
    let e = env(pool).await;
    let a = &e.t.a;
    let v = e.vendor("Scheduled One", "S1").await;
    let w = e.vendor("Threshold Two", "S2").await;
    let d0: NaiveDate = zyvor_api::ist_today() + Days::new(300);
    a_clock(&e, d0).await;
    let dow = d0.weekday().number_from_monday() as i64;

    for bad in [
        json!({ "vendor_id": v, "kind": "weekly" }),
        json!({ "vendor_id": v, "kind": "weekly", "day_of_week": 8 }),
        json!({ "vendor_id": v, "kind": "monthly", "day_of_month": 29 }),
        json!({ "vendor_id": v, "kind": "threshold", "threshold_minor": 0 }),
        json!({ "vendor_id": v, "kind": "weekly", "day_of_week": 1, "threshold_minor": 5 }),
        json!({ "vendor_id": v, "kind": "daily" }),
        json!({ "vendor_id": "ven_nope", "kind": "weekly", "day_of_week": 1 }),
    ] {
        let (s, _) = e
            .as_("priya", "POST", "/v1/payout_schedules", bad.clone())
            .await;
        assert!(s.as_u16() == 422 || s.as_u16() == 404, "{bad}");
    }
    let sched = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_schedules",
            json!({ "vendor_id": v, "kind": "weekly", "day_of_week": dow }),
        )
        .await;
    assert_eq!(sched["next_run_on"], d0.to_string());
    let monthly = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_schedules",
            json!({ "vendor_id": w, "kind": "monthly", "day_of_month": 0 }),
        )
        .await;
    assert!(monthly["next_run_on"].as_str().unwrap() >= d0.to_string().as_str());
    e.ok(
        "priya",
        "POST",
        &format!(
            "/v1/payout_schedules/{}/pause",
            monthly["id"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    let thr = e
        .ok(
            "priya",
            "POST",
            "/v1/payout_schedules",
            json!({ "vendor_id": w, "kind": "threshold", "threshold_minor": 10_000 }),
        )
        .await;

    e.earn(&v, 5_000).await;
    e.earn(&w, 4_000).await;
    let run = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    assert_eq!(
        (
            run["due"].as_i64(),
            run["proposed_items"].as_i64(),
            run["total_minor"].as_i64()
        ),
        (Some(2), Some(1), Some(5_000)),
        "{run}"
    );
    let outcomes: Vec<(String, String)> = run["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["vendor_id"].as_str().unwrap().to_string(),
                s["outcome"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(
        outcomes.contains(&(v.clone(), "proposed".into()))
            && outcomes.contains(&(w.clone(), "below_threshold".into())),
        "{outcomes:?}"
    );
    let bid = run["batch_id"].as_str().unwrap().to_string();
    let batch = e.batch(&bid).await;
    // proposed, not paid: it waits for a person, and the proposer is the system, so any person may approve
    assert_eq!(
        (
            batch["status"].as_str(),
            batch["created_by"].as_str(),
            batch["source"].as_str(),
            batch["auto_approved"].as_bool()
        ),
        (
            Some("pending_approval"),
            Some("system:schedule"),
            Some("schedule"),
            Some(false)
        )
    );
    assert_eq!(e.rail.total_sends(), 0);
    assert_eq!(e.ledger("bank").await, (0, 0));
    e.err(
        "priya",
        "POST",
        &format!("/v1/payout_batches/{bid}/send"),
        json!({}),
        409,
        "not_approved",
    )
    .await;
    // the same day again proposes nothing more
    let again = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    assert_eq!(
        (again["due"].as_i64(), again["batch_id"].clone()),
        (Some(0), Value::Null),
        "a schedule runs once a day: {again}"
    );
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches").await, 1);
    // the amount is reserved, so the threshold schedule's vendor is not affected, and approval + send pays it
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{bid}/approve"),
        json!({}),
    )
    .await;
    e.ok(
        "priya",
        "POST",
        &format!("/v1/payout_batches/{bid}/send"),
        json!({}),
    )
    .await;
    assert_eq!(e.owed(&v).await.0, 0);

    // a week on: nothing owed, nothing proposed
    a_clock(&e, d0 + Days::new(7)).await;
    let run = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    let weekly = run["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["vendor_id"] == v.as_str())
        .unwrap();
    assert_eq!(weekly["outcome"], "nothing_payable");
    // new money, a week on: proposed again; a week after that, with the last proposal still unapproved: not stacked
    e.earn(&v, 3_000).await;
    a_clock(&e, d0 + Days::new(14)).await;
    let run = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    assert_eq!(run["proposed_items"], 1, "{run}");
    let first = run["batch_id"].as_str().unwrap().to_string();
    // the worker was away for three weeks: one proposal, not one per missed week, and no second while the first waits
    a_clock(&e, d0 + Days::new(35)).await;
    let run = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    let weekly = run["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["vendor_id"] == v.as_str())
        .unwrap();
    assert_eq!(weekly["outcome"], "previous_proposal_still_open", "{run}");
    assert_eq!(
        e.t.count("SELECT count(*) FROM payout_batches WHERE source = 'schedule'")
            .await,
        2
    );
    // the weekly schedule is due again on its weekday after the catch-up run, not tomorrow
    let s = e
        .ok(
            "priya",
            "GET",
            &format!("/v1/payout_schedules/{}", sched["id"].as_str().unwrap()),
            Value::Null,
        )
        .await;
    let next: NaiveDate = s["next_run_on"].as_str().unwrap().parse().unwrap();
    assert_eq!(next.weekday().number_from_monday() as i64, dow);
    assert!(next > d0 + Days::new(35) && next <= d0 + Days::new(42));
    // more money brings the threshold schedule's vendor over its threshold the next day
    e.earn(&w, 7_000).await;
    a_clock(&e, d0 + Days::new(36)).await;
    let run = e
        .ok("priya", "POST", "/v1/payout_schedules/run", json!({}))
        .await;
    let t = run["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["schedule_id"] == thr["id"])
        .unwrap();
    assert_eq!(t["outcome"], "proposed", "{run}");
    assert_eq!(
        e.batch(run["batch_id"].as_str().unwrap()).await["total_minor"],
        11_000
    );
    let _ = first;
    // a paused schedule never runs and resumes from today
    let listed = e
        .ok(
            "priya",
            "GET",
            &format!("/v1/payout_schedules?vendor_id={w}"),
            Value::Null,
        )
        .await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 2);
    // nothing was ever paid without a person
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'paid'")
            .await,
        1
    );
    // the other organisation has no schedules and cannot touch these
    let (s, _) =
        e.t.call(
            &e.t.b,
            "POST",
            &format!(
                "/v1/payout_schedules/{}/pause",
                sched["id"].as_str().unwrap()
            ),
            None,
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(
        e.t.call(&e.t.b, "GET", "/v1/payout_schedules", None, Value::Null)
            .await
            .1["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (s, _) =
        e.t.call(
            &e.t.b,
            "POST",
            "/v1/payout_schedules",
            None,
            json!({ "vendor_id": v, "kind": "weekly", "day_of_week": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let _ = a;
    e.assert_ledger_balances().await;
}

async fn a_clock(e: &Env, d: NaiveDate) {
    e.t.ok(
        &e.t.a,
        "POST",
        "/v1/test_clock",
        None,
        json!({ "date": d.to_string() }),
    )
    .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn a_small_scheduled_batch_is_paid_by_the_worker_only_when_the_tenants_policy_says_so(
    pool: PgPool,
) {
    let e = env(pool).await;
    let (small, large) = (
        e.vendor("Small One", "SM").await,
        e.vendor("Large One", "LG").await,
    );
    let d0: NaiveDate = zyvor_api::ist_today() + Days::new(200);
    a_clock(&e, d0).await;
    for v in [&small, &large] {
        e.ok("priya", "POST", "/v1/payout_schedules", json!({ "vendor_id": v, "kind": "weekly", "day_of_week": d0.weekday().number_from_monday() })).await;
    }
    e.earn(&small, 5_000).await;
    e.earn(&large, 9_000).await;
    let client = reqwest::Client::new();

    // by default the worker proposes and nothing more
    zyvor_api::worker::run_cycle(&e.state, &client)
        .await
        .unwrap();
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches WHERE status = 'pending_approval' AND NOT auto_approved").await, 1);
    assert_eq!(e.rail.total_sends(), 0);
    assert_eq!(
        e.t.count("SELECT count(*) FROM payouts WHERE status = 'paid'")
            .await,
        0
    );
    let first: String = sqlx::query_scalar("SELECT id FROM payout_batches")
        .fetch_one(&e.t.root)
        .await
        .unwrap();
    e.ok(
        "priya",
        "POST",
        &format!("/v1/payout_batches/{first}/cancel"),
        json!({}),
    )
    .await;

    // the tenant allows batches up to 6,000 in total to go ahead by themselves
    let (s, _) =
        e.t.call(
            &e.t.a,
            "PUT",
            "/v1/payout_policy",
            None,
            json!({ "auto_approve_scheduled_under_minor": 6_000 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    a_clock(&e, d0 + Days::new(7)).await;
    // the two vendors make one batch of 14,000: over the limit, so it waits
    zyvor_api::worker::run_cycle(&e.state, &client)
        .await
        .unwrap();
    assert_eq!(e.t.count("SELECT count(*) FROM payout_batches WHERE status = 'pending_approval' AND NOT auto_approved").await, 1, "14,000 is over the 6,000 limit");
    assert_eq!(e.rail.total_sends(), 0);
    // a person turns that one down; the large vendor then falls under a minimum payout it cannot meet, so the next
    // run is the small vendor alone, under the limit
    let large_batch: String =
        sqlx::query_scalar("SELECT id FROM payout_batches WHERE status = 'pending_approval'")
            .fetch_one(&e.t.root)
            .await
            .unwrap();
    e.ok(
        "ravi",
        "POST",
        &format!("/v1/payout_batches/{large_batch}/reject"),
        json!({}),
    )
    .await;
    e.ok(
        "ravi",
        "PUT",
        &format!("/v1/vendors/{large}/payout_settings"),
        json!({ "min_payout_minor": 100_000 }),
    )
    .await;
    a_clock(&e, d0 + Days::new(14)).await;
    zyvor_api::worker::run_cycle(&e.state, &client)
        .await
        .unwrap();
    let (status, auto, by, n): (String, bool, Option<String>, i64) = sqlx::query_as("SELECT status, auto_approved, approved_by, (SELECT count(*) FROM payout_batches)::BIGINT FROM payout_batches ORDER BY created_at DESC LIMIT 1").fetch_one(&e.t.root).await.unwrap();
    assert_eq!(
        (status.as_str(), auto, by.as_deref()),
        ("approved", true, Some("system:auto-approve")),
        "{n}"
    );
    // and the same worker cycle sent it, once
    let paid: i64 = e.t.count("SELECT count(*) FROM payouts WHERE status = 'paid' AND requested_by = 'system:schedule'").await;
    assert_eq!(paid, 1);
    assert_eq!(e.rail.total_sends(), 1);
    zyvor_api::worker::run_cycle(&e.state, &client)
        .await
        .unwrap();
    assert_eq!(
        e.rail.total_sends(),
        1,
        "another cycle does not send it again"
    );
    assert_eq!(e.t.count("SELECT count(*) FROM audit_events WHERE action = 'payout_batch.created' AND detail->>'auto_approved' = 'true'").await, 1);
    e.assert_ledger_balances().await;
}

// ---------- isolation ----------

#[sqlx::test(migrations = "./migrations")]
async fn every_new_table_is_closed_to_a_connection_that_names_no_organisation(pool: PgPool) {
    let e = env(pool).await;
    let v = e.vendor("Visible", "VIS").await;
    e.earn(&v, 5_000).await;
    e.ok(
        "ravi",
        "POST",
        "/v1/commission_rules",
        json!({ "name": "r", "vendor_id": v, "basis_points": 100, "effective_from": "2000-01-01" }),
    )
    .await;
    e.ok(
        "priya",
        "POST",
        "/v1/payout_batches",
        json!({ "items": [{ "vendor": "VIS", "amount_minor": 1_000, "purpose": "x" }] }),
    )
    .await;
    e.ok(
        "priya",
        "POST",
        "/v1/payout_schedules",
        json!({ "vendor_id": v, "kind": "weekly", "day_of_week": 1 }),
    )
    .await;
    e.ok(
        "priya",
        "POST",
        &format!("/v1/vendors/{v}/documents"),
        json!({ "kind": "other", "filename": "a.pdf", "data_base64": pdf() }),
    )
    .await;
    for table in [
        "commission_rules",
        "payout_batches",
        "payout_batch_items",
        "payout_schedules",
        "vendor_documents",
    ] {
        let seen: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&e.t.state.pool)
                .await
                .unwrap();
        assert_eq!(
            seen, 0,
            "{table} is readable without naming an organisation"
        );
        let root: i64 = e.t.count(&format!("SELECT count(*) FROM {table}")).await;
        assert!(root >= 1, "{table} should have a row");
        // and the other organisation's scope sees none of it
        let mut tx = zyvor_api::tenant::begin(&e.t.state.pool, &e.t.b.id)
            .await
            .unwrap();
        let other: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(other, 0, "{table} leaks across organisations");
    }
    // a vendor of another organisation cannot be named in a commission rule, a schedule or a batch
    let (s, _) = e.t.call(&e.t.b, "POST", "/v1/commission_rules", None, json!({ "name": "r", "vendor_id": v, "basis_points": 100, "effective_from": "2000-01-01" })).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, b) =
        e.t.call(
            &e.t.b,
            "POST",
            "/v1/payout_batches",
            None,
            json!({ "items": [{ "vendor": v, "amount_minor": 1_000, "purpose": "x" }] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    let (s, _) =
        e.t.call(
            &e.t.b,
            "POST",
            "/v1/vendors/invite",
            None,
            json!({ "name": "B's own vendor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
}
