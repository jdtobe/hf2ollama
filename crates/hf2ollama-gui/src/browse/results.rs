//! The results list: rows, loading, error and empty states, plus the small
//! number and date formatters the Browse view shares.

/// `412300` -> `412k`, `1_900_000` -> `1.9M`, small numbers as they are.
pub fn compact_count(n: u64) -> String {
    let _ = n;
    unimplemented!()
}

/// Seconds since the Unix epoch for `2026-10-12T07:10:55.000Z`.
pub fn parse_iso(s: &str) -> Option<i64> {
    let _ = s;
    unimplemented!()
}

/// `3 weeks ago` style age of an ISO timestamp, or `—` if unparseable.
pub fn relative_age(iso: &str, now_unix: i64) -> String {
    let _ = (iso, now_unix);
    unimplemented!()
}

/// `4.68 GB`, or `812 MB` below one gigabyte.
pub fn format_size(bytes: u64) -> String {
    let _ = bytes;
    unimplemented!()
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
