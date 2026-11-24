// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `GET /v1/outbox/stats`, `POST /v1/outbox/replay_dead`, `GET /v1/settlements` filters and
//! `GET /v1/decline_advice` / `GET /v1/payment_attempts/{id}/retry_advice` through the real router.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;

async fn event(t: &T, tenant: &str, id: &str, ty: &str, status: &str, age_mins: i32) {
    sqlx::query("INSERT INTO outbox_events (id, tenant_id, type, payload, status, attempts, last_error, created_at) VALUES ($1, $2, $3, '{}', $4, 10, 'boom', now() - make_interval(mins => $5))")
        .bind(id).bind(tenant).bind(ty).bind(status).bind(age_mins)
        .execute(&t.root).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn stats_count_per_status_and_stay_in_the_tenant(root: PgPool) {
    let t = setup(root.clone()).await;
    event(&t, &t.a.id, "evt_a1", "payment.captured", "dead", 30).await;
    event(&t, &t.a.id, "evt_a2", "payment.captured", "dead", 20).await;
    event(&t, &t.a.id, "evt_a3", "refund.created", "pending", 10).await;
    event(&t, &t.a.id, "evt_a4", "refund.created", "delivered", 5).await;
    event(&t, &t.b.id, "evt_b1", "payment.captured", "dead", 5).await;
    for (ok, n) in [(true, 3), (false, 1)] {
        for i in 0..n {
            sqlx::query("INSERT INTO outbox_deliveries (tenant_id, event_id, attempt_no, ok) VALUES ($1, 'evt_a4', $2, $3)")
                .bind(&t.a.id).bind(i + 1).bind(ok).execute(&t.root).await.unwrap();
        }
    }
    let j = t
        .ok(&t.a, "GET", "/v1/outbox/stats", None, json!(null))
        .await;
    assert_eq!(j["by_status"]["dead"], 2, "{j}");
    assert_eq!(j["by_status"]["pending"], 1);
    assert_eq!(j["by_status"]["delivered"], 1);
    assert_eq!(j["dead_by_type"][0]["type"], "payment.captured");
    assert_eq!(j["dead_by_type"][0]["count"], 2);
    assert!(j["oldest_pending_at"].is_string());
    assert_eq!(j["deliveries_24h"]["total"], 4);
    assert_eq!(j["deliveries_24h"]["failed"], 1);
    assert_eq!(j["deliveries_24h"]["success_rate"], 0.75);
    // the other tenant sees only its own, and no deliveries gives a null rate
    let jb = t
        .ok(&t.b, "GET", "/v1/outbox/stats", None, json!(null))
        .await;
    assert_eq!(jb["by_status"]["dead"], 1);
    assert!(jb["deliveries_24h"]["success_rate"].is_null());
}

#[sqlx::test(migrations = "./migrations")]
async fn replay_dead_filters_limits_dry_runs_and_is_audited(root: PgPool) {
    let t = setup(root.clone()).await;
    event(&t, &t.a.id, "evt_d1", "payment.captured", "dead", 60).await;
    event(&t, &t.a.id, "evt_d2", "payment.captured", "dead", 50).await;
    event(&t, &t.a.id, "evt_d3", "refund.created", "dead", 40).await;
    event(&t, &t.a.id, "evt_ok", "payment.captured", "delivered", 40).await;
    event(&t, &t.b.id, "evt_bd", "payment.captured", "dead", 40).await;

    // a dry run changes nothing
    let j = t
        .ok(
            &t.a,
            "POST",
            "/v1/outbox/replay_dead",
            None,
            json!({ "event_type": "payment.captured", "dry_run": true }),
        )
        .await;
    assert_eq!(j["would_replay"], 2);
    assert_eq!(j["replayed"], 0);
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE status = 'dead'")
            .await,
        4
    );

    // limit takes the oldest first
    let j = t
        .ok(
            &t.a,
            "POST",
            "/v1/outbox/replay_dead",
            None,
            json!({ "limit": 1 }),
        )
        .await;
    assert_eq!(j["replayed"], 1);
    assert_eq!(j["ids"][0], "evt_d1");
    let st: String = sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = 'evt_d1'")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(st, "pending");
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM outbox_events WHERE id = 'evt_d1'")
            .fetch_one(&t.root)
            .await
            .unwrap();
    assert_eq!(attempts, 0);

    // the type filter leaves the other type dead; the delivered event and the other tenant are untouched
    let j = t
        .ok(
            &t.a,
            "POST",
            "/v1/outbox/replay_dead",
            None,
            json!({ "event_type": "payment.captured" }),
        )
        .await;
    assert_eq!(j["ids"], json!(["evt_d2"]));
    assert_eq!(
        t.count("SELECT count(*) FROM outbox_events WHERE status = 'dead'")
            .await,
        2
    ); // evt_d3 and b's
    let b_st: String = sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = 'evt_bd'")
        .fetch_one(&t.root)
        .await
        .unwrap();
    assert_eq!(b_st, "dead");
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'outbox.replayed_dead'")
            .await,
        2
    );

    // nothing left matching: no audit row, zero replayed
    let j = t
        .ok(
            &t.a,
            "POST",
            "/v1/outbox/replay_dead",
            None,
            json!({ "event_type": "no.such" }),
        )
        .await;
    assert_eq!(j["replayed"], 0);
    assert_eq!(
        t.count("SELECT count(*) FROM audit_events WHERE action = 'outbox.replayed_dead'")
            .await,
        2
    );

    // bad limits are refused
    for l in [0, 501, -3] {
        let (s, _) = t
            .call(
                &t.a,
                "POST",
                "/v1/outbox/replay_dead",
                None,
                json!({ "limit": l }),
            )
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{l}");
    }
}

