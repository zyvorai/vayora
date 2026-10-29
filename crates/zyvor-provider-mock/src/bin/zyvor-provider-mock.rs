//! Standalone sandbox provider: `PORT=9090 zyvor-provider-mock`.
#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "9090".into());
    let cfg = zyvor_provider_mock::MockConfig {
        razorpay_webhook_secret: std::env::var("MOCK_RAZORPAY_WEBHOOK_SECRET")
            .unwrap_or_else(|_| "rzp_whsec_sandbox".into()),
        cashfree_webhook_secret: std::env::var("MOCK_CASHFREE_WEBHOOK_SECRET")
            .unwrap_or_else(|_| "cf_whsec_sandbox".into()),
    };
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("bind");
    println!("zyvor-provider-mock (SIMULATED Razorpay + Cashfree) on :{port}");
    axum::serve(listener, zyvor_provider_mock::router(cfg))
        .await
        .expect("serve");
}
