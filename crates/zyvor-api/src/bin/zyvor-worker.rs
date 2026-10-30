use std::{env, time::Duration};

use sqlx::postgres::PgPoolOptions;
use zyvor_api::{AppState, Crypto, outbox, worker};

#[tokio::main]
async fn main() {
    let _telemetry = zyvor_api::telemetry::init("zyvor-worker");
    let url = env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect");
    let crypto = Crypto::from_hex(&env::var("ZYVOR_MASTER_KEY").expect("ZYVOR_MASTER_KEY"))
        .expect("valid key");
    let state = AppState::new(pool, crypto);
    let client = outbox::http_client();
    tracing::info!("zyvor-worker started");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { tracing::info!("shutting down"); break; }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                match worker::run_cycle(&state, &client).await {
                    Ok(s) if s.attempts_checked + s.refunds_checked + s.events_delivered > 0 => tracing::info!(?s, "cycle"),
                    Ok(_) => {}
                    Err(e) => tracing::error!(error = %e, "cycle failed"),
                }
            }
        }
    }
}
