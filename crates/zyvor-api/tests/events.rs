//! Events and ticketing, against the simulators only: seats never oversold, tickets only after a verified capture,
//! hold expiry, check-in, cancellation with bulk refunds, and tenant isolation.

mod common;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_api::{tenant, ticketing};

fn in_hours(h: i64) -> String {
    (Utc::now() + Duration::hours(h)).to_rfc3339()
}

/// A published event with a regular tier and (optionally) a registration form. Returns (event id, tier id).
async fn event(
    t: &T,
    tn: &Tenant,
    capacity: i32,
    tier_qty: i32,
    price: i64,
    extra: Value,
) -> (String, String) {
    let mut body = json!({
        "name": "Rust Meetup", "description": "Talks and snacks", "venue": "Hall 1",
        "starts_at": in_hours(48), "ends_at": in_hours(52), "capacity": capacity, "hold_minutes": 10,
        "tiers": [{ "name": "General", "price_minor": price, "quantity": tier_qty, "per_order_max": 10 }],
        "form_fields": [
            { "key": "company", "label": "Company", "kind": "text", "required": true },
            { "key": "terms", "label": "I agree", "kind": "checkbox", "required": true }
        ],
    });
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        body[k] = v;
    }
    let e = t.ok(tn, "POST", "/v1/events", None, body).await;
    let id = e["id"].as_str().unwrap().to_string();
    let tier = e["tiers"][0]["id"].as_str().unwrap().to_string();
    t.ok(
        tn,
        "POST",
        &format!("/v1/events/{id}/status"),
        None,
        json!({ "status": "on_sale" }),
    )
    .await;
    (id, tier)
}

fn order_body(tier: &str, qty: i32, email: &str) -> Value {
    json!({
        "buyer": { "name": "Asha Rao", "email": email, "phone": "+91 98765 43210" },
        "lines": [{ "tier_id": tier, "quantity": qty }],
        "answers": { "company": "Acme", "terms": true },
    })
}

async fn place(t: &T, tn: &Tenant, ev: &str, body: Value) -> (StatusCode, Value) {
    t.public(
        "POST",
        &format!("/v1/public/events/{}/{ev}/orders", tn.id),
        None,
        body,
    )
    .await
}

async fn place_ok(t: &T, tn: &Tenant, ev: &str, body: Value) -> Value {
    let (s, b) = place(t, tn, ev, body).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b
}

/// The buyer pays the order's intent in the Razorpay simulator and the webhook arrives. Returns the attempt id.
async fn pay(t: &T, tn: &Tenant, order: &Value) -> String {
    let iid = order["payment"]["intent_id"].as_str().unwrap();
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(iid)
        .execute(&t.root)
        .await
        .unwrap();
    let att = t
        .ok(
            tn,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some(&format!("att-{iid}")),
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
    att["id"].as_str().unwrap().to_string()
}

/// Start the payment but hold back the provider's webhook (the buyer is mid-approval). Returns (attempt id, webhook).
async fn start_unpaid(t: &T, tn: &Tenant, order: &Value, outcome: &str) -> (String, Value) {
    let iid = order["payment"]["intent_id"].as_str().unwrap();
    sqlx::query("UPDATE payment_intents SET allowed_providers = ARRAY['razorpay'] WHERE id = $1")
        .bind(iid)
        .execute(&t.root)
        .await
        .unwrap();
    let att = t
        .ok(
            tn,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some(&format!("att-{iid}")),
            json!({ "method": "upi" }),
        )
        .await;
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": att["provider_order_ref"], "outcome": outcome }),
        )
        .await;
    (
        att["id"].as_str().unwrap().to_string(),
        paid["webhook"].clone(),
    )
}

async fn order_page(t: &T, tn: &Tenant, order: &Value) -> Value {
    let id = order["order_id"].as_str().unwrap();
    let tok = order["order_token"].as_str().unwrap();
    t.ok_public(
        "GET",
        &format!("/v1/public/events/{}/orders/{id}?t={tok}", tn.id),
        Value::Null,
    )
    .await
}

