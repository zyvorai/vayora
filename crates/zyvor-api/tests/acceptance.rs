//! Acceptance tests for the money-safety guarantees. Providers are the local mock (SIMULATED).
#![allow(clippy::type_complexity)]

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;
use zyvor_api::{outbox, worker};
use zyvor_core::Currency;
use zyvor_ledger::{Entry, Transaction};

// 1 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn repeating_an_idempotent_request_cannot_create_a_second_operation(pool: PgPool) {
    let t = setup(pool).await;
    let body =
        json!({ "amount_minor": 49_900, "currency": "INR", "allowed_providers": ["razorpay"] });
    let (s1, a) = t
        .call(
            &t.a,
            "POST",
            "/v1/payment_intents",
            Some("same"),
            body.clone(),
        )
        .await;
    let (s2, b) = t
        .call(
            &t.a,
            "POST",
            "/v1/payment_intents",
            Some("same"),
            body.clone(),
        )
        .await;
    assert_eq!((s1, s2), (StatusCode::CREATED, StatusCode::CREATED));
    assert_eq!(a["id"], b["id"], "replay returns the original");
    assert_eq!(t.count("SELECT COUNT(*) FROM payment_intents").await, 1);
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            "/v1/payment_intents",
            Some("same"),
            json!({ "amount_minor": 1_000, "currency": "INR" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("idempotency_key_reused")
        )
    );
    assert_eq!(
        t.call(&t.a, "POST", "/v1/payment_intents", None, body)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // attempts: eight concurrent identical requests create exactly one attempt and one provider order
    let iid = a["id"].as_str().unwrap().to_string();
    let calls: Vec<_> = (0..8)
        .map(|_| {
            let (app, key, iid) = (t.app.clone(), t.a.key.clone(), iid.clone());
            tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    None,
                    "POST",
                    &format!("/v1/payment_intents/{iid}/attempts"),
                    Some("att-race"),
                    json!({ "method": "upi" }),
                )
                .await
            })
        })
        .collect();
    let mut ids = std::collections::BTreeSet::new();
    for c in calls {
        let (s, b) = c.await.unwrap();
        assert_eq!(s, StatusCode::CREATED, "{b}");
        ids.insert(b["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids.len(), 1);
    assert_eq!(t.count("SELECT COUNT(*) FROM payment_attempts").await, 1);
    assert_eq!(t.mock_stats().await["razorpay_payments"], 0);

    // refunds: a repeated request refunds once
    let (iid2, _, _) = t.paid_razorpay(&t.a, 50_000, json!({})).await;
    let r = |k: &'static str| {
        t.call(
            &t.a,
            "POST",
            Box::leak(format!("/v1/payment_intents/{iid2}/refunds").into_boxed_str()),
            Some(k),
            json!({ "amount_minor": 10_000 }),
        )
    };
    let (r1, r2) = (r("rf-1").await, r("rf-1").await);
    assert_eq!(r1.1["id"], r2.1["id"]);
    assert_eq!(t.count("SELECT COUNT(*) FROM refunds").await, 1);
    assert_eq!(
        t.count("SELECT refunded_minor FROM payment_attempts WHERE status = 'captured'")
            .await,
        10_000
    );
}

// 2 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn duplicate_and_out_of_order_webhooks_cannot_duplicate_postings_or_regress_state(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let intent = t.intent(&t.a, 49_900, json!({})).await;
    let iid = intent["id"].as_str().unwrap();
    let att = t
        .ok(
            &t.a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("a1"),
            json!({ "method": "card" }),
        )
        .await;
    assert_eq!(att["status"], "pending");
    assert_eq!(
        att["client_action"]["type"], "hosted_checkout",
        "card details are collected by the provider"
    );
    let order = att["provider_order_ref"].as_str().unwrap();

    let authorized = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "authorized" }),
        )
        .await;
    let pid = authorized["payment"]["id"].as_str().unwrap().to_string();
    let captured = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "captured", "payment_id": pid }),
        )
        .await;
    let other_failed = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "failed" }),
        )
        .await; // customer retried inside checkout; a different provider payment failed

    // arrival order: captured, authorized (late), captured (duplicate), failed (late, other payment), authorized (late again)
    let r1 = t.deliver("razorpay", &t.a.id, &captured["webhook"]).await;
    let r2 = t.deliver("razorpay", &t.a.id, &authorized["webhook"]).await;
    let r3 = t.deliver("razorpay", &t.a.id, &captured["webhook"]).await;
    let r4 = t
        .deliver("razorpay", &t.a.id, &other_failed["webhook"])
        .await;
    let r5 = t.deliver("razorpay", &t.a.id, &authorized["webhook"]).await;
    assert!(
        r1.1["outcome"].as_str().unwrap().contains("Captured"),
        "{:?}",
        r1.1
    );
    assert_eq!(
        r2.1["outcome"], "Ignored",
        "a late authorized must not regress captured"
    );
    assert_eq!(r3.1["status"], "duplicate");
    assert_eq!(r4.1["outcome"], "Ignored");
    assert_eq!(r5.1["status"], "duplicate");

    let cur = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/payment_intents/{iid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cur["intent"]["status"], "succeeded");
    assert_eq!(cur["attempts"][0]["status"], "captured");
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions WHERE reference LIKE 'capture:%'")
            .await,
        1
    );
    assert_eq!(t.count("SELECT COUNT(*) FROM ledger_entries").await, 2);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'payment.captured'")
            .await,
        1
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE type = 'payment.authorized'")
            .await,
        0
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM webhook_inbox").await,
        3,
        "three distinct provider events were recorded"
    );

    // signature and tenant checks
    let mut forged = captured["webhook"].clone();
    forged["body"] = json!(forged["body"].as_str().unwrap().replace("49900", "1"));
    assert_eq!(
        t.deliver("razorpay", &t.a.id, &forged).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.deliver("razorpay", "ten_nope", &captured["webhook"])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

// 3 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn a_timeout_after_provider_acceptance_is_unresolved_never_a_second_charge(pool: PgPool) {
    let t = setup(pool).await;
    let intent = t
        .intent(&t.a, 49_900, json!({ "allowed_providers": ["cashfree"] }))
        .await;
    let iid = intent["id"].as_str().unwrap();

    // Cashfree accepts the UPI collect, then the response never arrives.
    t.mock_post(
        "behaviour",
        json!({ "op": "cashfree.pay", "behaviour": "hang_after_accept" }),
    )
    .await;
    let att = t
        .ok(
            &t.a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("a1"),
            json!({ "method": "upi", "vpa": "asha@okvayora" }),
        )
        .await;
    assert_eq!(
        att["status"], "unknown",
        "outcome unknown, not failed, not captured"
    );
    let aid = att["id"].as_str().unwrap();

    // another payment (through ANY provider) is refused until this resolves
    let (s, e) = t
        .call(
            &t.a,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("a2"),
            json!({ "method": "upi", "vpa": "asha@okvayora" }),
        )
        .await;
    assert_eq!(
        (s, e["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("payment_outcome_unresolved"))
    );
    assert_eq!(
        t.mock_stats().await["cashfree_payments"],
        1,
        "exactly one collect exists at the provider"
    );
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'unresolved_payment' AND status = 'open'"
        )
        .await,
        1,
        "it shows up as work"
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_entries").await,
        0,
        "no money recorded while unknown"
    );

    // the payer approves; the worker asks the provider and adopts the real outcome
    t.mock_post(
        "cashfree/pay",
        json!({ "order_id": aid, "outcome": "SUCCESS" }),
    )
    .await;
    let client = outbox::http_client();
    let stats = worker::run_cycle(&t.state, &client).await.unwrap();
    assert!(stats.attempts_checked >= 1);
    let cur = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/payment_attempts/{aid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cur["status"], "captured");
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions").await,
        1,
        "posted once"
    );
    assert_eq!(
        t.count(
            "SELECT COUNT(*) FROM recon_cases WHERE kind = 'unresolved_payment' AND status = 'open'"
        )
        .await,
        0,
        "the task closed itself"
    );
    assert_eq!(
        t.mock_stats().await["charges"]["cashfree"],
        1,
        "the payer was charged exactly once"
    );
    worker::run_cycle(&t.state, &client).await.unwrap();
    assert_eq!(
        t.count("SELECT COUNT(*) FROM ledger_transactions").await,
        1,
        "re-running the resolver changes nothing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_attempt_the_provider_never_saw_resolves_to_failed_and_unblocks_the_intent(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let intent = t
        .intent(&t.a, 20_000, json!({ "allowed_providers": ["cashfree"] }))
        .await;
    let iid = intent["id"].as_str().unwrap();
    // simulate a crash after we persisted the attempt but before the provider call
    sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route) VALUES ('att_crash', $1, $2, 'cashfree', 'upi', 'created', 20000, 'INR', '{}')")
        .bind(&t.a.id)
        .bind(iid)
        .execute(&t.root)
        .await
        .unwrap();
    sqlx::query("UPDATE payment_intents SET status = 'processing' WHERE id = $1")
        .bind(iid)
        .execute(&t.root)
        .await
        .unwrap();
    worker::run_cycle(&t.state, &outbox::http_client())
        .await
        .unwrap();
    let cur = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/payment_intents/{iid}"),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(cur["attempts"][0]["status"], "failed");
    assert_eq!(cur["intent"]["status"], "requires_payment_method");
    t.ok(
        &t.a,
        "POST",
        &format!("/v1/payment_intents/{iid}/attempts"),
        Some("retry"),
        json!({ "method": "card" }),
    )
    .await;
}

