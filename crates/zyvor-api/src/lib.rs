//! Zyvor Payments API: multi-tenant payment orchestration, ledger and reconciliation.
// sqlx row tuples are the idiomatic shape for these queries.
#![allow(clippy::type_complexity)]

pub mod admin;
pub mod auth;
pub mod bankcsv;
pub mod billing;
pub mod cases;
pub mod checkout;
pub mod collections;
pub mod crypto;
pub mod disputes;
pub mod error;
pub mod idempotency;
pub mod outbox;
pub mod payment_links;
pub mod payments;
pub mod recon;
pub mod refunds;
pub mod registry;
pub mod state_machine;
pub mod telemetry;
pub mod tenant;
pub mod webhooks_in;
pub mod worker;

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    routing::{get, post},
};
use sqlx::PgPool;
use zyvor_providers::sim::SimProvider;

pub use error::ApiError;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub crypto: Crypto,
    pub sim: Arc<SimProvider>,
    /// Client timeout for provider calls; a timeout after sending means the outcome is unknown.
    pub provider_timeout: Duration,
    /// Attempts/refunds with an unknown outcome are re-queried by the worker after this long.
    pub resolve_after: Duration,
    pub limiter: Arc<ratelimit::RateLimiter>,
    /// How many reverse proxies we run in front; they vouch for `X-Forwarded-For` from the right.
    pub trusted_hops: u8,
}

pub use crypto::Crypto;
pub mod ratelimit;

impl AppState {
    pub fn new(pool: PgPool, crypto: Crypto) -> Self {
        Self {
            pool,
            crypto,
            sim: Arc::new(SimProvider::new()),
            provider_timeout: Duration::from_secs(10),
            resolve_after: Duration::from_secs(30),
            limiter: Arc::new(ratelimit::RateLimiter::new(Default::default())),
            trusted_hops: 0,
        }
    }
}

