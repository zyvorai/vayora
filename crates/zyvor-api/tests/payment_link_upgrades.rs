//! Payment link upgrades: bulk creation, presentation, partial payments, analytics, sharing and SMS.
//! Simulators only; synthetic names, numbers and addresses.

mod common;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const UA_PHONE: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Mobile Safari/537.36";
const UA_IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1";

/// A request with a chosen user-agent and forwarded address (the app trusts one proxy hop in these tests).
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    ua: Option<&str>,
    ip: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-forwarded-for", ip);
    if let Some(u) = ua {
        r = r.header("user-agent", u);
    }
    let res = app
        .clone()
        .oneshot(
            r.body(Body::from(if body.is_null() {
                String::new()
            } else {
                body.to_string()
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let st = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (st, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn text_with_key(
    app: &Router,
    key: &str,
    path: &str,
) -> (StatusCode, String, Option<String>) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let st = res.status();
    let ct = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (st, String::from_utf8_lossy(&b).to_string(), ct)
}

fn app_with(t: &T, f: impl FnOnce(&mut zyvor_api::AppState)) -> Router {
    let mut st = t.state.clone();
    st.checkout_public_url = Some("https://pay.example.com".into());
    st.trusted_hops = 1;
    f(&mut st);
    zyvor_api::app(st)
}

/// Start a checkout on a link and pay it in full through the simulated provider; returns the intent id.
async fn start_and_capture(t: &T, tenant: &Tenant, token: &str, amount: Option<i64>) -> String {
    let body = amount
        .map(|a| json!({ "amount_minor": a }))
        .unwrap_or(json!({}));
    let s = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", tenant.id),
            body,
        )
        .await;
    capture(t, tenant, &s).await;
    s["intent_id"].as_str().unwrap().to_string()
}

async fn capture(t: &T, tenant: &Tenant, started: &Value) {
    let iid = started["intent_id"].as_str().unwrap();
    let (st, att) = raw(
        &t.app,
        "",
        None,
        "POST",
        &format!("/v1/checkout/{}/{iid}/attempts", tenant.id),
        Some(&format!("k-{iid}")),
        json!({ "secret": started["client_secret"], "method": "upi", "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{att}");
    let paid = if att["provider"] == "razorpay" {
        t.mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "captured" }),
        )
        .await
    } else {
        t.mock_post(
            "cashfree/pay",
            json!({ "order_id": att["id"], "outcome": "SUCCESS" }),
        )
        .await
    };
    let provider = if att["provider"] == "razorpay" {
        "razorpay"
    } else {
        "cashfree"
    };
    assert_eq!(
        t.deliver(provider, &tenant.id, &paid["webhook"]).await.0,
        StatusCode::OK
    );
}

async fn fail(t: &T, tenant: &Tenant, started: &Value) {
    let iid = started["intent_id"].as_str().unwrap();
    let (st, att) = raw(
        &t.app,
        "",
        None,
        "POST",
        &format!("/v1/checkout/{}/{iid}/attempts", tenant.id),
        Some(&format!("f-{iid}")),
        json!({ "secret": started["client_secret"], "method": "upi", "vpa": "asha@okvayora" }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{att}");
    let failed = if att["provider"] == "razorpay" {
        t.mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "failed" }),
        )
        .await
    } else {
        t.mock_post(
            "cashfree/pay",
            json!({ "order_id": att["id"], "outcome": "FAILED" }),
        )
        .await
    };
    let provider = if att["provider"] == "razorpay" {
        "razorpay"
    } else {
        "cashfree"
    };
    assert_eq!(
        t.deliver(provider, &tenant.id, &failed["webhook"]).await.0,
        StatusCode::OK
    );
}

async fn info(t: &T, tenant: &Tenant, token: &str) -> Value {
    t.ok_public(
        "GET",
        &format!("/v1/checkout/links/{}/{token}", tenant.id),
        Value::Null,
    )
    .await
}

// ---------------------------------------------------------------- bulk creation

const HEAD: &str =
    "customer_name,customer_email,customer_phone,amount,description,reference,expires\n";

#[sqlx::test(migrations = "./migrations")]
async fn a_csv_makes_many_links_all_or_nothing_and_never_twice(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let app = app_with(&t, |_| {});
    let file = format!(
        "{HEAD}Asha Rao,asha@example.com,98765 43210,\"1,250.50\",Term 2 fee,FEE-1,2099-12-31\nBala Iyer,bala@example.com,,499,Books,FEE-2,\nAsha Rao,ASHA@example.com,,300,Uniform,,\n=SUM(A1),,,75,,FEE-4,\n"
    );

    // a dry run says what would happen and stores nothing
    let (st, dry) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": file, "dry_run": true }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{dry}");
    assert_eq!(
        (
            dry["ok"].clone(),
            dry["would_create"].clone(),
            dry["rows"].clone()
        ),
        (json!(true), json!(4), json!(4))
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 0);
    assert_eq!(t.count("SELECT count(*) FROM customers").await, 0);

    // a file with problems is refused whole, every problem with its line
    let bad = format!("{HEAD}A,not-an-email,,100,,R1,\nB,,,abc,,R2,\nC,,,5,,R1,\nD,,,5,,R9,\n");
    let (st, e) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": bad }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let errs = e["error"]["errors"].as_array().unwrap();
    let has = |line: i64, col: &str| errs.iter().any(|x| x["line"] == line && x["column"] == col);
    assert!(
        has(2, "customer_email") && has(3, "amount") && has(4, "reference"),
        "{errs:?}"
    );
    assert_eq!(e["error"]["stored"], json!(false));
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 0);
    assert_eq!(t.count("SELECT count(*) FROM customers").await, 0);

    // for real: four links, the repeated customer is one customer
    let (st, made) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": file, "filename": "term2.csv", "message": "Thank you for paying on time.", "allowed_methods": ["upi"] }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{made}");
    assert_eq!(
        (made["created"].clone(), made["existing"].clone()),
        (json!(4), json!(0))
    );
    let links = made["links"].as_array().unwrap();
    assert_eq!(links.len(), 4);
    assert!(
        links[0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://pay.example.com/pay/link/")
    );
    assert_eq!(links[0]["amount_minor"], json!(125_050));
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 4);
    assert_eq!(
        t.count("SELECT count(*) FROM customers WHERE name = 'Asha Rao'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payment_links WHERE message = 'Thank you for paying on time.' AND allowed_methods = ARRAY['upi']").await,
        4
    );
    // the phone number was normalised
    assert_eq!(
        t.count("SELECT count(*) FROM customers WHERE phone = '+919876543210'")
            .await,
        1
    );

    // sending the same file again creates nothing and returns the same import
    let (st, again) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": file, "filename": "term2.csv", "message": "Thank you for paying on time.", "allowed_methods": ["upi"] }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{again}");
    assert_eq!(
        (again["repeat"].clone(), again["import_id"].clone()),
        (json!(true), made["import_id"].clone())
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 4);

    // a changed file: the old rows are found, the new row is made
    let more = format!("{file}Dev,,,60,Fine,FEE-5,\n");
    let (st, grown) = raw(&app, &a.key, None, "POST", "/v1/payment_links/bulk", None, json!({ "csv": more, "message": "Thank you for paying on time.", "allowed_methods": ["upi"] })).await;
    assert_eq!(st, StatusCode::CREATED, "{grown}");
    assert_eq!(
        (grown["created"].clone(), grown["existing"].clone()),
        (json!(1), json!(4))
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 5);

    // a reference that already names a different link is an error, not a second link
    let clash = format!("{HEAD}Asha Rao,asha@example.com,,999,Term 2 fee,FEE-1,\n");
    let (st, e) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": clash }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(
        e["error"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("external_ref_conflict"),
        "{e}"
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_links").await, 5);

    // one bad row late in a big file leaves nothing behind, not even its customers
    let before_customers = t.count("SELECT count(*) FROM customers").await;
    let late = format!("{HEAD}New Person,new@example.com,,10,,N-1,\nOther,,,10,,N-2,2020-01-01\n");
    let (st, e) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": late }),
    )
    .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(
        e["error"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("expires_at"),
        "{e}"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM customers").await,
        before_customers
    );
    assert_eq!(
        t.count("SELECT count(*) FROM payment_links WHERE external_ref LIKE 'N-%'")
            .await,
        0
    );

    // export: the links with their addresses, formulas defused
    let id = made["import_id"].as_str().unwrap();
    let (st, csv, ct) = text_with_key(
        &app,
        &a.key,
        &format!("/v1/payment_links/imports/{id}/export.csv"),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(ct.unwrap().starts_with("text/csv"));
    assert!(
        csv.starts_with("line,reference,outcome,link_id,url,path,amount"),
        "{csv}"
    );
    assert_eq!(csv.lines().count(), 5);
    assert!(csv.contains("https://pay.example.com/pay/link/"));
    assert!(
        csv.contains("'=SUM(A1)"),
        "a name that looks like a formula is defused: {csv}"
    );
    assert!(csv.contains("1250.50"));

    // another business sees none of it, and may use the same references for its own links
    let (st, _, _) = text_with_key(
        &app,
        &b.key,
        &format!("/v1/payment_links/imports/{id}/export.csv"),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(
        t.ok(b, "GET", "/v1/payment_links", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (st, theirs) = raw(
        &app,
        &b.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": format!("{HEAD}X,,,5,,FEE-1,\n") }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{theirs}");

    // single create honours the same reference rule
    let one = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 777, "external_ref": "ONE" }),
        )
        .await;
    let two = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 777, "external_ref": "ONE" }),
        )
        .await;
    assert_eq!(
        (one["id"].clone(), two["existing"].clone()),
        (two["id"].clone(), json!(true))
    );
    let (st, e) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 778, "external_ref": "ONE" }),
        )
        .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("external_ref_conflict"))
    );
}

