//! When Zyvor's reviewers are working. Each reviewer sets weekly shifts in the review desk's local time (a fixed
//! UTC offset, `ZYVOR_REVIEW_UTC_OFFSET_MINUTES`, default 330 = India; there is no daylight saving). The rota shows
//! who is on duty now and feeds a gauge so an unattended queue is noticed. It informs: an off-duty reviewer can still
//! decide anything, because a late approval is a smaller problem than a request nobody could act on.

use axum::{Json, extract::State};
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use zyvor_core::new_id;

use crate::{ApiError, AppState, platform::PlatformAuth};

pub const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// One weekly shift: `weekday` 0 = Monday, minutes since local midnight, `end_min` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift {
    pub weekday: i16,
    pub start_min: i16,
    pub end_min: i16,
}

/// `"09:30"` or `"24:00"` to minutes since midnight.
pub fn parse_hm(v: &str) -> Option<i16> {
    let (h, m) = v.trim().split_once(':')?;
    let (h, m): (i16, i16) = (h.parse().ok()?, m.parse().ok()?);
    ((0..60).contains(&m) && (0..=24).contains(&h) && (h < 24 || m == 0)).then_some(h * 60 + m)
}

pub fn fmt_hm(min: i16) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

/// Whether any of these shifts covers `now`, given the desk's offset from UTC in minutes.
pub fn on_duty(shifts: &[Shift], now: DateTime<Utc>, offset_minutes: i32) -> bool {
    let local = now + Duration::minutes(i64::from(offset_minutes));
    let weekday = local.weekday().num_days_from_monday() as i16;
    let minute = (local.hour() * 60 + local.minute()) as i16;
    shifts
        .iter()
        .any(|s| s.weekday == weekday && s.start_min <= minute && minute < s.end_min)
}

async fn all_shifts(s: &AppState) -> Result<Vec<(String, String, Shift)>, ApiError> {
    let rows: Vec<(String, String, i16, i16, i16)> = sqlx::query_as(
        "SELECT r.id, r.name, COALESCE(h.weekday, 0::smallint), COALESCE(h.start_min, 0::smallint), COALESCE(h.end_min, 0::smallint)
         FROM platform_reviewers r LEFT JOIN reviewer_shifts h ON h.reviewer_id = r.id
         WHERE r.disabled_at IS NULL ORDER BY r.created_at, h.weekday, h.start_min",
    )
    .fetch_all(&s.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, weekday, start_min, end_min)| {
            (
                id,
                name,
                Shift {
                    weekday,
                    start_min,
                    end_min,
                },
            )
        })
        .collect())
}

/// Every active reviewer with their shifts (none for a reviewer who has set no hours), oldest reviewer first.
pub async fn people(s: &AppState) -> Result<Vec<(String, String, Vec<Shift>)>, ApiError> {
    let mut people: Vec<(String, String, Vec<Shift>)> = Vec::new();
    for (id, name, sh) in all_shifts(s).await? {
        if people.last().is_none_or(|(i, _, _)| *i != id) {
            people.push((id, name, Vec::new()));
        }
        // a reviewer with no shifts comes back as one all-zero row from the outer join
        if sh.end_min > 0 {
            people.last_mut().unwrap().2.push(sh);
        }
    }
    Ok(people)
}

/// The ids of the reviewers on duty now, oldest first; empty when nobody has set hours or nobody is working.
pub async fn on_duty_ids(s: &AppState) -> Vec<String> {
    let Ok(people) = people(s).await else {
        return Vec::new();
    };
    let now = Utc::now();
    people
        .into_iter()
        .filter(|(_, _, sh)| on_duty(sh, now, s.review_utc_offset_minutes))
        .map(|(id, _, _)| id)
        .collect()
}

/// Who gets the next item: the on-duty reviewer holding the fewest, the one who has been here longest on a tie, never
/// `exclude` (the first approver cannot give the second approval). `load` is in that seniority order.
pub fn pick(load: &[(String, i64)], exclude: Option<&str>) -> Option<String> {
    load.iter()
        .filter(|(id, _)| Some(id.as_str()) != exclude)
        .min_by_key(|(_, n)| *n)
        .map(|(id, _)| id.clone())
}

