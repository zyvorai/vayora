//! Billing engine: plans, subscriptions, metered usage, recurring invoicing, credit notes,
//! voiding and customer statements. Invoices carry the figures; tax is supplied by the GST product.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{Days, Months, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{ApiError, AppState, auth::Auth, payments::refresh_invoice_status, tenant};

fn advance(d: NaiveDate, unit: &str, count: u32) -> NaiveDate {
    match unit {
        "day" => d.checked_add_days(Days::new(count.into())),
        "week" => d.checked_add_days(Days::new(7 * u64::from(count))),
        "month" => d.checked_add_months(Months::new(count)),
        _ => d.checked_add_months(Months::new(12 * count)),
    }
    .unwrap_or(d)
}

// ---------- plans ----------

#[derive(Deserialize)]
pub struct NewPlan {
    name: String,
    currency: Option<String>,
    amount_minor: i64,
    interval_unit: String,
    interval_count: Option<i32>,
    usage_unit_minor: Option<i64>,
    due_days: Option<i32>,
}

pub async fn create_plan(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewPlan>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("billing:write")?;
    let ccy: zyvor_core::Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if !matches!(b.interval_unit.as_str(), "day" | "week" | "month" | "year") {
        return Err(ApiError::invalid(
            "invalid_interval",
            "interval_unit must be day, week, month or year",
        ));
    }
    if !(0..=zyvor_core::MAX_AMOUNT_MINOR).contains(&b.amount_minor)
        || b.interval_count.unwrap_or(1) < 1
        || !(0..=zyvor_core::MAX_AMOUNT_MINOR).contains(&b.usage_unit_minor.unwrap_or(0))
    {
        return Err(ApiError::invalid(
            "invalid_plan",
            "amounts must be non-negative and interval_count at least 1",
        ));
    }
    let id = new_id("plan");
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("INSERT INTO plans (id, tenant_id, name, currency, amount_minor, interval_unit, interval_count, usage_unit_minor, due_days) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.name).bind(ccy.code()).bind(b.amount_minor).bind(&b.interval_unit)
        .bind(b.interval_count.unwrap_or(1)).bind(b.usage_unit_minor.unwrap_or(0)).bind(b.due_days.unwrap_or(7))
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "plan.created",
        Some(&id),
        json!({ "name": b.name }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": b.name, "amount_minor": b.amount_minor, "interval_unit": b.interval_unit }),
        ),
    ))
}

pub async fn list_plans(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i64, String, i32, i64, bool)> = sqlx::query_as("SELECT id, name, currency, amount_minor, interval_unit, interval_count, usage_unit_minor, active FROM plans ORDER BY created_at DESC").fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, ccy, amt, unit, cnt, usage, active)| json!({ "id": id, "name": name, "currency": ccy, "amount_minor": amt, "interval_unit": unit, "interval_count": cnt, "usage_unit_minor": usage, "active": active })).collect::<Vec<_>>() }),
    ))
}

// ---------- subscriptions ----------

#[derive(Deserialize)]
pub struct NewSubscription {
    customer_id: String,
    plan_id: String,
    quantity: Option<i32>,
    /// First billing date; defaults to today.
    start_date: Option<NaiveDate>,
}

pub async fn create_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewSubscription>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("billing:write")?;
    let q = b.quantity.unwrap_or(1);
    if q < 1 {
        return Err(ApiError::invalid(
            "invalid_quantity",
            "quantity must be at least 1",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
        .bind(&b.customer_id)
        .fetch_one(&mut *tx)
        .await?
    {
        return Err(ApiError::not_found("customer"));
    }
    let active: Option<bool> = sqlx::query_scalar("SELECT active FROM plans WHERE id = $1")
        .bind(&b.plan_id)
        .fetch_optional(&mut *tx)
        .await?;
    if !active.ok_or_else(|| ApiError::not_found("plan"))? {
        return Err(ApiError::conflict(
            "plan_inactive",
            "plan is no longer offered",
        ));
    }
    let start = b
        .start_date
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    let id = new_id("sub");
    sqlx::query("INSERT INTO subscriptions (id, tenant_id, customer_id, plan_id, quantity, next_bill_date) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.customer_id).bind(&b.plan_id).bind(q).bind(start)
        .execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "subscription.created",
        Some(&id),
        json!({ "plan": b.plan_id }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "subscription.created",
        json!({ "id": id, "customer_id": b.customer_id, "plan_id": b.plan_id }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "active", "next_bill_date": start })),
    ))
}

