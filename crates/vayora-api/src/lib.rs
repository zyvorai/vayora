//! Vayora HTTP API: orders, simulated-UPI payments, refunds, ledger balance.

mod error;
mod idempotency;
mod reads;
mod routes;

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use vayora_upi::UpiGateway;

pub use error::ApiError;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub upi: Arc<dyn UpiGateway>,
    api_key: Arc<str>,
}

impl AppState {
    pub fn new(pool: PgPool, upi: Arc<dyn UpiGateway>, api_key: impl Into<Arc<str>>) -> Self {
        Self {
            pool,
            upi,
            api_key: api_key.into(),
        }
    }
}

pub fn app(state: AppState) -> Router {
    let v1 = Router::new()
        .route(
            "/orders",
            post(routes::create_order).get(reads::list_orders),
        )
        .route("/orders/{id}", get(routes::get_order))
        .route(
            "/payments",
            post(routes::create_payment).get(reads::list_payments),
        )
        .route("/payments/{id}", get(routes::get_payment))
        .route("/payments/{id}/refunds", post(routes::create_refund))
        .route("/refunds", get(reads::list_refunds))
        .route("/events", get(reads::list_events))
        .route("/events/{id}/retry", post(reads::retry_event))
        .route("/stats", get(reads::stats))
        .route("/balance", get(routes::balance))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_key,
        ));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/v1", v1)
        .with_state(state)
}

async fn require_api_key(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let supplied = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if supplied.as_bytes().ct_eq(state.api_key.as_bytes()).into() {
        Ok(next.run(req).await)
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid API key",
        ))
    }
}