// ---------------------------------------------------------------- presentation

#[sqlx::test(migrations = "./migrations")]
async fn a_link_carries_its_own_wording_brand_and_methods_safely(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let app = app_with(&t, |_| {});

    // text is plain: no web addresses, limited length
    for (field, v) in [
        ("message", json!("Pay at https://evil.example now")),
        ("message", json!("x".repeat(501))),
        ("thank_you", json!("see www.evil.example")),
    ] {
        let mut body = json!({ "amount_minor": 1000 });
        body[field] = v;
        assert_eq!(
            t.call(a, "POST", "/v1/payment_links", None, body)
                .await
                .0
                .as_u16(),
            422,
            "{field}"
        );
    }
    // return address: https only, and on the business's list when it has one
    for u in [
        "http://shop.example.com/done",
        "javascript:alert(1)",
        "https://user:pw@shop.example.com/",
    ] {
        assert_eq!(
            t.call(
                a,
                "POST",
                "/v1/payment_links",
                None,
                json!({ "amount_minor": 1000, "return_url": u })
            )
            .await
            .0
            .as_u16(),
            422,
            "{u}"
        );
    }
    sqlx::query("INSERT INTO tenant_settings (tenant_id, return_url_origins, email_accent, email_logo_url) VALUES ($1, '[\"https://shop.example.com\"]'::jsonb, '#ffe066', 'https://shop.example.com/logo.png') ON CONFLICT (tenant_id) DO UPDATE SET return_url_origins = EXCLUDED.return_url_origins, email_accent = EXCLUDED.email_accent, email_logo_url = EXCLUDED.email_logo_url")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    let (st, e) = t
        .call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 1000, "return_url": "https://other.example.com/x" }),
        )
        .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (422, json!("return_url_not_allowed"))
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 1000, "allowed_methods": ["upi", "cheque"] })
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        t.call(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 1000, "allowed_methods": [] })
        )
        .await
        .0
        .as_u16(),
        422
    );

    let link = t.ok(a, "POST", "/v1/payment_links", None, json!({
        "amount_minor": 2500, "description": "Workshop", "message": "Bring <b>your</b> ID.\n\n\n\nSee you there.",
        "thank_you": "Thanks! Seat confirmed.", "return_url": "https://shop.example.com/done",
        "use_branding": true, "allowed_methods": ["UPI", "upi"]
    })).await;
    let token = link["token"].as_str().unwrap();
    let i = info(&t, a, token).await;
    // stored as typed (no markup stripping: it is only ever shown as text), blank lines squeezed
    assert_eq!(
        i["message"],
        json!("Bring <b>your</b> ID.\n\nSee you there.")
    );
    assert_eq!(i["allowed_methods"], json!(["upi"]));
    // a pale yellow cannot carry a button on a white card: no colour, the page keeps its own; the logo is kept
    assert_eq!(i["brand"]["accent"], Value::Null);
    assert_eq!(
        i["brand"]["logo_url"],
        json!("https://shop.example.com/logo.png")
    );
    sqlx::query("UPDATE tenant_settings SET email_accent = '#0b7285' WHERE tenant_id = $1")
        .bind(&a.id)
        .execute(&t.root)
        .await
        .unwrap();
    let i = info(&t, a, token).await;
    assert_eq!(
        i["brand"]["accent"],
        json!({ "color": "#0b7285", "on_color": "#ffffff" })
    );
    // a link that did not ask for branding shows none
    let plain = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 2500 }),
        )
        .await;
    assert_eq!(
        info(&t, a, plain["token"].as_str().unwrap()).await["brand"],
        Value::Null
    );

    // the payer's checkout session carries the wording and only the allowed methods
    let s = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", a.id),
            json!({}),
        )
        .await;
    let iid = s["intent_id"].as_str().unwrap();
    let sess = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/{}/{iid}?secret={}",
                a.id,
                s["client_secret"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(sess["link"]["thank_you"], json!("Thanks! Seat confirmed."));
    assert_eq!(
        sess["link"]["return_url"],
        json!("https://shop.example.com/done")
    );
    assert_eq!(sess["methods"], json!(["upi"]));
    // a method the link does not accept is refused before any provider is called
    let (st, e) = raw(
        &app,
        "",
        None,
        "POST",
        &format!("/v1/checkout/{}/{iid}/attempts", a.id),
        Some("m1"),
        json!({ "secret": s["client_secret"], "method": "card" }),
    )
    .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (422, json!("method_not_allowed"))
    );
    assert_eq!(t.count("SELECT count(*) FROM payment_attempts").await, 0);
}

