//! Cron expressions of scheduled runs: the classic five fields (minute, hour, day of month, month,
//! day of week) read in an IANA time zone, plus the `@hourly` … `@yearly` shorthands. Day of month
//! and day of week both restricted means either matches, as in Vixie cron.

use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

/// How many days ahead a next run is looked for: `0 0 29 2 *` can be 8 years away (2096 → 2104).
const HORIZON_DAYS: u32 = 366 * 9;

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    /// Bit `n` set: value `n` matches.
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    /// 0 = Sunday.
    weekdays: u64,
    /// The day-of-month / day-of-week field starts with `*` (unrestricted, also `*/2`).
    any_day: bool,
    any_weekday: bool,
}

impl FromStr for Cron {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        let spec = match spec.to_ascii_lowercase().as_str() {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            "@yearly" | "@annually" => "0 0 1 1 *",
            s if s.starts_with('@') => return Err(format!("unknown shorthand `{spec}` (@hourly, @daily, @weekly, @monthly, @yearly)")),
            _ => spec,
        };
        let fields: Vec<&str> = spec.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(format!("a cron expression has 5 fields (minute hour day-of-month month day-of-week), not {}", fields.len()));
        };
        let mut weekdays = field(weekday, "day of week", 0, 7, &WEEKDAYS)?;
        if weekdays & 1 << 7 != 0 {
            weekdays = (weekdays | 1) & !(1 << 7);
        }
        let cron = Cron {
            minutes: field(minute, "minute", 0, 59, &[])?,
            hours: field(hour, "hour", 0, 23, &[])?,
            days: field(day, "day of month", 1, 31, &[])?,
            months: field(month, "month", 1, 12, &MONTHS)?,
            weekdays,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        };
        Ok(cron)
    }
}

/// Bits of one field: `*`, `n`, `a-b`, each optionally `/step`, separated by commas. `names`
/// (lower case) stand for `min`, `min + 1`, … (weekday names: 0 = sun).
fn field(spec: &str, what: &str, min: u32, max: u32, names: &[&str]) -> Result<u64, String> {
    let value = |s: &str| -> Result<u32, String> {
        let lower = s.to_ascii_lowercase();
        let n = match names.iter().position(|n| *n == lower) {
            Some(i) => min + i as u32,
            None => s.parse().map_err(|_| format!("{what}: `{s}` is not a number"))?,
        };
        if n < min || n > max {
            return Err(format!("{what}: {n} is outside {min}–{max}"));
        }
        Ok(n)
    };
    let mut bits = 0u64;
    for part in spec.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step.parse().ok().filter(|s| *s > 0).ok_or_else(|| format!("{what}: bad step in `{part}`"))?;
                (range, Some(step))
            }
            None => (part, None),
        };
        let (from, to) = match range {
            "*" => (min, max),
            r => match r.split_once('-') {
                Some((a, b)) => (value(a)?, value(b)?),
                // `5/15` = from 5 to the end in steps of 15.
                None if step.is_some() => (value(r)?, max),
                None => {
                    let n = value(r)?;
                    (n, n)
                }
            },
        };
        if from > to {
            return Err(format!("{what}: range `{range}` runs backwards"));
        }
        for n in (from..=to).step_by(step.unwrap_or(1) as usize) {
            bits |= 1 << n;
        }
    }
    Ok(bits)
}

impl Cron {
    fn day_matches(&self, date: Date) -> bool {
        if self.months & 1 << date.month() == 0 {
            return false;
        }
        let day = self.days & 1 << date.day() != 0;
        let weekday = self.weekdays & 1 << date.weekday().to_sunday_zero_offset() != 0;
        if self.any_day || self.any_weekday { day && weekday } else { day || weekday }
    }

    /// Shortest time between two runs on a matching day, in minutes (a day boundary counts too:
    /// the last run of one day to the first of the next).
    pub(crate) fn min_gap_minutes(&self) -> u32 {
        let times: Vec<u32> = (0..24u32)
            .filter(|h| self.hours & 1 << h != 0)
            .flat_map(|h| (0..60u32).filter(|m| self.minutes & 1 << m != 0).map(move |m| h * 60 + m))
            .collect();
        let (Some(first), Some(last)) = (times.first(), times.last()) else { return u32::MAX };
        let wrap = first + 24 * 60 - last;
        times.windows(2).map(|w| w[1] - w[0]).fold(wrap, u32::min)
    }

