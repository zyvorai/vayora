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
        st.events_delivered +=
            outbox::deliver_tenant(&s.pool, &s.crypto, client, &tid, s.allow_private_targets)
                .await?;
        // 3. recurring invoices that have come due
        {
            let mut tx = tenant::begin(&s.pool, &tid).await?;
            if crate::billing::run_billing(
                &mut tx,
                &tid,
                crate::ist_today(),
                s.mandate_notice_hours,
            )
            .await
            .is_ok()
            {
                tx.commit().await?;
            }
        }
        // 3a. UPI AutoPay: expire, give notice, send due debits, settle those in flight
        if let Err(e) = crate::mandates::run_cycle(s, &tid, &env).await {
            tracing::warn!(tenant = %tid, error = ?e.message, "mandate cycle failed");
        }
        // 3b. automatic reminders for tenants that opted in
        {
            let mut tx = tenant::begin(&s.pool, &tid).await?;
            let pol: Option<(String, i32, i32, bool)> = sqlx::query_as("SELECT dunning_channel, dunning_min_days, dunning_repeat_days, dunning_requires_approval FROM tenant_settings WHERE tenant_id = $1 AND auto_dunning")
                .bind(&tid).fetch_optional(&mut *tx).await?;
            if let Some((ch, min, rep, approval)) = pol {
                let today = crate::ist_today();
                if crate::collections::run_dunning_tx(
                    &mut tx,
                    &tid,
                    "dunning",
                    today,
                    &crate::collections::Policy {
                        min_days: min as i64,
                        repeat_days: rep as i64,
                        channel: &ch,
                        requires_approval: approval,
                    },
                )
                .await
                .is_ok()
                {
                    tx.commit().await?;
                }
            }
        }
        // 3c. seal the audit trail so tampering is detectable
        {
            let mut tx = tenant::begin(&s.pool, &tid).await?;
            if crate::audit::seal_tenant(&mut tx, &tid, 120).await.is_ok() {
                tx.commit().await?;
            }
        }
        // 4. payments that never reached a settlement
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
