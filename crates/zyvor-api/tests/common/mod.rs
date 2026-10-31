#![allow(dead_code)]
//! Shared harness. The app talks to Postgres as a NON-superuser role so row-level security is
//! really enforced: a handler that forgets to scope by tenant fails the isolation tests.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use zyvor_api::{AppState, Crypto, admin::bootstrap_org, app};
use zyvor_provider_mock::{MockConfig, spawn};

pub const RZ_HOOK: &str = "rz_whsec_A";
pub const CF_HOOK: &str = "cf_whsec_A";

#[derive(Clone)]
pub struct Tenant {
    pub id: String,
    pub key: String,
}

pub struct T {
    pub app: Router,
    pub state: AppState,
    /// Superuser pool: for assertions that need to see across tenants.
    pub root: PgPool,
    pub mock: String,
    pub http: reqwest::Client,
    pub a: Tenant,
    pub b: Tenant,
}

pub async fn setup(root: PgPool) -> T {
    sqlx::query("DO $$ BEGIN CREATE ROLE zyvor_app NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$").execute(&root).await.unwrap();
    sqlx::query("GRANT ALL ON ALL TABLES IN SCHEMA public TO zyvor_app")
        .execute(&root)
        .await
        .unwrap();
    sqlx::query("GRANT ALL ON ALL SEQUENCES IN SCHEMA public TO zyvor_app")
        .execute(&root)
        .await
        .unwrap();
    let app_pool = PgPoolOptions::new()
        .max_connections(12)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE zyvor_app")
                    .execute(c)
                    .await
                    .map(|_| ())
            })
        })
        .connect_with((*root.connect_options()).clone())
        .await
        .unwrap();

    let mock = spawn(MockConfig {
        razorpay_webhook_secret: RZ_HOOK.into(),
        cashfree_webhook_secret: CF_HOOK.into(),
    })
    .await;
    let mut state = AppState::new(
        app_pool.clone(),
        Crypto::from_hex(&"11".repeat(32)).unwrap(),
    );
    state.provider_timeout = std::time::Duration::from_secs(1);
    state.allow_private_targets = true; // the simulators listen on localhost
    state.resolve_after = std::time::Duration::from_secs(0);
    // most tests have one reviewer; the two-reviewer rules have their own tests that turn them on
    state.review_two_person = zyvor_api::platform::TwoPerson::Never;
    let router = app(state.clone());

    let mut tenants = Vec::new();
    for (name, rz_hook, cf_hook) in [
        ("Org A", RZ_HOOK, CF_HOOK),
        ("Org B", "rz_whsec_B", "cf_whsec_B"),
    ] {
        let b = bootstrap_org(&app_pool, name).await.unwrap();
        let t = Tenant {
            id: b.test_tenant,
            key: b.test_key,
        };
        for (provider, creds, hook) in [
            (
                "razorpay",
                json!({ "key_id": "rzp_test_k", "key_secret": "rzp_secret_x" }),
                rz_hook,
            ),
            (
                "cashfree",
                json!({ "client_id": "cf_id", "client_secret": "cf_secret_x" }),
                cf_hook,
            ),
        ] {
            let (s, body) = raw(&router, &t.key, None, "POST", "/v1/provider_accounts", None, json!({ "provider": provider, "credentials": creds, "webhook_secret": hook, "base_url": mock })).await;
            assert_eq!(s, StatusCode::CREATED, "{body}");
        }
        tenants.push(t);
    }
    let b = tenants.pop().unwrap();
    let a = tenants.pop().unwrap();
    T {
        app: router,
        state,
        root,
        mock,
        http: reqwest::Client::new(),
        a,
        b,
    }
}

