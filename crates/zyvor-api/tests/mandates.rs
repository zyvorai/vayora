//! UPI AutoPay mandates against the simulator: approval, scheduled debits, the rules that keep a payer safe, and
//! the way a debit joins the ordinary payment, ledger and invoice paths. The suite's usual non-superuser role
//! applies, so row-level security is exercised too.

mod common;

use axum::http::StatusCode;
use chrono::{Days, NaiveDate};
use common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

fn today() -> NaiveDate {
    zyvor_api::ist_today()
}
fn days(n: u64) -> NaiveDate {
    today().checked_add_days(Days::new(n)).unwrap()
}

/// A tenant with the simulator connected, a customer, and a mandate the simulated payer has approved.
struct Fixture {
    customer: String,
    mandate: String,
}

async fn with_sim(t: &T, tn: &Tenant) {
    t.state.sim.set_manual_payer(true);
    t.ok(
        tn,
        "POST",
        "/v1/provider_accounts",
        None,
        json!({ "provider": "sim", "credentials": {}, "webhook_secret": "sim-secret-123456" }),
    )
    .await;
}

async fn customer(t: &T, tn: &Tenant) -> String {
    t.ok(
        tn,
        "POST",
        "/v1/customers",
        None,
        json!({ "name": "Asha Rao", "phone": "9876543210" }),
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn new_mandate(t: &T, tn: &Tenant, cust: &str, max: i64, freq: &str, key: &str) -> Value {
    t.ok(
        tn,
        "POST",
        "/v1/mandates",
        Some(key),
        json!({ "customer_id": cust, "max_amount_minor": max, "frequency": freq, "end_date": days(365).to_string() }),
    )
    .await
}

async fn active_mandate(t: &T, tn: &Tenant, max: i64, freq: &str) -> Fixture {
    with_sim(t, tn).await;
    let customer = customer(t, tn).await;
    let m = new_mandate(t, tn, &customer, max, freq, "m-1").await;
    let id = m["id"].as_str().unwrap().to_string();
    assert_eq!(m["status"], "pending_authorization");
    assert!(
        m["authorization"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("upi://mandate?")
    );
    // the payer approves in their UPI app; Zyvor learns by asking
    assert_eq!(
        t.state.sim.mandate_decide(&id, true),
        Some(zyvor_providers::MandateStatus::Active)
    );
    let r = t
        .ok(
            tn,
            "POST",
            &format!("/v1/mandates/{id}/refresh"),
            None,
            json!({}),
        )
        .await;
    assert_eq!(r["status"], "active");
    Fixture {
        customer,
        mandate: id,
    }
}

async fn debit(
    t: &T,
    tn: &Tenant,
    mandate: &str,
    amount: i64,
    due: NaiveDate,
    key: &str,
) -> (StatusCode, Value) {
    t.call(
        tn,
        "POST",
        &format!("/v1/mandates/{mandate}/debits"),
        Some(key),
        json!({ "amount_minor": amount, "due_date": due.to_string() }),
    )
    .await
}

/// Make a debit due now and its notice old enough, as if the days had passed.
async fn time_passes(t: &T, debit_id: &str) {
    sqlx::query("UPDATE mandate_debits SET due_date = $2, notified_at = now() - interval '2 days' WHERE id = $1")
        .bind(debit_id)
        .bind(today())
        .execute(&t.root)
        .await
        .unwrap();
}

async fn pass(t: &T, tn: &Tenant) -> Value {
    t.ok(tn, "POST", "/v1/mandates/run", None, json!({})).await
}

async fn events(t: &T, tn: &Tenant, ty: &str) -> i64 {
    t.count(&format!(
        "SELECT count(*) FROM outbox_events WHERE tenant_id = '{}' AND type = '{ty}'",
        tn.id
    ))
    .await
}

async fn debit_state(t: &T, id: &str) -> String {
    sqlx::query_scalar("SELECT state FROM mandate_debits WHERE id = $1")
        .bind(id)
        .fetch_one(&t.root)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn approved_then_debited_once_with_one_balanced_ledger_pair(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 200_000, "monthly").await;

    let (st, d) = debit(&t, a, &f.mandate, 150_000, days(4), "d-1").await;
    assert_eq!(st, StatusCode::CREATED, "{d}");
    assert_eq!(d["state"], "scheduled");
    let did = d["id"].as_str().unwrap().to_string();
    assert_eq!(events(&t, a, "mandate.debit_scheduled").await, 1);

    // the first pass tells the payer; nothing is sent while the debit is not yet due
    let r = pass(&t, a).await;
    assert_eq!(
        (r["notified"].as_i64(), r["initiated"].as_i64()),
        (Some(1), Some(0))
    );
    assert_eq!(debit_state(&t, &did).await, "notified");
    assert_eq!(events(&t, a, "mandate.debit_notified").await, 1);

    time_passes(&t, &did).await;
    let r = pass(&t, a).await;
    assert_eq!(r["initiated"].as_i64(), Some(1));
    assert_eq!(debit_state(&t, &did).await, "captured");

    // a second pass does not debit again
    let r = pass(&t, a).await;
    assert_eq!(r["initiated"].as_i64(), Some(0));
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}'",
            f.mandate
        ))
        .await,
        1
    );

    // the money is an ordinary captured payment: one balanced ledger pair and one event
    assert_eq!(events(&t, a, "payment.captured").await, 1);
    let (debits, credits): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor) FILTER (WHERE e.side = 'debit'), 0)::BIGINT, COALESCE(SUM(amount_minor) FILTER (WHERE e.side = 'credit'), 0)::BIGINT FROM ledger_entries e JOIN ledger_transactions x ON x.id = e.transaction_id WHERE x.reference LIKE 'capture:att_%' AND x.tenant_id = $1",
    )
    .bind(&a.id)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((debits, credits), (150_000, 150_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn the_rules_that_keep_a_payer_safe_are_enforced(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 100_000, "monthly").await;

    // over the cap, or nothing
    assert_eq!(
        debit(&t, a, &f.mandate, 100_001, days(4), "x1").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        debit(&t, a, &f.mandate, 0, days(4), "x2").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // the payer must be told ahead: today and tomorrow are too soon with a 24 hour notice
    for (n, key) in [(0, "x3"), (1, "x4")] {
        let (st, b) = debit(&t, a, &f.mandate, 50_000, days(n), key).await;
        assert_eq!(
            (st, b["error"]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("notice_too_short")),
            "{b}"
        );
    }
    // outside the mandate's dates
    let (st, b) = debit(&t, a, &f.mandate, 50_000, days(400), "x5").await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("outside_mandate_dates")
        ),
        "{b}"
    );

    // one debit is fine; a second in the same month is not, and neither is the same day twice
    let first = days(4);
    assert_eq!(
        debit(&t, a, &f.mandate, 50_000, first, "ok").await.0,
        StatusCode::CREATED
    );
    let (st, b) = debit(&t, a, &f.mandate, 50_000, first, "x6").await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("period_already_debited")),
        "{b}"
    );
    // the next month's period is open
    let next_month = zyvor_providers::Frequency::Monthly
        .period_end(first)
        .unwrap();
    assert_eq!(
        debit(&t, a, &f.mandate, 50_000, next_month, "ok2").await.0,
        StatusCode::CREATED
    );

    // the same request twice with the same key is one debit, not two
    let (_, r1) = debit(&t, a, &f.mandate, 20_000, days(40), "same").await;
    let (_, r2) = debit(&t, a, &f.mandate, 20_000, days(40), "same").await;
    assert_eq!(r1["id"], r2["id"]);

    // creation rules: ceiling, currency, dates, a customer that exists
    let c = &f.customer;
    let bad = |body: Value, key: &'static str| {
        let t = &t;
        async move { t.call(&t.a, "POST", "/v1/mandates", Some(key), body).await }
    };
    let (st, b) = bad(json!({ "customer_id": c, "max_amount_minor": 2_000_000, "frequency": "monthly", "end_date": days(30).to_string() }), "c1").await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("exceeds_mandate_ceiling")
        ),
        "{b}"
    );
    let (st, b) = bad(json!({ "customer_id": c, "max_amount_minor": 1000, "frequency": "monthly", "currency": "USD", "end_date": days(30).to_string() }), "c2").await;
    assert_eq!(
        (st, b["error"]["code"].as_str()),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("unsupported_currency")
        ),
        "{b}"
    );
    let (st, _) = bad(json!({ "customer_id": c, "max_amount_minor": 1000, "frequency": "hourly", "end_date": days(30).to_string() }), "c3").await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (st, _) = bad(json!({ "customer_id": c, "max_amount_minor": 1000, "frequency": "monthly", "end_date": today().to_string() }), "c4").await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    let (st, _) = bad(json!({ "customer_id": "cus_nope", "max_amount_minor": 1000, "frequency": "monthly", "end_date": days(30).to_string() }), "c5").await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_paused_or_revoked_mandate_never_debits(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 200_000, "monthly").await;
    let (_, d) = debit(&t, a, &f.mandate, 100_000, days(4), "d-1").await;
    let did = d["id"].as_str().unwrap().to_string();
    pass(&t, a).await; // notice goes out
    time_passes(&t, &did).await;

    // paused: due and noticed, but it must not go out
    t.ok(
        a,
        "POST",
        &format!("/v1/mandates/{}/pause", f.mandate),
        None,
        json!({}),
    )
    .await;
    assert_eq!(pass(&t, a).await["initiated"].as_i64(), Some(0));
    assert_eq!(debit_state(&t, &did).await, "notified");
    // pausing twice, or resuming something not paused, is refused
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/mandates/{}/pause", f.mandate),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    t.ok(
        a,
        "POST",
        &format!("/v1/mandates/{}/resume", f.mandate),
        None,
        json!({}),
    )
    .await;
    assert_eq!(
        t.call(
            a,
            "POST",
            &format!("/v1/mandates/{}/resume", f.mandate),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // revoked: the debit is cancelled with it and nothing is ever sent
    let r = t
        .ok(
            a,
            "POST",
            &format!("/v1/mandates/{}/revoke", f.mandate),
            None,
            json!({}),
        )
        .await;
    assert_eq!(r["status"], "revoked");
    assert_eq!(debit_state(&t, &did).await, "skipped");
    assert_eq!(pass(&t, a).await["initiated"].as_i64(), Some(0));
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}'",
            f.mandate
        ))
        .await,
        0
    );
    assert_eq!(events(&t, a, "mandate.revoked").await, 1);
    assert_eq!(
        debit(&t, a, &f.mandate, 1000, days(9), "d-2").await.0,
        StatusCode::CONFLICT,
        "no new debits on a revoked mandate"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payer_revoke_is_learned_on_refresh_and_cancels_scheduled_debits(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 200_000, "monthly").await;
    let (_, d) = debit(&t, a, &f.mandate, 100_000, days(4), "d-1").await;
    let did = d["id"].as_str().unwrap().to_string();

    // the payer revokes in their own UPI app: Zyvor knows nothing until it asks
    assert!(t.state.sim.payer_revoke(&f.mandate).is_some());
    let m = t
        .ok(
            a,
            "GET",
            &format!("/v1/mandates/{}", f.mandate),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(m["mandate"]["status"], "active");
    let r = t
        .ok(
            a,
            "POST",
            &format!("/v1/mandates/{}/refresh", f.mandate),
            None,
            json!({}),
        )
        .await;
    assert_eq!(
        (r["status"].as_str(), r["revoked_by"].as_str()),
        (Some("revoked"), Some("payer"))
    );
    assert_eq!(debit_state(&t, &did).await, "skipped");
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_debit_is_resolved_by_asking_never_by_sending_again(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 1_000_000, "monthly").await;
    // an amount ending in 07 is accepted by the simulator but answered with a timeout
    let (_, d) = debit(&t, a, &f.mandate, 150_007, days(4), "d-1").await;
    let did = d["id"].as_str().unwrap().to_string();
    pass(&t, a).await;
    time_passes(&t, &did).await;
    assert_eq!(pass(&t, a).await["initiated"].as_i64(), Some(1));

    let status: String =
        sqlx::query_scalar("SELECT status FROM payment_attempts WHERE mandate_id = $1")
            .bind(&f.mandate)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(status, "unknown");
    assert_eq!(debit_state(&t, &did).await, "initiated");
    assert_eq!(
        t.count("SELECT count(*) FROM recon_cases WHERE kind = 'unresolved_payment'")
            .await,
        1
    );

    // more passes do not send it again
    for _ in 0..3 {
        assert_eq!(pass(&t, a).await["initiated"].as_i64(), Some(0));
    }
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}'",
            f.mandate
        ))
        .await,
        1
    );

    // the existing resolver asks the provider by our own reference, and the debit follows the answer
    let client = zyvor_api::outbox::http_client();
    zyvor_api::worker::run_cycle(&t.state, &client)
        .await
        .unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM payment_attempts WHERE mandate_id = $1")
            .bind(&f.mandate)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(status, "captured");
    assert_eq!(debit_state(&t, &did).await, "captured");
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}'",
            f.mandate
        ))
        .await,
        1,
        "still one attempt"
    );
    assert_eq!(events(&t, a, "payment.captured").await, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_declined_debit_retries_once_then_fails_and_says_so(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 1_000_000, "monthly").await;
    // an amount ending in 13 is declined for insufficient funds
    let (_, d) = debit(&t, a, &f.mandate, 150_013, days(4), "d-1").await;
    let did = d["id"].as_str().unwrap().to_string();
    pass(&t, a).await;
    time_passes(&t, &did).await;

    pass(&t, a).await;
    let (state, retry, due): (String, i32, NaiveDate) =
        sqlx::query_as("SELECT state, retry_no, due_date FROM mandate_debits WHERE id = $1")
            .bind(&did)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(
        (state.as_str(), retry, due),
        ("notified", 1, days(1)),
        "one retry, the next day"
    );
    assert_eq!(events(&t, a, "mandate.debit_failed").await, 0);

    time_passes(&t, &did).await; // the retry day arrives
    pass(&t, a).await;
    assert_eq!(debit_state(&t, &did).await, "failed");
    assert_eq!(events(&t, a, "mandate.debit_failed").await, 1);
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}'",
            f.mandate
        ))
        .await,
        2,
        "the original and one retry, never more"
    );
    assert_eq!(
        t.count(&format!(
            "SELECT count(*) FROM payment_attempts WHERE mandate_id = '{}' AND status = 'captured'",
            f.mandate
        ))
        .await,
        0
    );
    // a failed day can be scheduled again
    assert_eq!(
        debit(&t, a, &f.mandate, 100_000, days(5), "d-2").await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_subscription_with_a_mandate_collects_its_own_invoice(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    let f = active_mandate(&t, a, 200_000, "monthly").await;
    let plan = t
        .ok(
            a,
            "POST",
            "/v1/plans",
            None,
            json!({ "name": "Pro", "amount_minor": 99_900, "interval_unit": "month" }),
        )
        .await;
    let sub = t
        .ok(
            a,
            "POST",
            "/v1/subscriptions",
            None,
            json!({ "customer_id": f.customer, "plan_id": plan["id"] }),
        )
        .await;
    let sid = sub["id"].as_str().unwrap();
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{sid}/mandate"),
        None,
        json!({ "mandate_id": f.mandate }),
    )
    .await;

    // billing raises the invoice and the debit is scheduled with it
    t.ok(a, "POST", "/v1/billing/run", None, json!({})).await;
    let (inv, status): (String, String) =
        sqlx::query_as("SELECT id, status FROM invoices WHERE subscription_id = $1")
            .bind(sid)
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(status, "open");
    let (did, amount, invoice): (String, i64, Option<String>) = sqlx::query_as(
        "SELECT id, amount_minor, invoice_id FROM mandate_debits WHERE mandate_id = $1",
    )
    .bind(&f.mandate)
    .fetch_one(&t.root)
    .await
    .unwrap();
    assert_eq!((amount, invoice.as_deref()), (99_900, Some(inv.as_str())));

    pass(&t, a).await;
    time_passes(&t, &did).await;
    pass(&t, a).await;
    let status: String = sqlx::query_scalar("SELECT status FROM invoices WHERE id = $1")
        .bind(&inv)
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(
        status, "paid",
        "the captured debit paid the invoice through the ordinary path"
    );

    // a customer's mandate cannot be linked to someone else's subscription
    let other = t
        .ok(a, "POST", "/v1/customers", None, json!({ "name": "Ravi" }))
        .await;
    let sub2 = t
        .ok(
            a,
            "POST",
            "/v1/subscriptions",
            None,
            json!({ "customer_id": other["id"], "plan_id": plan["id"] }),
        )
        .await;
    let (st, _) = t
        .call(
            a,
            "POST",
            &format!("/v1/subscriptions/{}/mandate", sub2["id"].as_str().unwrap()),
            None,
            json!({ "mandate_id": f.mandate }),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_mandate_without_an_active_mandate_leaves_the_invoice_to_the_usual_reminders(
    pool: PgPool,
) {
    let t = setup(pool).await;
    let a = &t.a;
    with_sim(&t, a).await;
    let cust = customer(&t, a).await;
    // pending, never approved
    let m = new_mandate(&t, a, &cust, 200_000, "monthly", "m-1").await;
    let plan = t
        .ok(
            a,
            "POST",
            "/v1/plans",
            None,
            json!({ "name": "Pro", "amount_minor": 99_900, "interval_unit": "month" }),
        )
        .await;
    let sub = t
        .ok(
            a,
            "POST",
            "/v1/subscriptions",
            None,
            json!({ "customer_id": cust, "plan_id": plan["id"] }),
        )
        .await;
    t.ok(
        a,
        "POST",
        &format!("/v1/subscriptions/{}/mandate", sub["id"].as_str().unwrap()),
        None,
        json!({ "mandate_id": m["id"] }),
    )
    .await;
    t.ok(a, "POST", "/v1/billing/run", None, json!({})).await;
    assert_eq!(
        t.count("SELECT count(*) FROM invoices").await,
        1,
        "billing still works"
    );
    assert_eq!(
        t.count("SELECT count(*) FROM mandate_debits").await,
        0,
        "no debit without an approved mandate"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn mandates_are_private_to_their_tenant_scoped_and_the_sim_payer_is_gated(pool: PgPool) {
    let t = setup(pool).await;
    let (a, b) = (&t.a, &t.b);
    let f = active_mandate(&t, a, 200_000, "monthly").await;

    // another tenant sees nothing, by id or in a list, and cannot act on it
    assert_eq!(
        t.call(
            b,
            "GET",
            &format!("/v1/mandates/{}", f.mandate),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.ok(b, "GET", "/v1/mandates", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        t.call(
            b,
            "POST",
            &format!("/v1/mandates/{}/revoke", f.mandate),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        debit(&t, b, &f.mandate, 1000, days(5), "b-1").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.ok(a, "GET", "/v1/mandates", None, Value::Null).await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // a key without the scope can read nothing and create nothing
    let k = t
        .ok(
            a,
            "POST",
            "/v1/api_keys",
            None,
            json!({ "name": "payments only", "scopes": ["payments:read"] }),
        )
        .await;
    let narrow = Tenant {
        id: a.id.clone(),
        key: k["key"].as_str().unwrap().to_string(),
    };
    assert_eq!(
        t.call(&narrow, "GET", "/v1/mandates", None, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        t.call(&narrow, "POST", "/v1/mandates", Some("n1"), json!({ "customer_id": f.customer, "max_amount_minor": 1000, "frequency": "monthly", "end_date": days(30).to_string() }))
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // the simulated payer is off without ZYVOR_SIM_PAYER=1, and refuses links that are not the simulator's
    let (st, _) = t
        .public(
            "POST",
            "/v1/sim/mandate",
            None,
            json!({ "upi_uri": "upi://mandate?pa=sandbox@zyvor-sim&tn=x" }),
        )
        .await;
    assert!(
        st == StatusCode::NOT_FOUND || st == StatusCode::UNAUTHORIZED,
        "got {st}"
    );
    assert!(!t.state.sim_payer);

    // creating a mandate needs a connected provider that supports them
    let (st, b2) = t.call(b, "POST", "/v1/mandates", Some("b-m"), json!({ "customer_id": "x", "max_amount_minor": 1000, "frequency": "monthly", "end_date": days(30).to_string() })).await;
    assert!(
        st == StatusCode::NOT_FOUND || st == StatusCode::UNPROCESSABLE_ENTITY,
        "{st} {b2}"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn creating_the_same_mandate_twice_with_one_key_makes_one(pool: PgPool) {
    let t = setup(pool).await;
    let a = &t.a;
    with_sim(&t, a).await;
    let cust = customer(&t, a).await;
    let m1 = new_mandate(&t, a, &cust, 100_000, "weekly", "same-key").await;
    let m2 = new_mandate(&t, a, &cust, 100_000, "weekly", "same-key").await;
    assert_eq!(m1["id"], m2["id"]);
    assert_eq!(t.count("SELECT count(*) FROM mandates").await, 1);
    // no key, no mandate
    let (st, _) = t.call(a, "POST", "/v1/mandates", None, json!({ "customer_id": cust, "max_amount_minor": 1000, "frequency": "weekly", "end_date": days(30).to_string() })).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}
