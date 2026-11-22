// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! Payout schedules: when a vendor's balance should be turned into a proposed payout.
//!
//! A schedule names a vendor and a rule: weekly on a day of the week, monthly on a day of the month (1 to 28, or 0 for
//! the last day), or whenever the payable balance reaches a threshold (looked at once a day). When a schedule is due the
//! worker (or `POST /v1/payout_schedules/run`) adds the vendor's payable balance, past its hold days, to ONE proposed
//! batch for that run, which then waits for approval like any other batch. Nothing is paid by a schedule on its own,
//! unless the tenant has set `auto_approve_scheduled_under_minor` in its payout policy and the whole run is at most that
//! much; only then is the batch approved as it is made (and, in acquire mode, the worker sends it).
//!
//! Dates follow the organisation's calendar, which is the test clock in a test environment (`clock`), so what a
//! schedule does can be shown by moving the clock. A run is identified by its schedule and its date (each item's payout
//! reference is `sched:{schedule}:{date}`), so running twice on one day, or two workers at once, proposes nothing
//! twice. A schedule whose day was missed (the worker was down) runs once when it comes back, not once per missed day.
//! A vendor's schedule is skipped, with the reason recorded, when its earlier proposal is still open, the vendor is
//! not active, nothing is payable, the amount is under the vendor's minimum, or a limit is exceeded.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{Datelike, Days, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use zyvor_core::new_id;

use crate::{
    ApiError, AppState,
    auth::Auth,
    clock, onboarding,
    payout_batches::{self, Gate, Meta, Mode, Plan, PlannedItem},
    tenant,
};

/// The first date on or after `from` that the rule names. `kind` is `weekly` (day_of_week 1 = Monday to 7), `monthly`
/// (day_of_month 1 to 28, or 0 for the last day) or `threshold` (every day).
pub fn next_occurrence(
    kind: &str,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    from: NaiveDate,
) -> NaiveDate {
    match kind {
        "weekly" => {
            let want = day_of_week.unwrap_or(1).clamp(1, 7) as i64;
            let have = from.weekday().number_from_monday() as i64;
            from + Days::new(((want - have + 7) % 7) as u64)
        }
        "monthly" => {
            let dom = day_of_month.unwrap_or(1);
            let last_of = |d: NaiveDate| {
                let (y, m) = if d.month() == 12 {
                    (d.year() + 1, 1)
                } else {
                    (d.year(), d.month() + 1)
                };
                NaiveDate::from_ymd_opt(y, m, 1)
                    .and_then(|f| f.pred_opt())
                    .unwrap_or(d)
            };
            let this = if dom == 0 {
                last_of(from)
            } else {
                NaiveDate::from_ymd_opt(from.year(), from.month(), dom as u32).unwrap_or(from)
            };
            if this >= from {
                this
            } else {
                // the same day next month
                let first_next = last_of(from) + Days::new(1);
                if dom == 0 {
                    last_of(first_next)
                } else {
                    NaiveDate::from_ymd_opt(first_next.year(), first_next.month(), dom as u32)
                        .unwrap_or(first_next)
                }
            }
        }
        _ => from,
    }
}

#[derive(Deserialize)]
pub struct NewSchedule {
    vendor_id: String,
    /// `weekly`, `monthly` or `threshold`.
    kind: String,
    day_of_week: Option<i16>,
    day_of_month: Option<i16>,
    threshold_minor: Option<i64>,
    max_amount_minor: Option<i64>,
}

const COLS: &str = "jsonb_build_object('id', id, 'vendor_id', vendor_id, 'kind', kind, 'day_of_week', day_of_week, 'day_of_month', day_of_month, 'threshold_minor', threshold_minor, 'max_amount_minor', max_amount_minor, 'currency', currency, 'active', active, 'next_run_on', next_run_on, 'last_run_on', last_run_on, 'last_batch_id', last_batch_id, 'last_outcome', last_outcome, 'created_by', created_by, 'created_at', created_at)";