pub async fn list_subscriptions(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i32, String, NaiveDate)> = sqlx::query_as("SELECT id, customer_id, plan_id, quantity, status, next_bill_date FROM subscriptions ORDER BY created_at DESC LIMIT 200").fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, c, p, q, st, d)| json!({ "id": id, "customer_id": c, "plan_id": p, "quantity": q, "status": st, "next_bill_date": d })).collect::<Vec<_>>() }),
    ))
}

async fn set_status(
    s: &AppState,
    auth: &Auth,
    id: &str,
    from: &[&str],
    to: &str,
) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query("UPDATE subscriptions SET status = $2, canceled_at = CASE WHEN $2 = 'canceled' THEN now() ELSE canceled_at END WHERE id = $1 AND status = ANY($3)")
        .bind(id).bind(to).bind(from.iter().map(|x| x.to_string()).collect::<Vec<_>>())
        .execute(&mut *tx).await?.rows_affected();
    if n == 0 {
        return Err(ApiError::conflict(
            "invalid_transition",
            format!("subscription cannot move to {to} from its current state"),
        ));
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        &format!("subscription.{to}"),
        Some(id),
        json!({}),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        &format!("subscription.{to}"),
        json!({ "id": id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": to })))
}

pub async fn pause_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_status(&s, &auth, &id, &["active"], "paused").await
}
pub async fn resume_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_status(&s, &auth, &id, &["paused"], "active").await
}
pub async fn cancel_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_status(&s, &auth, &id, &["active", "paused"], "canceled").await
}

#[derive(Deserialize)]
pub struct NewUsage {
    quantity: i64,
    note: Option<String>,
    /// Makes retries safe: the same reference is recorded once.
    reference: Option<String>,
}

