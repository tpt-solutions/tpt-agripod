// SPDX-License-Identifier: MIT OR Apache-2.0

//! UTC timestamps in the strict RFC 3339 form the units doc requires (`YYYY-MM-DDTHH:MM:SSZ`).
//! Pure integer arithmetic: no wall clock, no time-zone database.

/// Seconds in a day.
pub const SECONDS_PER_DAY: i64 = 86_400;

/// Parses `YYYY-MM-DDTHH:MM:SSZ` into Unix seconds. Leap seconds and offsets are rejected.
pub fn parse_rfc3339_utc(s: &str) -> Result<i64, String> {
    let b = s.as_bytes();
    let shape_ok = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z';
    if !shape_ok {
        return Err(format!("timestamp '{s}' is not YYYY-MM-DDTHH:MM:SSZ"));
    }
    let num = |range: std::ops::Range<usize>| -> Result<i64, String> {
        s[range.clone()]
            .parse::<i64>()
            .ok()
            .filter(|_| s[range].bytes().all(|c| c.is_ascii_digit()))
            .ok_or_else(|| format!("timestamp '{s}' has a non-digit field"))
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=days_in_month(y, mo)).contains(&d) {
        return Err(format!("timestamp '{s}' has an invalid date"));
    }
    if h > 23 || mi > 59 || se > 59 {
        return Err(format!("timestamp '{s}' has an invalid time"));
    }
    Ok(days_from_civil(y, mo, d) * SECONDS_PER_DAY + h * 3600 + mi * 60 + se)
}

/// Formats Unix seconds (UTC) as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_rfc3339_utc(ts_s: i64) -> String {
    let days = ts_s.div_euclid(SECONDS_PER_DAY);
    let secs = ts_s.rem_euclid(SECONDS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Local hour of day in `[0, 24)` for a UTC timestamp at a fixed offset in hours.
/// Daylight saving is not modelled; callers pass the offset they want.
pub fn local_hour(ts_s: i64, utc_offset_h: f64) -> f64 {
    let local_s = ts_s as f64 + utc_offset_h * 3600.0;
    let day_s = local_s.rem_euclid(SECONDS_PER_DAY as f64);
    day_s / 3600.0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Days since 1970-01-01 (proleptic Gregorian). Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_epoch_values() {
        assert_eq!(parse_rfc3339_utc("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_rfc3339_utc("2026-01-01T00:00:00Z"), Ok(1_767_225_600));
        assert_eq!(parse_rfc3339_utc("2024-02-29T12:00:00Z"), Ok(1_709_208_000));
    }

    #[test]
    fn round_trips_across_years() {
        for ts in [0, 1_767_225_600 + 300, 951_782_400, 4_102_444_799] {
            assert_eq!(parse_rfc3339_utc(&format_rfc3339_utc(ts)), Ok(ts));
        }
    }

    #[test]
    fn rejects_malformed_and_impossible_timestamps() {
        for bad in [
            "2026-01-01T00:00:00",
            "2026-01-01 00:00:00Z",
            "2026-01-01T00:00:00+12:00",
            "2025-02-29T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:00:60Z",
            "20x6-01-01T00:00:00Z",
        ] {
            assert!(parse_rfc3339_utc(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn local_hour_applies_offset_and_wraps() {
        // 2026-01-01T18:00:00Z is 06:00 at UTC+12.
        let ts = parse_rfc3339_utc("2026-01-01T18:00:00Z").unwrap();
        assert!((local_hour(ts, 12.0) - 6.0).abs() < 1e-9);
        assert!((local_hour(ts, -1.0) - 17.0).abs() < 1e-9);
    }
}
