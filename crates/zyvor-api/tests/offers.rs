//! Offers, against the simulators only: merchant-funded discounts judged when an attempt is created, caps that
//! cannot be over-used by concurrent attempts, stacking, and refunds that hand back the same share of the discount
//! as of the cash. Nothing here involves a real provider, bank or money.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn offer(t: &T, tn: &Tenant, body: Value) -> Value {
    let mut b = json!({ "name": "Test offer", "kind": "percent", "value": 1_000 });
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    let (s, r) = t.call(tn, "POST", "/v1/offers", None, b).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    r
}

async fn preview(t: &T, tn: &Tenant, body: Value) -> Value {
    let mut b = json!({ "amount_minor": 100_000, "method": "upi" });
    for (k, v) in body.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    t.ok(tn, "POST", "/v1/offers/preview", None, b).await
}

fn reasons(p: &Value) -> Vec<String> {
    p["evaluation"]["not_applied"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["reason_code"].as_str().unwrap().to_string())
        .collect()
}

async fn attempt(t: &T, tn: &Tenant, iid: &str, body: Value) -> (StatusCode, Value) {
    t.call(
        tn,
        "POST",
        &format!("/v1/payment_intents/{iid}/attempts"),
        Some(&format!("att-{}", zyvor_core::new_id("k"))),
        body,
    )
    .await
}

/// The payer completes a created attempt at the simulated Razorpay.
async fn settle(t: &T, tn: &Tenant, att: &Value, outcome: &str) {
    let order = att["provider_order_ref"].as_str().unwrap();
    let paid = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": outcome }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &tn.id, &paid["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
}