pub async fn record_usage(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<NewUsage>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("billing:write")?;
    if b.quantity <= 0 {
        return Err(ApiError::invalid(
            "invalid_quantity",
            "quantity must be positive",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let st: Option<String> = sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    if st.ok_or_else(|| ApiError::not_found("subscription"))? == "canceled" {
        return Err(ApiError::conflict(
            "subscription_canceled",
            "cannot record usage on a canceled subscription",
        ));
    }
    let rid = new_id("use");
    let n = sqlx::query("INSERT INTO usage_records (id, tenant_id, subscription_id, quantity, note, idempotency_ref) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (tenant_id, idempotency_ref) DO NOTHING")
        .bind(&rid).bind(&auth.tenant_id).bind(&id).bind(b.quantity).bind(&b.note).bind(&b.reference)
        .execute(&mut *tx).await?.rows_affected();
    tx.commit().await?;
    Ok((
        if n == 1 {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(json!({ "recorded": n == 1, "quantity": b.quantity })),
    ))
}

// ---------- recurring invoicing ----------

/// Raise an invoice for every active subscription whose billing date has arrived. Safe to run
/// repeatedly and from several workers: each period produces exactly one invoice.
pub async fn run_billing(
    conn: &mut PgConnection,
    tenant_id: &str,
    today: NaiveDate,
) -> Result<usize, sqlx::Error> {
    let mut created = 0;
    let due: Vec<(String, String, i32, NaiveDate)> = sqlx::query_as("SELECT id, plan_id, quantity, next_bill_date FROM subscriptions WHERE status = 'active' AND next_bill_date <= $1 ORDER BY next_bill_date FOR UPDATE SKIP LOCKED LIMIT 500")
        .bind(today).fetch_all(&mut *conn).await?;
    for (sub_id, plan_id, qty, mut date) in due {
        let (name, ccy, amount, unit, count, usage_price, due_days): (String, String, i64, String, i32, i64, i32) = sqlx::query_as("SELECT name, currency, amount_minor, interval_unit, interval_count, usage_unit_minor, due_days FROM plans WHERE id = $1").bind(&plan_id).fetch_one(&mut *conn).await?;
        let customer: String =
            sqlx::query_scalar("SELECT customer_id FROM subscriptions WHERE id = $1")
                .bind(&sub_id)
                .fetch_one(&mut *conn)
                .await?;
        // catch up missed periods (a worker that was down), bounded so a bad date can't loop forever
        for _ in 0..24 {
            if date > today {
                break;
            }
            let usage: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(quantity), 0)::BIGINT FROM usage_records WHERE subscription_id = $1 AND billed_invoice_id IS NULL").bind(&sub_id).fetch_one(&mut *conn).await?;
            let total = amount * i64::from(qty) + usage * usage_price;
            if total > 0 {
                let inv = new_id("inv");
                let n = sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, due_date, subscription_id, period_start, external_ref) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT DO NOTHING")
                    .bind(&inv).bind(tenant_id).bind(&customer).bind(format!("SUB-{}-{}", &sub_id[sub_id.len().saturating_sub(8)..], date))
                    .bind(&ccy).bind(total).bind(date.checked_add_days(Days::new(due_days as u64))).bind(&sub_id).bind(date).bind(format!("plan:{name}"))
                    .execute(&mut *conn).await?.rows_affected();
                if n == 1 {
                    sqlx::query("UPDATE usage_records SET billed_invoice_id = $2 WHERE subscription_id = $1 AND billed_invoice_id IS NULL").bind(&sub_id).bind(&inv).execute(&mut *conn).await?;
                    tenant::emit(&mut *conn, tenant_id, "invoice.created", json!({ "id": inv, "subscription_id": sub_id, "amount_minor": total, "period_start": date })).await?;
                    created += 1;
                }
            }
            date = advance(date, &unit, count as u32);
        }
        sqlx::query("UPDATE subscriptions SET next_bill_date = $2 WHERE id = $1")
            .bind(&sub_id)
            .bind(date)
            .execute(&mut *conn)
            .await?;
    }
    Ok(created)
}

pub async fn billing_run(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = run_billing(&mut tx, &auth.tenant_id, chrono::Utc::now().date_naive()).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "billing.run",
        None,
        json!({ "invoices_created": n }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "invoices_created": n })))
}

// ---------- credit notes, voiding, statements ----------

#[derive(Deserialize)]
pub struct NewCreditNote {
    amount_minor: i64,
    reason: String,
}

pub async fn create_credit_note(
    State(s): State<AppState>,
    auth: Auth,
    Path(invoice_id): Path<String>,
    Json(b): Json<NewCreditNote>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("invoices:write")?;
    if b.reason.trim().is_empty() {
        return Err(ApiError::invalid(
            "reason_required",
            "a credit note needs a reason",
        ));
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT status, amount_minor, invoice_settled(id), tax_minor FROM invoices WHERE id = $1 FOR UPDATE",
    )
    .bind(&invoice_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (status, amount, settled, inv_tax) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if status == "void" {
        return Err(ApiError::conflict("invoice_void", "invoice is void"));
    }
    if b.amount_minor <= 0 || b.amount_minor > amount - settled {
        return Err(ApiError::invalid(
            "invalid_amount",
            format!(
                "a credit note must be between 1 and the outstanding {}",
                amount - settled
            ),
        ));
    }
    let id = new_id("cn");
    // on a GST invoice the credit reverses tax in proportion; the last paisa of rounding goes to the larger component
    let (cgst, sgst, igst) =
        crate::gst::credit_note_tax(&mut tx, &invoice_id, b.amount_minor, amount, inv_tax).await?;
    let taxable = b.amount_minor - cgst - sgst - igst;
    sqlx::query("INSERT INTO credit_notes (id, tenant_id, invoice_id, amount_minor, reason, created_by, taxable_minor, cgst_minor, sgst_minor, igst_minor) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(&invoice_id).bind(b.amount_minor).bind(&b.reason).bind(&auth.actor)
        .bind(taxable).bind(cgst).bind(sgst).bind(igst)
        .execute(&mut *tx).await?;
    refresh_invoice_status(&mut tx, &invoice_id).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "credit_note.created",
        Some(&invoice_id),
        json!({ "credit_note": id, "amount_minor": b.amount_minor }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "credit_note.created",
        json!({ "id": id, "invoice_id": invoice_id, "amount_minor": b.amount_minor }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "invoice_id": invoice_id, "amount_minor": b.amount_minor })),
    ))
}

