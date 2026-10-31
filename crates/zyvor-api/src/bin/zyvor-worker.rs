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
    let crypto = Crypto::from_env().expect("valid ZYVOR_MASTER_KEY");
    let mut state = AppState::new(pool, crypto);
    state.allow_private_targets = env::var("ZYVOR_ALLOW_PRIVATE_TARGETS").as_deref() == Ok("1");
    state.mailer = zyvor_api::mail::Mailer::from_env().expect("valid email settings");
    let client = outbox::http_client();
    // Mail has its own loop: a mail server that hangs must not hold up webhooks, polling or refunds.
    let mail_state = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = zyvor_api::mail::drain(&mail_state).await {
                tracing::error!(error = %e, "mail drain failed");
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
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
