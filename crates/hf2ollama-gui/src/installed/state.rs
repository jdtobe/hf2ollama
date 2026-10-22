//! Pure state of the Installed view: rows, filter, sort, two-click delete,
//! details cache and toast. No egui in here.

use crate::runtime::InstalledEntry;
use hf2ollama_core::ollama::{InstalledModel, ModelDetails};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant, SystemTime};

/// How long the "Confirm delete" state and the toast stay.
pub const ARM_WINDOW: Duration = Duration::from_secs(4);
pub const TOAST_TIME: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter {
    All,
    Imported,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Size,
    Name,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRow {
    pub model: InstalledModel,
    pub source_repo: Option<String>,
}

impl InstalledRow {
    pub fn name(&self) -> &str {
        &self.model.name
    }

    /// The Hugging Face page of the source repo; only imported rows have one.
    pub fn source_url(&self) -> Option<String> {
        unimplemented!("source_url")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListStatus {
    Loading,
    Error(String),
    Loaded,
}

/// What a confirmed delete asks the backend for: exactly this name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteAction {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSplit {
    pub imported: u64,
    pub library: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsState {
    Loading,
    Error(String),
    Loaded(ModelDetails),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub until: Instant,
}

#[derive(Debug)]
pub struct InstalledState {
    rows: Vec<InstalledRow>,
    pub status: ListStatus,
    pub filter_text: String,
    pub source: SourceFilter,
    pub sort: SortKey,
    armed: Option<(String, Instant)>,
    expanded: BTreeSet<String>,
    details: HashMap<String, DetailsState>,
    toast: Option<Toast>,
}

impl InstalledState {
    pub fn new() -> InstalledState {
        InstalledState {
            rows: Vec::new(),
            status: ListStatus::Loading,
            filter_text: String::new(),
            source: SourceFilter::All,
            sort: SortKey::Size,
            armed: None,
            expanded: BTreeSet::new(),
            details: HashMap::new(),
            toast: None,
        }
    }

    /// Replaces the list (or records why it could not be read).
    pub fn on_installed_loaded(&mut self, r: &Result<Vec<InstalledEntry>, String>) {
        match r {
            Ok(entries) => {
                self.rows = entries
                    .iter()
                    .map(|e| InstalledRow {
                        model: e.model.clone(),
                        source_repo: e.source_repo.clone(),
                    })
                    .collect();
                self.status = ListStatus::Loaded;
            }
            Err(e) => self.status = ListStatus::Error(e.clone()),
        }
    }

    pub fn rows(&self) -> &[InstalledRow] {
        &self.rows
    }

    /// Rows after the text filter, the source filter and the sort.
    pub fn visible(&self) -> Vec<&InstalledRow> {
        let needle = self.filter_text.trim().to_lowercase();
        let mut rows: Vec<&InstalledRow> = self
            .rows
            .iter()
            .filter(|r| needle.is_empty() || r.name().to_lowercase().contains(&needle))
            .filter(|r| match self.source {
                SourceFilter::All => true,
                SourceFilter::Imported => r.source_repo.is_some(),
                SourceFilter::Library => r.source_repo.is_none(),
            })
            .collect();
        match self.sort {
            SortKey::Size => rows.sort_by(|a, b| {
                b.model
                    .size
                    .cmp(&a.model.size)
                    .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
            }),
            SortKey::Name => rows.sort_by_key(|r| r.name().to_lowercase()),
            SortKey::Modified => rows.sort_by(|a, b| {
                let (ta, tb) = (
                    parse_rfc3339(&a.model.modified),
                    parse_rfc3339(&b.model.modified),
                );
                // `Some` beats `None`, and a later time beats an earlier one.
                tb.cmp(&ta)
                    .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
            }),
        }
        rows
    }
}

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

    // ---- filter and sort ----

    fn entry(name: &str, size: u64, modified: &str, repo: Option<&str>) -> InstalledEntry {
        InstalledEntry {
            model: InstalledModel {
                name: name.into(),
                size,
                params: Some("7B".into()),
                quant: Some("Q4_K_M".into()),
                family: Some("llama".into()),
                modified: modified.into(),
                digest: format!("sha256:{name}"),
            },
            source_repo: repo.map(str::to_string),
        }
    }

    fn sample() -> InstalledState {
        let mut s = InstalledState::new();
        s.on_installed_loaded(&Ok(vec![
            entry("Qwen2.5:7b", 4_000, "2026-10-01T00:00:00Z", Some("o/qwen")),
            entry("alpha:1b", 1_000, "2026-10-20T00:00:00Z", None),
            entry("llama3:8b", 9_000, "garbage", None),
            entry("beta:3b", 2_000, "2026-10-10T00:00:00Z", Some("o/beta")),
        ]));
        s
    }

    fn names(s: &InstalledState) -> Vec<String> {
        s.visible().iter().map(|r| r.name().to_string()).collect()
    }

    #[test]
    fn text_filter_matches_name() {
        let mut s = sample();
        s.filter_text = "QWEN".into();
        assert_eq!(names(&s), ["Qwen2.5:7b"]);
        s.filter_text = "  a".into();
        assert_eq!(
            names(&s).len(),
            3,
            "filter text is trimmed: alpha, llama3, beta"
        );
        s.filter_text = "zzz".into();
        assert!(names(&s).is_empty());
        s.filter_text = "".into();
        assert_eq!(names(&s).len(), 4);
    }

    #[test]
    fn source_filter_separates_imported_from_library() {
        let mut s = sample();
        s.sort = SortKey::Name;
        s.source = SourceFilter::Imported;
        assert_eq!(names(&s), ["beta:3b", "Qwen2.5:7b"]);
        s.source = SourceFilter::Library;
        assert_eq!(names(&s), ["alpha:1b", "llama3:8b"]);
        s.source = SourceFilter::All;
        assert_eq!(names(&s).len(), 4);
        s.source = SourceFilter::Imported;
        s.filter_text = "beta".into();
        assert_eq!(names(&s), ["beta:3b"]);
    }

    #[test]
    fn sort_size_desc_name_asc_modified_recent_first() {
        let mut s = sample();
        s.sort = SortKey::Size;
        assert_eq!(
            names(&s),
            ["llama3:8b", "Qwen2.5:7b", "beta:3b", "alpha:1b"]
        );
        s.sort = SortKey::Name;
        assert_eq!(
            names(&s),
            ["alpha:1b", "beta:3b", "llama3:8b", "Qwen2.5:7b"]
        );
        s.sort = SortKey::Modified;
        assert_eq!(
            names(&s),
            ["alpha:1b", "beta:3b", "Qwen2.5:7b", "llama3:8b"],
            "unparsable dates sort last"
        );
    }
}