async fn fetch(conn: &mut PgConnection, id: &str) -> Result<Value, ApiError> {
    let v: Option<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM payout_schedules WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(conn)
    .await?;
    v.ok_or_else(|| ApiError::not_found("payout schedule"))
}

/// `POST /v1/payout_schedules`
pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<NewSchedule>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require("payouts:write")?;
    let bad = |m: &str| Err(ApiError::invalid("invalid_schedule", m.to_string()));
    match b.kind.as_str() {
        "weekly"
            if b.day_of_week.is_some_and(|d| (1..=7).contains(&d))
                && b.day_of_month.is_none()
                && b.threshold_minor.is_none() => {}
        "monthly"
            if b.day_of_month.is_some_and(|d| (0..=28).contains(&d))
                && b.day_of_week.is_none()
                && b.threshold_minor.is_none() => {}
        "threshold"
            if b.threshold_minor
                .is_some_and(|t| t > 0 && t <= zyvor_core::MAX_AMOUNT_MINOR)
                && b.day_of_week.is_none()
                && b.day_of_month.is_none() => {}
        "weekly" => {
            return bad("weekly needs day_of_week 1 (Monday) to 7 (Sunday) and nothing else");
        }
        "monthly" => {
            return bad(
                "monthly needs day_of_month 1 to 28, or 0 for the last day, and nothing else",
            );
        }
        "threshold" => return bad("threshold needs a positive threshold_minor and nothing else"),
        _ => return bad("kind is weekly, monthly or threshold"),
    }
    if b.max_amount_minor
        .is_some_and(|m| m <= 0 || m > zyvor_core::MAX_AMOUNT_MINOR)
    {
        return bad("max_amount_minor must be positive");
    }
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let v: Option<bool> =
        sqlx::query_scalar("SELECT archived_at IS NOT NULL FROM vendors WHERE id = $1")
            .bind(&b.vendor_id)
            .fetch_optional(&mut *tx)
            .await?;
    match v {
        None => return Err(ApiError::not_found("vendor")),
        Some(true) => {
            return Err(ApiError::conflict(
                "vendor_archived",
                "this vendor is archived",
            ));
        }
        Some(false) => {}
    }
    let today = clock::today(&mut tx).await?;
    let id = new_id("psc");
    sqlx::query("INSERT INTO payout_schedules (id, tenant_id, vendor_id, kind, day_of_week, day_of_month, threshold_minor, max_amount_minor, next_run_on, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(&id).bind(&auth.tenant_id).bind(&b.vendor_id).bind(&b.kind).bind(b.day_of_week).bind(b.day_of_month)
        .bind(b.threshold_minor).bind(b.max_amount_minor)
        .bind(next_occurrence(&b.kind, b.day_of_week, b.day_of_month, today))
        .bind(&auth.actor)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "payout_schedule.created",
        Some(&id),
        json!({ "vendor_id": b.vendor_id, "kind": b.kind }),
    )
    .await?;
    let out = fetch(&mut tx, &id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Deserialize)]
pub struct PayoutScheduleListQuery {
    vendor_id: Option<String>,
}

/// `GET /v1/payout_schedules`
pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<PayoutScheduleListQuery>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let data: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT {COLS} FROM payout_schedules WHERE ($1::text IS NULL OR vendor_id = $1) ORDER BY created_at DESC, id DESC LIMIT 500")))
        .bind(&q.vendor_id)
        .fetch_all(&mut *tx)
        .await?;
    Ok(Json(json!({ "data": data })))
}

/// `GET /v1/payout_schedules/{id}`
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    Ok(Json(fetch(&mut tx, &id).await?))
}

async fn set_active(
    s: &AppState,
    auth: &Auth,
    id: &str,
    active: bool,
) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let row: Option<(String, Option<i16>, Option<i16>)> = sqlx::query_as(
        "SELECT kind, day_of_week, day_of_month FROM payout_schedules WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let (kind, dow, dom) = row.ok_or_else(|| ApiError::not_found("payout schedule"))?;
    let today = clock::today(&mut tx).await?;
    // resuming starts from today, so a schedule left paused for a month does not run for every day it missed
    sqlx::query("UPDATE payout_schedules SET active = $2, next_run_on = CASE WHEN $2 THEN $3 ELSE next_run_on END WHERE id = $1")
        .bind(id)
        .bind(active)
        .bind(next_occurrence(&kind, dow, dom, today))
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        if active {
            "payout_schedule.resumed"
        } else {
            "payout_schedule.paused"
        },
        Some(id),
        json!({}),
    )
    .await?;
    let out = fetch(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Json(out))
}

