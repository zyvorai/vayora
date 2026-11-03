//! Sends one real email through a real SMTP server, to check the mailer against a provider (the flow tests only
//! use a local sink). Ignored by default; it needs three variables and sends exactly one message:
//!
//! ```text
//! ZYVOR_TEST_SMTP_URL='smtp://USER:PASS@smtp.zoho.in:587?tls=required' \
//! ZYVOR_TEST_SMTP_FROM='Zyvor <no-reply@your-domain>' \
//! ZYVOR_TEST_SMTP_TO=you@example.com \
//!   cargo test -p zyvor-api --test smtp_real -- --ignored
//! ```

#[tokio::test]
#[ignore = "sends a real email; see the file's header"]
async fn one_real_message_is_delivered_with_a_plain_and_an_html_part() {
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"));
    let mailer =
        zyvor_api::mail::Mailer::smtp(&var("ZYVOR_TEST_SMTP_URL"), &var("ZYVOR_TEST_SMTP_FROM"))
            .expect("the SMTP settings are valid");
    let body = "This is a test from the Zyvor API's mailer.\n\nIf you can read this, outgoing email works.";
    let html = zyvor_api::mail::html_from_text(body);
    mailer
        .send(
            &var("ZYVOR_TEST_SMTP_TO"),
            "Zyvor mailer check",
            body,
            None,
            Some(&html),
        )
        .await
        .expect("the server accepted the message");
}
