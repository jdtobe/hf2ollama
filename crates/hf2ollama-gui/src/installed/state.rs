//! Pure state of the Installed view: rows, filter, sort, two-click delete,
//! details cache and toast. No egui in here.

use std::time::SystemTime;

/// Seconds since the Unix epoch for an RFC 3339 timestamp such as
/// `2026-10-20T09:30:00.123456789-08:00` or `...Z`.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(range)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    if b.len() < 20 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    if !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Optional fraction, then the zone.
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let offset = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let secs = oh * 3_600 + om * 60;
            if *sign == b'+' {
                secs
            } else {
                -secs
            }
        }
        _ => return None,
    };
    // Days from civil date (proleptic Gregorian), after Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss - offset)
}

/// `5 minutes ago` style age of an Ollama `modified_at` timestamp.
pub fn relative_time(modified: &str, now: SystemTime) -> String {
    let Some(then) = parse_rfc3339(modified) else {
        let raw: String = modified.trim().chars().take(10).collect();
        let raw = raw.trim_end().to_string();
        return if raw.is_empty() {
            "unknown".to_string()
        } else {
            raw
        };
    };
    let now = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let secs = (now - then).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    let days = secs / 86_400;
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => plural(secs / 60, "minute"),
        3_600..=86_399 => plural(secs / 3_600, "hour"),
        _ if days < 7 => plural(days, "day"),
        _ if days < 30 => plural(days / 7, "week"),
        _ if days < 365 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn at(secs: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    }

    #[test]
    fn relative_time_variants() {
        let base = parse_rfc3339("2026-10-20T09:30:00Z").unwrap();
        let m = "2026-10-20T09:30:00.123456789Z";
        let rel = |secs: i64| relative_time(m, at(base + secs));
        assert_eq!(rel(0), "just now");
        assert_eq!(rel(59), "just now");
        assert_eq!(rel(60), "1 minute ago");
        assert_eq!(rel(300), "5 minutes ago");
        assert_eq!(rel(3_600), "1 hour ago");
        assert_eq!(rel(7 * 3_600), "7 hours ago");
        assert_eq!(rel(86_400), "1 day ago");
        assert_eq!(rel(2 * 86_400), "2 days ago");
        assert_eq!(rel(6 * 86_400), "6 days ago");
        assert_eq!(rel(7 * 86_400), "1 week ago");
        assert_eq!(rel(21 * 86_400), "3 weeks ago");
        assert_eq!(rel(30 * 86_400), "1 month ago");
        assert_eq!(rel(65 * 86_400), "2 months ago");
        assert_eq!(rel(365 * 86_400), "1 year ago");
        assert_eq!(rel(800 * 86_400), "2 years ago");
        // A timestamp in the future is "just now", never negative.
        assert_eq!(rel(-3_600), "just now");
    }

    #[test]
    fn offsets_and_z_name_the_same_instant() {
        let z = parse_rfc3339("2026-10-20T17:30:00Z").unwrap();
        assert_eq!(parse_rfc3339("2026-10-20T09:30:00-08:00"), Some(z));
        assert_eq!(
            parse_rfc3339("2026-10-20T09:30:00.123456789-08:00"),
            Some(z)
        );
        assert_eq!(parse_rfc3339("2026-10-20T19:30:00+02:00"), Some(z));
        assert_eq!(parse_rfc3339("2026-10-20T17:30:00.5Z"), Some(z));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        let now = at(z + 2 * 86_400);
        assert_eq!(
            relative_time("2026-10-20T09:30:00.123456789-08:00", now),
            "2 days ago"
        );
    }

    #[test]
    fn unparsable_dates_fall_back_to_a_short_raw_string() {
        let now = at(0);
        assert_eq!(relative_time("garbage", now), "garbage");
        assert_eq!(relative_time("  not a date at all  ", now), "not a date");
        assert_eq!(relative_time("", now), "unknown");
        assert_eq!(relative_time("   ", now), "unknown");
        assert_eq!(parse_rfc3339("2026-13-40T00:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-10-20"), None);
        assert_eq!(parse_rfc3339("2026-10-20T09:30:00"), None);
    }
}
