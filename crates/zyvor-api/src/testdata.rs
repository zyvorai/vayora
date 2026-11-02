//! Reset test data: a test environment clears the payments and money records it created while trying
//! things out, and keeps its setup (settings, provider accounts, keys, people, branches, counters, vendors,
//! plans, coupons). Never for a live environment: the handler refuses, and the database lets the append-only
//! tables be emptied only for a tenant whose environment is `test` (migration 0051). The audit trail is kept,
//! and the reset itself is written to it.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{ApiError, AppState, auth::Auth, tenant};

pub const CONFIRM: &str = "reset test data";

/// Children first, so every foreign key is satisfied. All of these carry `tenant_id`.
const TABLES: &[&str] = &[
    "outbox_events",
    "webhook_inbox",
    "idempotency_keys",
    "recon_cases",
    "reminder_drafts",
    "invoice_reminders",
    "payment_promises",
    "usage_records",
    "credit_notes",
    "invoice_lines",
    "invoice_allocations",
    "payment_splits",
    "settlement_items",
    "bank_statement_lines",
    "settlements",
    "disputes",
    "refunds",
    "payment_attempts",
    "mandate_debits",
    "payment_intents",
    "payment_links",
    "ledger_entries",
    "ledger_transactions",
    "cash_entries",
    "closings",
    "milestones",
    "invoices",
    "installment_plans",
    "subscriptions",
    "mandates",
    "payouts",
    "customers",
];

#[derive(Deserialize)]
pub struct Body {
    /// Must be exactly `reset test data`.
    confirm: String,
}

/// `POST /v1/test_data/reset`: admin only, test environments only.
pub async fn reset(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<Body>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    if auth.env != "test" {
        return Err(ApiError::forbidden(
            "test data can be reset only in a test environment",
        ));
    }
    if b.confirm != CONFIRM {
        return Err(ApiError::invalid(
            "confirmation_required",
            format!("send confirm: \"{CONFIRM}\""),
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    // names this tenant for the append-only tables' triggers, for this transaction only
    sqlx::query("SELECT set_config('zyvor.test_reset', $1, true)")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    let mut removed = serde_json::Map::new();
    for table in TABLES {
        let n = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE tenant_id = $1"
        )))
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n > 0 {
            removed.insert((*table).into(), json!(n));
        }
    }
    sqlx::query("UPDATE coupons SET uses = 0 WHERE tenant_id = $1")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "test_data.reset",
        None,
        json!({ "removed": removed }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "ok": true, "removed": removed })))
}