/// Void an invoice nothing has been paid or credited against.
pub async fn void_invoice(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("invoices:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT status, invoice_settled(id) FROM invoices WHERE id = $1 FOR UPDATE")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, settled) = row.ok_or_else(|| ApiError::not_found("invoice"))?;
    if settled > 0 || status == "paid" {
        return Err(ApiError::conflict(
            "invoice_has_payments",
            "an invoice with payments or credits cannot be voided; issue a credit note",
        ));
    }
    let open_intents: i64 = sqlx::query_scalar("SELECT count(*) FROM payment_intents WHERE invoice_id = $1 AND status IN ('processing', 'requires_payment_method')").bind(&id).fetch_one(&mut *tx).await?;
    if open_intents > 0 {
        return Err(ApiError::conflict(
            "payment_in_flight",
            "cancel the open payments on this invoice first",
        ));
    }
    sqlx::query("UPDATE invoices SET status = 'void' WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "invoice.voided",
        Some(&id),
        json!({}),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "invoice.voided",
        json!({ "id": id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "void" })))
}

/// Everything a customer was billed, paid and credited, with the running balance.
pub async fn statement(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM customers WHERE id = $1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?;
    let name = name.ok_or_else(|| ApiError::not_found("customer"))?;
    let rows: Vec<(chrono::DateTime<chrono::Utc>, String, String, String, i64)> = sqlx::query_as(
        "SELECT t, kind, ref, currency, amount FROM (
            SELECT created_at t, 'invoice' kind, number ref, currency, amount_minor amount FROM invoices WHERE customer_id = $1 AND status <> 'void'
            UNION ALL SELECT a.created_at, 'payment', a.reference, i.currency, -a.amount_minor FROM invoice_allocations a JOIN invoices i ON i.id = a.invoice_id WHERE i.customer_id = $1
            UNION ALL SELECT p.updated_at, 'deposit', COALESCE(p.external_ref, p.id), p.currency, -p.amount_minor FROM payment_intents p WHERE p.customer_id = $1 AND p.invoice_id IS NULL AND p.status = 'succeeded'
            UNION ALL SELECT r.updated_at, 'deposit_refund', COALESCE(r.external_ref, r.id), r.currency, r.amount_minor FROM refunds r JOIN payment_intents p ON p.id = r.intent_id WHERE p.customer_id = $1 AND p.invoice_id IS NULL AND r.status = 'processed'
            UNION ALL SELECT c.created_at, 'credit_note', c.id, i.currency, -c.amount_minor FROM credit_notes c JOIN invoices i ON i.id = c.invoice_id WHERE i.customer_id = $1 AND i.status <> 'void'
         ) x ORDER BY t, kind",
    ).bind(&id).fetch_all(&mut *tx).await?;
    let mut balances: std::collections::BTreeMap<String, i64> = Default::default();
    let lines: Vec<Value> = rows
        .into_iter()
        .map(|(t, kind, r, ccy, amt)| {
            let b = balances.entry(ccy.clone()).or_default();
            *b += amt;
            json!({ "at": t, "type": kind, "reference": r, "currency": ccy, "amount_minor": amt, "balance_minor": *b })
        })
        .collect();
    Ok(Json(
        json!({ "customer_id": id, "name": name, "lines": lines, "balance_minor": balances }),
    ))
}

// ---------- plan changes with proration ----------

#[derive(Deserialize)]
pub struct ChangePlan {
    plan_id: String,
    quantity: Option<i32>,
}

