//! HTTP API. Orders are held in memory for now (Postgres comes with the vertical slice).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use vayora_core::Money;

#[derive(Clone, Serialize)]
pub struct Order {
    pub id: String,
    pub amount_paise: i64,
    pub currency: &'static str,
    pub status: &'static str,
}

#[derive(Deserialize)]
struct CreateOrder {
    amount_paise: i64,
}

#[derive(Default, Clone)]
struct AppState {
    // Idempotency-Key -> order. Replaced by Redis/Postgres later.
    orders: Arc<Mutex<HashMap<String, Order>>>,
}

pub fn app() -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/orders", post(create_order))
        .with_state(AppState::default())
}

async fn create_order(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateOrder>,
) -> Result<(StatusCode, Json<Order>), (StatusCode, &'static str)> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|k| !k.is_empty())
        .ok_or((StatusCode::BAD_REQUEST, "Idempotency-Key header required"))?
        .to_owned();
    let amount = Money::from_paise(body.amount_paise)
        .ok()
        .filter(|m| m.paise() > 0)
        .ok_or((
            StatusCode::UNPROCESSABLE_ENTITY,
            "amount_paise must be positive",
        ))?;

    let mut orders = state.orders.lock().unwrap();
    if let Some(existing) = orders.get(&key) {
        return Ok((StatusCode::OK, Json(existing.clone())));
    }
    let order = Order {
        id: format!("order_{}", uuid::Uuid::now_v7().simple()),
        amount_paise: amount.paise(),
        currency: "INR",
        status: "created",
    };
    orders.insert(key, order.clone());
    Ok((StatusCode::CREATED, Json(order)))
}
