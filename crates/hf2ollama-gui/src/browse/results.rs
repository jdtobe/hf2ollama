//! The results list: rows, loading, error and empty states, plus the small
//! number and date formatters the Browse view shares.

/// `412300` -> `412k`, `1_900_000` -> `1.9M`, small numbers as they are.
pub fn compact_count(n: u64) -> String {
    let scaled = |div: f64, unit: &str| {
        let v = n as f64 / div;
        if v >= 10.0 || (v - v.trunc()).abs() < 0.05 {
            format!("{}{unit}", v.trunc())
        } else {
            format!("{v:.1}{unit}")
        }
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => scaled(1e3, "k"),
        _ => scaled(1e6, "M"),
    }
}

/// Seconds since the Unix epoch for `2026-10-12T07:10:55.000Z`.
pub fn parse_iso(s: &str) -> Option<i64> {
    let num = |range: std::ops::Range<usize>| -> Option<i64> { s.get(range)?.parse().ok() };
    if s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19).unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil date (proleptic Gregorian), after Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// `3 weeks ago` style age of an ISO timestamp, or `—` if unparseable.
pub fn relative_age(iso: &str, now_unix: i64) -> String {
    let Some(then) = parse_iso(iso) else {
        return "—".to_string();
    };
    let secs = (now_unix - then).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    let days = secs / 86_400;
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => plural(secs / 3_600, "hour"),
        _ if days < 14 => plural(days, "day"),
        _ if days < 60 => plural(days / 7, "week"),
        _ if days < 365 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    }
}

/// `4.68 GB`, or `812 MB` below one gigabyte.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_count_abbreviates() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_000), "1k");
        assert_eq!(compact_count(412_300), "412k");
        assert_eq!(compact_count(1_900_000), "1.9M");
        assert_eq!(compact_count(2_738_344), "2.7M");
        assert_eq!(compact_count(12_000_000), "12M");
    }

    #[test]
    fn parse_iso_matches_known_epochs() {
        assert_eq!(parse_iso("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_iso("2026-10-12T07:10:55.000Z"), Some(1_791_789_055));
        assert_eq!(parse_iso("nope"), None);
        assert_eq!(parse_iso(""), None);
    }

    #[test]
    fn relative_age_uses_the_largest_unit() {
        let t = parse_iso("2026-10-01T00:00:00Z").unwrap_or_default();
        assert_eq!(relative_age("2026-10-01T00:00:00Z", t + 30), "just now");
        assert_eq!(relative_age("2026-10-01T00:00:00Z", t + 120), "2 min ago");
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 3_600),
            "1 hour ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 5 * 86_400),
            "5 days ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 21 * 86_400),
            "3 weeks ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 150 * 86_400),
            "5 months ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 800 * 86_400),
            "2 years ago"
        );
        assert_eq!(relative_age("garbage", t), "—");
    }

    #[test]
    fn format_size_switches_units() {
        assert_eq!(format_size(4_680_000_000), "4.68 GB");
        assert_eq!(format_size(812_000_000), "812 MB");
        assert_eq!(format_size(0), "0 MB");
    }
}
