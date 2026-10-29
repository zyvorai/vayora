//! Zyvor Payments API: multi-tenant payment orchestration, ledger and reconciliation.
// sqlx row tuples are the idiomatic shape for these queries.
#![allow(clippy::type_complexity)]

pub mod admin;
pub mod auth;
pub mod cases;
pub mod crypto;
pub mod error;
pub mod idempotency;
pub mod outbox;
pub mod payments;
pub mod recon;
pub mod refunds;
pub mod registry;
pub mod state_machine;
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
}

pub use crypto::Crypto;

impl AppState {
    pub fn new(pool: PgPool, crypto: Crypto) -> Self {
        Self {
            pool,
            crypto,
            sim: Arc::new(SimProvider::new()),
            provider_timeout: Duration::from_secs(10),
            resolve_after: Duration::from_secs(30),
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
        .route("/recon/cases", get(recon::list_cases))
        .route("/recon/cases/{id}/assign", post(recon::assign_case))
        .route("/recon/cases/{id}/resolve", post(recon::resolve_case))
        .route("/recon/scan", post(recon::scan))
        .route("/analytics/cost", get(recon::cost_analytics))
        // administration
        .route("/api_keys", post(create_key).get(list_keys))
        .route("/api_keys/{id}/revoke", post(revoke_key))
        .route(
            "/provider_accounts",
            post(create_provider_account).get(list_provider_accounts),
        )
        .route(
            "/provider_accounts/{provider}/enabled",
            post(set_provider_enabled),
        )
        .route("/customers", post(create_customer))
        .route("/customers/{id}", get(get_customer))
        .route("/branches", post(create_branch).get(list_branches))
        .route("/users", post(create_user))
        .route("/invoices", post(create_invoice).get(list_invoices))
        .route("/invoices/{id}", get(get_invoice))
        .route("/settings", get(get_settings).post(put_settings))
        .route("/audit_events", get(audit_log))
        .route("/export", get(export))
        // provider -> Zyvor (authenticated by signature, not API key)
        .route(
            "/webhooks/{provider}/{tenant_id}",
            post(webhooks_in::receive),
        );

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