// ---------------------------------------------------------------- partial payments

#[sqlx::test(migrations = "./migrations")]
async fn a_link_paid_in_parts_is_paid_only_when_captured_parts_add_up(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    for bad in [
        json!({ "amount_minor": 100_000, "allow_partial": true }),
        json!({ "amount_minor": 100_000, "min_part_minor": 100 }),
        json!({ "amount_minor": 100_000, "allow_partial": true, "min_part_minor": 100_001 }),
        json!({ "amount_minor": 100_000, "allow_partial": true, "min_part_minor": 100, "max_uses": 3 }),
        json!({ "open_amount": true, "allow_partial": true, "min_part_minor": 100 }),
        json!({ "amount_minor": 100_000, "allow_partial": true, "min_part_minor": 100, "balance_reminder_days": 3 }),
    ] {
        assert_eq!(
            t.call(a, "POST", "/v1/payment_links", None, bad.clone())
                .await
                .0
                .as_u16(),
            422,
            "{bad}"
        );
    }
    let link = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 100_000, "description": "Course fee", "allow_partial": true, "min_part_minor": 20_000 })).await;
    let token = link["token"].as_str().unwrap();
    let i = info(&t, a, token).await;
    assert_eq!(i["status"], json!("active"));
    assert_eq!(i["partial"]["balance_minor"], json!(100_000));
    assert_eq!(i["amount_minor"], json!(100_000));

    let start = |amt: Option<i64>| {
        let body = amt
            .map(|v| json!({ "amount_minor": v }))
            .unwrap_or(json!({}));
        let path = format!("/v1/checkout/links/{}/{token}/start", a.id);
        let t = &t;
        async move { t.public("POST", &path, None, body).await }
    };
    // below the minimum part, or above the balance
    for amt in [10_000, 100_001] {
        let (st, e) = start(Some(amt)).await;
        assert_eq!(
            (st.as_u16(), e["error"]["code"].clone()),
            (422, json!("amount_out_of_range")),
            "{amt}"
        );
    }
    // coupons do not apply to parts
    let (st, _) = t
        .public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", a.id),
            None,
            json!({ "coupon": "X", "amount_minor": 30_000 }),
        )
        .await;
    assert_eq!(st.as_u16(), 422);

    // first part: 30,000 captured
    let first = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", a.id),
            json!({ "amount_minor": 30_000 }),
        )
        .await;
    assert_eq!(first["amount_minor"], json!(30_000));
    // while it is in progress, others cannot claim more than what is left unclaimed
    let (st, e) = start(Some(80_000)).await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (422, json!("amount_out_of_range")),
        "{e}"
    );
    let i = info(&t, a, token).await;
    assert_eq!(
        (
            i["partial"]["balance_minor"].clone(),
            i["partial"]["available_minor"].clone()
        ),
        (json!(100_000), json!(70_000))
    );
    // an unfinished checkout is not a payment: the balance has not moved
    capture(&t, a, &first).await;
    let i = info(&t, a, token).await;
    assert_eq!(
        (
            i["status"].clone(),
            i["partial"]["paid_minor"].clone(),
            i["partial"]["balance_minor"].clone(),
            i["amount_minor"].clone()
        ),
        (json!("active"), json!(30_000), json!(70_000), json!(70_000))
    );

    // a failed payment moves nothing
    let bad = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", a.id),
            json!({ "amount_minor": 20_000 }),
        )
        .await;
    fail(&t, a, &bad).await;
    let i = info(&t, a, token).await;
    assert_eq!(i["partial"]["paid_minor"], json!(30_000));
    sqlx::query("UPDATE payment_intents SET created_at = now() - interval '2 hours' WHERE status <> 'succeeded'").execute(&t.root).await.unwrap();

    // the rest, by default the whole balance; then the link is paid and closed
    let last = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/links/{}/{token}/start", a.id),
            json!({}),
        )
        .await;
    assert_eq!(last["amount_minor"], json!(70_000));
    // another payer cannot start while that balance is held
    let (st, e) = start(None).await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("link_busy")),
        "{e}"
    );
    assert_eq!(info(&t, a, token).await["status"], json!("in_progress"));
    capture(&t, a, &last).await;
    let i = info(&t, a, token).await;
    assert_eq!(i["status"], json!("paid"));
    let (st, e) = start(None).await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("link_unavailable"))
    );
    // never marked paid from anything but captured payments: exactly two succeeded intents
    assert_eq!(
        t.count("SELECT count(*) FROM payment_intents WHERE status = 'succeeded'")
            .await,
        2
    );

    // per-link analytics reports the balance
    let an = t
        .ok(
            a,
            "GET",
            &format!(
                "/v1/payment_links/{}/analytics",
                link["id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(an["balance"]["owed_minor"], json!(0));
    assert_eq!(an["totals"]["paid"], json!(2));
    assert_eq!(an["totals"]["paid_minor"], json!(100_000));
    assert_eq!(an["totals"]["failed"], json!(1));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_part_paid_invoice_link_follows_the_invoice_balance(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha Rao", "email": "asha@example.com" }),
        )
        .await;
    let inv = t.ok(a, "POST", "/v1/invoices", None, json!({ "customer_id": cust["id"], "number": "INV-1", "amount_minor": 100_000, "currency": "INR" })).await;
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "invoice_id": inv["id"], "allow_partial": true, "min_part_minor": 10_000 }),
        )
        .await;
    let token = link["token"].as_str().unwrap();
    start_and_capture(&t, a, token, Some(40_000)).await;
    let i = info(&t, a, token).await;
    assert_eq!(
        (
            i["partial"]["paid_minor"].clone(),
            i["partial"]["balance_minor"].clone()
        ),
        (json!(40_000), json!(60_000))
    );
    let status: String = sqlx::query_scalar("SELECT status FROM invoices WHERE id = $1")
        .bind(inv["id"].as_str().unwrap())
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(status, "partially_paid");
    start_and_capture(&t, a, token, None).await;
    assert_eq!(info(&t, a, token).await["status"], json!("paid"));
    let status: String = sqlx::query_scalar("SELECT status FROM invoices WHERE id = $1")
        .bind(inv["id"].as_str().unwrap())
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(status, "paid");
}

