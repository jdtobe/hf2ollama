//! Pure state of the Installed view: rows, filter, sort, two-click delete,
//! details cache and toast. No egui in here.

use crate::confirm::Confirm;
use crate::runtime::InstalledEntry;
use roost_core::ollama::{InstalledModel, ModelDetails};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant, SystemTime};

/// How long the toast stays.
pub const TOAST_TIME: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRow {
    pub model: InstalledModel,
    pub source_repo: Option<String>,
}

impl InstalledRow {
    pub fn name(&self) -> &str {
        &self.model.name
    }

    /// The source repository, only when it is a plain `owner/name`. History
    /// is a file on disk, so anything else is shown but never followed.
    pub fn linkable_repo(&self) -> Option<&str> {
        let repo = self.source_repo.as_deref()?;
        let plain = |p: &str| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        let (owner, name) = repo.split_once('/')?;
        (plain(owner) && plain(name)).then_some(repo)
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
    armed: Confirm,
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
            armed: Confirm::default(),
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
                self.armed.disarm_unless(live);
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
    #[cfg(test)]
    pub fn arm_delete(&mut self, name: &str, now: Instant) {
        self.armed.arm(name, now);
    }

    /// True while `name` shows "Confirm delete".
    pub fn is_armed(&self, name: &str, now: Instant) -> bool {
        self.armed.is_armed(name, now)
    }

    /// True when a click on `name` would confirm: armed, past the minimum
    /// delay and inside the window.
    pub fn confirm_ready(&self, name: &str, now: Instant) -> bool {
        self.armed.ready(name, now)
    }

    /// Second click on the same row inside the window: the delete to send.
    #[cfg(test)]
    pub fn confirm_delete(&mut self, name: &str, now: Instant) -> Option<DeleteAction> {
        self.armed.confirm(name, now).then(|| DeleteAction {
            name: name.to_string(),
        })
    }

    /// One click on a Delete button: arms, or confirms when already armed.
    pub fn click_delete(&mut self, name: &str, now: Instant) -> Option<DeleteAction> {
        self.armed.click(name, now).then(|| DeleteAction {
            name: name.to_string(),
        })
    }

    /// Expires the armed state; returns when the next expiry is due.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        self.armed.expire(now);
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
        }
        // First wake-up when the button turns ready, then at expiry.
        let armed = self.armed.next_due(now);
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

    /// Makes sure a row's details are loaded or loading. Returns the model
    /// name to ask the backend about when nothing usable is cached yet.
    pub fn want_details(&mut self, name: &str) -> Option<String> {
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

    /// Rows after the text filter, most recently modified first.
    pub fn visible(&self) -> Vec<&InstalledRow> {
        let words: Vec<String> = self
            .filter_text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let mut rows: Vec<&InstalledRow> = self
            .rows
            .iter()
            .filter(|r| {
                let hay = format!(
                    "{} {} {}",
                    r.name(),
                    r.source_repo.as_deref().unwrap_or(""),
                    r.model.quant.as_deref().unwrap_or("")
                )
                .to_lowercase();
                words.iter().all(|w| hay.contains(w))
            })
            .collect();
        // Most recently modified first, then by name.
        rows.sort_by(|a, b| {
            let (ta, tb) = (
                parse_rfc3339(&a.model.modified),
                parse_rfc3339(&b.model.modified),
            );
            // `Some` beats `None`, and a later time beats an earlier one.
            tb.cmp(&ta)
                .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        });
        rows
    }
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
mod search_tests {
    use super::*;
    use roost_core::ollama::InstalledModel;

    fn entry(name: &str, repo: Option<&str>, quant: &str) -> crate::runtime::InstalledEntry {
        crate::runtime::InstalledEntry {
            model: InstalledModel {
                name: name.into(),
                size: 1,
                params: None,
                quant: Some(quant.into()),
                family: None,
                modified: String::new(),
                digest: String::new(),
            },
            source_repo: repo.map(str::to_string),
        }
    }

    #[test]
    fn the_search_words_must_all_appear_in_the_name_repo_or_quant() {
        let mut st = InstalledState::new();
        st.on_installed_loaded(&Ok(vec![
            entry(
                "qwen2.5-coder:32b",
                Some("bartowski/Qwen2.5-Coder-32B"),
                "Q4_K_M",
            ),
            entry("llama3.2:3b", None, "Q4_K_M"),
        ]));
        let names = |st: &InstalledState| {
            st.visible()
                .iter()
                .map(|r| r.name().to_string())
                .collect::<Vec<_>>()
        };
        st.filter_text = "coder q4".into();
        assert_eq!(names(&st), ["qwen2.5-coder:32b"]);
        st.filter_text = "bartowski".into();
        assert_eq!(names(&st), ["qwen2.5-coder:32b"]);
        st.filter_text = "  ".into();
        assert_eq!(names(&st).len(), 2);
        st.filter_text = "coder llama".into();
        assert!(names(&st).is_empty());
    }

    #[test]
    fn wanting_details_asks_once_and_keeps_the_row_open() {
        let mut st = InstalledState::new();
        assert_eq!(st.want_details("m:1"), Some("m:1".to_string()));
        assert_eq!(st.want_details("m:1"), None, "already loading");
        st.on_details("m:1", &Err("boom".into()));
        assert_eq!(
            st.want_details("m:1"),
            Some("m:1".to_string()),
            "retry after an error"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confirm::{ARM_MIN_DELAY, ARM_WINDOW};
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

    // ---- details, delete results, toast ----

    #[test]
    fn delete_ok_removes_row_and_toasts() {
        let mut s = sample();
        let t0 = Instant::now();
        s.want_details("alpha:1b");
        assert!(
            s.on_delete_done("alpha:1b", &Ok(()), t0),
            "asks for a refresh"
        );
        assert!(names(&s).iter().all(|n| n != "alpha:1b"));
        assert_eq!(s.rows().len(), 3);
        assert!(
            s.details_of("alpha:1b").is_none(),
            "its details are forgotten"
        );
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

    // ---- small formatters ----

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
        for ms in [0, 50, 99] {
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
            .click_delete("alpha:1b", late + Duration::from_millis(50))
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
        assert!(!s.confirm_ready("alpha:1b", t0 + Duration::from_millis(99)));
        assert!(s.confirm_ready("alpha:1b", t0 + ARM_MIN_DELAY));
        assert!(s.confirm_ready("alpha:1b", t0 + Duration::from_millis(3_999)));
        assert!(!s.confirm_ready("alpha:1b", t0 + ARM_WINDOW), "expired");
        assert!(!s.confirm_ready("beta:3b", t0 + ARM_MIN_DELAY), "other row");
        // The repaint wakes up when the button becomes ready, then at expiry.
        assert_eq!(s.tick(t0), Some(t0 + ARM_MIN_DELAY));
        assert_eq!(s.tick(t0 + ARM_MIN_DELAY), Some(t0 + ARM_WINDOW));
    }
}
