//! Instalment plans: an agreed total split into dated invoices. The split is exact (the remainder
//! goes to the last instalment, never lost), the whole plan is created or none of it, and repeating
//! a request with the same `external_ref` returns the original plan.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{Months, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::{Currency, new_id};

use crate::{ApiError, AppState, auth::Auth, tenant};

#[derive(Deserialize)]
pub struct NewPlan {
    customer_id: String,
    name: String,
    total_minor: i64,
    currency: Option<String>,
    /// 2 to 60.
    instalments: i32,
    first_due_date: NaiveDate,
    /// `month` (default) or `day`.
    every_unit: Option<String>,
    /// Gap between due dates in units, default 1.
    every: Option<u32>,
    /// Invoice numbers become `{number_prefix}-1`, `-2`, ...
    number_prefix: String,
    external_ref: Option<String>,
}

/// Amounts that sum exactly to `total`: equal shares, with the remainder on the last one.
pub fn split(total: i64, n: i32) -> Vec<i64> {
    let base = total / n as i64;
    let mut v = vec![base; n as usize];
    if let Some(last) = v.last_mut() {
        *last += total - base * n as i64;
    }
    v
}

pub fn due_dates(first: NaiveDate, n: i32, unit: &str, every: u32) -> Option<Vec<NaiveDate>> {
    (0..n as u32)
        .map(|i| match unit {
            "day" => first.checked_add_days(chrono::Days::new(i as u64 * every as u64)),
            _ => first.checked_add_months(Months::new(i * every)),
        })
        .collect()
}

async fn plan_json(tx: &mut sqlx::PgConnection, id: &str) -> Result<Value, ApiError> {
    let head: Option<(String, String, i64, String, i32, Option<String>)> = sqlx::query_as("SELECT customer_id, name, total_minor, currency, instalments, external_ref FROM installment_plans WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let (customer, name, total, currency, n, ext) =
        head.ok_or_else(|| ApiError::not_found("instalment plan"))?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, i32, i64, Option<NaiveDate>, String, i64)> = sqlx::query_as("SELECT id, number, installment_no, amount_minor, due_date, status, invoice_settled(id) FROM invoices WHERE installment_plan_id = $1 ORDER BY installment_no")
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
    let paid: i64 = rows.iter().map(|r| r.6).sum();
    let invs: Vec<Value> = rows.into_iter().map(|(iid, number, no, amount, due, status, settled)| json!({ "invoice_id": iid, "number": number, "installment": no, "amount_minor": amount, "due_date": due, "status": status, "settled_minor": settled })).collect();
    Ok(
        json!({ "id": id, "customer_id": customer, "name": name, "total_minor": total, "currency": currency, "instalments": n, "external_ref": ext, "settled_minor": paid, "remaining_minor": total - paid, "invoices": invs }),
    )
}

/// `POST /v1/installment_plans`
pub async fn create_plan(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewPlan>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("invoices:write")?;
    let ccy: Currency = b
        .currency
        .as_deref()
        .unwrap_or("INR")
        .parse()
        .map_err(|e: String| ApiError::invalid("invalid_currency", e))?;
    if !(2..=60).contains(&b.instalments)
        || b.total_minor > zyvor_core::MAX_AMOUNT_MINOR
        || b.total_minor < b.instalments as i64
        || b.name.trim().is_empty()
        || b.number_prefix.trim().is_empty()
    {
        return Err(ApiError::invalid(
            "invalid_plan",
            "need a name, a number prefix, 2-60 instalments and a total of at least one minor unit per instalment",
        ));
    }
    let unit = b.every_unit.as_deref().unwrap_or("month");
    let every = b.every.unwrap_or(1);
    if !matches!(unit, "month" | "day") || every == 0 || every > 366 {
        return Err(ApiError::invalid(
            "invalid_interval",
            "every_unit is month or day; every is 1-366",
        ));
    }
    let dates = due_dates(b.first_due_date, b.instalments, unit, every)
        .ok_or_else(|| ApiError::invalid("invalid_interval", "due dates are out of range"))?;
    let amounts = split(b.total_minor, b.instalments);
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    if let Some(r) = &b.external_ref {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM installment_plans WHERE external_ref = $1")
                .bind(r)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(id) = existing {
            return Ok((StatusCode::OK, Json(plan_json(&mut tx, &id).await?)));
        }
    }
    let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM customers WHERE id = $1)")
        .bind(&b.customer_id)
        .fetch_one(&mut *tx)
        .await?;
    if !known {
        return Err(ApiError::not_found("customer"));
    }
    let id = new_id("ipl");
    sqlx::query("INSERT INTO installment_plans (id, tenant_id, customer_id, name, total_minor, currency, instalments, external_ref, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.customer_id).bind(b.name.trim()).bind(b.total_minor).bind(ccy.code()).bind(b.instalments).bind(&b.external_ref).bind(&auth.actor)
        .execute(&mut *tx).await?;
    for (i, (amount, due)) in amounts.iter().zip(&dates).enumerate() {
        sqlx::query("INSERT INTO invoices (id, tenant_id, customer_id, number, currency, amount_minor, due_date, installment_plan_id, installment_no, external_ref) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(new_id("inv")).bind(&auth.tenant_id).bind(&b.customer_id).bind(format!("{}-{}", b.number_prefix.trim(), i + 1)).bind(ccy.code())
            .bind(amount).bind(due).bind(&id).bind(i as i32 + 1).bind(format!("{id}:{}", i + 1))
            .execute(&mut *tx).await?;
    }
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "installment_plan.created",
        Some(&id),
        json!({ "total_minor": b.total_minor, "instalments": b.instalments }),
    )
    .await?;
    let out = plan_json(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// `GET /v1/installment_plans/{id}`: the plan with each instalment's invoice and how much is paid.
pub async fn get_plan(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(plan_json(&mut tx, &id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_exactly_and_dates_step_by_month_or_day() {
        assert_eq!(split(100_000, 4), vec![25_000; 4]);
        assert_eq!(split(100_001, 3), vec![33_333, 33_333, 33_335]);
        assert_eq!(split(10, 3).iter().sum::<i64>(), 10);
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(
            due_dates(d(2026, 1, 31), 3, "month", 1).unwrap(),
            vec![d(2026, 1, 31), d(2026, 2, 28), d(2026, 3, 31)]
        );
        assert_eq!(
            due_dates(d(2026, 4, 1), 3, "day", 10).unwrap(),
            vec![d(2026, 4, 1), d(2026, 4, 11), d(2026, 4, 21)]
        );
        assert_eq!(
            due_dates(d(2026, 4, 1), 2, "month", 3).unwrap(),
            vec![d(2026, 4, 1), d(2026, 7, 1)]
        );
    }
}
