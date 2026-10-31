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
        if env::var("ZYVOR_AUTO_MIGRATE").as_deref() == Ok("1") {
            MIGRATOR.run(&pool).await.expect("run migrations");
        } else {
            // The restricted app role must not own the schema: check it is current instead of changing it.
            let applied: i64 =
                sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(0);
            let expected = MIGRATOR
                .migrations
                .iter()
                .filter(|m| !m.migration_type.is_down_migration())
                .count() as i64;
            assert!(
                applied >= expected,
                "database schema is behind this build ({applied} of {expected} migrations applied): \
                 run `zyvor-api migrate` with ZYVOR_MIGRATE_DATABASE_URL, or set ZYVOR_AUTO_MIGRATE=1 for a throwaway database"
            );
        }
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

    let crypto = Crypto::from_env().expect("valid ZYVOR_MASTER_KEY");

    // `zyvor-api rekey [--dry-run]` re-seals stored secrets under the current key (see crypto.rs).
    // Runs per tenant through row-level security, so the normal restricted role is enough.
    if args.get(1).map(String::as_str) == Some("rekey") {
        let (done, current, bad) =
            zyvor_api::admin::rekey_all(&pool, &crypto, args.iter().any(|a| a == "--dry-run"))
                .await
                .expect("rekey");
        println!("resealed {done}, already current {current}, unreadable {bad}");
        if bad > 0 {
            eprintln!(
                "{bad} secret(s) cannot be opened by any configured key: add the old key to ZYVOR_MASTER_KEY_PREVIOUS and run again"
            );
            std::process::exit(2);
        }
        return;
    }
    let mut state = AppState::new(pool, crypto);
    if let Ok(s) = env::var("ZYVOR_PROVIDER_TIMEOUT_SECS") {
        state.provider_timeout = Duration::from_secs(s.parse().expect("seconds"));
    }
    state.allow_private_targets = env::var("ZYVOR_ALLOW_PRIVATE_TARGETS").as_deref() == Ok("1");
    state.checkout_public_url = env::var("ZYVOR_CHECKOUT_PUBLIC_URL")
        .ok()
        .filter(|u| u.starts_with("http"));
    state.ai = zyvor_api::ai::AiConfig::from_env().map(Arc::new);
    state.actor_secret = env::var("ZYVOR_ACTOR_SECRET")
        .ok()
        .filter(|s| s.len() >= 16)
        .map(String::into_bytes);
    if let Ok(name) = env::var("ZYVOR_KYC_PROVIDER") {
        state.kyc = zyvor_api::kyc::Kyc::from_name(&name);
        match state.kyc {
            Some(_) => tracing::warn!(
                "KYC simulator is on: onboarding approvals are NOT real verification. Never use this for live merchants."
            ),
            None => panic!(
                "ZYVOR_KYC_PROVIDER={name} is not a known provider (only `simulator` exists)"
            ),
        }
    }
    state.mailer = zyvor_api::mail::Mailer::from_env().expect("valid email settings");
    if let Ok(u) = env::var("ZYVOR_CONSOLE_PUBLIC_URL") {
        state.console_url = u.trim_end_matches('/').to_string();
    }
    state.trusted_hops = env::var("ZYVOR_TRUST_PROXY_HOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    {
        let mut l = state.limiter.limits;
        let f = |k: &str, d: f64| env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        l.key.per_sec = f("ZYVOR_RATE_KEY_PER_SEC", l.key.per_sec);
        l.key.burst = f("ZYVOR_RATE_KEY_BURST", l.key.burst);
        l.ip_public.per_sec = f("ZYVOR_RATE_PUBLIC_PER_SEC", l.ip_public.per_sec);
        l.ip_public.burst = f("ZYVOR_RATE_PUBLIC_BURST", l.ip_public.burst);
        state.limiter = Arc::new(zyvor_api::ratelimit::RateLimiter::new(l));
    }
    let addr = env::var("ZYVOR_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!(%addr, "zyvor-api listening");
    let _ = Arc::new(());
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
    })
    .await
    .expect("server");
}