/// The seat counters must always equal what the orders and tickets say.
async fn assert_counters(t: &T, ev: &str) {
    let (held, sold, cap): (i32, i32, i32) =
        sqlx::query_as("SELECT seats_held, seats_sold, capacity FROM ticket_events WHERE id = $1")
            .bind(ev)
            .fetch_one(&t.root)
            .await
            .unwrap();
    let pending: i64 = t.count(&format!("SELECT COALESCE(sum(quantity), 0)::BIGINT FROM event_orders WHERE event_id = '{ev}' AND status = 'pending'")).await;
    let live: i64 = t.count(&format!("SELECT count(*) FROM tickets WHERE event_id = '{ev}' AND status IN ('issued', 'checked_in')")).await;
    assert_eq!(
        held as i64, pending,
        "held seats equal the quantities of unpaid orders"
    );
    assert_eq!(sold as i64, live, "sold seats equal the live tickets");
    assert!(held + sold <= cap, "never more than capacity");
    let bad: i64 = t.count(&format!("SELECT count(*) FROM ticket_tiers tt WHERE event_id = '{ev}' AND (tt.seats_held <> COALESCE((SELECT sum(l.quantity) FROM event_order_lines l JOIN event_orders o ON o.id = l.order_id WHERE l.tier_id = tt.id AND o.status = 'pending'), 0) OR tt.seats_sold <> (SELECT count(*) FROM tickets k WHERE k.tier_id = tt.id AND k.status IN ('issued', 'checked_in')))")).await;
    assert_eq!(bad, 0, "tier counters match too");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_ticket_is_issued_only_after_the_capture_and_only_once(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "POST",
        "/v1/coupons",
        None,
        json!({ "code": "EARLY10", "kind": "percent", "value": 1000, "max_uses": 1 }),
    )
    .await;
    let (ev, tier) = event(&t, a, 100, 50, 100_000, json!({})).await;

    // the public page shows the tier and a draft would not exist
    let page = t
        .ok_public(
            "GET",
            &format!("/v1/public/events/{}/{ev}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(page["tiers"][0]["price_minor"], 100_000);
    assert_eq!(page["tiers"][0]["can_buy"], true);
    assert!(page["online_url"].is_null());

    // an order holds seats and creates a payment intent; no ticket exists yet
    let mut body = order_body(&tier, 2, "asha@example.com");
    body["coupon"] = json!("early10");
    let o = place_ok(&t, a, &ev, body).await;
    assert_eq!(o["status"], "pending");
    assert_eq!(o["subtotal_minor"], 200_000);
    assert_eq!(o["discount_minor"], 20_000);
    assert_eq!(o["total_minor"], 180_000);
    assert_eq!(t.count("SELECT count(*) FROM tickets").await, 0);
    assert_eq!(
        order_page(&t, a, &o).await["tickets"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_counters(&t, &ev).await;
    let iid = o["payment"]["intent_id"].as_str().unwrap().to_string();
    let amount: i64 = sqlx::query_scalar("SELECT amount_minor FROM payment_intents WHERE id = $1")
        .bind(&iid)
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(amount, 180_000, "the intent is for the discounted total");
    // the coupon is spent and cannot be used again
    let (s, b) = place(&t, a, &ev, {
        let mut b = order_body(&tier, 1, "other@example.com");
        b["coupon"] = json!("EARLY10");
        b
    })
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["error"]["code"], "coupon_invalid");

    // a capture arrives: two tickets
    let aid = pay(&t, a, &o).await;
    let page = order_page(&t, a, &o).await;
    assert_eq!(page["status"], "paid");
    let tickets = page["tickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 2);
    for k in tickets {
        assert!(k["qr_payload"].as_str().unwrap().starts_with("ZT1."));
    }
    assert_counters(&t, &ev).await;

    // the same webhook again, a refresh and a direct second call: still two tickets
    let refreshed = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_attempts/{aid}/refresh"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(refreshed["status"], "captured");
    for _ in 0..2 {
        let mut tx = tenant::begin(&t.state.pool, &a.id).await.unwrap();
        ticketing::on_payment_captured(&mut tx, &a.id, &iid)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    assert_eq!(t.count("SELECT count(*) FROM tickets").await, 2);
    assert_eq!(t.count("SELECT count(DISTINCT code) FROM tickets").await, 2);
    assert_counters(&t, &ev).await;
    // the merchant was told once
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'ticket_order.paid'")
            .await,
        1
    );

    // the buyer's page needs the token
    let id = o["order_id"].as_str().unwrap();
    let (s, _) = t
        .public(
            "GET",
            &format!("/v1/public/events/{}/orders/{id}?t=eot_wrong", a.id),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // a QR image for a live ticket, none for a made-up one
    let tid = tickets[0]["id"].as_str().unwrap();
    let tok = o["order_token"].as_str().unwrap();
    let svg = t
        .text(&format!(
            "/v1/public/events/{}/orders/{id}/tickets/{tid}/qr.svg?t={tok}",
            a.id
        ))
        .await;
    assert!(svg.contains("<svg"));
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_buyers_cannot_oversell_the_event_or_a_tier(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // 5 seats in the event, but the tier alone would allow 8
    let (ev, tier) = event(&t, a, 5, 8, 50_000, json!({})).await;
    let mut tasks = Vec::new();
    for i in 0..24 {
        let app = t.app.clone();
        let path = format!("/v1/public/events/{}/{ev}/orders", a.id);
        let body = order_body(
            &tier,
            if i % 3 == 0 { 2 } else { 1 },
            &format!("buyer{i}@example.com"),
        );
        tasks.push(tokio::spawn(async move {
            raw(&app, "", None, "POST", &path, None, body).await
        }));
    }
    let (mut won, mut seats) = (0, 0);
    for task in tasks {
        let (s, b) = task.await.unwrap();
        match s {
            StatusCode::CREATED => {
                won += 1;
                seats += b["quantity"].as_i64().unwrap();
            }
            StatusCode::CONFLICT => assert_eq!(b["error"]["code"], "sold_out", "{b}"),
            other => panic!("unexpected {other}: {b}"),
        }
    }
    assert!(won >= 3, "some orders must succeed");
    assert!(seats <= 5, "{seats} seats granted for a capacity of 5");
    assert_eq!(
        seats, 5,
        "and the event fills up: nothing is left unsold while buyers were waiting"
    );
    assert_counters(&t, &ev).await;
    // the database itself refuses an oversell
    let r = sqlx::query("UPDATE ticket_events SET seats_held = seats_held + 1 WHERE id = $1")
        .bind(&ev)
        .execute(&t.root)
        .await;
    assert!(r.is_err(), "capacity check constraint");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_tier_cap_holds_even_when_the_event_has_room(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 100, 3, 50_000, json!({})).await;
    place_ok(&t, a, &ev, order_body(&tier, 2, "a@example.com")).await;
    let (s, b) = place(&t, a, &ev, order_body(&tier, 2, "b@example.com")).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("sold_out")),
        "{b}"
    );
    place_ok(&t, a, &ev, order_body(&tier, 1, "c@example.com")).await;
    // per-order maximum
    let (s, b) = place(&t, a, &ev, order_body(&tier, 11, "d@example.com")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_counters(&t, &ev).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unpaid_order_releases_its_seats_when_the_hold_runs_out(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    t.ok(
        a,
        "POST",
        "/v1/coupons",
        None,
        json!({ "code": "ONCE", "kind": "flat", "value": 10_000, "max_uses": 1 }),
    )
    .await;
    let (ev, tier) = event(&t, a, 2, 2, 100_000, json!({})).await;
    let mut body = order_body(&tier, 2, "a@example.com");
    body["coupon"] = json!("ONCE");
    let o = place_ok(&t, a, &ev, body).await;
    let (s, _) = place(&t, a, &ev, order_body(&tier, 1, "b@example.com")).await;
    assert_eq!(s, StatusCode::CONFLICT, "the seats are held");
    assert_eq!(
        t.count("SELECT uses::BIGINT FROM coupons WHERE code = 'ONCE'")
            .await,
        1
    );

    // not yet due: nothing happens
    let st = zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let _ = st;
    assert_eq!(
        t.count(&format!(
            "SELECT seats_held::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        2
    );

    // the hold runs out; the worker releases it
    sqlx::query("UPDATE event_orders SET hold_expires_at = now() - interval '1 minute'")
        .execute(&t.root)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(order_page(&t, a, &o).await["status"], "expired");
    assert_eq!(
        t.count(&format!(
            "SELECT seats_held::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        0
    );
    assert_eq!(
        t.count("SELECT uses::BIGINT FROM coupons WHERE code = 'ONCE'")
            .await,
        0,
        "the coupon use is given back"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'ticket_order.expired'")
            .await,
        1
    );
    assert_counters(&t, &ev).await;
    // the seats can be bought again, and the expired order's payment can no longer start
    place_ok(&t, a, &ev, order_body(&tier, 2, "c@example.com")).await;
    let iid = o["payment"]["intent_id"].as_str().unwrap();
    let (s, b) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("late"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["error"]["code"], "intent_not_payable");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_payment_gives_no_tickets_and_its_seats_come_back(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 3, 3, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 3, "a@example.com")).await;
    let (aid, hook) = start_unpaid(&t, a, &o, "failed").await;
    let (s, _) = t.deliver("razorpay", &a.id, &hook).await;
    assert_eq!(s, StatusCode::OK);
    let att = t
        .ok(
            a,
            "GET",
            &format!("/v1/payment_attempts/{aid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(att["status"], "failed");
    assert_eq!(t.count("SELECT count(*) FROM tickets").await, 0);
    // the buyer may retry inside the hold, so the seats stay held until it runs out
    assert_eq!(
        t.count(&format!(
            "SELECT seats_held::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        3
    );
    sqlx::query("UPDATE event_orders SET hold_expires_at = now() - interval '1 second'")
        .execute(&t.root)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(
        t.count(&format!(
            "SELECT seats_held::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        0
    );
    assert_counters(&t, &ev).await;
    // the buyer can also cancel an unpaid order at once
    let o2 = place_ok(&t, a, &ev, order_body(&tier, 1, "b@example.com")).await;
    let (id, tok) = (
        o2["order_id"].as_str().unwrap(),
        o2["order_token"].as_str().unwrap(),
    );
    let c = t
        .ok_public(
            "POST",
            &format!("/v1/public/events/{}/orders/{id}/cancel?t={tok}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(c["status"], "cancelled");
    assert_counters(&t, &ev).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payment_that_arrives_after_the_hold_takes_free_seats_or_is_refunded(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 2, 2, 100_000, json!({})).await;

    // (1) the seats are still free when the late payment lands: it takes them back
    let o1 = place_ok(&t, a, &ev, order_body(&tier, 2, "late@example.com")).await;
    let (aid1, hook1) = start_unpaid(&t, a, &o1, "captured").await;
    // a live attempt keeps the hold for a while
    sqlx::query("UPDATE event_orders SET hold_expires_at = now() - interval '1 minute'")
        .execute(&t.root)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(
        order_page(&t, a, &o1).await["status"],
        "pending",
        "a payment in flight keeps its seats"
    );
    // once the attempt has been quiet for a while, the hold is released
    sqlx::query(
        "UPDATE payment_attempts SET updated_at = now() - interval '10 minutes' WHERE id = $1",
    )
    .bind(&aid1)
    .execute(&t.root)
    .await
    .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    assert_eq!(order_page(&t, a, &o1).await["status"], "expired");
    let (s, _) = t.deliver("razorpay", &a.id, &hook1).await;
    assert_eq!(s, StatusCode::OK);
    let p = order_page(&t, a, &o1).await;
    assert_eq!(p["status"], "paid");
    assert_eq!(p["tickets"].as_array().unwrap().len(), 2);
    assert_counters(&t, &ev).await;

    // (2) the event is full when a second late payment lands: refunded, not oversold
    let (ev2, tier2) = event(&t, a, 2, 2, 100_000, json!({})).await;
    let o2 = place_ok(&t, a, &ev2, order_body(&tier2, 2, "slow@example.com")).await;
    let (aid2, hook2) = start_unpaid(&t, a, &o2, "captured").await;
    sqlx::query(
        "UPDATE payment_attempts SET updated_at = now() - interval '10 minutes' WHERE id = $1",
    )
    .bind(&aid2)
    .execute(&t.root)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE event_orders SET hold_expires_at = now() - interval '1 minute' WHERE event_id = $1",
    )
    .bind(&ev2)
    .execute(&t.root)
    .await
    .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let o3 = place_ok(&t, a, &ev2, order_body(&tier2, 2, "fast@example.com")).await;
    pay(&t, a, &o3).await;
    let (s, _) = t.deliver("razorpay", &a.id, &hook2).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(order_page(&t, a, &o2).await["status"], "needs_refund");
    assert_eq!(
        t.count(&format!(
            "SELECT seats_sold::BIGINT FROM ticket_events WHERE id = '{ev2}'"
        ))
        .await,
        2,
        "only the first buyer's seats"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'ticket_order_unfulfilled'")
            .await,
        1
    );
    // the worker sends the queued refund through the ordinary flow
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let rep = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev2}/refund_report"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(rep["processed"]["orders"], 1, "{rep}");
    assert_eq!(rep["processed"]["amount_minor"], 200_000);
    assert_eq!(order_page(&t, a, &o2).await["status"], "refunded");
    assert_eq!(t.count("SELECT count(*) FROM recon_cases WHERE kind = 'ticket_order_unfulfilled' AND status = 'resolved'").await, 1);
    assert_counters(&t, &ev2).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn check_in_catches_a_second_scan_forgeries_and_other_events(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let (ev_other, tier_other) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 2, "a@example.com")).await;
    pay(&t, a, &o).await;
    let o_other = place_ok(
        &t,
        a,
        &ev_other,
        order_body(&tier_other, 1, "b@example.com"),
    )
    .await;
    pay(&t, a, &o_other).await;
    let tickets = order_page(&t, a, &o).await["tickets"].clone();
    let (qr0, qr1) = (
        tickets[0]["qr_payload"].as_str().unwrap().to_string(),
        tickets[1]["qr_payload"].as_str().unwrap().to_string(),
    );
    let code0 = tickets[0]["code"].as_str().unwrap().to_string();
    let other_qr = order_page(&t, a, &o_other).await["tickets"][0]["qr_payload"]
        .as_str()
        .unwrap()
        .to_string();

    // door staff: a key that can only check in
    let door = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "door", "scopes": ["events:checkin"] }),
        )
        .await;
    let door = Tenant {
        id: a.id.clone(),
        key: door["key"].as_str().unwrap().to_string(),
    };
    let scan = |code: &str| json!({ "code": code });
    let path = format!("/v1/events/{ev}/checkin");

    let r = t.ok(&door, "POST", &path, None, scan(&qr0)).await;
    assert_eq!(
        (r["result"].as_str(), r["valid"].as_bool()),
        (Some("ok"), Some(true)),
        "{r}"
    );
    assert_eq!(r["ticket"]["tier"], "General");
    // the same ticket again, by QR and by typing its code: a duplicate that says when and by whom it first got in
    for again in [qr0.clone(), code0.to_lowercase()] {
        let r = t.ok(&door, "POST", &path, None, scan(&again)).await;
        assert_eq!(r["result"], "duplicate", "{r}");
        assert_eq!(r["valid"], false);
        assert!(r["checked_in_at"].is_string());
    }
    assert_eq!(
        t.count("SELECT count(*) FROM tickets WHERE status = 'checked_in'")
            .await,
        1
    );
    // a forged payload (right code, wrong signature), garbage, a ticket of another event
    let forged = format!("ZT1.{code0}.{}", "0".repeat(24));
    assert_eq!(
        t.ok(&door, "POST", &path, None, scan(&forged)).await["result"],
        "invalid"
    );
    assert_eq!(
        t.ok(&door, "POST", &path, None, scan("hello")).await["result"],
        "invalid"
    );
    assert_eq!(
        t.ok(&door, "POST", &path, None, scan(&other_qr)).await["result"],
        "invalid",
        "signed for another event"
    );
    let other_code = other_qr.split('.').nth(1).unwrap();
    assert_eq!(
        t.ok(&door, "POST", &path, None, scan(other_code)).await["result"],
        "wrong_event"
    );
    // scans are logged
    assert!(
        t.count(&format!(
            "SELECT count(*) FROM ticket_scans WHERE event_id = '{ev}'"
        ))
        .await
            >= 6
    );

    // scope separation: door staff cannot read attendees, an events:read key cannot check in
    assert_eq!(
        t.call(
            &door,
            "GET",
            &format!("/v1/events/{ev}/attendees"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let reader = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "reader", "scopes": ["events:read"] }),
        )
        .await;
    let reader = Tenant {
        id: a.id.clone(),
        key: reader["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&reader, "POST", &path, None, scan(&qr1)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(
            &reader,
            "GET",
            &format!("/v1/events/{ev}/attendees"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );

    // stats and the offline list: valid tickets only, with the one already in marked
    let stats = t
        .ok(
            &door,
            "GET",
            &format!("/v1/events/{ev}/checkin/stats"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (stats["checked_in"].as_i64(), stats["expected"].as_i64()),
        (Some(1), Some(1))
    );
    assert!(!stats["recent_duplicates"].as_array().unwrap().is_empty());
    let export = t
        .ok(
            &door,
            "GET",
            &format!("/v1/events/{ev}/checkin/export"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(export["count"], 2);
    let statuses: Vec<&str> = export["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["status"].as_str().unwrap())
        .collect();
    assert!(statuses.contains(&"checked_in") && statuses.contains(&"issued"));
    assert!(
        export["tickets"][0]["qr_payload"]
            .as_str()
            .unwrap()
            .starts_with("ZT1.")
    );
    assert!(!export.to_string().contains("secret"));

    // a device that was offline uploads its scans: oldest first, the second ticket gets in once
    let sync = t
        .ok(
            &door,
            "POST",
            &format!("/v1/events/{ev}/checkin/sync"),
            None,
            json!({ "scans": [
        { "code": qr1, "scanned_at": (Utc::now() - Duration::minutes(5)).to_rfc3339() },
        { "code": qr1, "scanned_at": (Utc::now() - Duration::minutes(1)).to_rfc3339() },
        { "code": qr0, "scanned_at": (Utc::now() - Duration::minutes(3)).to_rfc3339() },
        { "code": "nonsense", "scanned_at": Utc::now().to_rfc3339() },
    ] }),
        )
        .await;
    assert_eq!(sync["counts"]["ok"], 1, "{sync}");
    assert_eq!(sync["counts"]["duplicate"], 2);
    assert_eq!(sync["counts"]["invalid"], 1);
    assert_eq!(
        t.count("SELECT count(*) FROM tickets WHERE status = 'checked_in'")
            .await,
        2
    );
    assert_counters(&t, &ev).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn two_door_devices_scanning_at_once_admit_a_ticket_once(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 1, "a@example.com")).await;
    pay(&t, a, &o).await;
    let qr = order_page(&t, a, &o).await["tickets"][0]["qr_payload"]
        .as_str()
        .unwrap()
        .to_string();
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (app, key, path, qr) = (
            t.app.clone(),
            a.key.clone(),
            format!("/v1/events/{ev}/checkin"),
            qr.clone(),
        );
        tasks.push(tokio::spawn(async move {
            raw(&app, &key, None, "POST", &path, None, json!({ "code": qr })).await
        }));
    }
    let mut oks = 0;
    for task in tasks {
        let (s, b) = task.await.unwrap();
        assert_eq!(s, StatusCode::OK);
        oks += (b["result"] == "ok") as i32;
    }
    assert_eq!(oks, 1, "exactly one scan gets the ticket in");
}

#[sqlx::test(migrations = "./migrations")]
async fn attendees_can_be_searched_and_exported_without_formula_injection(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 20, 20, 100_000, json!({})).await;
    let mut b = order_body(&tier, 2, "asha@example.com");
    b["buyer"]["name"] = json!("=HYPERLINK(\"http://x\")");
    b["answers"]["company"] = json!("Acme, Inc");
    let o = place_ok(&t, a, &ev, b).await;
    pay(&t, a, &o).await;
    let o2 = place_ok(&t, a, &ev, order_body(&tier, 1, "ravi@example.com")).await;
    pay(&t, a, &o2).await;

    let all = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/attendees"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(all["data"].as_array().unwrap().len(), 3);
    let by_mail = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/attendees?q=ravi"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(by_mail["data"].as_array().unwrap().len(), 1);
    let code = all["data"][0]["code"].as_str().unwrap().to_lowercase();
    assert_eq!(
        t.ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/attendees?q={code}"),
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // % and _ are plain characters in a search
    assert_eq!(
        t.ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/attendees?q=%25"),
            None,
            Value::Null
        )
        .await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let page = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/attendees?limit=2"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(page["has_more"], true);

    let csv = t
        .text_authed(a, &format!("/v1/events/{ev}/attendees.csv"))
        .await;
    assert!(csv.starts_with("ticket_code,status,tier,holder_name,email,phone,order_id,paid_at,checked_in_at,checked_in_by,company,terms\n"), "{csv}");
    assert_eq!(csv.lines().count(), 4);
    assert!(
        csv.contains("\"'=HYPERLINK(\"\"http://x\"\")\""),
        "a formula is defused: {csv}"
    );
    assert!(csv.contains("\"Acme, Inc\""));
}

#[sqlx::test(migrations = "./migrations")]
async fn cancelling_an_event_refunds_every_paid_order_with_progress_and_approvals(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    // refunds above 1,500.00 need a second person
    t.ok(
        a,
        "POST",
        "/v1/settings",
        None,
        json!({ "refund_approval_threshold_minor": 150_000 }),
    )
    .await;
    let (ev, tier) = event(&t, a, 50, 50, 100_000, json!({})).await;
    let small = place_ok(&t, a, &ev, order_body(&tier, 1, "small@example.com")).await;
    pay(&t, a, &small).await;
    let big = place_ok(&t, a, &ev, order_body(&tier, 3, "big@example.com")).await;
    pay(&t, a, &big).await;
    let half = place_ok(&t, a, &ev, order_body(&tier, 2, "half@example.com")).await;
    let half_attempt = pay(&t, a, &half).await;
    let unpaid = place_ok(&t, a, &ev, order_body(&tier, 1, "unpaid@example.com")).await;
    // the organiser already refunded part of one order by hand: the bulk refund must only send the rest
    let half_intent = half["payment"]["intent_id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{half_intent}/refunds"),
        Some("manual-half"),
        json!({ "amount_minor": 50_000 }),
    )
    .await;

    // refunding needs both scopes
    let only_events = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "events", "scopes": ["events:write"] }),
        )
        .await;
    let only_events = Tenant {
        id: a.id.clone(),
        key: only_events["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(
            &only_events,
            "POST",
            &format!("/v1/events/{ev}/cancel"),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let r = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/events/{ev}/cancel"),
            None,
            json!({ "reason": "Venue flooded" }),
        )
        .await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.1);
    let r = r.1;
    assert_eq!(r["status"], "cancelled");
    assert_eq!(r["released_unpaid_orders"], 1);
    assert_eq!(r["paid_orders_cancelled"], 3);
    let batch = &r["refund_batch"];
    assert_eq!(batch["orders"], 3);
    assert_eq!(batch["queued"], 0, "a small event is sent in one go");
    assert_eq!(batch["refunds_by_status"]["processed"], 2, "{batch}");
    assert_eq!(
        batch["refunds_by_status"]["pending_approval"], 1,
        "the 3-ticket order is over the threshold: {batch}"
    );
    assert_eq!(batch["finished"], false);
    assert_eq!(order_page(&t, a, &unpaid).await["status"], "cancelled");
    // sales are over, seats are free again, the tickets no longer work
    assert_eq!(
        t.public(
            "POST",
            &format!("/v1/public/events/{}/{ev}/orders", a.id),
            None,
            order_body(&tier, 1, "z@example.com")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_counters(&t, &ev).await;
    assert_eq!(t.count(&format!("SELECT count(*) FROM tickets WHERE event_id = '{ev}' AND status IN ('issued', 'checked_in')")).await, 0);

    assert_eq!(
        order_page(&t, a, &big).await["status"],
        "cancelled",
        "refund not yet approved"
    );
    // the refund above the threshold waits; the person who asked cannot approve it, a second person can
    let big_refund: String =
        sqlx::query_scalar("SELECT id FROM refunds WHERE status = 'pending_approval'")
            .fetch_one(&t.root)
            .await
            .unwrap();
    let denied = t
        .call_as(
            a,
            "priya",
            "POST",
            &format!("/v1/refunds/{big_refund}/approve"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(denied.0, StatusCode::FORBIDDEN, "separation of duties");
    t.ok(
        &t.a,
        "POST",
        &format!("/v1/refunds/{big_refund}/approve"),
        None,
        Value::Null,
    )
    .await;

    // no payment was refunded more than it took, and the half-refunded one got exactly the rest
    let over: i64 = t.count("SELECT count(*) FROM payment_attempts WHERE refunded_minor + reserved_refund_minor > amount_minor").await;
    assert_eq!(over, 0);
    let refunded: i64 =
        sqlx::query_scalar("SELECT refunded_minor FROM payment_attempts WHERE id = $1")
            .bind(&half_attempt)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(refunded, 200_000);

    let rep = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/refund_report"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(rep["orders_to_refund"], 3);
    assert_eq!(rep["processed"]["orders"], 3, "{rep}");
    assert_eq!(rep["awaiting_approval"]["orders"], 0);
    assert_eq!(rep["batches"][0]["finished"], true);
    assert_eq!(rep["items"].as_array().unwrap().len(), 3);
    // the manual part-refund cannot have made the order "refunded" before the batch ran; afterwards all are refunded
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM event_orders WHERE event_id = '{ev}' AND status = 'refunded'"
        ))
        .await,
        3
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM tickets WHERE event_id = '{ev}' AND status = 'refunded'"
        ))
        .await,
        6
    );
    // calling cancel again returns the same batch, makes no new refunds
    let again = t
        .ok(
            a,
            "POST",
            &format!("/v1/events/{ev}/cancel"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(again["already_cancelled"], true);
    assert_eq!(
        t.count("SELECT count(*) FROM refunds").await,
        4,
        "3 batch refunds + the manual one"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE type = 'event.cancelled'")
            .await,
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_big_cancellation_is_finished_by_the_worker_in_slices(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 200, 200, 10_000, json!({})).await;
    for i in 0..55 {
        let o = place_ok(
            &t,
            a,
            &ev,
            order_body(&tier, 1, &format!("p{i}@example.com")),
        )
        .await;
        pay(&t, a, &o).await;
    }
    let r = t
        .ok(
            a,
            "POST",
            &format!("/v1/events/{ev}/cancel"),
            None,
            json!({}),
        )
        .await;
    let batch_id = r["refund_batch"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["refund_batch"]["orders"], 55);
    assert_eq!(
        r["refund_batch"]["queued"], 5,
        "50 sent in the request, 5 left for the worker"
    );
    assert_eq!(r["refund_batch"]["finished"], false);
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let p = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/refund_batches/{batch_id}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(
        (p["queued"].as_i64(), p["finished"].as_bool()),
        (Some(0), Some(true)),
        "{p}"
    );
    assert_eq!(p["refunds_by_status"]["processed"], 55);
}

#[sqlx::test(migrations = "./migrations")]
async fn one_order_can_be_cancelled_and_a_used_ticket_blocks_it(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 2, "a@example.com")).await;
    pay(&t, a, &o).await;
    let qr = order_page(&t, a, &o).await["tickets"][0]["qr_payload"]
        .as_str()
        .unwrap()
        .to_string();
    t.ok(
        a,
        "POST",
        &format!("/v1/events/{ev}/checkin"),
        None,
        json!({ "code": qr }),
    )
    .await;
    let id = o["order_id"].as_str().unwrap();
    let path = format!("/v1/events/{ev}/orders/{id}/cancel");
    let (s, b) = t.call(a, "POST", &path, None, json!({})).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("tickets_used")),
        "{b}"
    );
    let r = t
        .ok(
            a,
            "POST",
            &path,
            None,
            json!({ "force": true, "reason": "buyer asked" }),
        )
        .await;
    assert_eq!(r["order"]["status"], "refunded", "{r}");
    assert_eq!(r["refund_batch"]["refunds_by_status"]["processed"], 1);
    assert_eq!(
        t.count(&format!(
            "SELECT seats_sold::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        0,
        "its seats are back on sale"
    );
    assert_counters(&t, &ev).await;
    // and cancelling it again is refused
    assert_eq!(
        t.call(a, "POST", &path, None, json!({})).await.0,
        StatusCode::CONFLICT
    );
    // a refunded ticket does not get in
    let again = t
        .ok(
            a,
            "POST",
            &format!("/v1/events/{ev}/checkin"),
            None,
            json!({ "code": qr }),
        )
        .await;
    assert_eq!(again["result"], "refunded");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_full_refund_through_the_refund_endpoint_voids_the_tickets_too(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 2, "a@example.com")).await;
    pay(&t, a, &o).await;
    let iid = o["payment"]["intent_id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("full"),
        json!({}),
    )
    .await;
    assert_eq!(order_page(&t, a, &o).await["status"], "refunded");
    assert_eq!(
        t.count(&format!(
            "SELECT seats_sold::BIGINT FROM ticket_events WHERE id = '{ev}'"
        ))
        .await,
        0
    );
    assert_counters(&t, &ev).await;
}

#[sqlx::test(migrations = "./migrations")]
async fn tiers_can_limit_payment_methods_and_early_bird_prices_apply(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let e = t.ok(a, "POST", "/v1/events", None, json!({
        "name": "Conf", "starts_at": in_hours(72), "capacity": 100,
        "tiers": [
            { "name": "Early", "price_minor": 100_000, "quantity": 10, "early_bird_price_minor": 60_000,
              "early_bird_starts_at": in_hours(-1), "early_bird_ends_at": in_hours(5), "allowed_methods": ["card"] },
            { "name": "Later", "price_minor": 90_000, "quantity": 10, "sale_starts_at": in_hours(10) },
            { "name": "Closed", "price_minor": 90_000, "quantity": 10, "sale_starts_at": in_hours(-10), "sale_ends_at": in_hours(-5) },
        ],
    })).await;
    let ev = e["id"].as_str().unwrap();
    // a draft event is invisible to the public; sales need at least one tier
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/public/events/{}/{ev}", a.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/events/{ev}/status"),
        None,
        json!({ "status": "on_sale" }),
    )
    .await;
    let page = t
        .ok_public(
            "GET",
            &format!("/v1/public/events/{}/{ev}", a.id),
            Value::Null,
        )
        .await;
    let tiers = page["tiers"].as_array().unwrap();
    assert_eq!(
        (
            tiers[0]["price_minor"].as_i64(),
            tiers[0]["price_kind"].as_str()
        ),
        (Some(60_000), Some("early_bird"))
    );
    assert_eq!(tiers[0]["regular_price_minor"], 100_000);
    assert_eq!(tiers[1]["can_buy"], false);
    assert_eq!(tiers[1]["sale"]["state"], "upcoming");
    assert_eq!(tiers[2]["sale"]["state"], "ended");
    let (early, later, closed) = (
        tiers[0]["id"].as_str().unwrap(),
        tiers[1]["id"].as_str().unwrap(),
        tiers[2]["id"].as_str().unwrap(),
    );
    for tier in [later, closed] {
        let (s, b) = place(&t, a, ev, json!({ "buyer": { "name": "A", "email": "a@example.com" }, "lines": [{ "tier_id": tier, "quantity": 1 }] })).await;
        assert_eq!(
            (s, b["error"]["code"].as_str()),
            (StatusCode::CONFLICT, Some("tier_not_on_sale")),
            "{b}"
        );
    }
    let o = place_ok(&t, a, ev, json!({ "buyer": { "name": "A", "email": "a@example.com" }, "lines": [{ "tier_id": early, "quantity": 2 }] })).await;
    assert_eq!(
        o["total_minor"], 120_000,
        "the early-bird price, frozen on the order"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM event_order_lines WHERE price_kind = 'early_bird'")
            .await,
        1
    );
    // only the card method works for this order
    let iid = o["payment"]["intent_id"].as_str().unwrap();
    let cs = o["payment"]["client_secret"].as_str().unwrap();
    let session = t
        .ok_public(
            "GET",
            &format!("/v1/checkout/{}/{iid}?secret={cs}", a.id),
            Value::Null,
        )
        .await;
    assert_eq!(session["methods"], json!(["card"]));
    let (s, b) = t
        .call(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("upi-try"),
            json!({ "method": "upi" }),
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("method_not_allowed")),
        "{b}"
    );
    // two tiers with no method in common cannot share an order
    t.ok(a, "PUT", &format!("/v1/events/{ev}/tiers/{later}"), None, json!({ "name": "Later", "price_minor": 90_000, "quantity": 10, "allowed_methods": ["upi"] })).await;
    let (s, b) = place(&t, a, ev, json!({ "buyer": { "name": "A", "email": "b@example.com" }, "lines": [{ "tier_id": early, "quantity": 1 }, { "tier_id": later, "quantity": 1 }] })).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_registration_form_is_enforced(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let mut b = order_body(&tier, 1, "a@example.com");
    b["answers"] = json!({ "company": "Acme" });
    let (s, r) = place(&t, a, &ev, b).await;
    assert_eq!(
        (s, r["error"]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_answers")),
        "{r}"
    );
    let mut b = order_body(&tier, 1, "not-an-email");
    b["buyer"]["email"] = json!("nope");
    assert_eq!(
        place(&t, a, &ev, b).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(t.count("SELECT count(*) FROM event_orders").await, 0);
    assert_counters(&t, &ev).await;
    // a replayed request with the same Idempotency-Key returns the same order, not a second hold
    let path = format!("/v1/public/events/{}/{ev}/orders", a.id);
    let (s1, o1) = t
        .public(
            "POST",
            &path,
            Some("buy-1"),
            order_body(&tier, 1, "a@example.com"),
        )
        .await;
    let (s2, o2) = t
        .public(
            "POST",
            &path,
            Some("buy-1"),
            order_body(&tier, 1, "a@example.com"),
        )
        .await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert_eq!(o1["order_id"], o2["order_id"]);
    assert_eq!(t.count("SELECT count(*) FROM event_orders").await, 1);
    // one buyer cannot sit on more than three unpaid orders
    for _ in 0..2 {
        place_ok(&t, a, &ev, order_body(&tier, 1, "A@Example.com")).await;
    }
    let (s, r) = place(&t, a, &ev, order_body(&tier, 1, "a@example.com")).await;
    assert_eq!(
        (s, r["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("too_many_pending_orders")),
        "{r}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn free_tickets_need_no_payment_and_vendor_shares_follow_paid_orders(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let vendor = t.ok(a, "POST", "/v1/vendors", None, json!({ "name": "Speaker Co", "account_number": "1234567890123", "ifsc": "HDFC0001234" })).await;
    let vid = vendor["id"].as_str().unwrap();
    let (ev, paid_tier) = event(
        &t,
        a,
        20,
        10,
        100_000,
        json!({ "splits": [{ "vendor_id": vid, "share_bp": 2500 }] }),
    )
    .await;
    let free = t
        .ok(
            a,
            "POST",
            &format!("/v1/events/{ev}/tiers"),
            None,
            json!({ "name": "Student", "price_minor": 0, "quantity": 5 }),
        )
        .await;
    let free_tier = free["tiers"][1]["id"].as_str().unwrap();
    let f = place_ok(&t, a, &ev, order_body(free_tier, 2, "free@example.com")).await;
    assert_eq!(
        (f["status"].as_str(), f["payment"].is_null()),
        (Some("paid"), true),
        "{f}"
    );
    assert_eq!(
        order_page(&t, a, &f).await["tickets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_counters(&t, &ev).await;
    // a coupon on a free order is refused
    let mut b = order_body(free_tier, 1, "free2@example.com");
    b["coupon"] = json!("ANY");
    assert_eq!(
        place(&t, a, &ev, b).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let o = place_ok(&t, a, &ev, order_body(&paid_tier, 2, "paid@example.com")).await;
    pay(&t, a, &o).await;
    let share: i64 =
        sqlx::query_scalar("SELECT amount_minor FROM payment_splits WHERE intent_id = $1")
            .bind(o["payment"]["intent_id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(share, 50_000, "25% of 2,000.00");
    let bal = t
        .ok(
            a,
            "GET",
            &format!("/v1/vendors/{vid}/balance"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(bal["balances"][0]["owed_minor"], 50_000, "{bal}");
}

#[sqlx::test(migrations = "./migrations")]
async fn one_business_cannot_see_or_touch_anothers_events(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let (ev, tier) = event(&t, a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, a, &ev, order_body(&tier, 1, "a@example.com")).await;
    pay(&t, a, &o).await;
    let oid = o["order_id"].as_str().unwrap();
    let tok = o["order_token"].as_str().unwrap();
    let qr = order_page(&t, a, &o).await["tickets"][0]["qr_payload"]
        .as_str()
        .unwrap()
        .to_string();

    // the other business's key: nothing is visible
    assert_eq!(
        t.ok(b, "GET", "/v1/events", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    for (m, p) in [
        ("GET", format!("/v1/events/{ev}")),
        ("GET", format!("/v1/events/{ev}/orders")),
        ("GET", format!("/v1/events/{ev}/orders/{oid}")),
        ("GET", format!("/v1/events/{ev}/attendees")),
        ("GET", format!("/v1/events/{ev}/attendees.csv")),
        ("GET", format!("/v1/events/{ev}/refund_report")),
        ("GET", format!("/v1/events/{ev}/checkin/export")),
    ] {
        assert_eq!(
            t.call(b, m, &p, None, Value::Null).await.0,
            StatusCode::NOT_FOUND,
            "{m} {p}"
        );
    }
    for (p, body) in [
        (format!("/v1/events/{ev}/cancel"), json!({})),
        (
            format!("/v1/events/{ev}/status"),
            json!({ "status": "closed" }),
        ),
        (format!("/v1/events/{ev}/checkin"), json!({ "code": qr })),
        (format!("/v1/events/{ev}/orders/{oid}/cancel"), json!({})),
    ] {
        assert_eq!(
            t.call(b, "POST", &p, None, body).await.0,
            StatusCode::NOT_FOUND,
            "{p}"
        );
    }
    // public pages: another tenant's id does not reach it
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/public/events/{}/{ev}", b.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.public(
            "GET",
            &format!("/v1/public/events/{}/orders/{oid}?t={tok}", b.id),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        place(&t, b, &ev, order_body(&tier, 1, "x@example.com"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // a ticket of A is not valid at B's own event
    let (ev_b, _) = event(&t, b, 10, 10, 100_000, json!({})).await;
    let r = t
        .ok(
            b,
            "POST",
            &format!("/v1/events/{ev_b}/checkin"),
            None,
            json!({ "code": qr }),
        )
        .await;
    assert_eq!(r["result"], "invalid");
    // and the database agrees: B's scope sees no ticketing rows of A
    let mut tx = tenant::begin(&t.state.pool, &b.id).await.unwrap();
    for table in [
        "ticket_events",
        "ticket_tiers",
        "event_form_fields",
        "event_orders",
        "event_order_lines",
        "tickets",
        "ticket_scans",
        "event_refund_batches",
        "event_refund_items",
    ] {
        let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE tenant_id <> '{}'",
            b.id
        )))
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tickets")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // a row cannot be written into another tenant's scope either
    let r = sqlx::query("INSERT INTO ticket_scans (tenant_id, event_id, code, result, actor) VALUES ($1, $2, 'X', 'ok', 'x')").bind(&a.id).bind(&ev).execute(&mut *tx).await;
    assert!(r.is_err(), "with check policy");
}

#[sqlx::test(migrations = "./migrations")]
async fn tickets_are_emailed_once_when_a_checkout_address_is_set(pool: PgPool) {
    let mut t = setup(pool).await;
    let a = t.a.clone();
    let (ev, tier) = event(&t, &a, 10, 10, 100_000, json!({})).await;
    let o = place_ok(&t, &a, &ev, order_body(&tier, 1, "mail@example.com")).await;
    pay(&t, &a, &o).await;
    t.state.checkout_public_url = Some("https://tickets.example.test".into());
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    zyvor_api::worker::run_cycle(&t.state, &t.http)
        .await
        .unwrap();
    let body: String = sqlx::query_scalar("SELECT body FROM mail_queue WHERE to_email = 'mail@example.com' AND subject LIKE 'Your tickets for%'").fetch_one(&t.root).await.unwrap();
    let id = o["order_id"].as_str().unwrap();
    assert!(
        body.contains(&format!(
            "https://tickets.example.test/events/{}/orders/{id}?t={}",
            a.id,
            o["order_token"].as_str().unwrap()
        )),
        "{body}"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM mail_queue WHERE subject LIKE 'Your tickets for%'")
            .await,
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn event_setup_is_validated_and_sales_follow_the_status(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let bad = |v: Value| async {
        let (s, b) = t.call(a, "POST", "/v1/events", None, v).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    };
    bad(json!({ "name": "", "starts_at": in_hours(1), "capacity": 10 })).await;
    bad(json!({ "name": "x", "starts_at": in_hours(1), "capacity": 0 })).await;
    bad(json!({ "name": "x", "starts_at": in_hours(2), "ends_at": in_hours(1), "capacity": 5 }))
        .await;
    bad(json!({ "name": "x", "mode": "online", "starts_at": in_hours(1), "capacity": 5 })).await;
    bad(json!({ "name": "x", "starts_at": in_hours(1), "capacity": 5, "tiers": [{ "name": "t", "price_minor": 100, "quantity": 0 }] })).await;
    let (s, _) = t.call(a, "POST", "/v1/events", None, json!({ "name": "x", "starts_at": in_hours(1), "capacity": 5, "splits": [{ "vendor_id": "ven_missing", "share_bp": 100 }] })).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "a split to an unknown vendor");
    let e = t
        .ok(
            a,
            "POST",
            "/v1/events",
            None,
            json!({ "name": "No tiers", "starts_at": in_hours(5), "capacity": 5 }),
        )
        .await;
    let id = e["id"].as_str().unwrap();
    assert_eq!(e["status"], "draft");
    let (s, b) = t
        .call(
            a,
            "POST",
            &format!("/v1/events/{id}/status"),
            None,
            json!({ "status": "on_sale" }),
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("no_tiers"))
    );
    let (s, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/events/{id}/status"),
            None,
            json!({ "status": "closed" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "draft cannot be closed");
    // capacity cannot drop below what is held
    let (ev, tier) = event(&t, a, 5, 5, 1_000, json!({})).await;
    place_ok(&t, a, &ev, order_body(&tier, 3, "a@example.com")).await;
    let (s, b) = t
        .call(
            a,
            "PUT",
            &format!("/v1/events/{ev}"),
            None,
            json!({ "capacity": 2 }),
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("capacity_below_sold")),
        "{b}"
    );
    t.ok(
        a,
        "PUT",
        &format!("/v1/events/{ev}"),
        None,
        json!({ "capacity": 4 }),
    )
    .await;
    // closing stops new orders
    t.ok(
        a,
        "POST",
        &format!("/v1/events/{ev}/status"),
        None,
        json!({ "status": "closed" }),
    )
    .await;
    assert_eq!(
        place(&t, a, &ev, order_body(&tier, 1, "b@example.com"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    // archived tiers stop selling
    t.ok(
        a,
        "POST",
        &format!("/v1/events/{ev}/status"),
        None,
        json!({ "status": "on_sale" }),
    )
    .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/events/{ev}/tiers/{tier}/archive"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        place(&t, a, &ev, order_body(&tier, 1, "b@example.com"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    // the order list for the organiser
    let list = t
        .ok(
            a,
            "GET",
            &format!("/v1/events/{ev}/orders?status=pending"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["buyer"]["email"], "a@example.com");
}