// 4 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn concurrent_partial_refunds_cannot_exceed_the_refundable_amount(pool: PgPool) {
    let t = setup(pool).await;
    let (iid, aid, _) = t.paid_razorpay(&t.a, 100_000, json!({})).await;
    let calls: Vec<_> = (0..12)
        .map(|i| {
            let (app, key, iid) = (t.app.clone(), t.a.key.clone(), iid.clone());
            tokio::spawn(async move {
                raw(
                    &app,
                    &key,
                    None,
                    "POST",
                    &format!("/v1/payment_intents/{iid}/refunds"),
                    Some(&format!("rf-{i}")),
                    json!({ "amount_minor": 15_000 }),
                )
                .await
            })
        })
        .collect();
    let (mut ok, mut rejected) = (0, 0);
    for c in calls {
        match c.await.unwrap() {
            (StatusCode::CREATED, b) => {
                assert_eq!(b["status"], "processed");
                ok += 1
            }
            (StatusCode::UNPROCESSABLE_ENTITY, b) => {
                assert_eq!(b["error"]["code"], "refund_exceeds_refundable");
                rejected += 1
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((ok, rejected), (6, 6), "100k covers six refunds of 15k");
    let (refunded, reserved): (i64, i64) = sqlx::query_as(
        "SELECT refunded_minor, reserved_refund_minor FROM payment_attempts WHERE id = $1",
    )
    .bind(&aid)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((refunded, reserved), (90_000, 0));
    // the ledger agrees with the payment: capture 100k debit; refunds 90k credit on the provider receivable
    let credits: i64 = t.count("SELECT COALESCE(SUM(amount_minor),0)::BIGINT FROM ledger_entries WHERE side = 'credit' AND account LIKE 'provider_receivable:%'").await;
    assert_eq!(credits, 90_000);
    // and the database itself refuses an over-refund even if code were wrong
    let bad = sqlx::query("UPDATE payment_attempts SET refunded_minor = 100001 WHERE id = $1")
        .bind(&aid)
        .execute(&t.root)
        .await;
    assert!(bad.is_err());
}

// 5 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn ledger_balances_per_currency_corrections_are_reversals_and_history_is_immutable(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let tenant = &t.a.id;
    // the DB refuses to commit an unbalanced transaction, even if application code is bypassed
    let mut tx = t.root.begin().await.unwrap();
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO ledger_transactions (id, tenant_id, reference) VALUES ('jt_bad', $1, 'bad')",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO ledger_entries (tenant_id, transaction_id, account, side, amount_minor, currency) VALUES ($1, 'jt_bad', 'a', 'debit', 100, 'INR'), ($1, 'jt_bad', 'b', 'credit', 100, 'USD')").bind(tenant).execute(&mut *tx).await.unwrap();
    let err = tx.commit().await.unwrap_err().to_string();
    assert!(err.contains("does not balance"), "{err}");

    // a correct posting, then its reversal: net zero, both on the record
    let txn = Transaction::new(vec![
        Entry::debit("bank", 7_000, Currency::Inr),
        Entry::credit("collections", 7_000, Currency::Inr),
    ])
    .unwrap();
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, tenant)
        .await
        .unwrap();
    let id = txn.post(&mut tx, tenant, "ref-1", None).await.unwrap();
    tx.commit().await.unwrap();
    let mut dup = zyvor_api::tenant::begin(&t.state.pool, tenant)
        .await
        .unwrap();
    assert!(
        txn.post(&mut dup, tenant, "ref-1", None).await.is_err(),
        "a reference can only be posted once"
    );
    drop(dup);
    let mut tx = zyvor_api::tenant::begin(&t.state.pool, tenant)
        .await
        .unwrap();
    txn.reversal()
        .post(&mut tx, tenant, "ref-1:reversal", Some(&id))
        .await
        .unwrap();
    let (d, c) = zyvor_ledger::totals(&mut tx, tenant, "bank", Currency::Inr)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!((d, c), (7_000, 7_000));

    for sql in [
        "UPDATE ledger_entries SET amount_minor = 1",
        "DELETE FROM ledger_entries",
        "DELETE FROM ledger_transactions",
        "UPDATE audit_events SET actor = 'x'",
    ] {
        assert!(
            sqlx::query(sql).execute(&t.root).await.is_err(),
            "{sql} must be refused"
        );
    }
}