/// `POST /v1/payout_schedules/{id}/pause`
pub async fn pause(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_active(&s, &auth, &id, false).await
}

/// `POST /v1/payout_schedules/{id}/resume`
pub async fn resume(
    State(s): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    set_active(&s, &auth, &id, true).await
}

/// `POST /v1/payout_schedules/run`: do now what the worker does each cycle for the schedules that are due.
pub async fn run_now(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payouts:write")?;
    Ok(Json(run_due(&s, &auth.tenant_id).await?))
}

/// Propose a batch for every schedule that is due today. Idempotent for a given day.
pub async fn run_due(s: &AppState, tenant_id: &str) -> Result<Value, ApiError> {
    let mut tx = tenant::begin(&s.pool, tenant_id).await?;
    let today = clock::today(&mut tx).await?;
    let due: Vec<(String, String, String, Option<i16>, Option<i16>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT id, vendor_id, kind, day_of_week, day_of_month, threshold_minor, max_amount_minor FROM payout_schedules
         WHERE active AND next_run_on <= $1 ORDER BY id FOR UPDATE SKIP LOCKED",
    )
    .bind(today)
    .fetch_all(&mut *tx)
    .await?;
    if due.is_empty() {
        return Ok(
            json!({ "date": today, "due": 0, "batch_id": null, "proposed_items": 0, "schedules": [] }),
        );
    }
    // a business whose onboarding is unfinished, or with a change under review, proposes nothing and keeps its schedules due
    onboarding::require_approved(&mut tx).await?;
    let gate = Gate::load(&mut tx, &s.crypto).await?;
    let policy = payout_batches::load_policy(&mut tx).await?;
    let mut items: Vec<PlannedItem> = Vec::new();
    let mut outcomes = Vec::new();
    for (id, vendor_id, kind, dow, dom, threshold, max_amount) in due {
        let outcome: String = 'one: {
            let Some(v) = payout_batches::lock_vendor(&mut tx, &vendor_id).await? else {
                break 'one "vendor_missing".into();
            };
            let open: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM payout_batch_items i JOIN payouts p ON p.id = i.payout_id WHERE i.schedule_id = $1 AND p.status IN ('pending_approval', 'approved'))")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
            if open {
                break 'one "previous_proposal_still_open".into();
            }
            let (_, held, eligible) =
                crate::splits::eligible(&mut tx, &vendor_id, "INR", v.hold_days, today).await?;
            if eligible <= 0 {
                break 'one if held > 0 {
                    "held_back"
                } else {
                    "nothing_payable"
                }
                .into();
            }
            if threshold.is_some_and(|t| eligible < t) {
                break 'one "below_threshold".into();
            }
            let amount = max_amount.map_or(eligible, |m| eligible.min(m));
            let found = payout_batches::problems(
                &mut tx,
                &gate,
                &v,
                amount,
                "INR",
                true,
                Mode::New { already_minor: 0 },
            )
            .await?;
            if let Some((code, _)) = found.first() {
                break 'one (*code).into();
            }
            items.push(PlannedItem {
                line: items.len() + 1,
                vendor_id: vendor_id.clone(),
                vendor_name: v.name.clone(),
                amount_minor: amount,
                purpose: format!("Scheduled payout {today}"),
                reference: None,
                ext_ref: Some(format!("sched:{id}:{today}")),
                schedule_id: Some(id.clone()),
            });
            "proposed".into()
        };
        // the next time this schedule is due, counting from tomorrow (so one run per day, however many were missed)
        let tomorrow = today + Days::new(1);
        sqlx::query("UPDATE payout_schedules SET last_run_on = $2, last_outcome = $3, next_run_on = $4 WHERE id = $1")
            .bind(&id)
            .bind(today)
            .bind(&outcome)
            .bind(next_occurrence(&kind, dow, dom, tomorrow))
            .execute(&mut *tx)
            .await?;
        outcomes.push(json!({ "schedule_id": id, "vendor_id": vendor_id, "outcome": outcome }));
    }
    let mut batch_id = None;
    let (mut total, mut auto) = (0, false);
    if !items.is_empty() {
        let plan = Plan {
            items,
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        total = plan.total();
        auto = policy
            .auto_approve_scheduled_under_minor
            .is_some_and(|l| total <= l)
            && policy
                .max_payout_minor
                .is_none_or(|m| plan.items.iter().all(|i| i.amount_minor <= m));
        let fp =
            payout_batches::fingerprint("INR", true, Some(&format!("sched:{today}")), &plan.items);
        let name = format!("Scheduled payouts {today}");
        let id = payout_batches::persist(
            &mut tx,
            tenant_id,
            &plan,
            &Meta {
                name: &name,
                batch_ref: None,
                source: "schedule",
                currency: "INR",
                from_balance: true,
                rail: "imps",
                file_name: None,
                note: None,
                created_by: "system:schedule",
                fingerprint: &fp,
                auto_approve: auto,
            },
        )
        .await?;
        for it in &plan.items {
            if let Some(sid) = &it.schedule_id {
                sqlx::query("UPDATE payout_schedules SET last_batch_id = $2 WHERE id = $1")
                    .bind(sid)
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        batch_id = Some(id);
    }
    tx.commit().await?;
    Ok(json!({
        "date": today, "due": outcomes.len(), "batch_id": batch_id, "proposed_items": outcomes.iter().filter(|o| o["outcome"] == "proposed").count(),
        "total_minor": total, "auto_approved": auto, "schedules": outcomes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn weekly_finds_the_next_matching_weekday_today_included() {
        // 2026-11-18 is a Wednesday
        assert_eq!(
            next_occurrence("weekly", Some(3), None, d("2026-11-18")),
            d("2026-11-18")
        );
        assert_eq!(
            next_occurrence("weekly", Some(1), None, d("2026-11-18")),
            d("2026-11-23")
        );
        assert_eq!(
            next_occurrence("weekly", Some(7), None, d("2026-11-18")),
            d("2026-11-22")
        );
        assert_eq!(
            next_occurrence("weekly", Some(2), None, d("2026-11-18")),
            d("2026-11-24")
        );
    }

    #[test]
    fn monthly_handles_month_ends_year_ends_and_leap_years() {
        assert_eq!(
            next_occurrence("monthly", None, Some(15), d("2026-11-10")),
            d("2026-11-15")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(15), d("2026-11-15")),
            d("2026-11-15")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(15), d("2026-11-16")),
            d("2026-12-15")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(1), d("2026-12-02")),
            d("2027-01-01")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(28), d("2026-02-28")),
            d("2026-02-28")
        );
        // the last day of the month, whatever its length
        assert_eq!(
            next_occurrence("monthly", None, Some(0), d("2026-02-10")),
            d("2026-02-28")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(0), d("2028-02-10")),
            d("2028-02-29")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(0), d("2026-12-31")),
            d("2026-12-31")
        );
        // counting from the day after a run on the last day gives the next month's last day
        assert_eq!(
            next_occurrence("monthly", None, Some(0), d("2027-01-01")),
            d("2027-01-31")
        );
        assert_eq!(
            next_occurrence("monthly", None, Some(0), d("2026-11-30") + Days::new(1)),
            d("2026-12-31")
        );
    }

    #[test]
    fn a_missed_run_catches_up_once() {
        // due on the 3rd, the worker comes back on the 20th: tomorrow's occurrence is the next 3rd, not the 4th
        let today = d("2026-11-20");
        assert_eq!(
            next_occurrence("monthly", None, Some(3), today + Days::new(1)),
            d("2026-12-03")
        );
        assert_eq!(
            next_occurrence("threshold", None, None, today + Days::new(1)),
            d("2026-11-21")
        );
    }
}
