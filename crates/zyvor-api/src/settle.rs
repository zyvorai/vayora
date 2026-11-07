//! Acquire-mode settlement batches and rail-backed payouts.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::{Currency, new_id};
use zyvor_ledger::MoneyMode;
use zyvor_settle::{
    BatchTotals, HttpPayoutRail, PayoutInstruction, PayoutRail, PayoutRailStatus, PayoutResult,
    RailError, RailKind, SimPayoutRail, escrow_proof_ok, post_escrow_available,
    post_settlement_banked, post_settlement_release,
};

use crate::{ApiError, AppState, auth::Auth, money_mode, tenant};

/// Enrol a capture / refund / hold into the open acquire batch for this currency.
/// No-op in orchestrate mode. Idempotent on `(tenant_id, reference, kind)`.
pub async fn enrol_batch_item(
    conn: &mut PgConnection,
    tenant_id: &str,
    kind: &str,
    reference: &str,
    amount_minor: i64,
    currency: Currency,
) -> Result<Option<String>, ApiError> {
    if amount_minor <= 0 {
        return Ok(None);
    }
    if !matches!(kind, "capture" | "refund" | "fee" | "hold") {
        return Err(ApiError::invalid(
            "invalid_kind",
            "kind must be capture, refund, fee or hold",
        ));
    }
    let mode = money_mode::load_for_tenant(conn, tenant_id).await?;
    if mode != MoneyMode::Acquire {
        return Ok(None);
    }
    let ccy = currency.code();
    let mut batch_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM settlement_batches WHERE status = 'open' AND currency = $1
         ORDER BY created_at ASC LIMIT 1 FOR UPDATE",
    )
    .bind(ccy)
    .fetch_optional(&mut *conn)
    .await?;
    if batch_id.is_none() {
        let id = new_id("sbat");
        sqlx::query(
            "INSERT INTO settlement_batches (id, tenant_id, currency, status) VALUES ($1, $2, $3, 'open')",
        )
        .bind(&id)
        .bind(tenant_id)
        .bind(ccy)
        .execute(&mut *conn)
        .await?;
        tenant::audit(
            conn,
            tenant_id,
            "system",
            "settlement_batch.open",
            Some(&id),
            json!({ "currency": ccy, "auto": true }),
        )
        .await?;
        batch_id = Some(id);
    }
    let batch_id = batch_id.expect("batch just ensured");
    let item_id = new_id("sbi");
    let inserted = sqlx::query(
        "INSERT INTO settlement_batch_items (id, tenant_id, batch_id, kind, reference, amount_minor, currency)
         VALUES ($1,$2,$3,$4,$5,$6,$7)
         ON CONFLICT (tenant_id, reference, kind) DO NOTHING",
    )
    .bind(&item_id)
    .bind(tenant_id)
    .bind(&batch_id)
    .bind(kind)
    .bind(reference.trim())
    .bind(amount_minor)
    .bind(ccy)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Ok(None);
    }
    match kind {
        "capture" => {
            sqlx::query(
                "UPDATE settlement_batches SET captures_minor = captures_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(amount_minor)
            .execute(&mut *conn)
            .await?;
        }
        "refund" => {
            sqlx::query(
                "UPDATE settlement_batches SET refunds_minor = refunds_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(amount_minor)
            .execute(&mut *conn)
            .await?;
        }
        "fee" => {
            sqlx::query("UPDATE settlement_batches SET fees_minor = fees_minor + $2 WHERE id = $1")
                .bind(&batch_id)
                .bind(amount_minor)
                .execute(&mut *conn)
                .await?;
        }
        "hold" => {
            sqlx::query(
                "UPDATE settlement_batches SET holds_minor = holds_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(amount_minor)
            .execute(&mut *conn)
            .await?;
        }
        _ => unreachable!(),
    }
    sqlx::query(
        "UPDATE settlement_batches SET net_minor = captures_minor - refunds_minor - fees_minor - holds_minor WHERE id = $1",
    )
    .bind(&batch_id)
    .execute(&mut *conn)
    .await?;
    Ok(Some(item_id))
}

/// Release open acquire batches whose settlement delay has elapsed (worker).
pub async fn auto_release_due(s: &AppState, tenant_id: &str) -> Result<usize, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let mode = money_mode::load_for_tenant(&mut tx, tenant_id).await?;
    if mode != MoneyMode::Acquire {
        return Ok(0);
    }
    let delay: i32 = sqlx::query_scalar(
        "SELECT COALESCE(settlement_delay_days, 1) FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(1);
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM settlement_batches
         WHERE status = 'open' AND net_minor > 0
           AND created_at + make_interval(days => $1) <= now()
         ORDER BY created_at ASC LIMIT 20",
    )
    .bind(delay)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut n = 0;
    for id in ids {
        match release_batch_inner(s, tenant_id, "system", &id).await {
            Ok(_) => n += 1,
            Err(e) => {
                tracing::warn!(tenant = %tenant_id, batch = %id, error = ?e.message, "auto-release failed")
            }
        }
    }
    Ok(n)
}

/// Prefer `ZYVOR_PAYOUT_RAIL_URL` + `ZYVOR_PAYOUT_RAIL_KEY` (sponsor HTTP rail); otherwise the sim rail.
async fn rail_send(insn: &PayoutInstruction) -> Result<PayoutResult, RailError> {
    let url = std::env::var("ZYVOR_PAYOUT_RAIL_URL")
        .ok()
        .filter(|u| !u.is_empty());
    let key = std::env::var("ZYVOR_PAYOUT_RAIL_KEY").unwrap_or_default();
    match url {
        Some(base) => {
            HttpPayoutRail::new(base, key, std::time::Duration::from_secs(30))
                .send(insn)
                .await
        }
        None => SimPayoutRail.send(insn).await,
    }
}

#[derive(Deserialize)]
pub struct OpenBatch {
    currency: String,
}

pub async fn list_batches(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, currency, status, captures_minor, refunds_minor, fees_minor, holds_minor, net_minor,
                rail, rail_ref, created_at, released_at
         FROM settlement_batches ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let data: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                currency,
                status,
                captures_minor,
                refunds_minor,
                fees_minor,
                holds_minor,
                net_minor,
                rail,
                rail_ref,
                created_at,
                released_at,
            )| {
                json!({
                    "id": id,
                    "currency": currency,
                    "status": status,
                    "captures_minor": captures_minor,
                    "refunds_minor": refunds_minor,
                    "fees_minor": fees_minor,
                    "holds_minor": holds_minor,
                    "net_minor": net_minor,
                    "rail": rail,
                    "rail_ref": rail_ref,
                    "created_at": created_at,
                    "released_at": released_at,
                })
            },
        )
        .collect();
    Ok(Json(json!({ "data": data })))
}