/// `GET /v1/platform/rota`: every active reviewer's weekly shifts and whether they are on duty now.
pub async fn show(State(s): State<AppState>, p: PlatformAuth) -> Result<Json<Value>, ApiError> {
    let now = Utc::now();
    let people = people(&s).await?;
    let configured = people.iter().any(|(_, _, sh)| !sh.is_empty());
    let local = now + Duration::minutes(i64::from(s.review_utc_offset_minutes));
    let reviewers: Vec<Value> = people
        .iter()
        .map(|(id, name, sh)| {
            json!({
                "id": id, "name": name, "me": *id == p.id,
                "on_duty": on_duty(sh, now, s.review_utc_offset_minutes),
                "shifts": sh.iter().map(|x| json!({ "weekday": DAYS[x.weekday as usize], "start": fmt_hm(x.start_min), "end": fmt_hm(x.end_min) })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({
        "configured": configured,
        "utc_offset_minutes": s.review_utc_offset_minutes,
        "local_now": local.format("%a %H:%M").to_string(),
        "on_duty_now": people.iter().filter(|(_, _, sh)| on_duty(sh, now, s.review_utc_offset_minutes)).map(|(_, n, _)| n.clone()).collect::<Vec<_>>(),
        "reviewers": reviewers,
    })))
}

#[derive(Deserialize)]
pub struct ShiftIn {
    weekday: String,
    start: String,
    end: String,
}

#[derive(Deserialize)]
pub struct RotaIn {
    shifts: Vec<ShiftIn>,
}

/// `PUT /v1/platform/rota/me`: replace the caller's weekly shifts (an empty list clears them).
pub async fn set_mine(
    State(s): State<AppState>,
    p: PlatformAuth,
    Json(b): Json<RotaIn>,
) -> Result<Json<Value>, ApiError> {
    let bad = |m: String| ApiError::invalid("invalid_shift", m);
    if b.shifts.len() > 28 {
        return Err(bad("at most 28 shifts".into()));
    }
    let mut parsed = Vec::new();
    for x in &b.shifts {
        let weekday = DAYS
            .iter()
            .position(|d| *d == x.weekday.trim().to_lowercase())
            .ok_or_else(|| bad(format!("weekday is one of {DAYS:?}, not {:?}", x.weekday)))?
            as i16;
        let (start, end) = (
            parse_hm(&x.start)
                .ok_or_else(|| bad(format!("{:?} is not a time like 09:30", x.start)))?,
            parse_hm(&x.end).ok_or_else(|| bad(format!("{:?} is not a time like 18:00", x.end)))?,
        );
        if start >= end || start > 1439 {
            return Err(bad(format!(
                "{} {}-{}: a shift ends after it starts; one that crosses midnight is two shifts (to 24:00, then from 00:00)",
                x.weekday, x.start, x.end
            )));
        }
        parsed.push(Shift {
            weekday,
            start_min: start,
            end_min: end,
        });
    }
    let mut tx = s.pool.begin().await?;
    sqlx::query("DELETE FROM reviewer_shifts WHERE reviewer_id = $1")
        .bind(&p.id)
        .execute(&mut *tx)
        .await?;
    for sh in &parsed {
        sqlx::query("INSERT INTO reviewer_shifts (id, reviewer_id, weekday, start_min, end_min) VALUES ($1,$2,$3,$4,$5)")
            .bind(new_id("shf"))
            .bind(&p.id)
            .bind(sh.weekday)
            .bind(sh.start_min)
            .bind(sh.end_min)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({ "shifts": parsed.len(), "on_duty": on_duty(&parsed, Utc::now(), s.review_utc_offset_minutes) }),
    ))
}

/// How many active reviewers are on duty now, or `-1` when nobody has set any hours (the rota is not in use, so an
/// empty desk means nothing) or the database could not be asked.
pub async fn on_duty_count(s: &AppState) -> i64 {
    let Ok(people) = people(s).await else {
        return -1;
    };
    if people.iter().all(|(_, _, sh)| sh.is_empty()) {
        return -1;
    }
    let now = Utc::now();
    people
        .iter()
        .filter(|(_, _, sh)| on_duty(sh, now, s.review_utc_offset_minutes))
        .count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn times_parse_and_print_and_nothing_else_does() {
        assert_eq!(parse_hm("09:30"), Some(570));
        assert_eq!(parse_hm("24:00"), Some(1440));
        assert_eq!(parse_hm("00:00"), Some(0));
        assert_eq!(fmt_hm(570), "09:30");
        assert_eq!(fmt_hm(1440), "24:00");
        for bad in ["24:30", "9", "09:60", "-1:00", "ab:cd", ""] {
            assert_eq!(parse_hm(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_shift_covers_its_local_hours_and_the_end_is_exclusive() {
        // Monday 2026-11-02, 09:00-18:00 local (UTC+5:30)
        let mon = [Shift {
            weekday: 0,
            start_min: 540,
            end_min: 1080,
        }];
        // 09:00 IST is 03:30 UTC
        assert!(on_duty(&mon, at(2026, 11, 2, 3, 30), 330));
        assert!(!on_duty(&mon, at(2026, 11, 2, 3, 29), 330));
        // 17:59 IST is on, 18:00 IST is off
        assert!(on_duty(&mon, at(2026, 11, 2, 12, 29), 330));
        assert!(!on_duty(&mon, at(2026, 11, 2, 12, 30), 330));
        // Tuesday has no shift
        assert!(!on_duty(&mon, at(2026, 11, 3, 5, 0), 330));
    }

    #[test]
    fn the_local_day_decides_not_the_utc_day() {
        // Sunday 22:00 UTC is already Monday 03:30 in India: a Monday-morning shift is on, a Sunday one is off
        let early_mon = [Shift {
            weekday: 0,
            start_min: 0,
            end_min: 600,
        }];
        let sun = [Shift {
            weekday: 6,
            start_min: 0,
            end_min: 1440,
        }];
        let t = at(2026, 11, 1, 22, 0);
        assert!(on_duty(&early_mon, t, 330));
        assert!(!on_duty(&sun, t, 330));
        // and with no offset it is still Sunday
        assert!(on_duty(&sun, t, 0));
    }

    #[test]
    fn the_least_loaded_reviewer_gets_the_next_one_and_never_the_excluded() {
        let load = vec![
            ("a".to_string(), 2),
            ("b".to_string(), 1),
            ("c".to_string(), 1),
        ];
        assert_eq!(
            pick(&load, None).as_deref(),
            Some("b"),
            "fewest first, then the longest-serving"
        );
        assert_eq!(pick(&load, Some("b")).as_deref(), Some("c"));
        assert_eq!(pick(&load, Some("a")).as_deref(), Some("b"));
        assert_eq!(
            pick(&[("a".to_string(), 0)], Some("a")),
            None,
            "the first approver alone cannot be the second"
        );
        assert_eq!(pick(&[], None), None);
    }
}