// 6 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn cross_tenant_access_fails_across_apis_exports_jobs_webhooks_and_the_database(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let (iid, aid, _) = t.paid_razorpay(&t.a, 50_000, json!({})).await;
    let cust = t
        .ok(
            &t.a,
            "POST",
            "/v1/customers",
            None,
            json!({ "name": "Asha" }),
        )
        .await;

    // APIs: tenant B sees nothing of A's
    for (m, p) in [
        ("GET", format!("/v1/payment_intents/{iid}")),
        ("GET", format!("/v1/payment_attempts/{aid}")),
        ("POST", format!("/v1/payment_attempts/{aid}/refresh")),
        (
            "GET",
            format!("/v1/customers/{}", cust["id"].as_str().unwrap()),
        ),
    ] {
        assert_eq!(
            t.call(&t.b, m, &p, None, Value::Null).await.0,
            StatusCode::NOT_FOUND,
            "{m} {p}"
        );
    }
    assert_eq!(
        t.call(
            &t.b,
            "POST",
            &format!("/v1/payment_intents/{iid}/refunds"),
            Some("x"),
            json!({ "amount_minor": 100 })
        )
        .await
        .0,
        StatusCode::CONFLICT,
        "B cannot refund A's payment (it does not exist for B)"
    );
    assert_eq!(
        t.call(
            &t.b,
            "POST",
            &format!("/v1/payment_intents/{iid}/attempts"),
            Some("y"),
            json!({ "method": "upi" })
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let list = t
        .ok(&t.b, "GET", "/v1/payment_intents", None, Value::Null)
        .await;
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
    // B cannot even reference A's customer
    let (s, _) = t
        .call(
            &t.b,
            "POST",
            "/v1/payment_intents",
            Some("z"),
            json!({ "amount_minor": 100, "currency": "INR", "customer_id": cust["id"] }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // exports
    let export = t.ok(&t.b, "GET", "/v1/export", None, Value::Null).await;
    for k in [
        "payment_intents",
        "payment_attempts",
        "ledger_entries",
        "customers",
    ] {
        assert_eq!(export[k].as_array().unwrap().len(), 0, "{k}");
    }
    assert_eq!(
        t.ok(&t.a, "GET", "/v1/export", None, Value::Null).await["payment_intents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // webhooks: A's genuine webhook is rejected on B's URL (B holds a different secret)
    let o = t
        .ok(
            &t.a,
            "GET",
            &format!("/v1/payment_intents/{iid}"),
            None,
            Value::Null,
        )
        .await;
    let order = o["attempts"][0]["provider_order_ref"].as_str().unwrap();
    let ev = t
        .mock_post(
            "razorpay/pay",
            json!({ "order_id": order, "outcome": "captured" }),
        )
        .await;
    assert_eq!(
        t.deliver("razorpay", &t.b.id, &ev["webhook"]).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.count("SELECT COUNT(*) FROM webhook_inbox").await,
        1,
        "the forged delivery to B left no trace"
    );

    // jobs: each tenant's events go only to that tenant's endpoint
    let (recv_a, hits_a) = receiver().await;
    let (recv_b, hits_b) = receiver().await;
    for (tn, url) in [(&t.a, &recv_a), (&t.b, &recv_b)] {
        t.ok(
            tn,
            "POST",
            "/v1/settings",
            None,
            json!({ "outbound_webhook_url": url, "outbound_webhook_secret": "whsec_out" }),
        )
        .await;
    }
    t.ok(
        &t.b,
        "POST",
        "/v1/customers",
        None,
        json!({ "name": "B only" }),
    )
    .await;
    let bi = t.intent(&t.b, 1_000, json!({})).await; // B has its own event stream
    t.ok(
        &t.b,
        "POST",
        &format!(
            "/v1/payment_intents/{}/attempts",
            bi["id"].as_str().unwrap()
        ),
        Some("b1"),
        json!({ "method": "upi" }),
    )
    .await;
    worker::run_cycle(&t.state, &outbox::http_client())
        .await
        .unwrap();
    let a_events: Vec<Value> = hits_a
        .lock()
        .unwrap()
        .iter()
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    assert!(a_events.iter().any(|e| e["type"] == "payment.captured"));
    assert!(
        a_events
            .iter()
            .all(|e| e["data"]["intent_id"] == iid.as_str()),
        "A's endpoint received only A's events: {a_events:?}"
    );
    assert!(
        hits_b
            .lock()
            .unwrap()
            .iter()
            .all(|b| !String::from_utf8_lossy(b).contains(&iid)),
        "B's endpoint never saw A's data"
    );

    // database: row-level security hides A's rows from B's scope, and from an unscoped session
    let mut tx = t.state.pool.begin().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payment_intents")
            .fetch_one(&mut *tx)
            .await
            .unwrap(),
        0,
        "no scope, no rows"
    );
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(&t.b.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payment_intents WHERE id = $1")
            .bind(&iid)
            .fetch_one(&mut *tx)
            .await
            .unwrap(),
        0
    );
    assert!(
        sqlx::query("INSERT INTO customers (id, tenant_id, name) VALUES ('c_x', $1, 'smuggled')")
            .bind(&t.a.id)
            .execute(&mut *tx)
            .await
            .is_err(),
        "cannot write into another tenant"
    );
}

/// Merchant endpoint: records bodies, always answers 200.
async fn receiver() -> (String, std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>) {
    let hits: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Default::default();
    let h = hits.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(move |body: axum::body::Bytes| {
            let h = h.clone();
            async move {
                h.lock().unwrap().push(body.to_vec());
                StatusCode::OK
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, hits)
}

// 7 ───────────────────────────────────────────────────────────────────────────
#[sqlx::test(migrations = "./migrations")]
async fn worker_crashes_and_restarts_do_not_lose_acknowledged_events(pool: PgPool) {
    let t = setup(pool).await;
    t.paid_razorpay(&t.a, 50_000, json!({})).await;
    t.paid_razorpay(&t.a, 30_000, json!({})).await;
    let total = t.count("SELECT COUNT(*) FROM outbox_events").await;
    assert!(total >= 2);

    // flaky receiver: fails the first delivery, accepts the rest
    let hits: std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>> = Default::default();
    let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (h, n2) = (hits.clone(), n.clone());
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let (h, n) = (h.clone(), n2.clone());
                async move {
                    if n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        return StatusCode::INTERNAL_SERVER_ERROR;
                    }
                    h.lock().unwrap().push((
                        headers["zyvor-signature"].to_str().unwrap().to_string(),
                        body.to_vec(),
                    ));
                    StatusCode::OK
                }
            },
        ),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    t.ok(
        &t.a,
        "POST",
        "/v1/settings",
        None,
        json!({ "outbound_webhook_url": url, "outbound_webhook_secret": "whsec_out" }),
    )
    .await;

    // a worker takes the batch, then dies before recording anything
    {
        let mut tx = zyvor_api::tenant::begin(&t.state.pool, &t.a.id)
            .await
            .unwrap();
        let locked: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM outbox_events WHERE status = 'pending' FOR UPDATE SKIP LOCKED",
        )
        .fetch_all(&mut *tx)
        .await
        .unwrap();
        assert_eq!(locked.len() as i64, total);
        // tx dropped here without commit == crash
    }
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE status = 'pending'")
            .await,
        total,
        "nothing was lost or marked delivered"
    );

    // restart: first delivery fails (stays pending, backoff), the rest succeed
    let client = outbox::http_client();
    let d1 = outbox::deliver_tenant(&t.state.pool, &t.state.crypto, &client, &t.a.id)
        .await
        .unwrap() as i64;
    assert_eq!(d1, total - 1);
    sqlx::query("UPDATE outbox_events SET next_attempt_at = now() WHERE status = 'pending'")
        .execute(&t.root)
        .await
        .unwrap();
    let d2 = outbox::deliver_tenant(&t.state.pool, &t.state.crypto, &client, &t.a.id)
        .await
        .unwrap() as i64;
    assert_eq!(d1 + d2, total);
    assert_eq!(
        t.count("SELECT COUNT(*) FROM outbox_events WHERE status = 'delivered'")
            .await,
        total
    );
    let hits = hits.lock().unwrap();
    assert_eq!(hits.len() as i64, total);
    let now = chrono::Utc::now().timestamp();
    assert!(
        hits.iter()
            .all(|(sig, body)| outbox::verify(b"whsec_out", sig, body, now, 300)),
        "every delivery is signed"
    );
}
