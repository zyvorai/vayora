// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! The hosted seller page: slug rules, public page and images, orders that take stock exactly (including a race),
//! stock holds that lapse, payment through the ordinary intent flow, fulfil and refund, and tenant isolation. All
//! against the simulators; no real provider or money.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";

/// A page for the tenant with a published flag and a few items; returns the slug and the item ids.
async fn page(t: &T, tn: &Tenant, slug: &str) -> Vec<String> {
    t.ok(tn, "PUT", "/v1/storefront", None, json!({ "slug": slug, "title": "Raju Sweets", "description": "Fresh every morning.", "published": true })).await;
    let mut ids = Vec::new();
    for (name, price, stock) in [
        ("Kaju katli 250g", 30_000, json!(5)),
        ("Laddu box", 18_000, json!(null)),
        ("Barfi 500g", 45_000, json!(2)),
    ] {
        let it = t
            .ok(
                tn,
                "POST",
                "/v1/storefront/items",
                None,
                json!({ "name": name, "price_minor": price, "stock": stock }),
            )
            .await;
        ids.push(it["id"].as_str().unwrap().to_string());
    }
    ids
}

fn order_body(items: Value) -> Value {
    json!({ "buyer_name": "Meena Iyer", "buyer_phone": "+91 98765 43210", "buyer_email": "meena@example.com", "delivery_note": "Ring the bell twice", "items": items })
}

async fn place(t: &T, slug: &str, items: Value) -> (StatusCode, Value) {
    t.public(
        "POST",
        &format!("/v1/store/{slug}/orders"),
        None,
        order_body(items),
    )
    .await
}

/// Pay an order's intent with the Razorpay simulator, as a buyer would; the webhook marks it captured.
async fn pay(t: &T, tn: &Tenant, intent_id: &str) {
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(intent_id)
        .execute(&t.root)
        .await
        .unwrap();
    let att = t
        .ok(
            tn,
            "POST",
            &format!("/v1/payment_intents/{intent_id}/attempts"),
            Some(&format!("att-{intent_id}")),
            json!({ "method": "upi" }),
        )
        .await;
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": "captured" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &tn.id, &paid["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
}

async fn stock(t: &T, item: &str) -> Option<i32> {
    sqlx::query_scalar("SELECT stock FROM storefront_items WHERE id = $1")
        .bind(item)
        .fetch_one(&t.root)
        .await
        .unwrap()
}