#[sqlx::test(migrations = "./migrations")]
async fn balance_reminders_go_by_email_and_sms_a_few_times_and_stop_when_paid(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha Rao", "email": "asha@example.com", "phone": "9876543210" }),
        )
        .await;
    let link = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 100_000, "customer_id": cust["id"], "allow_partial": true, "min_part_minor": 10_000, "balance_reminder_days": 3 })).await;
    let (lid, token) = (
        link["id"].as_str().unwrap(),
        link["token"].as_str().unwrap(),
    );
    let remind = |on: chrono::NaiveDate, sms: zyvor_api::sms::Mode| {
        let pool = t.state.pool.clone();
        let tid = a.id.clone();
        async move {
            let mut tx = zyvor_api::tenant::begin(&pool, &tid).await.unwrap();
            let n = zyvor_api::payment_links::remind_balances(
                &mut tx,
                &tid,
                "https://pay.example.com",
                on,
                &sms,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
            n
        }
    };
    let today = zyvor_api::ist_today();
    // nothing paid yet: nothing to remind
    assert_eq!(
        remind(
            today + chrono::Duration::days(10),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        0
    );
    start_and_capture(&t, a, token, Some(30_000)).await;
    // too soon
    assert_eq!(
        remind(
            today + chrono::Duration::days(1),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        0
    );
    // three days on: one reminder by email and one SMS (sandbox: logged, not sent)
    assert_eq!(
        remind(
            today + chrono::Duration::days(4),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        1
    );
    let mail: Vec<String> =
        sqlx::query_scalar("SELECT body FROM mail_queue WHERE subject LIKE '%still left to pay%'")
            .fetch_all(&t.root)
            .await
            .unwrap();
    assert_eq!(mail.len(), 1);
    assert!(
        mail[0].contains("₹300.00") && mail[0].contains("₹700.00") && mail[0].contains(token),
        "{}",
        mail[0]
    );
    let sms: Vec<(String, String)> = sqlx::query_as("SELECT status, to_phone FROM sms_queue")
        .fetch_all(&t.root)
        .await
        .unwrap();
    assert_eq!(
        sms,
        vec![("logged".to_string(), "+919876543210".to_string())]
    );
    // the same day again: nothing new; later: the next one, three at most
    assert_eq!(
        remind(
            today + chrono::Duration::days(4),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        0
    );
    assert_eq!(
        remind(
            today + chrono::Duration::days(8),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        1
    );
    assert_eq!(
        remind(
            today + chrono::Duration::days(12),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        1
    );
    assert_eq!(
        remind(
            today + chrono::Duration::days(30),
            zyvor_api::sms::Mode::Sandbox
        )
        .await,
        0
    );
    assert_eq!(
        t.count("SELECT count(*) FROM mail_queue WHERE subject LIKE '%still left to pay%'")
            .await,
        3
    );
    // SMS off: email only
    let link2 = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 50_000, "customer_id": cust["id"], "allow_partial": true, "min_part_minor": 10_000, "balance_reminder_days": 1 })).await;
    start_and_capture(&t, a, link2["token"].as_str().unwrap(), Some(10_000)).await;
    let before = t.count("SELECT count(*) FROM sms_queue").await;
    assert_eq!(
        remind(today + chrono::Duration::days(3), zyvor_api::sms::Mode::Off).await,
        1
    );
    assert_eq!(t.count("SELECT count(*) FROM sms_queue").await, before);
    // paid in full: no reminders for it
    let link3 = t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 20_000, "customer_id": cust["id"], "allow_partial": true, "min_part_minor": 10_000, "balance_reminder_days": 1 })).await;
    start_and_capture(&t, a, link3["token"].as_str().unwrap(), None).await;
    remind(
        today + chrono::Duration::days(20),
        zyvor_api::sms::Mode::Off,
    )
    .await;
    let t3 = link3["token"].as_str().unwrap().to_string();
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM mail_queue WHERE body LIKE '%{t3}%' AND subject LIKE '%still left to pay%'"
        ))
        .await,
        0
    );
    let _ = lid;
}