    /// The first matching minute strictly after `after`, in `tz`. A time that does not exist on
    /// that day (skipped by a DST change) runs at the shifted time, e.g. 02:30 → 03:30. `None`
    /// when nothing matches within ~9 years (e.g. `0 0 31 2 *`).
    pub(crate) fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        let start = after.to_zoned(tz.clone()).datetime();
        let mut date = start.date();
        for day in 0..HORIZON_DAYS {
            if self.day_matches(date) {
                for hour in (0..24i8).filter(|h| self.hours & 1 << h != 0) {
                    if day == 0 && hour < start.hour() {
                        continue;
                    }
                    for minute in (0..60i8).filter(|m| self.minutes & 1 << m != 0) {
                        if day == 0 && hour == start.hour() && minute <= start.minute() {
                            continue;
                        }
                        let Ok(zoned) = date.at(hour, minute, 0, 0).to_zoned(tz.clone()) else { continue };
                        if zoned.timestamp() > after {
                            return Some(zoned.timestamp());
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

/// The time zone named `name` (IANA, e.g. `Europe/Berlin`, or `UTC`).
pub(crate) fn zone(name: &str) -> Result<TimeZone, String> {
    TimeZone::get(name).map_err(|_| format!("unknown time zone `{name}` (use an IANA name like Europe/Berlin)"))
}

/// IANA name of this host's time zone; `UTC` when it has none.
pub(crate) fn host_zone() -> String {
    TimeZone::system().iana_name().unwrap_or("UTC").to_owned()
}

/// Unix ms of an ISO 8601 date-time: with an offset (`2026-11-01T09:00+01:00`) as given, without
/// one as a wall-clock time in `tz`.
pub(crate) fn parse_time(text: &str, tz: &TimeZone) -> Result<i64, String> {
    let text = text.trim();
    if let Ok(ts) = text.parse::<Timestamp>() {
        return Ok(ts.as_millisecond());
    }
    let local: jiff::civil::DateTime = text.parse().map_err(|_| format!("`{text}` is not an ISO 8601 date-time like 2026-11-01T09:00"))?;
    local.to_zoned(tz.clone()).map(|z| z.timestamp().as_millisecond()).map_err(|e| e.to_string())
}

/// `ms` as a wall-clock time in `tz`, e.g. `Sun 2026-11-01 09:00 CET`.
pub(crate) fn format_time(ms: i64, tz: &TimeZone) -> String {
    match Timestamp::from_millisecond(ms) {
        Ok(ts) => ts.to_zoned(tz.clone()).strftime("%a %Y-%m-%d %H:%M %Z").to_string(),
        Err(_) => ms.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn next(cron: &str, zone_name: &str, after: &str) -> Option<String> {
        let cron: Cron = cron.parse().unwrap();
        let tz = zone(zone_name).unwrap();
        cron.next_after(ts(after), &tz).map(|t| t.to_zoned(tz).datetime().to_string())
    }

    #[test]
    fn next_run_is_strictly_after_and_in_the_zone() {
        // 07:00 UTC is 09:00 in Berlin (summer time): today's 09:00 has passed.
        assert_eq!(next("0 9 * * *", "Europe/Berlin", "2026-07-01T07:00:00Z").as_deref(), Some("2026-07-02T09:00:00"));
        assert_eq!(next("0 9 * * *", "Europe/Berlin", "2026-07-01T06:59:59Z").as_deref(), Some("2026-07-01T09:00:00"));
        assert_eq!(next("*/15 * * * *", "UTC", "2026-07-01T10:15:30Z").as_deref(), Some("2026-07-01T10:30:00"));
    }

    #[test]
    fn weekdays_names_and_sunday_seven() {
        // 2026-10-24 is a Saturday.
        assert_eq!(next("30 8 * * 1-5", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-10-26T08:30:00"));
        assert_eq!(next("0 7 * * MON", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-10-26T07:00:00"));
        assert_eq!(next("0 7 * * 7", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-10-25T07:00:00"));
        assert_eq!(next("@weekly", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-10-25T00:00:00"));
    }

    #[test]
    fn day_of_month_and_weekday_both_restricted_match_either() {
        // The 1st (a Sunday in November 2026) or any Friday: Friday 30 Oct comes first.
        assert_eq!(next("0 0 1 * 5", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-10-30T00:00:00"));
        // With `*` as weekday only the day of month counts.
        assert_eq!(next("0 0 1 * *", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2026-11-01T00:00:00"));
        assert_eq!(next("0 0 29 2 *", "UTC", "2026-10-24T12:00:00Z").as_deref(), Some("2028-02-29T00:00:00"));
    }

    #[test]
    fn a_time_skipped_by_dst_runs_shifted_once() {
        // Berlin jumps from 02:00 to 03:00 on 2026-03-29.
        assert_eq!(next("30 2 * * *", "Europe/Berlin", "2026-03-28T12:00:00Z").as_deref(), Some("2026-03-29T03:30:00"));
        assert_eq!(next("30 2 * * *", "Europe/Berlin", "2026-03-29T01:30:00Z").as_deref(), Some("2026-03-30T02:30:00"));
    }

    #[test]
    fn impossible_dates_never_run() {
        assert_eq!(next("0 0 31 2 *", "UTC", "2026-10-24T12:00:00Z"), None);
    }

    #[test]
    fn rejects_malformed_expressions() {
        for (spec, error) in [
            ("0 9 * *", "5 fields"),
            ("60 * * * *", "minute: 60 is outside 0–59"),
            ("0 9 * * 8", "day of week: 8"),
            ("0 9-7 * * *", "backwards"),
            ("*/0 * * * *", "bad step"),
            ("x * * * *", "not a number"),
            ("@often", "unknown shorthand"),
        ] {
            let err = spec.parse::<Cron>().unwrap_err();
            assert!(err.contains(error), "{spec}: {err}");
        }
    }

    #[test]
    fn shortest_gap_spans_hour_and_day_boundaries() {
        let gap = |spec: &str| spec.parse::<Cron>().unwrap().min_gap_minutes();
        assert_eq!(gap("*/5 * * * *"), 5);
        assert_eq!(gap("0 9 * * *"), 24 * 60);
        assert_eq!(gap("0,59 * * * *"), 1);
        assert_eq!(gap("0 0,23 * * *"), 60);
    }

    #[test]
    fn parses_times_with_and_without_offset() {
        let berlin = zone("Europe/Berlin").unwrap();
        let utc = ts("2026-11-01T08:00:00Z").as_millisecond();
        assert_eq!(parse_time("2026-11-01T09:00", &berlin), Ok(utc));
        assert_eq!(parse_time("2026-11-01T09:00:00+01:00", &berlin), Ok(utc));
        assert!(parse_time("tomorrow", &berlin).is_err());
        assert!(zone("Mars/Olympus").is_err());
    }
}
