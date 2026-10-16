use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;

#[derive(Clone, Copy)]
pub(crate) enum Cadence {
    Daily,
    Weekly(i16),
    Monthly(i16),
}

pub(crate) fn occurrence(
    now: DateTime<Tz>,
    time: NaiveTime,
    cadence: Cadence,
    forward: bool,
) -> DateTime<Tz> {
    let direction = if forward { 1 } else { -1 };
    let today = now.date_naive();
    let aligned = match cadence {
        Cadence::Daily => today,
        Cadence::Weekly(day) => {
            let current = now.weekday().num_days_from_sunday() as i16;
            let delta = if direction < 0 {
                -((current - day + 7) % 7)
            } else {
                (day - current + 7) % 7
            };
            today + Duration::days(i64::from(delta))
        }
        Cadence::Monthly(day) => month_day(today, day),
    };
    let candidate = local(now.timezone(), aligned, time);
    let accepted = if forward {
        candidate > now
    } else {
        candidate <= now
    };
    if accepted {
        return candidate;
    }
    let adjacent = match cadence {
        Cadence::Daily => aligned + Duration::days(direction),
        Cadence::Weekly(_) => aligned + Duration::days(7 * direction),
        Cadence::Monthly(day) => {
            let first = today.with_day(1).expect("calendar month start");
            let adjacent = if direction < 0 {
                first - Duration::days(1)
            } else {
                first + Duration::days(32)
            };
            month_day(adjacent, day)
        }
    };
    local(now.timezone(), adjacent, time)
}
fn month_day(month: NaiveDate, day: i16) -> NaiveDate {
    let (year, next) = if month.month() == 12 {
        (month.year() + 1, 1)
    } else {
        (month.year(), month.month() + 1)
    };
    let last = NaiveDate::from_ymd_opt(year, next, 1)
        .unwrap_or_else(|| month.with_day(1).expect("calendar month start"))
        - Duration::days(1);
    month
        .with_day((day.max(1) as u32).min(last.day()))
        .unwrap_or(month)
}
fn local(zone: Tz, date: NaiveDate, time: NaiveTime) -> DateTime<Tz> {
    let naive = NaiveDateTime::new(date, time);
    match zone.from_local_datetime(&naive) {
        chrono::LocalResult::Single(value) => value,
        chrono::LocalResult::Ambiguous(first, second) => first.min(second),
        chrono::LocalResult::None => zone.from_utc_datetime(&naive),
    }
}
