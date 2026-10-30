use std::{env, sync::Arc, time::Duration};

use sqlx::postgres::PgPoolOptions;
use zyvor_api::{AppState, Crypto, MIGRATOR, admin::bootstrap_org, app};

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::main]
async fn main() {
    let _telemetry = zyvor_api::telemetry::init("zyvor-api");
    // Migrations need the schema owner; the running service should connect as a restricted, non-superuser
    // role so row-level security binds it. Set ZYVOR_MIGRATE_DATABASE_URL to migrate with a different login.
    let args: Vec<String> = env::args().collect();
    let migrate_url = env::var("ZYVOR_MIGRATE_DATABASE_URL").ok();
    if let Some(url) = &migrate_url {
        let owner = PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .expect("connect (migrate)");
        MIGRATOR.run(&owner).await.expect("run migrations");
        if args.get(1).map(String::as_str) == Some("migrate") {
            println!("migrations applied");
            return;
        }
    }
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&required("DATABASE_URL"))
        .await
        .expect("connect to postgres");
    if migrate_url.is_none() {
        MIGRATOR.run(&pool).await.expect("run migrations");
    }

    // `zyvor-api bootstrap "Acme Pharmacy"` creates an organisation with test + live tenants and prints each admin key once.
    if args.get(1).map(String::as_str) == Some("bootstrap") {
        let name = args.get(2).map(String::as_str).unwrap_or("My Organisation");
        let b = bootstrap_org(&pool, name).await.expect("bootstrap");
        println!(
            "organisation {}\n\ntest tenant {}\n  key: {}\n\nlive tenant {}\n  key: {}\n\nStore these keys now; they cannot be shown again.",
            b.organisation_id, b.test_tenant, b.test_key, b.live_tenant, b.live_key
        );
        return;
    }

    let crypto = Crypto::from_hex(&required("ZYVOR_MASTER_KEY")).expect("valid ZYVOR_MASTER_KEY");
    let mut state = AppState::new(pool, crypto);
    if let Ok(s) = env::var("ZYVOR_PROVIDER_TIMEOUT_SECS") {
        state.provider_timeout = Duration::from_secs(s.parse().expect("seconds"));
    }
    let addr = env::var("ZYVOR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!(%addr, "zyvor-api listening");
    let _ = Arc::new(());
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await
        .expect("server");
}