/// Switch plan mid-period. An upgrade is charged now for the unused part of the period at the price
/// difference; a downgrade takes effect at no charge and the new price applies from the next invoice.
/// (Refunding a downgrade is a deliberate human decision: issue a credit note.)
pub async fn change_plan(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Json(b): Json<ChangePlan>,
) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let sub: Option<(String, String, i32, String, NaiveDate)> = sqlx::query_as("SELECT customer_id, plan_id, quantity, status, next_bill_date FROM subscriptions WHERE id = $1 FOR UPDATE").bind(&id).fetch_optional(&mut *tx).await?;
    let (customer, old_plan, old_qty, status, next) =
        sub.ok_or_else(|| ApiError::not_found("subscription"))?;
    if status != "active" {
        return Err(ApiError::conflict(
            "subscription_not_active",
            format!("subscription is {status}"),
        ));
    }
    let get = |pid: &str| {
        sqlx::query_as::<_, (String, i64, String, i32, bool)>("SELECT currency, amount_minor, interval_unit, interval_count, active FROM plans WHERE id = $1").bind(pid.to_string())
    };
    let old = get(&old_plan).fetch_one(&mut *tx).await?;
    let new = get(&b.plan_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("plan"))?;
    if !new.4 {
        return Err(ApiError::conflict(
            "plan_inactive",
            "plan is no longer offered",
        ));
    }
    if new.0 != old.0 {
        return Err(ApiError::invalid(
            "currency_mismatch",
            "cannot change to a plan in another currency",
        ));
    }
    let new_qty = b.quantity.unwrap_or(old_qty);
    if new_qty < 1 {
        return Err(ApiError::invalid(
            "invalid_quantity",
            "quantity must be at least 1",
        ));
    }
    // the period being paid for runs from (next - interval) to next
    let today = chrono::Utc::now().date_naive();
    let period_start = match old.2.as_str() {
        "day" => next.checked_sub_days(Days::new(old.3 as u64)),
        "week" => next.checked_sub_days(Days::new(7 * old.3 as u64)),
        "month" => next.checked_sub_months(Months::new(old.3 as u32)),
        _ => next.checked_sub_months(Months::new(12 * old.3 as u32)),
    }
    .unwrap_or(next);
    let total_days = (next - period_start).num_days().max(1);
    let remaining = (next - today).num_days().clamp(0, total_days);
    let diff = new.1 * i64::from(new_qty) - old.1 * i64::from(old_qty);
    let charge = if diff > 0 {
        diff * remaining / total_days
    } else {
        0
    };
    let mut invoice_id = None;
    if charge > 0 {
        let inv = new_id("inv");
        sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, due_date, external_ref) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(&inv).bind(&auth.tenant_id).bind(&customer)
            .bind(format!("PRO-{}-{}", &id[id.len().saturating_sub(8)..], new_id("p")))
            .bind(&old.0).bind(charge).bind(today).bind(format!("proration:{old_plan}->{}", b.plan_id))
            .execute(&mut *tx).await?;
        tenant::emit(
            &mut tx,
            &auth.tenant_id,
            "invoice.created",
            json!({ "id": inv, "subscription_id": id, "amount_minor": charge, "proration": true }),
        )
        .await?;
        invoice_id = Some(inv);
    }
    sqlx::query("UPDATE subscriptions SET plan_id = $2, quantity = $3 WHERE id = $1")
        .bind(&id)
        .bind(&b.plan_id)
        .bind(new_qty)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "subscription.plan_changed",
        Some(&id),
        json!({ "from": old_plan, "to": b.plan_id, "proration_minor": charge }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "plan_id": b.plan_id, "quantity": new_qty, "proration_minor": charge, "proration_invoice_id": invoice_id, "remaining_days": remaining, "period_days": total_days }),
    ))
}

// ---------- milestones ----------

#[derive(Deserialize)]
pub struct NewMilestone {
    customer_id: String,
    name: String,
    currency: Option<String>,
    amount_minor: i64,
    due_days: Option<i32>,
    external_ref: Option<String>,
}

pub async fn create_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewMilestone>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("billing:write")?;
    if b.amount_minor <= 0 || b.amount_minor > zyvor_core::MAX_AMOUNT_MINOR {
        return Err(ApiError::invalid(
            "invalid_amount",
            "amount_minor must be positive and within the supported limit",
        ));
    }
    let ccy: zyvor_core::Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
        .bind(&b.customer_id)
        .fetch_one(&mut *tx)
        .await?
    {
        return Err(ApiError::not_found("customer"));
    }
    let id = new_id("ms");
    sqlx::query("INSERT INTO milestones (id, tenant_id, customer_id, name, currency, amount_minor, due_days, external_ref) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.customer_id).bind(&b.name).bind(ccy.code()).bind(b.amount_minor).bind(b.due_days.unwrap_or(7)).bind(&b.external_ref)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "status": "pending" })),
    ))
}

