//! Zyvor Payments API: multi-tenant payment orchestration, ledger and reconciliation.
// sqlx row tuples are the idiomatic shape for these queries.
#![allow(clippy::type_complexity)]

pub mod accounts;
pub mod admin;
pub mod ai;
pub mod audit;
pub mod auth;
pub mod bankcsv;
pub mod billing;
pub mod cases;
pub mod checkout;
pub mod clientenv;
pub mod closing;
pub mod collections;
pub mod console_users;
pub mod counters;
pub mod coupons;
pub mod crypto;
pub mod disputes;
pub mod error;
pub mod events;
pub mod fx;
pub mod gst;
pub mod idempotency;
pub mod insights;
pub mod installments;
pub mod kyc;
pub mod kyc_cashfree;
pub mod mail;
pub mod mandates;
pub mod onboarding;
pub mod outbox;
pub mod payment_links;
pub mod payments;
pub mod payouts;
pub mod platform;
pub mod recon;
pub mod refunds;
pub mod registry;
pub mod reminders;
pub mod signup;
pub mod sim_mandate;
pub mod sim_pay;
pub mod state_machine;
pub mod suggestions;
pub mod telemetry;
pub mod tenant;
pub mod timeline;
pub mod webhooks_in;
pub mod worker;

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    routing::{get, post, put},
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
    /// Sandboxes only: let merchant URLs (webhooks) point at private addresses and plain http.
    pub allow_private_targets: bool,
    /// Optional reminder-drafting assistant; `None` means the feature is off.
    pub ai: Option<Arc<ai::AiConfig>>,
    /// Public address of the checkout app, for QR codes (`ZYVOR_CHECKOUT_PUBLIC_URL`).
    pub checkout_public_url: Option<String>,
    /// When set, `Zyvor-Actor` is believed only with a valid `Zyvor-Actor-Signature` made with this secret
    /// (`ZYVOR_ACTOR_SECRET`, shared with the console). Without it the header is taken at its word.
    pub actor_secret: Option<Vec<u8>>,
    /// Identity and bank checks for onboarding (`ZYVOR_KYC_PROVIDER`). `None`: every submission goes to a person.
    pub kyc: Option<kyc::Kyc>,
    /// Outgoing email (`ZYVOR_SMTP_URL`); the log-only mailer when unset.
    pub mailer: mail::Mailer,
    /// Where the console is reached, for links in emails (`ZYVOR_CONSOLE_PUBLIC_URL`).
    pub console_url: String,
    /// Test environments only: let a payer app approve simulated QR payments (`ZYVOR_SIM_PAYER=1`).
    pub sim_payer: bool,
    /// The highest per-debit limit a mandate may be created with, in minor units (`ZYVOR_MANDATE_MAX_MINOR`).
    /// The default is the strict reading of the no-extra-authentication limit; check it against the current
    /// NPCI circular and your provider before any live use.
    pub mandate_max_minor: i64,
    /// How long before a debit the payer must be told, in hours (`ZYVOR_MANDATE_NOTICE_HOURS`).
    pub mandate_notice_hours: i64,
}

pub use crypto::Crypto;
pub mod ratelimit;
pub mod risk;
pub mod splits;
pub mod ssrf;

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
            allow_private_targets: false,
            ai: None,
            checkout_public_url: None,
            actor_secret: None,
            kyc: None,
            mailer: mail::Mailer::Log,
            console_url: "http://localhost:3000".to_string(),
            sim_payer: false,
            mandate_max_minor: 1_500_000,
            mandate_notice_hours: 24,
        }
    }
}