pub async fn get_batch(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, currency, status, captures_minor, refunds_minor, fees_minor, holds_minor, net_minor,
                rail, rail_ref, created_at, released_at
         FROM settlement_batches WHERE id = $1",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(r) = row else {
        return Err(ApiError::not_found("settlement batch"));
    };
    let items: Vec<(String, String, String, i64, String)> = sqlx::query_as(
        "SELECT id, kind, reference, amount_minor, currency FROM settlement_batch_items
         WHERE batch_id = $1 ORDER BY created_at",
    )
    .bind(&id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "id": r.0,
        "currency": r.1,
        "status": r.2,
        "captures_minor": r.3,
        "refunds_minor": r.4,
        "fees_minor": r.5,
        "holds_minor": r.6,
        "net_minor": r.7,
        "rail": r.8,
        "rail_ref": r.9,
        "created_at": r.10,
        "released_at": r.11,
        "items": items.into_iter().map(|(id, kind, reference, amount_minor, currency)| json!({
            "id": id, "kind": kind, "reference": reference, "amount_minor": amount_minor, "currency": currency
        })).collect::<Vec<_>>(),
    })))
}

pub async fn open_batch(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<OpenBatch>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("admin")?;
    let ccy: Currency = b
        .currency
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mode = money_mode::load_for_tenant(&mut tx, &auth.tenant_id).await?;
    if mode != MoneyMode::Acquire {
        return Err(ApiError::conflict(
            "orchestrate_mode",
            "settlement batches are for acquire mode only",
        ));
    }
    let id = new_id("sbat");
    sqlx::query(
        "INSERT INTO settlement_batches (id, tenant_id, currency, status) VALUES ($1, $2, $3, 'open')",
    )
    .bind(&id)
    .bind(&auth.tenant_id)
    .bind(ccy.code())
    .execute(&mut *tx)
    .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "settlement_batch.open",
        Some(&id),
        json!({ "currency": ccy }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "open", "currency": ccy })),
    ))
}

#[derive(Deserialize)]
pub struct AddItem {
    kind: String,
    reference: String,
    amount_minor: i64,
}