// ---------------------------------------------------------------- analytics

#[sqlx::test(migrations = "./migrations")]
async fn analytics_count_visitors_once_ignore_bots_and_keep_no_personal_data(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let app = app_with(&t, |_| {});
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 5_000 }),
        )
        .await;
    let (lid, token) = (
        link["id"].as_str().unwrap(),
        link["token"].as_str().unwrap(),
    );
    let path = format!("/v1/checkout/links/{}/{token}", a.id);

    // the same visitor reloading 25 times is one opened
    for _ in 0..25 {
        assert_eq!(
            send(
                &app,
                "GET",
                &path,
                Some(UA_PHONE),
                "203.0.113.5",
                Value::Null
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    // another address in the same /24 with the same kind of phone is the same coarse visitor
    send(
        &app,
        "GET",
        &path,
        Some(UA_PHONE),
        "203.0.113.99",
        Value::Null,
    )
    .await;
    // a different network, a different kind of device: two more
    send(
        &app,
        "GET",
        &path,
        Some(UA_PHONE),
        "198.51.100.7",
        Value::Null,
    )
    .await;
    send(
        &app,
        "GET",
        &path,
        Some(UA_IPHONE),
        "203.0.113.5",
        Value::Null,
    )
    .await;
    // machines are not counted at all: crawlers, link previews, tools, no user-agent
    for ua in [
        Some("Googlebot/2.1 (+http://www.google.com/bot.html)"),
        Some("WhatsApp/2.23.20.0 A"),
        Some("curl/8.4.0"),
        Some("Slackbot-LinkExpanding 1.0"),
        None,
    ] {
        for n in 0..10 {
            send(&app, "GET", &path, ua, &format!("192.0.2.{n}"), Value::Null).await;
        }
    }
    // typing a coupon is the same visit
    send(
        &app,
        "GET",
        &format!("{path}?coupon=NOPE"),
        Some(UA_PHONE),
        "203.0.113.200",
        Value::Null,
    )
    .await;

    let an = t
        .ok(
            a,
            "GET",
            &format!("/v1/payment_links/{lid}/analytics"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(an["totals"]["opened"], json!(3), "{an}");
    assert_eq!(an["totals"]["started"], json!(0));

    // starting twice from the same visitor is one start (two checkouts exist, bots cannot inflate the count)
    let s1 = send(
        &app,
        "POST",
        &format!("{path}/start"),
        Some(UA_PHONE),
        "203.0.113.5",
        json!({}),
    )
    .await;
    let s2 = send(
        &app,
        "POST",
        &format!("{path}/start"),
        Some(UA_PHONE),
        "203.0.113.5",
        json!({}),
    )
    .await;
    assert_eq!((s1.0, s2.0), (StatusCode::CREATED, StatusCode::CREATED));
    for n in 0..20 {
        send(
            &app,
            "POST",
            &format!("{path}/start"),
            Some("python-requests/2.31"),
            &format!("192.0.2.{n}"),
            json!({}),
        )
        .await;
    }
    assert_eq!(
        t.count("SELECT count(*) FROM payment_intents WHERE metadata->>'payment_link' IS NOT NULL")
            .await,
        22
    );
    // pay one, fail one
    capture(&t, a, &s1.1).await;
    fail(&t, a, &s2.1).await;

    let an = t
        .ok(
            a,
            "GET",
            &format!("/v1/payment_links/{lid}/analytics?days=7"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(an["totals"]["started"], json!(1));
    assert_eq!(an["totals"]["paid"], json!(1));
    assert_eq!(an["totals"]["failed"], json!(1));
    assert_eq!(an["daily"].as_array().unwrap().len(), 7);
    assert_eq!(an["funnel"]["started_of_opened"], json!(0.3333));
    assert_eq!(an["funnel"]["paid_of_started"], json!(1.0));
    assert!(an["notes"].as_array().unwrap().len() >= 3);

    // the aggregate over all links, for the business only
    let all = t
        .ok(a, "GET", "/v1/payment_links/analytics", None, Value::Null)
        .await;
    assert_eq!(
        (
            all["totals"]["opened"].clone(),
            all["totals"]["paid"].clone()
        ),
        (json!(3), json!(1))
    );
    assert_eq!(all["links"]["currently_usable"], json!(1));
    let theirs = t
        .ok(b, "GET", "/v1/payment_links/analytics", None, Value::Null)
        .await;
    assert_eq!(
        (
            theirs["totals"]["opened"].clone(),
            theirs["totals"]["paid"].clone()
        ),
        (json!(0), json!(0))
    );
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/payment_links/{lid}/analytics"),
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        404
    );
    assert_eq!(
        t.call(
            a,
            "GET",
            "/v1/payment_links/analytics?days=0",
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        422
    );
    assert_eq!(
        t.call(
            a,
            "GET",
            "/v1/payment_links/analytics?days=91",
            None,
            Value::Null
        )
        .await
        .0
        .as_u16(),
        422
    );

    // what is stored holds no address and no user-agent text: hashes and numbers only
    let cols: Vec<String> = sqlx::query_scalar("SELECT column_name FROM information_schema.columns WHERE table_name IN ('link_visitors_seen', 'link_events_daily') ORDER BY 1")
        .fetch_all(&t.root)
        .await
        .unwrap();
    assert_eq!(
        cols,
        [
            "day",
            "day",
            "event",
            "event",
            "link_id",
            "link_id",
            "n",
            "tenant_id",
            "tenant_id",
            "visitor"
        ]
    );
    let tokens: Vec<String> = sqlx::query_scalar("SELECT visitor FROM link_visitors_seen")
        .fetch_all(&t.root)
        .await
        .unwrap();
    assert!(
        !tokens.is_empty()
            && tokens
                .iter()
                .all(|v| v.len() == 32 && v.chars().all(|c| c.is_ascii_hexdigit()))
    );
    // the worker forgets the hashes after two days; the counters stay
    sqlx::query("UPDATE link_visitors_seen SET day = day - 3")
        .execute(&t.root)
        .await
        .unwrap();
    let gone = zyvor_api::link_stats::purge_tenant(&t.state.pool, &a.id)
        .await
        .unwrap();
    assert_eq!(gone as usize, tokens.len());
    assert_eq!(
        t.ok(
            a,
            "GET",
            &format!("/v1/payment_links/{lid}/analytics"),
            None,
            Value::Null
        )
        .await["totals"]["opened"],
        json!(3)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn expired_unpaid_links_are_counted_on_the_day_they_expired_and_paid_ones_are_not(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    let soon = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let unpaid = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100, "expires_at": soon }),
        )
        .await;
    let paid = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100, "expires_at": soon }),
        )
        .await;
    start_and_capture(&t, a, paid["token"].as_str().unwrap(), None).await;
    sqlx::query("UPDATE payment_links SET expires_at = now() - interval '1 hour'")
        .execute(&t.root)
        .await
        .unwrap();
    let all = t
        .ok(
            a,
            "GET",
            "/v1/payment_links/analytics?days=3",
            None,
            Value::Null,
        )
        .await;
    assert_eq!(all["totals"]["expired"], json!(1), "{all}");
    let one = t
        .ok(
            a,
            "GET",
            &format!(
                "/v1/payment_links/{}/analytics",
                unpaid["id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(one["totals"]["expired"], json!(1));
    let two = t
        .ok(
            a,
            "GET",
            &format!(
                "/v1/payment_links/{}/analytics",
                paid["id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(two["totals"]["expired"], json!(0));
    // the payer sees the state in words
    assert_eq!(
        info(&t, a, unpaid["token"].as_str().unwrap()).await["status"],
        json!("expired")
    );
}

// ---------------------------------------------------------------- sharing and SMS

#[sqlx::test(migrations = "./migrations")]
async fn a_link_can_be_shared_in_plain_words_and_texted_only_when_sms_is_on(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let with_phone = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha Rao", "phone": "98765 43210" }),
        )
        .await;
    let no_phone = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "No Phone", "email": "n@example.com" }),
        )
        .await;
    let mk = |c: &Value| {
        t.ok(a, "POST", "/v1/payment_links", None, json!({ "amount_minor": 125_050, "description": "Term 2 fee", "customer_id": c["id"], "expires_at": (chrono::Utc::now() + chrono::Duration::days(5)).to_rfc3339() }))
    };
    let (good, bare) = (mk(&with_phone).await, mk(&no_phone).await);
    let (gid, bid) = (good["id"].as_str().unwrap(), bare["id"].as_str().unwrap());

    // SMS is off by default
    let off = app_with(&t, |_| {});
    let (st, e) = raw(
        &off,
        &a.key,
        None,
        "POST",
        &format!("/v1/payment_links/{gid}/send_sms"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("sms_not_configured"))
    );

    // the share text is plain and honest
    let (st, sh) = raw(
        &off,
        &a.key,
        None,
        "GET",
        &format!("/v1/payment_links/{gid}/share"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{sh}");
    let text = sh["text"].as_str().unwrap();
    assert!(
        text.contains("₹1250.50")
            && text.contains("Term 2 fee")
            && text.contains(sh["url"].as_str().unwrap()),
        "{text}"
    );
    assert!(
        text.contains("Pay only if you were expecting this"),
        "{text}"
    );
    for hype in ["hurry", "last chance", "limited", "!"] {
        assert!(!text.to_lowercase().contains(hype), "{hype}");
    }
    assert!(
        sh["sms_text"].as_str().unwrap().chars().count() < 300,
        "{}",
        sh["sms_text"]
    );
    assert_eq!(
        (sh["can_sms"].clone(), sh["sms_mode"].clone()),
        (json!(false), json!("off"))
    );

    // sandbox: logged, nothing sent, once a day
    let sandbox = app_with(&t, |s| s.sms = zyvor_api::sms::Mode::Sandbox);
    let (st, r) = raw(
        &sandbox,
        &a.key,
        None,
        "POST",
        &format!("/v1/payment_links/{gid}/send_sms"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        (st.as_u16(), r["mode"].clone()),
        (202, json!("sandbox")),
        "{r}"
    );
    let (st, e) = raw(
        &sandbox,
        &a.key,
        None,
        "POST",
        &format!("/v1/payment_links/{gid}/send_sms"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("already_sent_today"))
    );
    let (st, e) = raw(
        &sandbox,
        &a.key,
        None,
        "POST",
        &format!("/v1/payment_links/{bid}/send_sms"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        (st.as_u16(), e["error"]["code"].clone()),
        (409, json!("no_customer_phone"))
    );
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT status, body FROM sms_queue")
        .fetch_all(&t.root)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "logged");
    assert!(
        rows[0].1.contains("/pay/link/") && rows[0].1.contains("Pay only if you expected this")
    );
    // another business cannot text this link
    let (st, _) = raw(
        &sandbox,
        &t.b.key,
        None,
        "POST",
        &format!("/v1/payment_links/{gid}/send_sms"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(st.as_u16(), 404);
    // canceled links are not shared
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_links/{bid}/cancel"),
        None,
        json!({}),
    )
    .await;
    let (st, _) = raw(
        &sandbox,
        &a.key,
        None,
        "GET",
        &format!("/v1/payment_links/{bid}/share"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(st.as_u16(), 409);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_sms_hook_receives_signed_messages_and_forgets_them_once_sent(pool: PgPool) {
    use std::sync::{Arc, Mutex};
    let t = setup(pool).await;
    let a = &t.a;
    // a stand-in for the business's own SMS relay
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(vec![]));
    let sink = seen.clone();
    let hook = Router::new().route(
        "/sms",
        axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
            let sink = sink.clone();
            async move {
                let sig = headers
                    .get("zyvor-signature")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                sink.lock().unwrap().push((sig, body));
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, hook).await.unwrap() });
    let secret = b"hook-secret-for-tests-only-123".to_vec();
    let mode = zyvor_api::sms::Mode::Hook {
        url: format!("http://{addr}/sms"),
        secret: secret.clone(),
    };
    let mut state = t.state.clone();
    state.sms = mode.clone();
    let mut tx = zyvor_api::tenant::begin(&state.pool, &a.id).await.unwrap();
    let q = zyvor_api::sms::enqueue(
        &mut tx,
        &mode,
        &a.id,
        "+919876543210",
        "Hello from the test",
        "k1",
    )
    .await
    .unwrap();
    assert_eq!(q, Some(zyvor_api::sms::Queued::Queued));
    let dup = zyvor_api::sms::enqueue(
        &mut tx,
        &mode,
        &a.id,
        "+919876543210",
        "Hello from the test",
        "k1",
    )
    .await
    .unwrap();
    assert_eq!(dup, Some(zyvor_api::sms::Queued::Duplicate));
    tx.commit().await.unwrap();
    let client = zyvor_api::outbox::http_client();
    assert_eq!(
        zyvor_api::sms::drain_tenant(&state, &client, &a.id)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        zyvor_api::sms::drain_tenant(&state, &client, &a.id)
            .await
            .unwrap(),
        0
    );
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    let v: Value = serde_json::from_str(&got[0].1).unwrap();
    assert_eq!(
        (v["to"].clone(), v["body"].clone()),
        (json!("+919876543210"), json!("Hello from the test"))
    );
    assert!(zyvor_api::outbox::verify(
        &secret,
        &got[0].0,
        got[0].1.as_bytes(),
        chrono::Utc::now().timestamp(),
        300
    ));
    // once sent, the number and text are blanked
    let row: (String, String, String) =
        sqlx::query_as("SELECT status, to_phone, body FROM sms_queue")
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(row, ("sent".to_string(), String::new(), String::new()));
    // off queues nothing
    let mut tx = zyvor_api::tenant::begin(&state.pool, &a.id).await.unwrap();
    assert_eq!(
        zyvor_api::sms::enqueue(
            &mut tx,
            &zyvor_api::sms::Mode::Off,
            &a.id,
            "+919876543210",
            "x",
            "k2"
        )
        .await
        .unwrap(),
        None
    );
}

// ---------------------------------------------------------------- tenant isolation of the new tables

#[sqlx::test(migrations = "./migrations")]
async fn the_new_tables_are_invisible_to_other_businesses(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let app = app_with(&t, |s| s.sms = zyvor_api::sms::Mode::Sandbox);
    let cust = t
        .ok(
            a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha Rao", "phone": "9876543210" }),
        )
        .await;
    let (st, made) = raw(
        &app,
        &a.key,
        None,
        "POST",
        "/v1/payment_links/bulk",
        None,
        json!({ "csv": format!("{HEAD}Asha Rao,,9876543210,10,,R1,\n") }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{made}");
    let link = t
        .ok(
            a,
            "POST",
            "/v1/payment_links",
            None,
            json!({ "amount_minor": 100, "customer_id": cust["id"] }),
        )
        .await;
    send(
        &app,
        "GET",
        &format!(
            "/v1/checkout/links/{}/{}",
            a.id,
            link["token"].as_str().unwrap()
        ),
        Some(UA_PHONE),
        "203.0.113.5",
        Value::Null,
    )
    .await;
    raw(
        &app,
        &a.key,
        None,
        "POST",
        &format!(
            "/v1/payment_links/{}/send_sms",
            link["id"].as_str().unwrap()
        ),
        None,
        Value::Null,
    )
    .await;
    for table in [
        "payment_link_imports",
        "payment_link_import_rows",
        "link_events_daily",
        "link_visitors_seen",
        "sms_queue",
    ] {
        let mine = t
            .count(&format!(
                "SELECT count(*) FROM {table} WHERE tenant_id = '{}'",
                a.id
            ))
            .await;
        assert!(mine > 0, "{table} has rows for the first business");
        // as the app role with the other business's scope, none are visible; with no scope, none either
        let mut tx = zyvor_api::tenant::begin(&t.state.pool, &b.id)
            .await
            .unwrap();
        let seen: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert_eq!(seen, 0, "{table}");
        let none: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.state.pool)
                .await
                .unwrap();
        assert_eq!(none, 0, "{table} without a scope");
    }
}
