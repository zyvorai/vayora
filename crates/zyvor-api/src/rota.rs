//! When Zyvor's reviewers are working. Each reviewer sets weekly shifts in the review desk's local time: an IANA zone
//! (`ZYVOR_REVIEW_TIMEZONE`, for example `Europe/London`, which follows daylight saving) or, when none is set, a fixed
//! UTC offset (`ZYVOR_REVIEW_UTC_OFFSET_MINUTES`, default 330 = India). The rota shows
//! who is on duty now and feeds a gauge so an unattended queue is noticed. It informs: an off-duty reviewer can still
//! decide anything, because a late approval is a smaller problem than a request nobody could act on.
//!
//! Days off sit on top of the weekly shifts: leave for one reviewer, or a desk holiday for everyone (whole local days,
//! both ends inclusive). A reviewer on leave, or anyone on a holiday, is not on duty however their shifts read.

use std::collections::HashSet;

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Offset, Timelike, Utc};
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

/// The desk's wall clock at `now`: in its time zone when one is set (daylight saving included), else at the fixed offset.
pub fn desk_local(s: &AppState, now: DateTime<Utc>) -> NaiveDateTime {
    match s.review_timezone {
        Some(tz) => now.with_timezone(&tz).naive_local(),
        None => (now + Duration::minutes(i64::from(s.review_utc_offset_minutes))).naive_local(),
    }
}

/// The desk's offset from UTC at `now`, in minutes (it moves with daylight saving when a zone is set).
pub fn desk_offset_minutes(s: &AppState, now: DateTime<Utc>) -> i32 {
    match s.review_timezone {
        Some(tz) => now.with_timezone(&tz).offset().fix().local_minus_utc() / 60,
        None => s.review_utc_offset_minutes,
    }
}

