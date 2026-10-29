//! Background jobs. Each cycle walks tenants one at a time under that tenant's scope, so a job can
//! never touch another tenant's rows.

use sqlx::PgPool;

use crate::{
    AppState, outbox, payments::refresh_attempt_internal, recon::scan_missing,
    refunds::refresh_refund_internal, tenant,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub attempts_checked: usize,
    pub refunds_checked: usize,
    pub events_delivered: usize,
}

pub async fn run_cycle(s: &AppState, client: &reqwest::Client) -> Result<Stats, sqlx::Error> {
    let tenants: Vec<(String, String)> = sqlx::query_as("SELECT id, env FROM tenants ORDER BY id")
        .fetch_all(&s.pool)
        .await?;
    let mut st = Stats::default();
    let older = s.resolve_after.as_secs_f64();
    for (tid, env) in tenants {
        // 1. outcomes we don't know yet: ask the provider, never re-send
        let (attempts, refunds) = {
            let mut tx = tenant::begin(&s.pool, &tid).await?;
            let a: Vec<String> = sqlx::query_scalar("SELECT id FROM payment_attempts WHERE status IN ('unknown', 'created') AND updated_at < now() - make_interval(secs => $1::float8) ORDER BY updated_at LIMIT 50").bind(older).fetch_all(&mut *tx).await?;
            let r: Vec<String> = sqlx::query_scalar("SELECT id FROM refunds WHERE status IN ('unknown', 'processing') AND updated_at < now() - make_interval(secs => $1::float8) ORDER BY updated_at LIMIT 50").bind(older).fetch_all(&mut *tx).await?;
            (a, r)
        };
        for id in attempts {
            if let Err(e) = refresh_attempt_internal(s, &tid, &env, &id).await {
                tracing::warn!(attempt = %id, error = ?e.message, "resolver failed");
            }
            st.attempts_checked += 1;
        }
        for id in refunds {
            if let Err(e) = refresh_refund_internal(s, &tid, &env, &id).await {
                tracing::warn!(refund = %id, error = ?e.message, "refund resolver failed");
            }
            st.refunds_checked += 1;
        }
        // 2. merchant webhooks
        st.events_delivered += outbox::deliver_tenant(&s.pool, &s.crypto, client, &tid).await?;
        // 3. payments that never reached a settlement
        let mut tx = tenant::begin(&s.pool, &tid).await?;
        if scan_missing(&mut tx, &tid, 3).await.is_ok() {
            tx.commit().await?;
        }
    }
    Ok(st)
}

pub async fn queue_depths(pool: &PgPool) -> Result<(i64, i64, i64), sqlx::Error> {
    // Cross-tenant counts for metrics only: aggregate numbers, no rows. Runs per tenant to respect RLS.
    let tenants: Vec<String> = sqlx::query_scalar("SELECT id FROM tenants")
        .fetch_all(pool)
        .await?;
    let (mut pending, mut dead, mut cases) = (0, 0, 0);
    for t in tenants {
        let mut tx = tenant::begin(pool, &t).await?;
        let (p, d): (i64, i64) = sqlx::query_as("SELECT COUNT(*) FILTER (WHERE status = 'pending'), COUNT(*) FILTER (WHERE status = 'dead') FROM outbox_events").fetch_one(&mut *tx).await?;
        let c: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM recon_cases WHERE status <> 'resolved'")
                .fetch_one(&mut *tx)
                .await?;
        pending += p;
        dead += d;
        cases += c;
    }
    Ok((pending, dead, cases))
}