pub fn app(state: AppState) -> Router {
    use admin::*;
    let v1 = Router::new()
        // payments
        .route(
            "/payment_intents",
            post(payments::create_intent).get(payments::list_intents),
        )
        .route("/payment_intents/{id}", get(payments::get_intent))
        .route(
            "/payment_intents/{id}/attempts",
            post(payments::create_attempt),
        )
        .route(
            "/payment_intents/{id}/refunds",
            post(refunds::create_refund),
        )
        .route("/payment_attempts/{id}", get(payments::get_attempt))
        .route(
            "/payment_attempts/{id}/refresh",
            post(payments::refresh_attempt),
        )
        .route(
            "/payment_attempts/{id}/cancel",
            post(payments::cancel_attempt),
        )
        .route(
            "/payment_attempts/{id}/capture",
            post(payments::capture_attempt),
        )
        // refunds
        .route("/refunds", get(refunds::list_refunds))
        .route("/refunds/{id}", get(refunds::get_refund))
        .route("/refunds/{id}/approve", post(refunds::approve_refund))
        .route("/refunds/{id}/reject", post(refunds::reject_refund))
        .route("/refunds/{id}/retry", post(refunds::retry_refund))
        .route("/refunds/{id}/refresh", post(refunds::refresh_refund))
        // reconciliation
        .route("/settlements", get(recon::list_settlements))
        .route("/settlements/import", post(recon::import_settlement))
        .route("/settlements/sync", post(recon::sync_settlements))
        .route("/settlements/{id}/explain", get(recon::explain))
        .route("/bank_statements/import", post(recon::import_bank))
        .route("/bank_statements/import_csv", post(bankcsv::import_csv))
        .route("/recon/cases", get(recon::list_cases))
        .route("/recon/cases/{id}/assign", post(recon::assign_case))
        .route("/recon/cases/{id}/resolve", post(recon::resolve_case))
        .route("/recon/scan", post(recon::scan))
        .route("/analytics/cost", get(recon::cost_analytics))
        // administration
        .route("/api_keys", post(create_key).get(list_keys))
        .route("/api_keys/{id}/revoke", post(revoke_key))
        .route("/api_keys/{id}/rotate", post(rotate_key))
        .route(
            "/provider_accounts",
            post(create_provider_account).get(list_provider_accounts),
        )
        .route(
            "/provider_accounts/{provider}/enabled",
            post(set_provider_enabled),
        )
        .route(
            "/customers",
            post(create_customer).get(billing::list_customers),
        )
        .route("/customers/{id}", get(get_customer))
        .route("/branches", post(create_branch).get(list_branches))
        .route("/users", post(create_user))
        .route("/invoices", post(create_invoice).get(list_invoices))
        .route("/invoices/{id}", get(get_invoice))
        // disputes
        .route(
            "/disputes",
            post(disputes::open_dispute).get(disputes::list_disputes),
        )
        .route("/disputes/{id}/evidence", post(disputes::add_evidence))
        .route("/disputes/{id}/resolve", post(disputes::resolve_dispute))
        // billing
        .route(
            "/plans",
            post(billing::create_plan).get(billing::list_plans),
        )
        .route(
            "/subscriptions",
            post(billing::create_subscription).get(billing::list_subscriptions),
        )
        .route(
            "/subscriptions/{id}/pause",
            post(billing::pause_subscription),
        )
        .route(
            "/subscriptions/{id}/resume",
            post(billing::resume_subscription),
        )
        .route(
            "/subscriptions/{id}/cancel",
            post(billing::cancel_subscription),
        )
        .route("/subscriptions/{id}/usage", post(billing::record_usage))
        .route(
            "/subscriptions/{id}/change_plan",
            post(billing::change_plan),
        )
        .route(
            "/milestones",
            post(billing::create_milestone).get(billing::list_milestones),
        )
        .route(
            "/milestones/{id}/complete",
            post(billing::complete_milestone),
        )
        .route("/milestones/{id}/cancel", post(billing::cancel_milestone))
        .route("/billing/run", post(billing::billing_run))
        .route(
            "/invoices/{id}/credit_notes",
            post(billing::create_credit_note),
        )
        .route("/invoices/{id}/void", post(billing::void_invoice))
        .route("/customers/{id}/statement", get(billing::statement))
        // collections
        .route("/collections/ageing", get(collections::ageing))
        .route("/collections/promises", get(collections::list_promises))
        .route(
            "/collections/promises/{id}/cancel",
            post(collections::cancel_promise),
        )
        .route("/invoices/{id}/promises", post(collections::create_promise))
        .route(
            "/invoices/{id}/reminders",
            post(collections::create_reminder).get(collections::list_reminders),
        )
        .route("/collections/dunning/run", post(collections::run_dunning))
        .route("/settings", get(get_settings).post(put_settings))
        .route("/audit_events", get(audit_log))
        .route("/export", get(export))
        .route("/outbox", get(list_outbox))
        .route("/outbox/{id}/replay", post(replay_outbox))
        .route("/inbox", get(list_inbox))
        // payment links
        .route(
            "/payment_links",
            post(payment_links::create_link).get(payment_links::list_links),
        )
        .route(
            "/payment_links/{id}/cancel",
            post(payment_links::cancel_link),
        )
        .route(
            "/checkout/links/{tenant_id}/{token}",
            get(payment_links::link_info),
        )
        .route(
            "/checkout/links/{tenant_id}/{token}/start",
            post(payment_links::link_start),
        )
        // public checkout: authorised by the intent's client secret, not an API key
        .route("/checkout/{tenant_id}/{intent_id}", get(checkout::session))
        .route(
            "/checkout/{tenant_id}/{intent_id}/attempts",
            post(checkout::pay),
        )
        // provider -> Zyvor (authenticated by signature, not API key)
        .route(
            "/webhooks/{provider}/{tenant_id}",
            post(webhooks_in::receive),
        );

    let v1 = v1
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ratelimit::limit,
        ))
        .layer(axum::middleware::from_fn(telemetry::request_span));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .nest("/v1", v1)
        .with_state(state)
}

async fn readyz(
    axum::extract::State(s): axum::extract::State<AppState>,
) -> Result<&'static str, axum::http::StatusCode> {
    sqlx::query("SELECT 1")
        .execute(&s.pool)
        .await
        .map(|_| "ready")
        .map_err(|_| axum::http::StatusCode::SERVICE_UNAVAILABLE)
}

/// Prometheus text format: queue depths and open reconciliation cases.
async fn metrics(axum::extract::State(s): axum::extract::State<AppState>) -> String {
    let (pending, dead, cases) = worker::queue_depths(&s.pool).await.unwrap_or((-1, -1, -1));
    format!(
        "# TYPE zyvor_outbox_pending gauge\nzyvor_outbox_pending {pending}\n# TYPE zyvor_outbox_dead gauge\nzyvor_outbox_dead {dead}\n# TYPE zyvor_recon_cases_open gauge\nzyvor_recon_cases_open {cases}\n"
    )
}