pub async fn add_item(
    State(s): State<AppState>,
    auth: Auth,
    Path(batch_id): Path<String>,
    Json(b): Json<AddItem>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    if b.amount_minor <= 0 {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be positive",
        ));
    }
    if !matches!(b.kind.as_str(), "capture" | "refund" | "fee" | "hold") {
        return Err(ApiError::invalid(
            "invalid_kind",
            "kind must be capture, refund, fee or hold",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM settlement_batches WHERE id = $1 FOR UPDATE")
            .bind(&batch_id)
            .fetch_optional(&mut *tx)
            .await?;
    let status = status.ok_or_else(|| ApiError::not_found("settlement batch"))?;
    if status != "open" {
        return Err(ApiError::conflict(
            "batch_closed",
            format!("batch is {status}"),
        ));
    }
    let ccy: String = sqlx::query_scalar("SELECT currency FROM settlement_batches WHERE id = $1")
        .bind(&batch_id)
        .fetch_one(&mut *tx)
        .await?;
    let item_id = new_id("sbi");
    sqlx::query(
        "INSERT INTO settlement_batch_items (id, tenant_id, batch_id, kind, reference, amount_minor, currency)
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(&item_id)
    .bind(&auth.tenant_id)
    .bind(&batch_id)
    .bind(&b.kind)
    .bind(b.reference.trim())
    .bind(b.amount_minor)
    .bind(&ccy)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            ApiError::conflict("duplicate_item", "that reference/kind is already in a batch")
        }
        e => e.into(),
    })?;
    match b.kind.as_str() {
        "capture" => {
            sqlx::query(
                "UPDATE settlement_batches SET captures_minor = captures_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(b.amount_minor)
            .execute(&mut *tx)
            .await?;
        }
        "refund" => {
            sqlx::query(
                "UPDATE settlement_batches SET refunds_minor = refunds_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(b.amount_minor)
            .execute(&mut *tx)
            .await?;
        }
        "fee" => {
            sqlx::query("UPDATE settlement_batches SET fees_minor = fees_minor + $2 WHERE id = $1")
                .bind(&batch_id)
                .bind(b.amount_minor)
                .execute(&mut *tx)
                .await?;
        }
        "hold" => {
            sqlx::query(
                "UPDATE settlement_batches SET holds_minor = holds_minor + $2 WHERE id = $1",
            )
            .bind(&batch_id)
            .bind(b.amount_minor)
            .execute(&mut *tx)
            .await?;
        }
        _ => unreachable!(),
    }
    sqlx::query(
        "UPDATE settlement_batches SET net_minor = captures_minor - refunds_minor - fees_minor - holds_minor WHERE id = $1",
    )
    .bind(&batch_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": item_id, "batch_id": batch_id, "kind": b.kind, "amount_minor": b.amount_minor }),
    ))
}

pub async fn release_batch(
    State(s): State<AppState>,
    auth: Auth,
    Path(batch_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    let out = release_batch_inner(&s, &auth.tenant_id, &auth.actor, &batch_id).await?;
    Ok(Json(out))
}

async fn release_batch_inner(
    s: &AppState,
    tenant_id: &str,
    actor: &str,
    batch_id: &str,
) -> Result<Value, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let row: Option<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT status, currency, captures_minor, refunds_minor, fees_minor, holds_minor, net_minor
         FROM settlement_batches WHERE id = $1 FOR UPDATE",
    )
    .bind(batch_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, ccy, cap, refn, fees, holds, net) =
        row.ok_or_else(|| ApiError::not_found("settlement batch"))?;
    if status != "open" {
        return Err(ApiError::conflict(
            "batch_not_open",
            format!("batch is {status}"),
        ));
    }
    let delay: i32 = sqlx::query_scalar(
        "SELECT COALESCE(settlement_delay_days, 1) FROM tenant_settings WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(1);
    if delay > 0 {
        let ready: bool = sqlx::query_scalar(
            "SELECT created_at + make_interval(days => $2) <= now() FROM settlement_batches WHERE id = $1",
        )
        .bind(batch_id)
        .bind(delay)
        .fetch_one(&mut *tx)
        .await?;
        if !ready {
            return Err(ApiError::conflict(
                "settlement_delay",
                format!(
                    "batch cannot release until {delay} day(s) after it was opened (settlement_delay_days)"
                ),
            ));
        }
    }
    let currency: Currency = ccy.parse().map_err(|_| ApiError::internal())?;
    let org = money_mode::organisation_id(&mut tx, tenant_id).await?;
    let totals = BatchTotals {
        captures_minor: cap,
        refunds_minor: refn,
        fees_minor: fees,
        holds_minor: holds,
    };
    if totals.net() != net {
        return Err(ApiError::conflict(
            "batch_inconsistent",
            "net_minor does not match totals",
        ));
    }
    if net <= 0 {
        return Err(ApiError::invalid(
            "nothing_to_release",
            "batch net must be positive",
        ));
    }
    if cap > 0 {
        post_escrow_available(cap, currency)
            .map_err(zyvor_ledger::StoreError::from)?
            .post(
                &mut tx,
                tenant_id,
                &format!("escrow_avail:{batch_id}"),
                None,
            )
            .await?;
    }
    sqlx::query("UPDATE settlement_batches SET status = 'releasing' WHERE id = $1")
        .bind(batch_id)
        .execute(&mut *tx)
        .await?;
    post_settlement_release(MoneyMode::Acquire, &org, net, fees, currency)
        .map_err(zyvor_ledger::StoreError::from)?
        .post(
            &mut tx,
            tenant_id,
            &format!("settle_release:{batch_id}"),
            None,
        )
        .await?;
    tenant::audit(
        &mut tx,
        tenant_id,
        actor,
        "settlement_batch.release",
        Some(batch_id),
        json!({ "net_minor": net }),
    )
    .await?;
    tx.commit().await?;

    let insn = PayoutInstruction {
        payout_id: batch_id.to_string(),
        amount_minor: net,
        currency,
        rail: RailKind::Imps,
        account_number: None,
        ifsc: None,
        vpa: None,
        beneficiary_name: org.clone(),
    };
    let result = rail_send(&insn)
        .await
        .map_err(|e| ApiError::conflict("rail_failed", e.to_string()))?;

    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    match result.status {
        PayoutRailStatus::Settled | PayoutRailStatus::Accepted => {
            post_settlement_banked(net, currency)
                .map_err(zyvor_ledger::StoreError::from)?
                .post(&mut tx, tenant_id, &format!("settle_bank:{batch_id}"), None)
                .await?;
            sqlx::query(
                "UPDATE settlement_batches SET status = 'released', rail = $2, rail_ref = $3, released_at = now() WHERE id = $1",
            )
            .bind(batch_id)
            .bind(RailKind::Imps.as_str())
            .bind(&result.rail_ref)
            .execute(&mut *tx)
            .await?;
            tenant::emit(
                &mut tx,
                tenant_id,
                "settlement_batch.released",
                json!({ "batch_id": batch_id, "net_minor": net, "rail_ref": result.rail_ref }),
            )
            .await?;
        }
        PayoutRailStatus::Failed => {
            sqlx::query(
                "UPDATE settlement_batches SET status = 'failed', failure_reason = $2 WHERE id = $1",
            )
            .bind(batch_id)
            .bind(result.failure_reason.as_deref().unwrap_or("rail_failed"))
            .execute(&mut *tx)
            .await?;
        }
        PayoutRailStatus::Unknown => {
            sqlx::query(
                "UPDATE settlement_batches SET status = 'failed', failure_reason = $2 WHERE id = $1",
            )
            .bind(batch_id)
            .bind("rail_unknown")
            .execute(&mut *tx)
            .await?;
        }
    }
    let out: Value = sqlx::query_scalar(
        "SELECT json_build_object('id', id, 'status', status, 'net_minor', net_minor, 'rail_ref', rail_ref) FROM settlement_batches WHERE id = $1",
    )
    .bind(batch_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(out)
}

pub async fn escrow_proof(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let escrow: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(CASE side WHEN 'debit' THEN amount_minor ELSE -amount_minor END),0)::BIGINT
         FROM ledger_entries WHERE account = 'escrow:available'",
    )
    .fetch_one(&mut *tx)
    .await?;
    let payables: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(CASE side WHEN 'credit' THEN amount_minor ELSE -amount_minor END),0)::BIGINT
         FROM ledger_entries WHERE account LIKE 'merchant_payable:%' OR account LIKE 'vendor_payable:%'",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({
        "escrow_available_net": escrow,
        "open_payables_net": payables,
        "balanced": escrow_proof_ok(escrow, payables),
    })))
}