/// Today's date in India. Dates the product defaults to (billing, dunning, reports, FX) are Indian business days,
/// so the UTC date is wrong from 00:00 to 05:30 IST.
pub fn ist_today() -> chrono::NaiveDate {
    chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(19_800).expect("IST"))
        .date_naive()
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
        .route(
            "/recon/cases/{id}/suggestions",
            get(suggestions::case_suggestions),
        )
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
            "/provider_accounts/{provider}/check",
            post(admin::check_provider_account),
        )
        .route(
            "/provider_accounts/{provider}",
            put(replace_provider_account),
        )
        .route(
            "/provider_accounts/{provider}/enabled",
            post(set_provider_enabled),
        )
        .route(
            "/customers",
            post(create_customer).get(billing::list_customers),
        )
        .route("/customers/{id}", get(get_customer).patch(update_customer))
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
            "/mandates",
            post(mandates::create_mandate).get(mandates::list_mandates),
        )
        .route("/mandates/run", post(mandates::run_now))
        .route("/mandates/{id}", get(mandates::get_mandate))
        .route("/mandates/{id}/refresh", post(mandates::refresh_mandate))
        .route("/mandates/{id}/pause", post(mandates::pause_mandate))
        .route("/mandates/{id}/resume", post(mandates::resume_mandate))
        .route("/mandates/{id}/revoke", post(mandates::revoke_mandate))
        .route(
            "/mandates/{id}/debits",
            post(mandates::create_debit).get(mandates::list_debits),
        )
        .route(
            "/subscriptions/{id}/mandate",
            post(mandates::link_subscription),
        )
        .route(
            "/invoices/{id}/credit_notes",
            post(billing::create_credit_note),
        )
        .route("/invoices/{id}/void", post(billing::void_invoice))
        // payouts to vendors
        .route(
            "/vendors",
            post(payouts::create_vendor).get(payouts::list_vendors),
        )
        .route("/vendors/{id}/balance", get(splits::balance))
        .route("/vendors/{id}/details", get(payouts::vendor_details))
        .route(
            "/payouts",
            post(payouts::create_payout).get(payouts::list_payouts),
        )
        .route("/payouts/bulk", post(payouts::create_bulk))
        .route("/payouts/{id}", get(payouts::get_payout))
        .route("/payouts/{id}/approve", post(payouts::approve_payout))
        .route("/payouts/{id}/reject", post(payouts::reject_payout))
        .route("/payouts/{id}/mark_paid", post(payouts::mark_paid))
        .route("/payouts/{id}/cancel", post(payouts::cancel_payout))
        .route("/installment_plans", post(installments::create_plan))
        .route("/installment_plans/{id}", get(installments::get_plan))
        .route("/fx/rates", post(fx::set_rate).get(fx::list_rates))
        .route("/fx/convert", get(fx::convert))
        .route("/fx/exposure", get(fx::exposure))
        .route("/events/catalogue", get(events::catalogue))
        .route("/invoices/{id}/lines", get(gst::invoice_lines))
        .route("/invoices/{id}/einvoice", get(gst::einvoice))
        .route("/gst/gstr1", get(gst::gstr1))
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
        .route("/collections/drafts", get(reminders::list_drafts))
        .route(
            "/collections/drafts/{id}/approve",
            post(reminders::approve_draft),
        )
        .route(
            "/collections/drafts/{id}/reject",
            post(reminders::reject_draft),
        )
        .route(
            "/invoices/{id}/reminders/draft",
            post(reminders::create_draft),
        )
        .route("/settings", get(get_settings).post(put_settings))
        .route("/audit_events", get(audit_log))
        // console users (the console signs people in through these)
        .route(
            "/console/users",
            post(console_users::create_user).get(console_users::list_users),
        )
        .route(
            "/console/users/{id}/disable",
            post(console_users::disable_user),
        )
        .route(
            "/console/users/{id}/enable",
            post(console_users::enable_user),
        )
        .route(
            "/console/users/{id}/password",
            post(console_users::set_password),
        )
        .route(
            "/console/users/{id}/mfa/begin",
            post(console_users::mfa_begin),
        )
        .route(
            "/console/users/{id}/mfa/confirm",
            post(console_users::mfa_confirm),
        )
        .route(
            "/console/users/{id}/mfa/reset",
            post(console_users::reset_mfa),
        )
        .route("/console/branches", get(console_users::branches))
        .route("/console/authenticate", post(console_users::authenticate))
        // public self-serve signup
        .route("/signup", post(signup::signup))
        .route("/signup/verify", post(signup::verify))
        .route("/signup/resend", post(signup::resend))
        .route(
            "/console/invitations",
            post(accounts::invite).get(accounts::list_invitations),
        )
        .route(
            "/console/invitations/{id}/revoke",
            post(accounts::revoke_invitation),
        )
        .route("/invitations/lookup", post(accounts::lookup_invitation))
        .route("/invitations/accept", post(accounts::accept_invitation))
        .route("/password_reset/request", post(accounts::request_reset))
        .route("/password_reset/confirm", post(accounts::confirm_reset))
        .route("/console/login", post(signup::console_login))
        .route("/console/switch_env", post(signup::switch_env))
        .route("/console/logout", post(signup::console_logout))
        // Zyvor-operated review before live mode
        .route(
            "/live_access",
            get(platform::live_access_status).post(platform::request_live_access),
        )
        .route("/platform/reviews", get(platform::list_reviews))
        .route(
            "/platform/reviews/{organisation_id}",
            get(platform::show_review),
        )
        .route(
            "/platform/reviews/{organisation_id}/decision",
            post(platform::decide),
        )
        .route(
            "/platform/reviews/{organisation_id}/revoke",
            post(platform::revoke),
        )
        .route(
            "/platform/reviews/{organisation_id}/restore",
            post(platform::restore),
        )
        .route("/audit_events/seal", post(audit::seal))
        .route("/audit_events/verify", get(audit::verify))
        .route("/export", get(export))
        .route("/outbox", get(list_outbox))
        .route("/outbox/test", post(admin::send_test_event))
        .route("/outbox/{id}/replay", post(replay_outbox))
        .route("/inbox", get(list_inbox))
        // payment links
        .route(
            "/payment_links",
            post(payment_links::create_link).get(payment_links::list_links),
        )
        .route(
            "/coupons",
            post(coupons::create_coupon).get(coupons::list_coupons),
        )
        .route("/coupons/{id}/deactivate", post(coupons::deactivate_coupon))
        // counters and branch summaries
        .route(
            "/counters",
            post(counters::create_counter).get(counters::list_counters),
        )
        .route(
            "/counters/{id}/deactivate",
            post(counters::deactivate_counter),
        )
        .route("/branches/{id}/summary", get(counters::branch_summary))
        .route(
            "/payment_intents/{id}/timeline",
            get(timeline::payment_timeline),
        )
        .route("/insights/cashflow", get(insights::cashflow))
        .route(
            "/insights/refund_anomalies",
            get(insights::refund_anomalies),
        )
        // cash and daily closing
        .route("/cash_entries", post(closing::add_cash))
        .route(
            "/closings",
            post(closing::create_closing).get(closing::list_closings),
        )
        .route("/closings/{id}/signoff", post(closing::sign_off))
        // merchant onboarding
        .route(
            "/onboarding",
            get(onboarding::get_onboarding).put(onboarding::put_profile),
        )
        .route("/onboarding/documents", post(onboarding::add_document))
        .route(
            "/onboarding/bank_accounts",
            post(onboarding::add_bank_account),
        )
        .route(
            "/onboarding/bank_accounts/{id}/verify",
            post(onboarding::verify_bank_account),
        )
        .route("/onboarding/submit", post(onboarding::submit))
        .route("/onboarding/review", post(onboarding::review))
        .route("/payment_links/{id}/qr.svg", get(payment_links::link_qr))
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
        .route(
            "/checkout/{tenant_id}/{intent_id}/upi-qr.svg",
            get(checkout::upi_qr),
        )
        // simulated payer (test builds only; 404 unless ZYVOR_SIM_PAYER=1)
        .route("/sim/pay", post(sim_pay::pay))
        .route("/sim/mandate", post(sim_mandate::decide))
        .route("/sim/mandate/revoke", post(sim_mandate::revoke))
        .route("/sim/mandate/{id}", get(sim_mandate::view))
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
    let live = platform::pending_count(&s.pool).await;
    let (mail_pending, mail_dead) = mail::counts(&s.pool).await;
    format!(
        "# TYPE zyvor_mail_pending gauge\nzyvor_mail_pending {mail_pending}\n# TYPE zyvor_mail_dead gauge\nzyvor_mail_dead {mail_dead}\n# TYPE zyvor_outbox_pending gauge\nzyvor_outbox_pending {pending}\n# TYPE zyvor_outbox_dead gauge\nzyvor_outbox_dead {dead}\n# TYPE zyvor_recon_cases_open gauge\nzyvor_recon_cases_open {cases}\n# TYPE zyvor_live_access_pending gauge\nzyvor_live_access_pending {live}\n"
    )
}