/// Whether any of these shifts covers the desk's wall clock `local`. In a zone with daylight saving a shift is
/// wall-clock: 09:00-18:00 stays 09:00-18:00 when the clocks change, so the hour that does not exist (spring) is
/// never on a shift and the hour that happens twice (autumn) is covered both times.
pub fn on_duty(shifts: &[Shift], local: NaiveDateTime) -> bool {
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

/// Who is away on one local day: the desk's holiday (its note) if there is one, and the reviewers on leave.
#[derive(Default)]
pub struct Away {
    pub holiday: Option<String>,
    pub on_leave: HashSet<String>,
}

impl Away {
    pub fn covers(&self, reviewer: &str) -> bool {
        self.holiday.is_some() || self.on_leave.contains(reviewer)
    }
}

/// The local day `now` falls on in the desk's time.
fn local_day(s: &AppState, now: DateTime<Utc>) -> NaiveDate {
    desk_local(s, now).date()
}

/// Who is away on `day`. A database error counts as nobody away, so a hiccup does not empty the desk.
pub async fn away_on(s: &AppState, day: NaiveDate) -> Away {
    let rows: Vec<(Option<String>, String)> = sqlx::query_as(
        "SELECT reviewer_id, note FROM reviewer_days_off WHERE first_day <= $1 AND last_day >= $1",
    )
    .bind(day)
    .fetch_all(&s.pool)
    .await
    .unwrap_or_default();
    let mut away = Away::default();
    for (who, note) in rows {
        match who {
            Some(id) => {
                away.on_leave.insert(id);
            }
            None => {
                away.holiday = Some(if note.is_empty() {
                    "Holiday".into()
                } else {
                    note
                });
            }
        }
    }
    away
}

/// The ids of the reviewers on duty now, oldest first; empty when nobody has set hours or nobody is working.
pub async fn on_duty_ids(s: &AppState) -> Vec<String> {
    let Ok(people) = people(s).await else {
        return Vec::new();
    };
    let now = Utc::now();
    let local_now = desk_local(s, now);
    let away = away_on(s, local_day(s, now)).await;
    people
        .into_iter()
        .filter(|(id, _, sh)| on_duty(sh, local_now) && !away.covers(id))
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
    let local_now = desk_local(&s, now);
    let today = local_now.date();
    let away = away_on(&s, today).await;
    let working = |id: &str, sh: &[Shift]| on_duty(sh, local_now) && !away.covers(id);
    let days_off: Vec<(
        String,
        Option<String>,
        Option<String>,
        NaiveDate,
        NaiveDate,
        String,
    )> = sqlx::query_as(
        "SELECT d.id, d.reviewer_id, r.name, d.first_day, d.last_day, d.note
         FROM reviewer_days_off d LEFT JOIN platform_reviewers r ON r.id = d.reviewer_id
         WHERE d.last_day >= $1 AND (d.reviewer_id IS NULL OR r.disabled_at IS NULL)
         ORDER BY d.first_day, d.created_at",
    )
    .bind(today)
    .fetch_all(&s.pool)
    .await?;
    let reviewers: Vec<Value> = people
        .iter()
        .map(|(id, name, sh)| {
            json!({
                "id": id, "name": name, "me": *id == p.id,
                "on_duty": working(id, sh),
                "on_leave": away.on_leave.contains(id),
                "shifts": sh.iter().map(|x| json!({ "weekday": DAYS[x.weekday as usize], "start": fmt_hm(x.start_min), "end": fmt_hm(x.end_min) })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(Json(json!({
        "configured": configured,
        "utc_offset_minutes": desk_offset_minutes(&s, now),
        "timezone": s.review_timezone.map(|z| z.name()),
        "local_now": local_now.format("%a %H:%M").to_string(),
        "local_date": today.to_string(),
        "on_duty_now": people.iter().filter(|(id, _, sh)| working(id, sh)).map(|(_, n, _)| n.clone()).collect::<Vec<_>>(),
        "holiday_today": away.holiday,
        "reviewers": reviewers,
        "days_off": days_off.iter().map(|(id, who, name, first, last, note)| json!({
            "id": id, "reviewer_id": who, "reviewer": name, "desk": who.is_none(),
            "mine": who.as_deref() == Some(p.id.as_str()),
            "first_day": first.to_string(), "last_day": last.to_string(), "note": note,
        })).collect::<Vec<_>>(),
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
    let now = Utc::now();
    let away = away_on(&s, local_day(&s, now)).await;
    Ok(Json(
        json!({ "shifts": parsed.len(), "on_duty": on_duty(&parsed, desk_local(&s, now)) && !away.covers(&p.id) }),
    ))
}

#[derive(Deserialize)]
pub struct DayOffIn {
    /// First day, `2026-12-25`.
    from: String,
    /// Last day, inclusive; the same day when absent.
    to: Option<String>,
    note: Option<String>,
    /// A holiday for the whole desk rather than the caller's own leave.
    #[serde(default)]
    desk: bool,
}

/// `POST /v1/platform/rota/days_off`: book leave for the caller, or a desk holiday (`desk: true`) for everyone.
pub async fn add_day_off(
    State(s): State<AppState>,
    p: PlatformAuth,
    Json(b): Json<DayOffIn>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let bad = |m: String| ApiError::invalid("invalid_day_off", m);
    let day = |v: &str| {
        NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d")
            .map_err(|_| bad(format!("{v:?} is not a day like 2026-12-25")))
    };
    let first = day(&b.from)?;
    let last = match &b.to {
        Some(t) => day(t)?,
        None => first,
    };
    let today = local_day(&s, Utc::now());
    if last < first {
        return Err(bad("the last day is before the first".into()));
    }
    if last < today {
        return Err(bad(format!(
            "{last} is already over (it is {today} at the desk)"
        )));
    }
    if (last - first).num_days() > 365 {
        return Err(bad("at most a year at a time".into()));
    }
    let note = b.note.unwrap_or_default().trim().to_string();
    if note.chars().count() > 200 {
        return Err(bad("the note is at most 200 characters".into()));
    }
    let (upcoming,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM reviewer_days_off WHERE created_by = $1 AND last_day >= $2",
    )
    .bind(&p.id)
    .bind(today)
    .fetch_one(&s.pool)
    .await?;
    if upcoming >= 100 {
        return Err(bad(
            "100 upcoming days off already; remove some first".into()
        ));
    }
    let id = new_id("off");
    sqlx::query(
        "INSERT INTO reviewer_days_off (id, reviewer_id, first_day, last_day, note, created_by) VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(&id)
    .bind((!b.desk).then_some(&p.id))
    .bind(first)
    .bind(last)
    .bind(&note)
    .bind(&p.id)
    .execute(&s.pool)
    .await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "id": id, "desk": b.desk, "first_day": first.to_string(), "last_day": last.to_string(), "note": note,
        })),
    ))
}

/// `DELETE /v1/platform/rota/days_off/{id}`: take back your own leave, or any desk holiday. Someone else's leave is
/// theirs to remove.
pub async fn remove_day_off(
    State(s): State<AppState>,
    p: PlatformAuth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT reviewer_id FROM reviewer_days_off WHERE id = $1")
            .bind(&id)
            .fetch_optional(&s.pool)
            .await?;
    let (owner,) = row.ok_or_else(|| ApiError::not_found("day off"))?;
    if owner.is_some_and(|o| o != p.id) {
        return Err(ApiError::forbidden(
            "that is another reviewer's leave; only they can remove it",
        ));
    }
    sqlx::query("DELETE FROM reviewer_days_off WHERE id = $1")
        .bind(&id)
        .execute(&s.pool)
        .await?;
    Ok(Json(json!({ "id": id, "deleted": true })))
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
    let local_now = desk_local(s, now);
    let away = away_on(s, local_day(s, now)).await;
    people
        .iter()
        .filter(|(id, _, sh)| on_duty(sh, local_now) && !away.covers(id))
        .count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    /// `on_duty` at a UTC instant for a desk on a fixed offset.
    fn fixed(shifts: &[Shift], now: DateTime<Utc>, offset_minutes: i32) -> bool {
        on_duty(
            shifts,
            (now + Duration::minutes(i64::from(offset_minutes))).naive_utc(),
        )
    }

    /// `on_duty` at a UTC instant for a desk in a time zone.
    fn zoned(shifts: &[Shift], now: DateTime<Utc>, tz: chrono_tz::Tz) -> bool {
        on_duty(shifts, now.with_timezone(&tz).naive_local())
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
        assert!(fixed(&mon, at(2026, 11, 2, 3, 30), 330));
        assert!(!fixed(&mon, at(2026, 11, 2, 3, 29), 330));
        // 17:59 IST is on, 18:00 IST is off
        assert!(fixed(&mon, at(2026, 11, 2, 12, 29), 330));
        assert!(!fixed(&mon, at(2026, 11, 2, 12, 30), 330));
        // Tuesday has no shift
        assert!(!fixed(&mon, at(2026, 11, 3, 5, 0), 330));
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
        assert!(fixed(&early_mon, t, 330));
        assert!(!fixed(&sun, t, 330));
        // and with no offset it is still Sunday
        assert!(fixed(&sun, t, 0));
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

    #[test]
    fn a_zone_moves_the_shift_in_utc_when_the_clocks_change_but_not_on_the_wall() {
        // London, Monday 09:00-18:00 wall clock. BST began on Sunday 2026-03-29, so the same shift sits an hour earlier in UTC.
        let tz = chrono_tz::Europe::London;
        let mon = [Shift {
            weekday: 0,
            start_min: 540,
            end_min: 1080,
        }];
        // Monday 2026-03-23, GMT: 09:00 local is 09:00 UTC
        assert!(!zoned(&mon, at(2026, 3, 23, 8, 59), tz));
        assert!(zoned(&mon, at(2026, 3, 23, 9, 0), tz));
        // Monday 2026-03-30, BST: 09:00 local is 08:00 UTC
        assert!(!zoned(&mon, at(2026, 3, 30, 7, 59), tz));
        assert!(zoned(&mon, at(2026, 3, 30, 8, 0), tz));
        assert!(zoned(&mon, at(2026, 3, 30, 16, 59), tz));
        assert!(!zoned(&mon, at(2026, 3, 30, 17, 0), tz));
        // a fixed UTC+0 desk would have got the 30th wrong by an hour
        assert!(!fixed(&mon, at(2026, 3, 30, 8, 0), 0));
    }

    #[test]
    fn the_hour_that_does_not_exist_is_never_on_and_the_one_that_repeats_is_on_both_times() {
        let tz = chrono_tz::Europe::London;
        // 2026-03-29: 01:00-02:00 does not exist. A shift 01:00-02:00 never matches anything that day.
        let gap = [Shift {
            weekday: 6,
            start_min: 60,
            end_min: 120,
        }];
        for h in 0..24 {
            assert!(!zoned(&gap, at(2026, 3, 29, h, 30), tz), "{h}");
        }
        // 2026-10-25: 01:00-02:00 happens twice (01:00 BST, then 01:00 GMT): on both times
        let twice = [Shift {
            weekday: 6,
            start_min: 60,
            end_min: 120,
        }];
        assert!(zoned(&twice, at(2026, 10, 25, 0, 30), tz), "first 01:30");
        assert!(zoned(&twice, at(2026, 10, 25, 1, 30), tz), "second 01:30");
        assert!(!zoned(&twice, at(2026, 10, 25, 2, 30), tz));
    }
}
