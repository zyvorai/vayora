//! The test clock. A test organisation can move its own date forward to see what the next billing period, a trial
//! ending or an overdue invoice looks like, and everything that follows "today" for billing, reminders and the dates
//! the API stamps on subscriptions and invoices follows it, the worker's automatic runs included. The clock is
//! frozen at the day it was set (it does not tick on its own) until it is moved again or cleared. Live environments
//! cannot set it, so they always use the calendar.
//!
//! Not on the clock: UPI AutoPay notice windows (measured in real hours), payment timestamps, session
//! lifetimes and anything else stamped with the database's own `now()`. Mandate end dates, payment-link
//! expiry reminders and insights cashflow do follow the clock.

use axum::{Json, extract::State};
use chrono::{Days, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::{ApiError, AppState, auth::Auth, tenant};

/// The organisation's "today": its test clock when one is set, otherwise the calendar day in India.
pub async fn today(conn: &mut PgConnection) -> Result<NaiveDate, sqlx::Error> {
    let set: Option<Option<NaiveDate>> =
        sqlx::query_scalar("SELECT clock_date FROM tenant_settings")
            .fetch_optional(&mut *conn)
            .await?;
    Ok(set.flatten().unwrap_or_else(crate::ist_today))
}

fn require_test(auth: &Auth) -> Result<(), ApiError> {
    if auth.env != "test" {
        return Err(ApiError::forbidden(
            "the test clock is available only in a test environment",
        ));
    }
    Ok(())
}

/// `GET /v1/test_clock`: the date the organisation sees, whether it is the test clock, and the calendar date.
pub async fn get(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("payments:read")?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let now = today(&mut tx).await?;
    let real = crate::ist_today();
    Ok(Json(
        json!({ "date": now, "frozen": is_set(&mut tx).await?, "real_date": real }),
    ))
}

async fn is_set(conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
    let set: Option<Option<NaiveDate>> =
        sqlx::query_scalar("SELECT clock_date FROM tenant_settings")
            .fetch_optional(&mut *conn)
            .await?;
    Ok(set.flatten().is_some())
}

#[derive(Deserialize)]
pub struct ClockMove {
    /// The day to move to; not before the organisation's current day.
    date: Option<NaiveDate>,
    /// Or: this many days forward from the organisation's current day.
    advance_days: Option<u32>,
}

/// `POST /v1/test_clock` (admin, test environments only): set the clock to `date`, or move it `advance_days` on.
pub async fn set(
    State(s): State<AppState>,
    auth: Auth,
    Json(b): Json<ClockMove>,
) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    require_test(&auth)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    let now = today(&mut tx).await?;
    let to = match (b.date, b.advance_days) {
        (Some(d), None) => d,
        (None, Some(n)) => now
            .checked_add_days(Days::new(u64::from(n)))
            .ok_or_else(|| ApiError::invalid("invalid_clock", "too far ahead"))?,
        _ => {
            return Err(ApiError::invalid(
                "invalid_clock",
                "send exactly one of date or advance_days",
            ));
        }
    };
    let latest = crate::ist_today()
        .checked_add_days(Days::new(365 * 5))
        .unwrap_or(now);
    if to < now {
        return Err(ApiError::invalid(
            "invalid_clock",
            "the clock only moves forward; clear it to go back to real time",
        ));
    }
    if to > latest {
        return Err(ApiError::invalid(
            "invalid_clock",
            "the clock can be at most five years ahead of today",
        ));
    }
    sqlx::query("INSERT INTO tenant_settings (tenant_id, clock_date) VALUES ($1, $2) ON CONFLICT (tenant_id) DO UPDATE SET clock_date = $2")
        .bind(&auth.tenant_id)
        .bind(to)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "test_clock.set",
        None,
        json!({ "from": now, "to": to }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({ "date": to, "frozen": true, "real_date": crate::ist_today() }),
    ))
}

/// `DELETE /v1/test_clock` (admin, test environments only): back to real time.
pub async fn clear(State(s): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require("admin")?;
    require_test(&auth)?;
    let mut tx = tenant::begin(&s.pool, &auth.tenant_id).await?;
    sqlx::query("UPDATE tenant_settings SET clock_date = NULL WHERE tenant_id = $1")
        .bind(&auth.tenant_id)
        .execute(&mut *tx)
        .await?;
    tenant::audit(
        &mut tx,
        &auth.tenant_id,
        &auth.actor,
        "test_clock.cleared",
        None,
        json!({}),
    )
    .await?;
    tx.commit().await?;
    let real = crate::ist_today();
    Ok(Json(
        json!({ "date": real, "frozen": false, "real_date": real }),
    ))
}
