//! The Cashfree adapter against Cashfree's real SANDBOX. Ignored by default: it needs credentials and the
//! network, and it never runs in CI. Run it with sandbox keys in the environment:
//!
//!   CASHFREE_SANDBOX_CLIENT_ID=... CASHFREE_SANDBOX_CLIENT_SECRET=... \
//!     cargo test -p zyvor-providers --test cashfree_sandbox -- --ignored --nocapture
//!
//! The sandbox moves no money. It cannot show that a real UPI app (BHIM, PhonePe, Google Pay) completes a
//! payment: that needs small-value live payments on real devices (docs/zyvor/PARTNER_SETUP.md).

use std::time::Duration;

use zyvor_core::Currency;
use zyvor_providers::{cashfree::Cashfree, *};

fn adapter() -> Cashfree {
    let id = std::env::var("CASHFREE_SANDBOX_CLIENT_ID").expect("CASHFREE_SANDBOX_CLIENT_ID");
    let secret =
        std::env::var("CASHFREE_SANDBOX_CLIENT_SECRET").expect("CASHFREE_SANDBOX_CLIENT_SECRET");
    Cashfree::new(
        "https://sandbox.cashfree.com",
        id,
        secret,
        Duration::from_secs(20),
    )
    .with_upi_intent(true)
}

fn pay(device: &str, os: &str) -> CreatePayment {
    CreatePayment {
        attempt_id: zyvor_core::new_id("att"),
        amount_minor: 49_900,
        currency: Currency::Inr,
        method: Method::Upi,
        vpa: None,
        customer_id: None,
        customer_phone: Some("9999999999".into()),
        capture_automatic: true,
        return_url: None,
        upi_intent: true,
        client: Some(ClientEnv {
            device: device.into(),
            os: os.into(),
            rendering: "mweb".into(),
            browser: "chrome".into(),
        }),
    }
}

#[tokio::test]
#[ignore = "needs Cashfree sandbox credentials and the network"]
async fn a_phone_gets_an_intent_and_a_desktop_gets_a_qr_from_the_real_sandbox() {
    let cf = adapter();
    assert!(cf.capabilities().upi_intent, "opted in for this run");

    // phone -> intent link
    let req = pay("mobile", "android");
    let r = cf.initiate(&req).await.expect("sandbox intent");
    assert_eq!(r.state.status, PaymentStatus::Pending);
    assert!(r.state.provider_payment_ref.is_some());
    match &r.action {
        ClientAction::UpiIntent { uri, apps, .. } => {
            assert!(
                uri.starts_with("https://") || uri.starts_with("upi://"),
                "{uri}"
            );
            println!(
                "intent: default link {} chars, apps {:?}",
                uri.len(),
                apps.keys().collect::<Vec<_>>()
            );
        }
        other => panic!("expected an intent, got {other:?}"),
    }
    // our attempt id is Cashfree's order id, so a lost response can be resolved by asking
    let looked_up = cf.fetch_payment(&req.attempt_id).await.expect("lookup");
    assert_eq!(looked_up.status, PaymentStatus::Pending);

    // desktop -> partner-rendered QR
    let req = pay("desktop", "macos");
    let r = cf.initiate(&req).await.expect("sandbox qr");
    match &r.action {
        ClientAction::UpiQr { image } => {
            assert!(image.starts_with("data:image/png;base64,"));
            println!("qr: PNG data URL of {} chars", image.len());
        }
        other => panic!("expected a QR, got {other:?}"),
    }
    assert_eq!(
        cf.fetch_payment(&req.attempt_id).await.unwrap().status,
        PaymentStatus::Pending
    );
}

#[tokio::test]
#[ignore = "needs Cashfree sandbox credentials and the network"]
async fn without_the_opt_in_the_hosted_flow_is_still_used() {
    let id = std::env::var("CASHFREE_SANDBOX_CLIENT_ID").unwrap();
    let secret = std::env::var("CASHFREE_SANDBOX_CLIENT_SECRET").unwrap();
    let cf = Cashfree::new(
        "https://sandbox.cashfree.com",
        id,
        secret,
        Duration::from_secs(20),
    );
    assert!(!cf.capabilities().upi_intent);
    let r = cf.initiate(&pay("mobile", "android")).await.unwrap();
    assert!(
        matches!(r.action, ClientAction::HostedCheckout { .. }),
        "{:?}",
        r.action
    );
}