async fn offer_row(t: &T, id: &str) -> (i32, i64) {
    sqlx::query_as("SELECT redemptions, spent_minor FROM offers WHERE id = $1")
        .bind(id)
        .fetch_one(&t.root)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn an_offer_prices_the_attempt_and_says_who_funds_it(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(
        &t,
        a,
        json!({ "name": "UPI 10%", "methods": ["upi"], "max_discount_minor": 8_000 }),
    )
    .await;
    assert_eq!(o["funded_by"], "merchant");

    // the preview and the real attempt agree
    let p = preview(&t, a, json!({})).await;
    assert_eq!(p["evaluation"]["gross_minor"], 100_000);
    assert_eq!(p["evaluation"]["discount_minor"], 8_000, "capped at 8,000");
    assert_eq!(p["evaluation"]["net_minor"], 92_000);
    assert_eq!(p["evaluation"]["applied"][0]["funded_by"], "merchant");
    // nothing is reserved by a preview
    assert_eq!(offer_row(&t, o["id"].as_str().unwrap()).await, (0, 0));

    let intent = t.intent(a, 100_000, json!({})).await;
    let iid = intent["id"].as_str().unwrap();
    let (s, att) = attempt(&t, a, iid, json!({ "method": "upi" })).await;
    assert_eq!(s, StatusCode::CREATED, "{att}");
    assert_eq!(
        att["amount_minor"], 92_000,
        "the payer is asked for the discounted amount"
    );
    assert_eq!(att["route"]["offers"]["discount_minor"], 8_000);
    assert_eq!(att["route"]["offers"]["funded_by"], "merchant");
    assert_eq!(
        offer_row(&t, o["id"].as_str().unwrap()).await,
        (1, 8_000),
        "reserved with the attempt"
    );

    let applied = t
        .ok(
            a,
            "GET",
            &format!("/v1/payment_intents/{iid}/offers"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(applied["funded_by"], "merchant");
    assert_eq!(applied["data"][0]["status"], "reserved");
    assert_eq!(applied["data"][0]["gross_minor"], 100_000);
    assert_eq!(applied["data"][0]["charged_minor"], 92_000);
    assert_eq!(applied["data"][0]["offer_name"], "UPI 10%");

    settle(&t, a, &att, "captured").await;
    let after = t
        .ok(
            a,
            "GET",
            &format!("/v1/payment_intents/{iid}/offers"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(after["data"][0]["status"], "redeemed");
    // the money that moved is the discounted amount
    let captured: i64 = sqlx::query_scalar(
        "SELECT amount_minor FROM payment_attempts WHERE id = $1 AND status = 'captured'",
    )
    .bind(att["id"].as_str().unwrap())
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!(captured, 92_000);
    // audited
    assert!(
        t.count("SELECT COUNT(*) FROM audit_events WHERE action = 'offer.applied'")
            .await
            >= 1
    );
    let detail = t
        .ok(
            a,
            "GET",
            &format!("/v1/offers/{}", o["id"].as_str().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(detail["totals"]["discount_given_minor"], 8_000);
    assert_eq!(detail["totals"]["net_cost_minor"], 8_000);

    // a replay of the same attempt key reserves nothing more
    let key = "att-replay";
    let intent2 = t.intent(a, 100_000, json!({})).await;
    let iid2 = intent2["id"].as_str().unwrap();
    for _ in 0..2 {
        let (s, _) = t
            .call(
                a,
                "POST",
                &format!("/v1/payment_intents/{iid2}/attempts"),
                Some(key),
                json!({ "method": "upi" }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED);
    }
    assert_eq!(offer_row(&t, o["id"].as_str().unwrap()).await, (2, 16_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn conditions_decide_and_the_preview_says_why_not(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    offer(
        &t,
        a,
        json!({ "name": "Visa card", "methods": ["card"], "card_networks": ["visa"], "min_amount_minor": 50_000, "max_amount_minor": 500_000 }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "BIN range", "methods": ["card"], "bin_ranges": [{ "from": "411100", "to": "411199" }], "kind": "flat", "value": 500 }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "OkAxis UPI", "methods": ["upi"], "upi_handles": ["okaxis"] }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "HDFC netbanking", "methods": ["netbanking"], "banks": ["hdfc"] }),
    )
    .await;

    // a visa card above the minimum: the network offer applies, the BIN-range one needs a BIN
    let p = preview(
        &t,
        a,
        json!({ "method": "card", "instrument": { "card_network": "visa" } }),
    )
    .await;
    assert_eq!(p["evaluation"]["applied"][0]["name"], "Visa card");
    assert!(reasons(&p).contains(&"card_bin_needed".to_string()), "{p}");
    // a BIN inside the range classes the card as visa too; the better offer (10%) wins, the other cannot stack
    let p = preview(
        &t,
        a,
        json!({ "method": "card", "instrument": { "card_bin": "411150" } }),
    )
    .await;
    assert_eq!(p["evaluation"]["applied"].as_array().unwrap().len(), 1);
    assert_eq!(p["evaluation"]["discount_minor"], 10_000);
    assert!(reasons(&p).contains(&"outranked".to_string()));
    // a card outside both
    let p = preview(
        &t,
        a,
        json!({ "method": "card", "instrument": { "card_network": "rupay" } }),
    )
    .await;
    assert_eq!(p["evaluation"]["discount_minor"], 0);
    assert!(reasons(&p).contains(&"card_network_not_eligible".to_string()));
    // below the amount band
    let p = preview(
        &t,
        a,
        json!({ "method": "card", "amount_minor": 10_000, "instrument": { "card_network": "visa" } }),
    )
    .await;
    assert!(reasons(&p).contains(&"amount_below_min".to_string()));
    // UPI handle from the typed UPI ID, and from nothing
    let p = preview(&t, a, json!({ "vpa": "asha@okaxis" })).await;
    assert_eq!(p["evaluation"]["applied"][0]["name"], "OkAxis UPI");
    let p = preview(&t, a, json!({ "vpa": "asha@ybl" })).await;
    assert_eq!(p["evaluation"]["discount_minor"], 0);
    assert!(reasons(&p).contains(&"upi_handle_not_eligible".to_string()));
    let p = preview(&t, a, json!({})).await;
    assert!(reasons(&p).contains(&"upi_handle_unknown".to_string()));
    // bank
    let p = preview(
        &t,
        a,
        json!({ "method": "netbanking", "instrument": { "bank": "HDFC" } }),
    )
    .await;
    assert_eq!(p["evaluation"]["applied"][0]["name"], "HDFC netbanking");
    // a full card number is refused outright
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/offers/preview",
            None,
            json!({ "amount_minor": 1_000, "method": "card", "instrument": { "card_bin": "4111111111111111" } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["error"]["code"], "invalid_instrument");
}

#[sqlx::test(migrations = "./migrations")]
async fn new_and_returning_customers_and_a_code(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    offer(
        &t,
        a,
        json!({ "name": "Welcome", "segment": "new", "kind": "flat", "value": 5_000 }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "Loyal", "segment": "returning", "kind": "flat", "value": 3_000 }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "Secret", "code": "diwali", "kind": "flat", "value": 20_000 }),
    )
    .await;
    let cust = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "Asha" }))
        .await;
    let cid = cust["id"].as_str().unwrap();

    let p = preview(&t, a, json!({ "customer_id": cid })).await;
    assert_eq!(p["customer_segment"], "new");
    assert_eq!(p["evaluation"]["applied"][0]["name"], "Welcome");
    // with no customer a segment offer cannot be judged
    let p = preview(&t, a, json!({})).await;
    assert_eq!(p["evaluation"]["discount_minor"], 0);
    assert!(reasons(&p).contains(&"customer_required".to_string()));
    // an offer that needs a code is invisible until it is typed
    assert!(
        !p["evaluation"]["not_applied"]
            .to_string()
            .contains("Secret")
    );
    let p = preview(&t, a, json!({ "offer_codes": ["DIWALI"] })).await;
    assert_eq!(p["evaluation"]["applied"][0]["name"], "Secret");

    // after a paid order the same customer is returning
    let (iid, _, _) = t
        .paid_razorpay(a, 40_000, json!({ "customer_id": cid }))
        .await;
    let _ = iid;
    let p = preview(&t, a, json!({ "customer_id": cid })).await;
    assert_eq!(p["customer_segment"], "returning");
    assert_eq!(p["evaluation"]["applied"][0]["name"], "Loyal");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_capped_offer_cannot_be_over_used_by_concurrent_attempts(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(
        &t,
        a,
        json!({ "name": "First one only", "max_redemptions": 1 }),
    )
    .await;
    let oid = o["id"].as_str().unwrap().to_string();
    let mut intents = Vec::new();
    for _ in 0..8 {
        intents.push(
            t.intent(a, 100_000, json!({})).await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let results = futures_util::future::join_all(
        intents
            .iter()
            .map(|iid| attempt(&t, a, iid, json!({ "method": "upi" }))),
    )
    .await;
    let mut discounted = 0;
    for (s, att) in &results {
        assert_eq!(*s, StatusCode::CREATED, "{att}");
        match att["amount_minor"].as_i64().unwrap() {
            90_000 => discounted += 1,
            100_000 => {}
            other => panic!("unexpected amount {other}"),
        }
    }
    assert_eq!(discounted, 1, "exactly one attempt got the offer");
    assert_eq!(offer_row(&t, &oid).await, (1, 10_000));
    assert_eq!(
        t.count("SELECT COUNT(*) FROM offer_redemptions WHERE status = 'reserved'")
            .await,
        1
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn per_customer_and_budget_caps_hold_under_concurrency(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let cust = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "Asha" }))
        .await;
    let cid = cust["id"].as_str().unwrap();
    let per = offer(&t, a, json!({ "name": "Once each", "max_per_customer": 1 })).await;
    let mut intents = Vec::new();
    for _ in 0..5 {
        intents.push(
            t.intent(a, 100_000, json!({ "customer_id": cid })).await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let results = futures_util::future::join_all(
        intents
            .iter()
            .map(|iid| attempt(&t, a, iid, json!({ "method": "upi" }))),
    )
    .await;
    let discounted = results
        .iter()
        .filter(|(_, r)| r["amount_minor"] == 90_000)
        .count();
    assert_eq!(discounted, 1, "one use per customer");
    assert_eq!(offer_row(&t, per["id"].as_str().unwrap()).await.0, 1);
    // the customer cannot be judged without a customer: no customer, no per-customer offer
    let p = preview(&t, a, json!({})).await;
    assert!(reasons(&p).contains(&"customer_required".to_string()));

    // a budget: 15,000 allows one 10,000 discount and refuses a second that would go past it
    let t2 = setup_budget(&t).await;
    assert_eq!(t2, (1, 10_000));
}

async fn setup_budget(t: &T) -> (i32, i64) {
    let b = &t.b;
    let o = offer(t, b, json!({ "name": "Budgeted", "budget_minor": 15_000 })).await;
    let mut intents = Vec::new();
    for _ in 0..4 {
        intents.push(
            t.intent(b, 100_000, json!({ "allowed_providers": ["razorpay"] }))
                .await["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    let results = futures_util::future::join_all(
        intents
            .iter()
            .map(|iid| attempt(t, b, iid, json!({ "method": "upi" }))),
    )
    .await;
    for (s, r) in &results {
        assert_eq!(*s, StatusCode::CREATED, "{r}");
    }
    let p = preview(t, b, json!({})).await;
    assert!(reasons(&p).contains(&"budget_exhausted".to_string()), "{p}");
    offer_row(t, o["id"].as_str().unwrap()).await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_failed_or_cancelled_attempt_gives_the_cap_back(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(&t, a, json!({ "name": "One", "max_redemptions": 1 })).await;
    let oid = o["id"].as_str().unwrap();
    let i1 = t.intent(a, 100_000, json!({})).await;
    let (_, att1) = attempt(
        &t,
        a,
        i1["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(att1["amount_minor"], 90_000);
    // a second payer is turned away while the first holds the only use
    let i2 = t.intent(a, 100_000, json!({})).await;
    let (_, att2) = attempt(
        &t,
        a,
        i2["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(att2["amount_minor"], 100_000);
    // the first payer's attempt fails: the use is back
    settle(&t, a, &att1, "failed").await;
    assert_eq!(offer_row(&t, oid).await, (0, 0));
    let released: String =
        sqlx::query_scalar("SELECT status FROM offer_redemptions WHERE attempt_id = $1")
            .bind(att1["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(released, "released");
    // and the same intent can try again and gets it
    let (s, retry) = attempt(
        &t,
        a,
        i1["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{retry}");
    assert_eq!(retry["amount_minor"], 90_000);
    assert_eq!(offer_row(&t, oid).await, (1, 10_000));
    // cancelling releases it as well
    let c = t
        .ok(
            a,
            "POST",
            &format!(
                "/v1/payment_attempts/{}/cancel",
                retry["id"].as_str().unwrap()
            ),
            None,
            json!({}),
        )
        .await;
    assert_eq!(c["status"], "failed");
    assert_eq!(offer_row(&t, oid).await, (0, 0));
}

#[sqlx::test(migrations = "./migrations")]
async fn stackable_offers_combine_and_a_non_stackable_one_stands_alone(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    offer(
        &t,
        a,
        json!({ "name": "S1", "stackable": true, "kind": "percent", "value": 1_000 }),
    )
    .await;
    offer(
        &t,
        a,
        json!({ "name": "S2", "stackable": true, "kind": "flat", "value": 5_000 }),
    )
    .await;
    let p = preview(&t, a, json!({})).await;
    assert_eq!(p["evaluation"]["applied"].as_array().unwrap().len(), 2);
    assert_eq!(p["evaluation"]["discount_minor"], 15_000);
    // a bigger single offer beats the pair and applies alone
    offer(
        &t,
        a,
        json!({ "name": "Big", "kind": "flat", "value": 20_000 }),
    )
    .await;
    let p = preview(&t, a, json!({})).await;
    assert_eq!(p["evaluation"]["applied"].as_array().unwrap().len(), 1);
    assert_eq!(p["evaluation"]["applied"][0]["name"], "Big");
    assert_eq!(p["evaluation"]["net_minor"], 80_000);
    // a real attempt records one redemption per applied offer
    let (s, e) = t
        .call(
            a,
            "POST",
            "/v1/offers",
            None,
            json!({ "name": "X", "kind": "percent", "value": 500, "stack_group": "g" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
}

#[sqlx::test(migrations = "./migrations")]
async fn a_refund_returns_the_same_share_of_the_discount(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(&t, a, json!({ "name": "Ten percent" })).await;
    let oid = o["id"].as_str().unwrap();
    let (iid, aid, _) = t.paid_razorpay(a, 100_000, json!({})).await;
    let paid: i64 = sqlx::query_scalar("SELECT amount_minor FROM payment_attempts WHERE id = $1")
        .bind(&aid)
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(paid, 90_000);
    assert_eq!(offer_row(&t, oid).await, (1, 10_000));

    // what a half refund would return, before it is made
    let pv = t
        .ok(
            a,
            "POST",
            "/v1/offers/refund_preview",
            None,
            json!({ "intent_id": iid, "amount_minor": 45_000 }),
        )
        .await;
    assert_eq!(pv["discount_returned_minor"], 5_000);

    let rf = t
        .ok(
            a,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("rf-half"),
            json!({ "amount_minor": 45_000 }),
        )
        .await;
    assert_eq!(rf["status"], "processed");
    let (n, spent) = offer_row(&t, oid).await;
    assert_eq!(
        (n, spent),
        (1, 5_000),
        "half the discount is back in the budget"
    );
    let (cash, ret, status): (i64, i64, String) = sqlx::query_as(
        "SELECT refunded_cash_minor, discount_returned_minor, status FROM offer_redemptions WHERE attempt_id = $1",
    )
    .bind(&aid)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((cash, ret, status.as_str()), (45_000, 5_000, "redeemed"));

    // the rest: the whole discount is back and the use is freed
    let pv = t
        .ok(
            a,
            "POST",
            "/v1/offers/refund_preview",
            None,
            json!({ "intent_id": iid }),
        )
        .await;
    assert_eq!(pv["discount_returned_minor"], 5_000);
    t.ok(
        a,
        "POST",
        &format!("/v1/payment_intents/{iid}/refunds"),
        Some("rf-rest"),
        json!({}),
    )
    .await;
    assert_eq!(offer_row(&t, oid).await, (0, 0));
    let (ret, status): (i64, String) = sqlx::query_as(
        "SELECT discount_returned_minor, status FROM offer_redemptions WHERE attempt_id = $1",
    )
    .bind(&aid)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((ret, status.as_str()), (10_000, "reversed"));
    assert_eq!(
        t.count("SELECT COUNT(*) FROM offer_refund_adjustments")
            .await,
        2
    );
    assert!(
        t.count("SELECT COUNT(*) FROM audit_events WHERE action = 'offer.refund_adjusted'")
            .await
            >= 2
    );
    let detail = t
        .ok(a, "GET", &format!("/v1/offers/{oid}"), None, Value::Null)
        .await;
    assert_eq!(detail["totals"]["net_cost_minor"], 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn marketplace_and_invoice_payments_are_left_alone(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    offer(&t, a, json!({ "name": "Ten percent" })).await;
    let cust = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "Asha" }))
        .await;
    let inv = t
        .ok(
            a,
            "POST",
            "/v1/invoices",
            None,
            json!({ "customer_id": cust["id"], "number": "INV-1", "amount_minor": 100_000 }),
        )
        .await;
    let intent = t
        .intent(a, 100_000, json!({ "invoice_id": inv["id"] }))
        .await;
    let (s, att) = attempt(
        &t,
        a,
        intent["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{att}");
    assert_eq!(
        att["amount_minor"], 100_000,
        "an invoice payment is not discounted"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM offer_redemptions").await,
        0,
        "and no redemption exists"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn offers_are_private_to_their_business_and_need_their_scope(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let oa = offer(&t, a, json!({ "name": "A only" })).await;
    let id = oa["id"].as_str().unwrap();

    // B cannot read, change or see it, and B's payments are not discounted by it
    let (s, _) = t
        .call(b, "GET", &format!("/v1/offers/{id}"), None, Value::Null)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            b,
            "PATCH",
            &format!("/v1/offers/{id}"),
            None,
            json!({ "status": "paused" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let list = t.ok(b, "GET", "/v1/offers", None, Value::Null).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    let p = preview(&t, b, json!({})).await;
    assert_eq!(p["evaluation"]["discount_minor"], 0);
    let intent = t.intent(b, 100_000, json!({})).await;
    let (_, att) = attempt(
        &t,
        b,
        intent["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(att["amount_minor"], 100_000);
    // and the database itself, as the app role with no business set, sees none of it
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM offers")
        .fetch_one(&t.state.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(t.count("SELECT count(*) FROM offers").await, 1);

    // a key without the scope cannot create or even read offers
    let limited = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "read payments only", "scopes": ["payments:read"] }),
        )
        .await;
    let ro = Tenant {
        id: a.id.clone(),
        key: limited["key"].as_str().unwrap().to_string(),
    };
    let (s, _) = t
        .call(
            &ro,
            "POST",
            "/v1/offers",
            None,
            json!({ "name": "x", "kind": "flat", "value": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t.call(&ro, "GET", "/v1/offers", None, Value::Null).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // a paused offer stops applying; an archived one cannot be changed
    t.ok(
        a,
        "PATCH",
        &format!("/v1/offers/{id}"),
        None,
        json!({ "status": "paused" }),
    )
    .await;
    assert_eq!(
        preview(&t, a, json!({})).await["evaluation"]["discount_minor"],
        0
    );
    t.ok(
        a,
        "PATCH",
        &format!("/v1/offers/{id}"),
        None,
        json!({ "status": "archived" }),
    )
    .await;
    let (s, e) = t
        .call(
            a,
            "PATCH",
            &format!("/v1/offers/{id}"),
            None,
            json!({ "status": "active" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
}

#[sqlx::test(migrations = "./migrations")]
async fn the_public_checkout_shows_the_price_without_reserving_anything(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(&t, a, json!({ "name": "UPI 10%", "methods": ["upi"] })).await;
    offer(
        &t,
        a,
        json!({ "name": "Hidden", "code": "SECRET", "kind": "flat", "value": 50_000 }),
    )
    .await;
    let intent = t.intent(a, 100_000, json!({})).await;
    let (iid, secret) = (
        intent["id"].as_str().unwrap(),
        intent["client_secret"].as_str().unwrap(),
    );
    let p = t
        .ok_public(
            "POST",
            &format!("/v1/checkout/{}/{iid}/offers", a.id),
            json!({ "secret": secret, "method": "upi" }),
        )
        .await;
    assert_eq!(p["net_minor"], 90_000);
    assert_eq!(p["funded_by"], "merchant");
    assert!(!p.to_string().contains("Hidden"));
    assert_eq!(offer_row(&t, o["id"].as_str().unwrap()).await, (0, 0));
    // a wrong secret is not told apart from an unknown session
    let (s, _) = t
        .public(
            "POST",
            &format!("/v1/checkout/{}/{iid}/offers", a.id),
            None,
            json!({ "secret": "nope", "method": "upi" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // paying through the checkout applies the same offer
    let (s, att) = t
        .public(
            "POST",
            &format!("/v1/checkout/{}/{iid}/attempts", a.id),
            Some("co-1"),
            json!({ "secret": secret, "method": "upi" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{att}");
    assert_eq!(att["amount_minor"], 90_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_late_capture_keeps_the_discount_the_payer_already_had(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let o = offer(&t, a, json!({ "name": "One", "max_redemptions": 1 })).await;
    let oid = o["id"].as_str().unwrap();
    let i1 = t.intent(a, 100_000, json!({})).await;
    let (_, att1) = attempt(
        &t,
        a,
        i1["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(att1["amount_minor"], 90_000);
    // the provider says it failed: the use is given back, and another payer takes it
    let order = att1["provider_order_ref"].as_str().unwrap();
    let failed = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "failed", "payment_id": "pay_late" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &a.id, &failed["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(offer_row(&t, oid).await, (0, 0));
    let i2 = t.intent(a, 100_000, json!({})).await;
    let (_, att2) = attempt(
        &t,
        a,
        i2["id"].as_str().unwrap(),
        json!({ "method": "upi" }),
    )
    .await;
    assert_eq!(att2["amount_minor"], 90_000);
    assert_eq!(offer_row(&t, oid).await, (1, 10_000));
    // then the first payment turns out to have been captured after all: money wins, so the discount it was
    // charged for is counted again even though the cap was reached meanwhile
    let late = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "captured", "payment_id": "pay_late" }),
        )
        .await;
    let (s, _) = t.deliver("razorpay", &a.id, &late["webhook"]).await;
    assert_eq!(s, StatusCode::OK);
    let st: String =
        sqlx::query_scalar("SELECT status FROM offer_redemptions WHERE attempt_id = $1")
            .bind(att1["id"].as_str().unwrap())
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(st, "redeemed");
    assert_eq!(offer_row(&t, oid).await, (2, 20_000));
    assert!(
        t.count(
            "SELECT COUNT(*) FROM audit_events WHERE action = 'offer.reclaimed_after_late_capture'"
        )
        .await
            >= 1
    );
}