pub async fn raw(
    app: &Router,
    key: &str,
    actor: Option<&str>,
    method: &str,
    path: &str,
    idem: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"));
    if let Some(i) = idem {
        req = req.header("idempotency-key", i);
    }
    if let Some(a) = actor {
        req = req.header("zyvor-actor", a);
    }
    let res = app
        .clone()
        .oneshot(
            req.body(Body::from(if body.is_null() {
                String::new()
            } else {
                body.to_string()
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

impl T {
    pub async fn call(
        &self,
        t: &Tenant,
        method: &str,
        path: &str,
        idem: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        raw(&self.app, &t.key, None, method, path, idem, body).await
    }
    pub async fn call_as(
        &self,
        t: &Tenant,
        actor: &str,
        method: &str,
        path: &str,
        idem: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        raw(&self.app, &t.key, Some(actor), method, path, idem, body).await
    }
    pub async fn ok(
        &self,
        t: &Tenant,
        method: &str,
        path: &str,
        idem: Option<&str>,
        body: Value,
    ) -> Value {
        let (s, b) = self.call(t, method, path, idem, body).await;
        assert!(s.is_success(), "{method} {path} -> {s}: {b}");
        b
    }
    /// A request with no API key, as a browser would send.
    pub async fn public(
        &self,
        method: &str,
        path: &str,
        idem: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        raw(&self.app, "", None, method, path, idem, body).await
    }
    /// The plain-text body of a public GET (for `/metrics`).
    pub async fn text(&self, path: &str) -> String {
        let res = self
            .app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }
    /// A public (no API key) call that must succeed.
    pub async fn ok_public(&self, method: &str, path: &str, body: Value) -> Value {
        let (s, b) = self.public(method, path, None, body).await;
        assert!(s.is_success(), "{method} {path} -> {s}: {b}");
        b
    }
    pub async fn mock_post(&self, path: &str, body: Value) -> Value {
        self.http
            .post(format!("{}/_sim/{path}", self.mock))
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    pub async fn mock_stats(&self) -> Value {
        self.http
            .get(format!("{}/_sim/stats", self.mock))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    /// Deliver a provider webhook (as produced by the mock) to the platform.
    pub async fn deliver(
        &self,
        provider: &str,
        tenant_id: &str,
        hook: &Value,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/v1/webhooks/{provider}/{tenant_id}"));
        for (k, v) in hook["headers"].as_object().unwrap() {
            req = req.header(k, v.as_str().unwrap());
        }
        let res = self
            .app
            .clone()
            .oneshot(
                req.body(Body::from(hook["body"].as_str().unwrap().to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    pub async fn intent(&self, t: &Tenant, amount: i64, extra: Value) -> Value {
        let mut b =
            json!({ "amount_minor": amount, "currency": "INR", "allowed_providers": ["razorpay"] });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            b[k] = v;
        }
        let key = format!("pi-{}", zyvor_core::new_id("k"));
        self.ok(t, "POST", "/v1/payment_intents", Some(&key), b)
            .await
    }

    /// Intent + Razorpay attempt + customer pays + webhook delivered. Returns (intent_id, attempt_id, provider_payment_ref).
    pub async fn paid_razorpay(
        &self,
        t: &Tenant,
        amount: i64,
        extra: Value,
    ) -> (String, String, String) {
        let intent = self.intent(t, amount, extra).await;
        let iid = intent["id"].as_str().unwrap().to_string();
        let att = self
            .ok(
                t,
                "POST",
                &format!("/v1/payment_intents/{iid}/attempts"),
                Some(&format!("att-{iid}")),
                json!({ "method": "upi" }),
            )
            .await;
        let order = att["provider_order_ref"].as_str().unwrap().to_string();
        let paid = self
            .mock_post(
                "razorpay/pay",
                json!({ "order_id": order, "outcome": "captured" }),
            )
            .await;
        let (s, _) = self.deliver("razorpay", &t.id, &paid["webhook"]).await;
        assert_eq!(s, StatusCode::OK);
        (
            iid,
            att["id"].as_str().unwrap().to_string(),
            paid["payment"]["id"].as_str().unwrap().to_string(),
        )
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
            .fetch_one(&self.root)
            .await
            .unwrap()
    }
}
