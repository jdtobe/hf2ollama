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
        let repo = self.source_repo.as_deref()?;
        // History is a file on disk: only a plain `owner/name` becomes a link.
        let plain = |p: &str| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        let (owner, name) = repo.split_once('/')?;
        (plain(owner) && plain(name)).then(|| format!("https://huggingface.co/{repo}"))
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

    /// First click on Delete: the row shows "Confirm delete" for `ARM_WINDOW`.
    /// Arming another row disarms the first.
    pub fn arm_delete(&mut self, name: &str, now: Instant) {
        self.armed = Some((name.to_string(), now));
    }

    /// True while `name` shows "Confirm delete".
    pub fn is_armed(&self, name: &str, now: Instant) -> bool {
        self.armed
            .as_ref()
            .is_some_and(|(n, at)| n == name && now < *at + ARM_WINDOW)
    }

    /// Second click on the same row inside the window: the delete to send.
    pub fn confirm_delete(&mut self, name: &str, now: Instant) -> Option<DeleteAction> {
        if !self.is_armed(name, now) {
            return None;
        }
        self.armed = None;
        Some(DeleteAction {
            name: name.to_string(),
        })
    }

    /// One click on a Delete button: arms, or confirms when already armed.
    pub fn click_delete(&mut self, name: &str, now: Instant) -> Option<DeleteAction> {
        let action = self.confirm_delete(name, now);
        if action.is_none() {
            self.arm_delete(name, now);
        }
        action
    }

    /// Expires the armed state; returns when the next expiry is due.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        if self
            .armed
            .as_ref()
            .is_some_and(|(_, at)| now >= *at + ARM_WINDOW)
        {
            self.armed = None;
        }
        self.armed.as_ref().map(|(_, at)| *at + ARM_WINDOW)
    }

    /// Bytes on disk by source, over every installed model.
    pub fn usage(&self) -> UsageSplit {
        self.rows.iter().fold(UsageSplit::default(), |mut u, r| {
            if r.source_repo.is_some() {
                u.imported = u.imported.saturating_add(r.model.size);
            } else {
                u.library = u.library.saturating_add(r.model.size);
            }
            u
        })
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

    // ---- delete arming, usage, source link ----

    #[test]
    fn first_click_arms_second_click_confirms() {
        let mut s = sample();
        let t0 = Instant::now();
        assert!(s.click_delete("alpha:1b", t0).is_none());
        assert!(s.is_armed("alpha:1b", t0));
        let act = s.click_delete("alpha:1b", t0 + Duration::from_secs(2));
        assert_eq!(
            act,
            Some(DeleteAction {
                name: "alpha:1b".into()
            })
        );
        assert!(!s.is_armed("alpha:1b", t0 + Duration::from_secs(2)));
        // The confirmation is spent; the next click arms again.
        assert!(s
            .click_delete("alpha:1b", t0 + Duration::from_secs(3))
            .is_none());
    }

    #[test]
    fn armed_state_expires_after_4_seconds() {
        let mut s = sample();
        let t0 = Instant::now();
        s.arm_delete("alpha:1b", t0);
        assert!(s.is_armed("alpha:1b", t0 + Duration::from_millis(3_999)));
        assert!(!s.is_armed("alpha:1b", t0 + Duration::from_secs(4)));
        assert_eq!(s.tick(t0), Some(t0 + Duration::from_secs(4)));
        assert!(s.is_armed("alpha:1b", t0 + Duration::from_millis(3_999)));
        assert!(s.tick(t0 + Duration::from_millis(3_999)).is_some());
        assert_eq!(s.tick(t0 + Duration::from_secs(4)), None);
        assert!(!s.is_armed("alpha:1b", t0));
        assert!(s
            .confirm_delete("alpha:1b", t0 + Duration::from_secs(5))
            .is_none());
    }

    #[test]
    fn arming_another_row_disarms_the_first() {
        let mut s = sample();
        let t0 = Instant::now();
        s.arm_delete("alpha:1b", t0);
        s.arm_delete("beta:3b", t0 + Duration::from_secs(1));
        assert!(!s.is_armed("alpha:1b", t0 + Duration::from_secs(1)));
        assert!(s.is_armed("beta:3b", t0 + Duration::from_secs(1)));
        assert!(s
            .confirm_delete("alpha:1b", t0 + Duration::from_secs(1))
            .is_none());
        assert!(s
            .confirm_delete("beta:3b", t0 + Duration::from_secs(1))
            .is_some());
    }

    #[test]
    fn click_after_expiry_rearms_instead_of_confirming() {
        let mut s = sample();
        let t0 = Instant::now();
        assert!(s.click_delete("alpha:1b", t0).is_none());
        let late = t0 + Duration::from_secs(5);
        assert!(
            s.click_delete("alpha:1b", late).is_none(),
            "expired: re-arm"
        );
        assert!(s.is_armed("alpha:1b", late));
        assert!(s
            .click_delete("alpha:1b", late + Duration::from_secs(1))
            .is_some());
    }

    #[test]
    fn confirm_returns_exact_name_only() {
        let mut s = sample();
        let t0 = Instant::now();
        s.arm_delete("Qwen2.5:7b", t0);
        assert!(s.confirm_delete("qwen2.5:7b", t0).is_none(), "case differs");
        assert!(s.confirm_delete("Qwen2.5", t0).is_none(), "prefix differs");
        assert!(s.confirm_delete("beta:3b", t0).is_none());
        assert!(s.is_armed("Qwen2.5:7b", t0), "a wrong name does not disarm");
        assert_eq!(
            s.confirm_delete("Qwen2.5:7b", t0).map(|a| a.name),
            Some("Qwen2.5:7b".to_string())
        );
        assert!(s.confirm_delete("Qwen2.5:7b", t0).is_none(), "single use");
    }

    #[test]
    fn usage_split_sums_by_source() {
        let s = sample();
        assert_eq!(
            s.usage(),
            UsageSplit {
                imported: 6_000,
                library: 10_000
            }
        );
        assert_eq!(InstalledState::new().usage(), UsageSplit::default());
    }

    #[test]
    fn source_link_is_huggingface_url_for_imported_only() {
        let s = sample();
        let url = |n: &str| {
            s.rows()
                .iter()
                .find(|r| r.name() == n)
                .and_then(|r| r.source_url())
        };
        assert_eq!(
            url("Qwen2.5:7b"),
            Some("https://huggingface.co/o/qwen".to_string())
        );
        assert_eq!(url("alpha:1b"), None);
    }
}
