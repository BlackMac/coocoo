//! Local days and hours for a fixed UTC offset (the client's).

const DAY_MS: i64 = 86_400_000;

/// Days since 1970-01-01 of the local day containing `ts` (Unix ms).
pub fn day_index(ts: i64, tz_offset_min: i32) -> i64 {
    (ts + i64::from(tz_offset_min) * 60_000).div_euclid(DAY_MS)
}

/// Unix ms of the local midnight starting day `index`.
pub fn day_start(index: i64, tz_offset_min: i32) -> i64 {
    index * DAY_MS - i64::from(tz_offset_min) * 60_000
}

/// Local weekday (0 = Monday) and hour of `ts`.
pub fn weekday_hour(ts: i64, tz_offset_min: i32) -> (usize, usize) {
    let local = ts + i64::from(tz_offset_min) * 60_000;
    let day = local.div_euclid(DAY_MS);
    // 1970-01-01 was a Thursday.
    let weekday = (day + 3).rem_euclid(7) as usize;
    let hour = (local.rem_euclid(DAY_MS) / 3_600_000) as usize;
    (weekday, hour)
}

/// `YYYY-MM-DD` of day `index` (proleptic Gregorian, Howard Hinnant's civil-from-days).
pub fn date(index: i64) -> String {
    let z = index + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM` of the UTC month containing `ts` (event log file names).
pub fn utc_month(ts: i64) -> String {
    date(ts.div_euclid(DAY_MS))[..7].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_days_follow_the_offset() {
        // 2026-10-23 23:30 UTC is already the 24th in UTC+2 and a Saturday.
        let ts = 1_792_798_200_000;
        assert_eq!(date(day_index(ts, 0)), "2026-10-23");
        assert_eq!(date(day_index(ts, 120)), "2026-10-24");
        assert_eq!(weekday_hour(ts, 120), (5, 1));
        assert_eq!(day_start(day_index(ts, 120), 120), 1_792_792_800_000);
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(utc_month(ts), "2026-10");
    }
}