pub async fn list_milestones(
    State(s): State<AppState>,
    auth: Auth,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, String, i64, String, Option<String>)> = sqlx::query_as("SELECT id, customer_id, name, amount_minor, status, invoice_id FROM milestones ORDER BY created_at DESC LIMIT 200").fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, c, n, a, st, inv)| json!({ "id": id, "customer_id": c, "name": n, "amount_minor": a, "status": st, "invoice_id": inv })).collect::<Vec<_>>() }),
    ))
}

/// Mark a milestone accepted and invoice it. Completing twice returns the same invoice.
pub async fn complete_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, String, i64, i32, String, Option<String>, Option<String>, String)> = sqlx::query_as("SELECT customer_id, currency, amount_minor, due_days, status, invoice_id, external_ref, name FROM milestones WHERE id = $1 FOR UPDATE").bind(&id).fetch_optional(&mut *tx).await?;
    let (customer, ccy, amount, due_days, status, existing, ext, name) =
        row.ok_or_else(|| ApiError::not_found("milestone"))?;
    match status.as_str() {
        "invoiced" => {
            return Ok(Json(
                json!({ "id": id, "status": "invoiced", "invoice_id": existing }),
            ));
        }
        "canceled" => {
            return Err(ApiError::conflict(
                "milestone_canceled",
                "milestone was canceled",
            ));
        }
        _ => {}
    }
    let inv = new_id("inv");
    let today = chrono::Utc::now().date_naive();
    sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, due_date, external_ref) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&inv).bind(&auth.tenant_id).bind(&customer).bind(format!("MS-{}", &id[id.len().saturating_sub(10)..])).bind(&ccy).bind(amount)
        .bind(today.checked_add_days(Days::new(due_days as u64))).bind(ext.unwrap_or_else(|| format!("milestone:{name}")))
        .execute(&mut *tx).await?;
    sqlx::query("UPDATE milestones SET status = 'invoiced', invoice_id = $2, completed_by = $3 WHERE id = $1").bind(&id).bind(&inv).bind(&auth.actor).execute(&mut *tx).await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "milestone.completed",
        Some(&id),
        json!({ "invoice_id": inv }),
    )
    .await?;
    tenant::emit(
        &mut tx,
        &auth.tenant_id,
        "invoice.created",
        json!({ "id": inv, "milestone_id": id, "amount_minor": amount }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "id": id, "status": "invoiced", "invoice_id": inv }),
    ))
}

pub async fn cancel_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("billing:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let n = sqlx::query(
        "UPDATE milestones SET status = 'canceled' WHERE id = $1 AND status = 'pending'",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(ApiError::conflict(
            "invalid_transition",
            "only a pending milestone can be canceled",
        ));
    }
    tx.commit().await?;
    Ok(Json(json!({ "id": id, "status": "canceled" })))
}

#[derive(Deserialize)]
pub struct CustomerQuery {
    /// Your own id for the customer: a student number, patient id, dealer code.
    external_ref: Option<String>,
    limit: Option<i64>,
}

/// Find customers, most usefully by the reference your own system uses for them.
pub async fn list_customers(
    State(s): State<AppState>,
    auth: Auth,
    axum::extract::Query(q): axum::extract::Query<CustomerQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let rows: Vec<(String, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as("SELECT id, name, email, phone, external_ref FROM customers WHERE ($1::text IS NULL OR external_ref = $1) ORDER BY created_at DESC LIMIT $2")
        .bind(&q.external_ref).bind(q.limit.unwrap_or(50).clamp(1, 200)).fetch_all(&mut *tx).await?;
    Ok(Json(
        json!({ "data": rows.into_iter().map(|(id, name, email, phone, ext)| json!({ "id": id, "name": name, "email": email, "phone": phone, "external_ref": ext })).collect::<Vec<_>>() }),
    ))
}