async fn raw_get(t: &T, path: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = t
        .app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (
        status,
        headers,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn the_page_follows_the_slug_rules_and_stays_hidden_until_published(pool: PgPool) {
    let t = setup(pool).await;
    for (slug, code) in [
        ("admin", "reserved_slug"),
        ("my-razorpay-shop", "reserved_slug"),
        ("ab", "invalid_slug"),
        ("Has-Caps", "invalid_slug"),
        ("a--b", "invalid_slug"),
        ("12345", "invalid_slug"),
    ] {
        let (s, e) = t
            .call(
                &t.a,
                "PUT",
                "/v1/storefront",
                None,
                json!({ "slug": slug, "title": "x" }),
            )
            .await;
        assert_eq!(
            (s, e["error"]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some(code)),
            "{slug}"
        );
    }
    let (s, e) = t
        .call(
            &t.a,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "title": "no slug" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("slug_required"))
    );
    let (s, e) = t
        .call(
            &t.a,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "slug": "raju-sweets", "title": "x\u{202E}y" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_text"))
    );

    let made = t
        .ok(
            &t.a,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "slug": "raju-sweets", "title": "Raju <b>Sweets</b>" }),
        )
        .await;
    assert_eq!(
        (made["published"].as_bool(), made["noindex"].as_bool()),
        (Some(false), Some(true)),
        "hidden and noindex by default"
    );
    // another business cannot take the slug; the owner cannot change it
    let (s, e) = t
        .call(
            &t.b,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "slug": "raju-sweets", "title": "Copycat" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("slug_taken"))
    );
    let (s, e) = t
        .call(
            &t.a,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "slug": "raju-sweets-2", "title": "x" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("slug_locked"))
    );

    let (s, _) = t
        .public("GET", "/v1/store/raju-sweets", None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "unpublished");
    t.ok(
        &t.a,
        "PUT",
        "/v1/storefront",
        None,
        json!({ "title": "Raju <b>Sweets</b>", "published": true, "noindex": false }),
    )
    .await;
    let view = t
        .ok_public("GET", "/v1/store/raju-sweets", Value::Null)
        .await;
    // markup is data: it comes back as the text typed, to be escaped by whatever shows it
    assert_eq!(
        (view["title"].as_str(), view["noindex"].as_bool()),
        (Some("Raju <b>Sweets</b>"), Some(false))
    );
    let (s, _) = t
        .public("GET", "/v1/store/nobody-here", None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn items_and_images_are_validated_and_images_are_served_safely(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    for (bad, code) in [
        (json!({ "name": "", "price_minor": 100 }), "invalid_text"),
        (json!({ "name": "x", "price_minor": 0 }), "invalid_price"),
        (
            json!({ "name": "x", "price_minor": 100, "stock": -1 }),
            "invalid_stock",
        ),
        (
            json!({ "name": "x".repeat(81), "price_minor": 100 }),
            "text_too_long",
        ),
        (
            json!({ "name": "x", "price_minor": 100, "image_id": "sfi_nope" }),
            "not_found",
        ),
    ] {
        let (s, e) = t
            .call(&t.a, "POST", "/v1/storefront/items", None, bad.clone())
            .await;
        assert_eq!(
            (s.is_client_error(), e["error"]["code"].as_str()),
            (true, Some(code)),
            "{bad}"
        );
    }
    // images: sniffed by their bytes, SVG and oversize refused
    let up = |bytes: Vec<u8>| {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/storefront/images")
            .header("authorization", format!("Bearer {}", t.a.key))
            .body(Body::from(bytes))
            .unwrap();
        let app = t.app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let s = res.status();
            (
                s,
                serde_json::from_slice::<Value>(
                    &res.into_body().collect().await.unwrap().to_bytes(),
                )
                .unwrap_or(Value::Null),
            )
        }
    };
    assert_eq!(
        up(b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec())
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        up(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
            .into_iter()
            .chain(vec![0u8; 201 * 1024])
            .collect())
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, img) = up(PNG.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED);
    let image_id = img["id"].as_str().unwrap();
    t.ok(
        &t.a,
        "PATCH",
        &format!("/v1/storefront/items/{}", ids[0]),
        None,
        json!({ "image_id": image_id }),
    )
    .await;

    let view = t
        .ok_public("GET", "/v1/store/raju-sweets", Value::Null)
        .await;
    let url = view["items"][0]["image_url"].as_str().unwrap().to_string();
    assert_eq!(url, format!("/api/store/raju-sweets/images/{image_id}"));
    let (s, h, body) = raw_get(&t, &format!("/v1/store/raju-sweets/images/{image_id}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "image/png");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(
        h["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    assert_eq!(body, PNG);
    // another business's image cannot be attached to this page's item, nor fetched through this page
    let (s, e) = t
        .call(
            &t.b,
            "PATCH",
            &format!("/v1/storefront/items/{}", ids[0]),
            None,
            json!({ "name": "hijack" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("not_found"))
    );
    t.ok(
        &t.b,
        "PUT",
        "/v1/storefront",
        None,
        json!({ "slug": "other-shop", "title": "Other", "published": true }),
    )
    .await;
    let (s, _, _) = raw_get(&t, &format!("/v1/store/other-shop/images/{image_id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, e) = t
        .call(
            &t.b,
            "POST",
            "/v1/storefront/items",
            None,
            json!({ "name": "x", "price_minor": 100, "image_id": image_id }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::NOT_FOUND, Some("not_found"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_order_is_priced_by_the_page_paid_fulfilled_and_refunded(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let (s, o) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[0], "quantity": 2 }, { "item_id": ids[1], "quantity": 1 }]),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{o}");
    // 2 x 30000 + 18000, worked out from the page's own prices
    assert_eq!(o["amount_minor"], 78_000);
    assert_eq!(
        o["path"],
        format!("/pay/{}/{}", t.a.id, o["intent_id"].as_str().unwrap())
    );
    assert!(o["client_secret"].as_str().unwrap().starts_with("pis_"));
    assert_eq!(stock(&t, &ids[0]).await, Some(3), "stock is taken at once");
    assert_eq!(
        stock(&t, &ids[1]).await,
        None,
        "an uncounted item stays uncounted"
    );
    let (oid, iid) = (
        o["order_id"].as_str().unwrap().to_string(),
        o["intent_id"].as_str().unwrap().to_string(),
    );

    // the buyer's checkout session works with the secret
    let sess = t
        .ok_public(
            "GET",
            &format!(
                "/v1/checkout/{}/{iid}?secret={}",
                t.a.id,
                o["client_secret"].as_str().unwrap()
            ),
            Value::Null,
        )
        .await;
    assert_eq!(sess["amount_minor"], 78_000);

    let listed = t
        .ok(&t.a, "GET", "/v1/storefront/orders", None, Value::Null)
        .await;
    assert_eq!(listed["data"][0]["status"], "pending");
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/fulfill"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_paid"))
    );
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/refund"),
            Some("r-0"),
            json!({}),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_paid"))
    );

    pay(&t, &t.a, &iid).await;
    let order = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/storefront/orders/{oid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(order["status"], "paid");
    assert_eq!(order["buyer"]["phone"], "+919876543210");
    assert_eq!(order["delivery_note"], "Ring the bell twice");
    assert_eq!(order["items"].as_array().unwrap().len(), 2);
    let done = t
        .ok(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/fulfill"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(done["status"], "fulfilled");

    // a full refund through the ordinary flow, putting the stock back and cancelling the order
    let (s, r) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/refund"),
            Some("r-1"),
            json!({ "reason": "damaged", "restock": true }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(r["amount_minor"], 78_000);
    let order = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/storefront/orders/{oid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (order["status"].as_str(), order["refunded_minor"].as_i64()),
        (Some("cancelled"), Some(78_000))
    );
    assert_eq!(stock(&t, &ids[0]).await, Some(5), "restocked once");
    // repeating the request neither refunds nor restocks again
    let (s, _) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/refund"),
            Some("r-1"),
            json!({ "reason": "damaged", "restock": true }),
        )
        .await;
    assert!(s.is_success() || s == StatusCode::CONFLICT);
    assert_eq!(stock(&t, &ids[0]).await, Some(5));
    // buyer details and amounts never reach the audit trail
    let leaks = t.count("SELECT count(*) FROM audit_events WHERE detail::text ILIKE '%meena%' OR detail::text ILIKE '%9876543210%' OR detail::text ILIKE '%bell%'").await;
    assert_eq!(leaks, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn bad_orders_are_refused_before_anything_is_taken(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let theirs = t
        .ok(
            &t.b,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "slug": "other-shop", "title": "Other", "published": true }),
        )
        .await;
    assert_eq!(theirs["published"], true);
    let b_item = t
        .ok(
            &t.b,
            "POST",
            "/v1/storefront/items",
            None,
            json!({ "name": "B thing", "price_minor": 100, "stock": 9 }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(
        &t.a,
        "PATCH",
        &format!("/v1/storefront/items/{}", ids[1]),
        None,
        json!({ "active": false }),
    )
    .await;

    let one = json!([{ "item_id": ids[0], "quantity": 1 }]);
    let mut cases: Vec<(Value, &str)> = vec![
        (
            json!({ "buyer_name": "", "buyer_phone": "9876543210", "items": one }),
            "invalid_text",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "12", "items": one }),
            "invalid_phone",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "buyer_email": "not-an-email", "items": one }),
            "invalid_email",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "delivery_note": "x".repeat(301), "items": one }),
            "text_too_long",
        ),
        (
            json!({ "buyer_name": "Mee\u{0}na", "buyer_phone": "9876543210", "items": one }),
            "invalid_text",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [] }),
            "invalid_items",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": ids[0], "quantity": 0 }] }),
            "invalid_items",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": ids[0], "quantity": 21 }] }),
            "invalid_items",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": ids[0], "quantity": 15 }, { "item_id": ids[0], "quantity": 15 }] }),
            "invalid_items",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": ids[1], "quantity": 1 }] }),
            "item_unavailable",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": b_item, "quantity": 1 }] }),
            "item_unavailable",
        ),
        (
            json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": [{ "item_id": ids[2], "quantity": 3 }] }),
            "out_of_stock",
        ),
    ];
    let many: Vec<Value> = (0..21)
        .map(|_| json!({ "item_id": ids[0], "quantity": 1 }))
        .collect();
    cases.push((
        json!({ "buyer_name": "Meena", "buyer_phone": "9876543210", "items": many }),
        "invalid_items",
    ));
    for (body, code) in cases {
        let (s, e) = t
            .public("POST", "/v1/store/raju-sweets/orders", None, body.clone())
            .await;
        assert_eq!(e["error"]["code"].as_str(), Some(code), "{body}: {s}");
        assert!(s.is_client_error());
    }
    // a good line followed by a bad one takes nothing at all
    let (s, _) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[0], "quantity": 2 }, { "item_id": ids[2], "quantity": 3 }]),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        (stock(&t, &ids[0]).await, stock(&t, &ids[2]).await),
        (Some(5), Some(2))
    );
    assert_eq!(t.count("SELECT count(*) FROM storefront_orders").await, 0);
    assert_eq!(t.count("SELECT count(*) FROM payment_intents").await, 0);
    // an unpublished page takes no orders
    t.ok(
        &t.a,
        "PUT",
        "/v1/storefront",
        None,
        json!({ "title": "Raju", "published": false }),
    )
    .await;
    assert_eq!(place(&t, "raju-sweets", one).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_orders_never_sell_more_than_the_stock(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    // 5 kaju katli and 2 barfi in stock; 12 visitors race for one of each, and 6 more for barfi alone
    let mut tasks = Vec::new();
    for i in 0..18 {
        let app = t.app.clone();
        let items = if i < 12 {
            json!([{ "item_id": ids[0], "quantity": 1 }, { "item_id": ids[2], "quantity": 1 }])
        } else {
            json!([{ "item_id": ids[2], "quantity": 1 }, { "item_id": ids[0], "quantity": 1 }])
        };
        tasks.push(tokio::spawn(async move {
            raw(
                &app,
                "",
                None,
                "POST",
                "/v1/store/raju-sweets/orders",
                None,
                order_body(items),
            )
            .await
        }));
    }
    let results: Vec<(StatusCode, Value)> = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    let made = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::CREATED)
        .count();
    let refused: Vec<_> = results
        .iter()
        .filter(|(s, _)| *s != StatusCode::CREATED)
        .collect();
    assert_eq!(made, 2, "only two barfi existed: {results:?}");
    assert!(
        refused
            .iter()
            .all(|(s, e)| *s == StatusCode::CONFLICT && e["error"]["code"] == "out_of_stock"),
        "{refused:?}"
    );
    assert_eq!(
        (stock(&t, &ids[0]).await, stock(&t, &ids[2]).await),
        (Some(3), Some(0))
    );
    let sold: i64 = t
        .count("SELECT COALESCE(SUM(quantity), 0)::BIGINT FROM storefront_order_items")
        .await;
    assert_eq!(sold, 4, "two orders of one katli and one barfi each");

    // and a single counted item, raced on its own, ends at exactly zero with no more orders than stock
    let item = t
        .ok(
            &t.a,
            "POST",
            "/v1/storefront/items",
            None,
            json!({ "name": "Last box", "price_minor": 5_000, "stock": 5 }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let (app, item) = (t.app.clone(), item.clone());
        tasks.push(tokio::spawn(async move {
            raw(
                &app,
                "",
                None,
                "POST",
                "/v1/store/raju-sweets/orders",
                None,
                order_body(json!([{ "item_id": item, "quantity": 1 }])),
            )
            .await
            .0
        }));
    }
    let codes: Vec<StatusCode> = futures_util::future::join_all(tasks)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        codes.iter().filter(|c| **c == StatusCode::CREATED).count(),
        5
    );
    assert_eq!(
        codes.iter().filter(|c| **c == StatusCode::CONFLICT).count(),
        7
    );
    assert_eq!(stock(&t, &item).await, Some(0));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_lapsed_hold_gives_the_stock_back_and_closes_the_intent_unless_a_payment_is_in_flight(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let lapse = |t: &T| {
        let root = t.root.clone();
        async move {
            sqlx::query("UPDATE storefront_orders SET hold_until = now() - interval '1 minute' WHERE status = 'pending'").execute(&root).await.unwrap();
        }
    };
    // an abandoned order
    let (_, abandoned) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[2], "quantity": 2 }]),
    )
    .await;
    assert_eq!(stock(&t, &ids[2]).await, Some(0));
    let (s, e) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[2], "quantity": 1 }]),
    )
    .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("out_of_stock"))
    );
    // an order whose buyer has started paying (the outcome is not known yet) keeps its stock
    let (_, paying) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[0], "quantity": 1 }]),
    )
    .await;
    let piid = paying["intent_id"].as_str().unwrap();
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(piid)
        .execute(&t.root)
        .await
        .unwrap();
    t.ok(
        &t.a,
        "POST",
        &format!("/v1/payment_intents/{piid}/attempts"),
        Some("att-flight"),
        json!({ "method": "upi" }),
    )
    .await;
    lapse(&t).await;
    // the next visit to the page gives back what can be given back
    let view = t
        .ok_public("GET", "/v1/store/raju-sweets", Value::Null)
        .await;
    assert_eq!(stock(&t, &ids[2]).await, Some(2));
    assert_eq!(view["items"][2]["available"], 2);
    assert_eq!(
        stock(&t, &ids[0]).await,
        Some(4),
        "in-flight payment keeps its stock"
    );
    let aid = abandoned["order_id"].as_str().unwrap();
    let o = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/storefront/orders/{aid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (o["status"].as_str(), o["stock_held"].as_bool()),
        (Some("expired"), Some(false))
    );
    // nobody can pay for stock that went back
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!(
                "/v1/payment_intents/{}/attempts",
                abandoned["intent_id"].as_str().unwrap()
            ),
            Some("att-late"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("intent_not_payable"))
    );
    // the worker's sweep does the same for a quiet page, and never twice
    sqlx::query("UPDATE storefront_orders SET hold_until = now() - interval '1 minute' WHERE status = 'pending'").execute(&t.root).await.unwrap();
    let (_, quiet) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[2], "quantity": 1 }]),
    )
    .await;
    lapse(&t).await;
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, &t.a.id)
        .await
        .unwrap();
    let n = zyvor_api::storefront::release_lapsed(&mut tx, None)
        .await
        .unwrap();
    let again = zyvor_api::storefront::release_lapsed(&mut tx, None)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!((n >= 1, again), (true, 0));
    assert_eq!(stock(&t, &ids[2]).await, Some(2));
    let q = t
        .ok(
            &t.a,
            "GET",
            &format!(
                "/v1/storefront/orders/{}",
                quiet["order_id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(q["status"], "expired");
}

#[sqlx::test(migrations = "./migrations")]
async fn money_that_arrives_after_the_stock_went_back_still_wins(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    // two orders whose stock was given back (made to look so directly: the API closes the intent first)
    let mut made = Vec::new();
    for _ in 0..2 {
        let (_, o) = place(
            &t,
            "raju-sweets",
            json!([{ "item_id": ids[2], "quantity": 1 }]),
        )
        .await;
        made.push(o);
    }
    sqlx::query("UPDATE storefront_orders SET status = 'expired', stock_held = false")
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE storefront_items SET stock = 1 WHERE id = $1")
        .bind(&ids[2])
        .execute(&t.root)
        .await
        .unwrap();
    pay(&t, &t.a, made[0]["intent_id"].as_str().unwrap()).await;
    let first = t
        .ok(
            &t.a,
            "GET",
            &format!(
                "/v1/storefront/orders/{}",
                made[0]["order_id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            first["status"].as_str(),
            first["needs_attention"].as_bool(),
            first["stock_held"].as_bool()
        ),
        (Some("paid"), Some(false), Some(true))
    );
    assert_eq!(
        stock(&t, &ids[2]).await,
        Some(0),
        "the stock was taken again"
    );
    pay(&t, &t.a, made[1]["intent_id"].as_str().unwrap()).await;
    let second = t
        .ok(
            &t.a,
            "GET",
            &format!(
                "/v1/storefront/orders/{}",
                made[1]["order_id"].as_str().unwrap()
            ),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (
            second["status"].as_str(),
            second["needs_attention"].as_bool()
        ),
        (Some("paid"), Some(true)),
        "paid, but nothing left to ship: flagged"
    );
    assert_eq!(stock(&t, &ids[2]).await, Some(0), "never below zero");
}

#[sqlx::test(migrations = "./migrations")]
async fn cancelling_an_unpaid_order_returns_the_stock_and_ends_the_payment(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let (_, o) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[0], "quantity": 4 }]),
    )
    .await;
    let oid = o["order_id"].as_str().unwrap();
    assert_eq!(stock(&t, &ids[0]).await, Some(1));
    let c = t
        .ok(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/cancel"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(c["status"], "cancelled");
    assert_eq!(stock(&t, &ids[0]).await, Some(5));
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!(
                "/v1/payment_intents/{}/attempts",
                o["intent_id"].as_str().unwrap()
            ),
            Some("att-x"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("intent_not_payable"))
    );
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/storefront/orders/{oid}/cancel"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("not_pending"))
    );
    assert_eq!(stock(&t, &ids[0]).await, Some(5), "given back once");
}