async fn settlement(
    t: &T,
    id: &str,
    provider: &str,
    utr: Option<&str>,
    on: &str,
    status_done: bool,
) {
    sqlx::query("INSERT INTO settlements (id, tenant_id, provider, provider_ref, utr, currency, gross_minor, refunds_minor, fee_minor, adjustment_minor, net_minor, settled_on) VALUES ($1, $2, $3, $1, $4, 'INR', 1000, 0, 0, 0, 1000, $5::date)")
        .bind(id).bind(&t.a.id).bind(provider).bind(utr).bind(on)
        .execute(&t.root).await.unwrap();
    if status_done {
        sqlx::query("UPDATE settlements SET status = (SELECT status FROM settlements WHERE id = $1) WHERE id = $1")
            .bind(id).execute(&t.root).await.unwrap();
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn settlements_filter_by_utr_provider_and_dates(root: PgPool) {
    let t = setup(root.clone()).await;
    settlement(&t, "stl_1", "razorpay", Some("UTR111"), "2026-11-01", false).await;
    settlement(&t, "stl_2", "razorpay", Some("UTR222"), "2026-11-10", false).await;
    settlement(&t, "stl_3", "cashfree", Some("UTR333"), "2026-11-20", false).await;
    settlement(&t, "stl_4", "cashfree", None, "2026-11-21", false).await;
    let ids = |j: &serde_json::Value| -> Vec<String> {
        j["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    let j = t
        .ok(&t.a, "GET", "/v1/settlements?utr=UTR222", None, json!(null))
        .await;
    assert_eq!(ids(&j), vec!["stl_2"]);
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/settlements?provider=cashfree",
            None,
            json!(null),
        )
        .await;
    assert_eq!(ids(&j), vec!["stl_4", "stl_3"]);
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/settlements?from=2026-11-05&to=2026-11-20",
            None,
            json!(null),
        )
        .await;
    assert_eq!(ids(&j), vec!["stl_3", "stl_2"]);
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/settlements?provider=razorpay&from=2026-11-05",
            None,
            json!(null),
        )
        .await;
    assert_eq!(ids(&j), vec!["stl_2"]);
    let j = t
        .ok(&t.a, "GET", "/v1/settlements", None, json!(null))
        .await;
    assert_eq!(ids(&j).len(), 4);
    let j = t
        .ok(&t.a, "GET", "/v1/settlements?utr=nope", None, json!(null))
        .await;
    assert!(ids(&j).is_empty());
    // a reversed range is refused, and another tenant sees nothing of ours
    let (s, _) = t
        .call(
            &t.a,
            "GET",
            "/v1/settlements?from=2026-11-20&to=2026-11-01",
            None,
            json!(null),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let j = t
        .ok(&t.b, "GET", "/v1/settlements?utr=UTR222", None, json!(null))
        .await;
    assert!(ids(&j).is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn decline_advice_for_a_reason_and_a_stored_attempt(root: PgPool) {
    let t = setup(root.clone()).await;
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/decline_advice?reason=stolen_card&provider=razorpay",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "new_instrument");
    assert_eq!(j["hard_decline"], true);
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/decline_advice?reason=insufficient_funds",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "retry_later");
    assert_eq!(j["retry_after_hours"], json!([24, 72, 120]));
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/decline_advice?reason=something_odd",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "review");
    let j = t
        .ok(&t.a, "GET", "/v1/decline_advice", None, json!(null))
        .await;
    assert_eq!(j["action"], "review");
    let long = "x".repeat(201);
    let (s, _) = t
        .call(
            &t.a,
            "GET",
            &format!("/v1/decline_advice?reason={long}"),
            None,
            json!(null),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // stored attempts: failed with a reason, cancelled by the merchant, and one that did not fail
    let iid = "pi_adv";
    sqlx::query("INSERT INTO payment_intents (id, tenant_id, amount_minor, currency, status) VALUES ($1, $2, 1000, 'INR', 'requires_payment_method')")
        .bind(iid).bind(&t.a.id).execute(&t.root).await.unwrap();
    for (id, status, reason, class) in [
        ("att_f", "failed", Some("lost_card"), Some("declined")),
        (
            "att_c",
            "failed",
            Some("canceled_by_merchant"),
            Some("cancelled"),
        ),
        ("att_ok", "captured", None, None),
    ] {
        sqlx::query("INSERT INTO payment_attempts (id, tenant_id, intent_id, provider, method, status, amount_minor, currency, route, failure_reason, failure_class) VALUES ($1, $2, $3, 'razorpay', 'card', $4, 1000, 'INR', '{}', $5, $6)")
            .bind(id).bind(&t.a.id).bind(iid).bind(status).bind(reason).bind(class)
            .execute(&t.root).await.unwrap();
    }
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/payment_attempts/att_f/retry_advice",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "new_instrument");
    assert_eq!(j["provider"], "razorpay");
    assert_eq!(j["failure_class"], "declined");
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/payment_attempts/att_c/retry_advice",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "cancelled_by_merchant");
    let j = t
        .ok(
            &t.a,
            "GET",
            "/v1/payment_attempts/att_ok/retry_advice",
            None,
            json!(null),
        )
        .await;
    assert_eq!(j["action"], "not_failed");
    // unknown id and another tenant's attempt are both 404
    let (s, _) = t
        .call(
            &t.a,
            "GET",
            "/v1/payment_attempts/att_nope/retry_advice",
            None,
            json!(null),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            &t.b,
            "GET",
            "/v1/payment_attempts/att_f/retry_advice",
            None,
            json!(null),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
