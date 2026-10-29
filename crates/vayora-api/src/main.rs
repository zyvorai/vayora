use std::{env, sync::Arc};

use sqlx::postgres::PgPoolOptions;
use vayora_api::{AppState, MIGRATOR, app};
use vayora_upi::SimulatedUpi;
use vayora_webhooks::Dispatcher;

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&required("DATABASE_URL"))
        .await
        .expect("connect to postgres");
    MIGRATOR.run(&pool).await.expect("run migrations");

    // Webhooks are delivered only when a target is configured.
    if let Ok(url) = env::var("VAYORA_WEBHOOK_URL") {
        let secret = required("VAYORA_WEBHOOK_SECRET");
        tokio::spawn(Dispatcher::new(pool.clone(), url, secret.into_bytes()).run_forever());
    }

    let state = AppState::new(pool, Arc::new(SimulatedUpi), required("VAYORA_API_KEY"));
    let addr = env::var("VAYORA_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!(%addr, "vayora-api listening (simulated UPI)");
    axum::serve(listener, app(state)).await.expect("server");
}