#[sqlx::test(migrations = "./migrations")]
async fn stock_can_be_adjusted_safely_and_never_goes_below_zero(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let path = format!("/v1/storefront/items/{}", ids[0]);
    let p = |body: Value| t.call(&t.a, "PATCH", &path, None, body);
    assert_eq!(p(json!({ "stock_delta": 7 })).await.1["stock"], 12);
    let (s, e) = p(json!({ "stock_delta": -13 })).await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("stock_below_zero"))
    );
    assert_eq!(p(json!({ "stock": 3 })).await.1["stock"], 3);
    assert_eq!(p(json!({ "stock": null })).await.1["stock"], Value::Null);
    assert_eq!(
        p(json!({ "stock": 4, "stock_delta": 1 })).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn orders_are_rate_limited_per_visitor_address(pool: PgPool) {
    let t = setup(pool).await;
    page(&t, &t.a, "raju-sweets").await;
    let mut st = t.state.clone();
    st.trusted_hops = 1;
    let app = zyvor_api::app(st);
    let send = |ip: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .method("POST")
                .uri("/v1/store/raju-sweets/orders")
                .header("content-type", "application/json")
                .header("x-forwarded-for", ip)
                .body(Body::from(order_body(json!([])).to_string()))
                .unwrap();
            app.oneshot(req).await.unwrap().status()
        }
    };
    // the first ten (each refused for its empty basket) use the visitor's allowance; the next is turned away
    for _ in 0..10 {
        assert_eq!(send("203.0.113.9").await, StatusCode::UNPROCESSABLE_ENTITY);
    }
    assert_eq!(send("203.0.113.9").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        send("203.0.113.10").await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "another visitor is unaffected"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tenants_see_only_their_own_storefront_data_and_scopes_are_enforced(pool: PgPool) {
    let t = setup(pool).await;
    let ids = page(&t, &t.a, "raju-sweets").await;
    let (_, o) = place(
        &t,
        "raju-sweets",
        json!([{ "item_id": ids[0], "quantity": 1 }]),
    )
    .await;
    let oid = o["order_id"].as_str().unwrap();
    pay(&t, &t.a, o["intent_id"].as_str().unwrap()).await;

    // B has a page of its own and sees none of A's rows
    t.ok(
        &t.b,
        "PUT",
        "/v1/storefront",
        None,
        json!({ "slug": "other-shop", "title": "Other" }),
    )
    .await;
    assert_eq!(
        t.ok(&t.b, "GET", "/v1/storefront/orders", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.ok(&t.b, "GET", "/v1/storefront/items", None, Value::Null)
            .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    for (m, p) in [
        ("GET", format!("/v1/storefront/orders/{oid}")),
        ("POST", format!("/v1/storefront/orders/{oid}/fulfill")),
        ("POST", format!("/v1/storefront/orders/{oid}/cancel")),
        ("PATCH", format!("/v1/storefront/items/{}", ids[0])),
    ] {
        let (s, _) = t.call(&t.b, m, &p, Some("k"), json!({})).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {p}");
    }
    let (s, _) = t
        .call(
            &t.b,
            "POST",
            &format!("/v1/storefront/orders/{oid}/refund"),
            Some("k2"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // with no tenant in scope, row-level security hides every row of every new table
    for table in [
        "storefronts",
        "storefront_images",
        "storefront_items",
        "storefront_orders",
        "storefront_order_items",
        "dues_groups",
        "dues_heads",
        "dues_members",
        "dues_assignments",
        "dues_assignment_invoices",
        "vertical_settings",
        "donation_campaigns",
        "donations",
        "donation_pledges",
    ] {
        let n: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&t.state.pool)
                .await
                .unwrap();
        assert_eq!(n, 0, "{table} is visible without a tenant");
    }
    let mine: i64 = {
        let mut tx = zyvor_api::tenant::begin(&t.state.pool, &t.a.id)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT count(*) FROM storefront_orders")
            .fetch_one(&mut *tx)
            .await
            .unwrap()
    };
    assert_eq!(mine, 1);

    // a key without the storefront scopes cannot touch the seller side; one with read only cannot write
    let ro = t
        .ok(
            &t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "reader", "scopes": ["storefront:read"] }),
        )
        .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    let pr = t
        .ok(
            &t.a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "payments", "scopes": ["payments:read"] }),
        )
        .await["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "GET",
            "/v1/storefront/orders",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "POST",
            &format!("/v1/storefront/orders/{oid}/fulfill"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        raw(
            &t.app,
            &ro,
            None,
            "PUT",
            "/v1/storefront",
            None,
            json!({ "title": "x" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        raw(
            &t.app,
            &pr,
            None,
            "GET",
            "/v1/storefront/orders",
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
