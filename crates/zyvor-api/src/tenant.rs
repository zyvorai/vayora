//! Tenant-scoped transactions, audit trail and the merchant-facing outbox.

use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};

/// Begin a transaction scoped to `tenant_id`: row-level security then hides every other tenant's rows.
pub async fn begin(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('zyvor.tenant_id', $1, true)")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

pub async fn audit(
    conn: &mut PgConnection,
    tenant_id: &str,
    actor: &str,
    action: &str,
    subject: Option<&str>,
    detail: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO audit_events (tenant_id, actor, action, subject, detail) VALUES ($1, $2, $3, $4, $5)")
        .bind(tenant_id)
        .bind(actor)
        .bind(action)
        .bind(subject)
        .bind(detail)
        .execute(conn)
        .await?;
    Ok(())
}

/// Queue an event for the merchant's webhook endpoint. Commits atomically with the change that caused it.
pub async fn emit(
    conn: &mut PgConnection,
    tenant_id: &str,
    event_type: &str,
    data: Value,
) -> Result<String, sqlx::Error> {
    debug_assert!(
        crate::events::known(event_type),
        "event type {event_type} is missing from events::CATALOGUE"
    );
    let id = zyvor_core::new_id("evt");
    // events the merchant did not subscribe to are never queued
    let wanted: Option<Vec<String>> =
        sqlx::query_scalar("SELECT webhook_events FROM tenant_settings WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    if !crate::events::subscribed(wanted.as_deref(), event_type) {
        return Ok(id);
    }
    let payload = serde_json::json!({ "id": id, "type": event_type, "created_at": chrono::Utc::now(), "data": data });
    sqlx::query("INSERT INTO outbox_events (id, tenant_id, type, payload) VALUES ($1, $2, $3, $4)")
        .bind(&id)
        .bind(tenant_id)
        .bind(event_type)
        .bind(payload)
        .execute(conn)
        .await?;
    Ok(id)
}
