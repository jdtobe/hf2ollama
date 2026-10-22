//! Pure state of the Installed view: rows, filter, sort, two-click delete,
//! details cache and toast. No egui in here.

use crate::runtime::InstalledEntry;
use hf2ollama_core::ollama::{InstalledModel, ModelDetails};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant, SystemTime};

/// How long the "Confirm delete" state and the toast stay.
pub const ARM_WINDOW: Duration = Duration::from_secs(4);
/// A click on the armed row is ignored until this long after arming, so a
/// double-click on Delete can never delete.
pub const ARM_MIN_DELAY: Duration = Duration::from_millis(600);
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
                // Forget what belongs to rows that are gone.
                let rows = &self.rows;
                let live = |n: &str| rows.iter().any(|r| r.name() == n);
                self.expanded.retain(|n| live(n));
                self.details.retain(|n, _| live(n));
                if self.armed.as_ref().is_some_and(|(n, _)| !live(n)) {
                    self.armed = None;
                }
            }
            Err(e) => self.status = ListStatus::Error(e.clone()),
        }
    }

    /// Ollama stopped answering: a list that never loaded becomes an error.
    pub fn on_unreachable(&mut self) {
        if self.status != ListStatus::Loaded {
            self.status = ListStatus::Error("Ollama is not reachable.".to_string());
        }
    }

    /// A new read of the list is under way (after an error or a Retry).
    pub fn begin_load(&mut self) {
        if matches!(self.status, ListStatus::Error(_)) {
            self.status = ListStatus::Loading;
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

    /// True when a click on `name` would confirm: armed, past the minimum
    /// delay and inside the window.
    pub fn confirm_ready(&self, name: &str, now: Instant) -> bool {
        self.is_armed(name, now)
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
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
        }
        let armed = self.armed.as_ref().map(|(_, at)| *at + ARM_WINDOW);
        let toast = self.toast.as_ref().map(|t| t.until);
        match (armed, toast) {
            (Some(a), Some(t)) => Some(a.min(t)),
            (a, t) => a.or(t),
        }
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

    pub fn is_expanded(&self, name: &str) -> bool {
        self.expanded.contains(name)
    }

    /// Opens or closes a row's details. Returns the model name to ask the
    /// backend about when nothing usable is cached yet.
    pub fn toggle_expanded(&mut self, name: &str) -> Option<String> {
        if self.expanded.remove(name) {
            return None;
        }
        self.expanded.insert(name.to_string());
        match self.details.get(name) {
            Some(DetailsState::Loaded(_) | DetailsState::Loading) => None,
            Some(DetailsState::Error(_)) | None => {
                self.details.insert(name.to_string(), DetailsState::Loading);
                Some(name.to_string())
            }
        }
    }

    pub fn details_of(&self, name: &str) -> Option<&DetailsState> {
        self.details.get(name)
    }

    pub fn on_details(&mut self, name: &str, r: &Result<ModelDetails, String>) {
        let state = match r {
            Ok(d) => DetailsState::Loaded(d.clone()),
            Err(e) => DetailsState::Error(e.clone()),
        };
        self.details.insert(name.to_string(), state);
    }

    /// Applies a finished delete. Returns true when the list should be
    /// re-read from Ollama to reconcile (after a success).
    pub fn on_delete_done(&mut self, name: &str, r: &Result<(), String>, now: Instant) -> bool {
        let (text, ok) = match r {
            Ok(()) => {
                self.rows.retain(|row| row.name() != name);
                self.expanded.remove(name);
                self.details.remove(name);
                (format!("Deleted {name}"), true)
            }
            Err(e) => (format!("Could not delete {name}: {e}"), false),
        };
        self.toast = Some(Toast {
            text,
            until: now + TOAST_TIME,
        });
        ok
    }

    /// The toast to show, while it has not run out.
    pub fn toast(&self, now: Instant) -> Option<&Toast> {
        self.toast.as_ref().filter(|t| now < t.until)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Model,
    Params,
    Quant,
    Size,
    Source,
    Modified,
    Actions,
}

/// Fixed width of a column; Model takes whatever is left.
pub const fn column_width(c: Column) -> f32 {
    match c {
        Column::Model => 220.0,
        Column::Params => 64.0,
        Column::Quant => 84.0,
        Column::Size => 84.0,
        Column::Source => 190.0,
        Column::Modified => 110.0,
        Column::Actions => 190.0,
    }
}

/// Space between columns.
pub const COLUMN_GAP: f32 = 12.0;

/// Columns that fit in `width`: Modified, Quant, Params and Source are
/// dropped in that order before Model, Size or the actions lose room.
pub fn visible_columns(width: f32) -> Vec<Column> {
    const ORDER: [Column; 7] = [
        Column::Model,
        Column::Params,
        Column::Quant,
        Column::Size,
        Column::Source,
        Column::Modified,
        Column::Actions,
    ];
    // Dropped first to last.
    const DROP: [Column; 4] = [
        Column::Modified,
        Column::Quant,
        Column::Params,
        Column::Source,
    ];
    let need = |cols: &[Column]| -> f32 {
        cols.iter()
            .map(|c| column_width(*c) + if *c == Column::Model { 0.0 } else { COLUMN_GAP })
            .sum()
    };
    let mut cols = ORDER.to_vec();
    for d in DROP {
        // `!(need <= width)` also drops columns for a NaN width.
        if need(&cols) <= width {
            break;
        }
        cols.retain(|c| *c != d);
    }
    cols
}

/// Horizontal extent `(column, left, right)` of each column in a row
/// `width` wide: fixed widths, gaps between, and Model taking the rest.
pub fn column_spans(cols: &[Column], width: f32) -> Vec<(Column, f32, f32)> {
    let others: f32 = cols
        .iter()
        .filter(|c| **c != Column::Model)
        .map(|c| column_width(*c) + COLUMN_GAP)
        .sum();
    let model = (width - others).max(0.0);
    let mut x = 0.0;
    cols.iter()
        .map(|c| {
            let w = if *c == Column::Model {
                model
            } else {
                x += COLUMN_GAP;
                column_width(*c)
            };
            let span = (*c, x, x + w);
            x += w;
            span
        })
        .collect()
}

/// What to paste in a terminal to chat with a model. Names with anything
/// unusual in them are single-quoted so they stay one word.
pub fn run_command(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@'));
    if plain {
        format!("ollama run {name}")
    } else {
        format!("ollama run '{}'", name.replace('\'', "'\\''"))
    }
}

/// `sha256:` plus the first 12 hex characters, or the whole short string.
pub fn short_digest(digest: &str) -> String {
    let digest = digest.trim();
    if digest.is_empty() {
        return "unknown".to_string();
    }
    match digest.strip_prefix("sha256:") {
        Some(hex) if hex.chars().count() > 12 => {
            format!("sha256:{}", hex.chars().take(12).collect::<String>())
        }
        _ => digest.to_string(),
    }
}

/// `32768` as `32,768`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
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
        assert_eq!(s.tick(t0), Some(t0 + ARM_MIN_DELAY));
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
            .confirm_delete("alpha:1b", t0 + Duration::from_secs(2))
            .is_none());
        assert!(s
            .confirm_delete("beta:3b", t0 + Duration::from_secs(2))
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
        let t1 = t0 + Duration::from_secs(1);
        assert!(s.confirm_delete("qwen2.5:7b", t1).is_none(), "case differs");
        assert!(s.confirm_delete("Qwen2.5", t1).is_none(), "prefix differs");
        assert!(s.confirm_delete("beta:3b", t1).is_none());
        assert!(s.is_armed("Qwen2.5:7b", t1), "a wrong name does not disarm");
        assert_eq!(
            s.confirm_delete("Qwen2.5:7b", t1).map(|a| a.name),
            Some("Qwen2.5:7b".to_string())
        );
        assert!(s.confirm_delete("Qwen2.5:7b", t1).is_none(), "single use");
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

    // ---- details, delete results, toast ----

    fn details(family: &str) -> ModelDetails {
        ModelDetails {
            family: family.into(),
            context: Some(32768),
            capabilities: vec!["completion".into(), "tools".into()],
            digest: "sha256:abc".into(),
            template: None,
        }
    }

    #[test]
    fn details_requested_once_and_cached() {
        let mut s = sample();
        assert_eq!(s.toggle_expanded("alpha:1b"), Some("alpha:1b".to_string()));
        assert!(s.is_expanded("alpha:1b"));
        assert_eq!(s.details_of("alpha:1b"), Some(&DetailsState::Loading));
        s.on_details("alpha:1b", &Ok(details("llama")));
        assert_eq!(
            s.details_of("alpha:1b"),
            Some(&DetailsState::Loaded(details("llama")))
        );
        assert_eq!(s.toggle_expanded("alpha:1b"), None, "collapse");
        assert!(!s.is_expanded("alpha:1b"));
        assert_eq!(
            s.toggle_expanded("alpha:1b"),
            None,
            "cached: no new request"
        );
        assert!(s.is_expanded("alpha:1b"));
        // Another row is independent.
        assert_eq!(s.toggle_expanded("beta:3b"), Some("beta:3b".to_string()));
        // A failed lookup is retried the next time the row opens.
        s.on_details("beta:3b", &Err("boom".into()));
        assert_eq!(
            s.details_of("beta:3b"),
            Some(&DetailsState::Error("boom".into()))
        );
        s.toggle_expanded("beta:3b");
        assert_eq!(s.toggle_expanded("beta:3b"), Some("beta:3b".to_string()));
    }

    #[test]
    fn delete_ok_removes_row_and_toasts() {
        let mut s = sample();
        let t0 = Instant::now();
        s.toggle_expanded("alpha:1b");
        assert!(
            s.on_delete_done("alpha:1b", &Ok(()), t0),
            "asks for a refresh"
        );
        assert!(names(&s).iter().all(|n| n != "alpha:1b"));
        assert_eq!(s.rows().len(), 3);
        assert!(!s.is_expanded("alpha:1b"));
        assert_eq!(
            s.toast(t0).map(|t| t.text.as_str()),
            Some("Deleted alpha:1b")
        );
    }

    #[test]
    fn delete_error_keeps_row_and_shows_error() {
        let mut s = sample();
        let t0 = Instant::now();
        assert!(!s.on_delete_done("alpha:1b", &Err("model is in use".into()), t0));
        assert_eq!(s.rows().len(), 4);
        let text = s.toast(t0).map(|t| t.text.clone()).unwrap_or_default();
        assert!(text.contains("model is in use"), "{text}");
        assert!(text.contains("alpha:1b"), "{text}");
    }

    #[test]
    fn toast_expires_after_4_seconds() {
        let mut s = sample();
        let t0 = Instant::now();
        s.on_delete_done("alpha:1b", &Ok(()), t0);
        assert!(s.toast(t0 + Duration::from_millis(3_999)).is_some());
        assert!(s.toast(t0 + Duration::from_secs(4)).is_none());
        assert_eq!(s.tick(t0), Some(t0 + Duration::from_secs(4)));
        assert_eq!(s.tick(t0 + Duration::from_secs(4)), None);
    }

    #[test]
    fn list_errors_keep_the_message_and_reload_clears_it() {
        let mut s = InstalledState::new();
        assert_eq!(s.status, ListStatus::Loading);
        s.on_installed_loaded(&Err("Ollama is not reachable".into()));
        assert_eq!(
            s.status,
            ListStatus::Error("Ollama is not reachable".into())
        );
        s.on_installed_loaded(&Ok(vec![]));
        assert_eq!(s.status, ListStatus::Loaded);
    }

    // ---- columns ----

    #[test]
    fn visible_columns_degrade_with_width() {
        use Column::*;
        let all = vec![Model, Params, Quant, Size, Source, Modified, Actions];
        assert_eq!(visible_columns(1132.0), all, "1180 window");
        assert_eq!(visible_columns(1014.0), all, "exact fit");
        let no_mod = vec![Model, Params, Quant, Size, Source, Actions];
        assert_eq!(visible_columns(1013.9), no_mod);
        assert_eq!(visible_columns(892.0), no_mod);
        let no_quant = vec![Model, Params, Size, Source, Actions];
        assert_eq!(visible_columns(891.9), no_quant);
        assert_eq!(visible_columns(852.0), no_quant, "900 window");
        assert_eq!(visible_columns(796.0), no_quant);
        let no_params = vec![Model, Size, Source, Actions];
        assert_eq!(visible_columns(795.9), no_params);
        assert_eq!(visible_columns(720.0), no_params);
        let core = vec![Model, Size, Actions];
        assert_eq!(visible_columns(719.9), core);
        assert_eq!(visible_columns(100.0), core, "never drops the essentials");
        assert_eq!(visible_columns(f32::NAN), core);
    }

    // ---- small formatters ----

    #[test]
    fn column_spans_fill_the_row_and_give_model_the_rest() {
        let cols = visible_columns(1132.0);
        let spans = column_spans(&cols, 1132.0);
        assert_eq!(spans.len(), 7);
        let (c, l, r) = spans[0];
        assert_eq!((c, l), (Column::Model, 0.0));
        assert!((r - 338.0).abs() < 0.01, "model right {r}");
        for pair in spans.windows(2) {
            assert!(
                (pair[1].1 - pair[0].2 - COLUMN_GAP).abs() < 0.01,
                "gap between {:?} and {:?}",
                pair[0].0,
                pair[1].0
            );
        }
        let last = spans[spans.len() - 1];
        assert_eq!(last.0, Column::Actions);
        assert!((last.2 - 1132.0).abs() < 0.01);
        assert!((last.2 - last.1 - column_width(Column::Actions)).abs() < 0.01);
        // The 900 window leaves Model more than its minimum.
        let narrow = column_spans(&visible_columns(852.0), 852.0);
        assert!(narrow[0].2 - narrow[0].1 >= column_width(Column::Model));
        // Never negative, even when asked for too little room.
        assert!(column_spans(&visible_columns(50.0), 50.0)[0].2 >= 0.0);
    }

    #[test]
    fn run_command_quotes_only_unusual_names() {
        assert_eq!(run_command("llama3.2:3b"), "ollama run llama3.2:3b");
        assert_eq!(
            run_command("hf.co/org/Name-GGUF:Q4_K_M"),
            "ollama run hf.co/org/Name-GGUF:Q4_K_M"
        );
        assert_eq!(run_command("a b"), "ollama run 'a b'");
        assert_eq!(run_command("x;rm -rf ~"), "ollama run 'x;rm -rf ~'");
        assert_eq!(run_command("it's"), "ollama run 'it'\\''s'");
    }

    #[test]
    fn digest_and_number_formatting() {
        let full = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(short_digest(&full), "sha256:abababababab");
        assert_eq!(short_digest("abc123"), "abc123");
        assert_eq!(short_digest(""), "unknown");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(32_768), "32,768");
        assert_eq!(thousands(1_048_576), "1,048,576");
    }

    // ---- minimum delay between arm and confirm ----

    #[test]
    fn second_click_inside_min_delay_is_ignored_and_does_not_rearm() {
        let mut s = sample();
        let t0 = Instant::now();
        assert!(s.click_delete("alpha:1b", t0).is_none());
        for ms in [0, 100, 599] {
            let now = t0 + Duration::from_millis(ms);
            assert!(s.click_delete("alpha:1b", now).is_none(), "{ms} ms");
            assert!(s.confirm_delete("alpha:1b", now).is_none(), "{ms} ms");
            assert!(s.is_armed("alpha:1b", now), "{ms} ms: still armed");
            assert!(!s.confirm_ready("alpha:1b", now), "{ms} ms");
        }
        // Ignored clicks did not restart the delay or the 4 s window.
        let ready = t0 + ARM_MIN_DELAY;
        assert!(s.confirm_ready("alpha:1b", ready));
        assert!(!s.is_armed("alpha:1b", t0 + ARM_WINDOW));
    }

    #[test]
    fn second_click_after_min_delay_confirms_with_exact_name() {
        let mut s = sample();
        let t0 = Instant::now();
        s.click_delete("alpha:1b", t0);
        let act = s.click_delete("alpha:1b", t0 + ARM_MIN_DELAY);
        assert_eq!(
            act,
            Some(DeleteAction {
                name: "alpha:1b".into()
            })
        );
    }

    #[test]
    fn click_after_expiry_rearms_and_restarts_the_delay() {
        let mut s = sample();
        let t0 = Instant::now();
        s.click_delete("alpha:1b", t0);
        let late = t0 + Duration::from_secs(5);
        assert!(s.click_delete("alpha:1b", late).is_none(), "re-arms");
        assert!(s.is_armed("alpha:1b", late));
        assert!(!s.confirm_ready("alpha:1b", late), "delay restarted");
        assert!(s
            .click_delete("alpha:1b", late + Duration::from_millis(300))
            .is_none());
        assert!(s.click_delete("alpha:1b", late + ARM_MIN_DELAY).is_some());
    }

    #[test]
    fn armed_button_is_dimmed_during_the_delay_and_enabled_after() {
        let mut s = sample();
        let t0 = Instant::now();
        assert!(!s.confirm_ready("alpha:1b", t0), "not armed");
        s.arm_delete("alpha:1b", t0);
        assert!(!s.confirm_ready("alpha:1b", t0));
        assert!(!s.confirm_ready("alpha:1b", t0 + Duration::from_millis(599)));
        assert!(s.confirm_ready("alpha:1b", t0 + ARM_MIN_DELAY));
        assert!(s.confirm_ready("alpha:1b", t0 + Duration::from_millis(3_999)));
        assert!(!s.confirm_ready("alpha:1b", t0 + ARM_WINDOW), "expired");
        assert!(!s.confirm_ready("beta:3b", t0 + ARM_MIN_DELAY), "other row");
        // The repaint wakes up when the button becomes ready, then at expiry.
        assert_eq!(s.tick(t0), Some(t0 + ARM_MIN_DELAY));
        assert_eq!(s.tick(t0 + ARM_MIN_DELAY), Some(t0 + ARM_WINDOW));
    }
}