#[derive(Deserialize)]
pub struct RailPayout {
    rail: Option<String>,
    account_number: Option<String>,
    ifsc: Option<String>,
    vpa: Option<String>,
    beneficiary_name: Option<String>,
}

/// Send an approved payout over the sponsor rail (acquire). Updates status like mark_paid on success.
pub async fn send_payout_rail(
    State(s): State<AppState>,
    auth: Auth,
    Path(payout_id): Path<String>,
    Json(b): Json<RailPayout>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let rail_kind = match b.rail.as_deref().unwrap_or("imps") {
        "imps" => RailKind::Imps,
        "neft" => RailKind::Neft,
        "upi" => RailKind::Upi,
        _ => {
            return Err(ApiError::invalid(
                "invalid_rail",
                "rail must be imps, neft or upi",
            ));
        }
    };
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let mode = money_mode::load_for_tenant(&mut tx, &auth.tenant_id).await?;
    if mode != MoneyMode::Acquire {
        return Err(ApiError::conflict(
            "orchestrate_mode",
            "use mark_paid in orchestrate mode; rail payouts are acquire-only",
        ));
    }
    let r: Option<(String, String, i64, String, String)> = sqlx::query_as(
        "SELECT id, vendor_id, amount_minor, currency, status FROM payouts WHERE id = $1 FOR UPDATE",
    )
    .bind(&payout_id)
    .fetch_optional(&mut *tx)
    .await?;
    let r = r.ok_or_else(|| ApiError::not_found("payout"))?;
    if r.4 != "approved" {
        return Err(ApiError::conflict(
            "not_approved",
            format!("payout is {}", r.4),
        ));
    }
    let currency: Currency = r.3.parse().map_err(|_| ApiError::internal())?;
    let amount_minor = r.2;
    let vendor_id = r.1.clone();
    tx.commit().await?;

    let insn = PayoutInstruction {
        payout_id: payout_id.clone(),
        amount_minor,
        currency,
        rail: rail_kind,
        account_number: b.account_number,
        ifsc: b.ifsc,
        vpa: b.vpa,
        beneficiary_name: b.beneficiary_name.unwrap_or_else(|| vendor_id.clone()),
    };
    let result = rail_send(&insn)
        .await
        .map_err(|e| ApiError::conflict("rail_failed", e.to_string()))?;
    if !matches!(
        result.status,
        PayoutRailStatus::Settled | PayoutRailStatus::Accepted
    ) {
        return Err(ApiError::conflict(
            "rail_not_settled",
            result
                .failure_reason
                .unwrap_or_else(|| format!("{:?}", result.status)),
        ));
    }
    let bank_ref = result
        .rail_ref
        .unwrap_or_else(|| format!("RAIL{}", &payout_id[payout_id.len().saturating_sub(8)..]));
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let r: (String, String, i64, String, String) = sqlx::query_as(
        "SELECT id, vendor_id, amount_minor, currency, status FROM payouts WHERE id = $1 FOR UPDATE",
    )
    .bind(&payout_id)
    .fetch_one(&mut *tx)
    .await?;
    if r.4 != "approved" {
        return Err(ApiError::conflict(
            "not_approved",
            format!("payout is {}", r.4),
        ));
    }
    let ccy: Currency = r.3.parse().map_err(|_| ApiError::internal())?;
    let source: String = sqlx::query_scalar("SELECT source FROM payouts WHERE id = $1")
        .bind(&payout_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE payouts SET status = 'paid', paid_by = $2, bank_ref = $3, updated_at = now() WHERE id = $1",
    )
    .bind(&payout_id)
    .bind(&auth.actor)
    .bind(&bank_ref)
    .execute(&mut *tx)
    .await?;
    let debit_account = if source == "balance" {
        zyvor_ledger::accounts::vendor_payable(&r.1)
    } else {
        zyvor_ledger::accounts::PAYOUTS.to_string()
    };
    zyvor_ledger::Transaction::new(vec![
        zyvor_ledger::Entry::debit(debit_account, r.2, ccy),
        zyvor_ledger::Entry::credit(zyvor_ledger::accounts::BANK, r.2, ccy),
    ])
    .map_err(zyvor_ledger::StoreError::from)?
    .post(
        &mut tx,
        &auth.tenant_id,
        &format!("payout:{payout_id}"),
        None,
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "payout.paid",
        json!({ "payout_id": payout_id, "amount_minor": r.2, "bank_ref": bank_ref, "rail": rail_kind.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "payout_id": payout_id, "status": "paid", "bank_ref": bank_ref }),
    ))
}
