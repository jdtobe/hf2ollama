//! Pure state of the Settings view: text buffers with their validation
//! errors, the change flags, token and connection status, the cached files
//! list and the two-step clear. No egui in here.

use super::validate::{
    check_staging_dir, clamp_max_parallel, parse_gb, validate_hf_endpoint, validate_ollama_url,
    validate_staging_dir,
};
use crate::browse::results::format_size;
use crate::confirm::Confirm;
use crate::runtime::{BackendRequest, CachedRow, HfToken, HfTokenStatus};
use roost_core::config::{AutoDismiss, Settings, ThemeFamily, ThemeMode};
use roost_core::fit::Machine;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How long "Saved" stays next to the heading.
pub const SAVED_SHOWN: Duration = Duration::from_secs(3);
/// How long the "Freed X GB" note stays.
pub const NOTE_SHOWN: Duration = Duration::from_secs(4);
/// Key of the one armed button in this view.
pub const CLEAR_KEY: &str = "clear-cached";

/// Which fields a user action changed, so the app persists and applies only
/// what changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsChange {
    pub ollama_url: bool,
    pub staging_dir: bool,
    pub delete_after_import: bool,
    pub keep_partials: bool,
    pub max_parallel: bool,
    pub auto_dismiss: bool,
    pub hf_endpoint: bool,
    pub hardware: bool,
    pub theme: bool,
}

impl SettingsChange {
    pub fn any(self) -> bool {
        self != SettingsChange::default()
    }

    pub fn merge(&mut self, other: SettingsChange) {
        self.ollama_url |= other.ollama_url;
        self.staging_dir |= other.staging_dir;
        self.delete_after_import |= other.delete_after_import;
        self.keep_partials |= other.keep_partials;
        self.max_parallel |= other.max_parallel;
        self.auto_dismiss |= other.auto_dismiss;
        self.hf_endpoint |= other.hf_endpoint;
        self.hardware |= other.hardware;
        self.theme |= other.theme;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Idle,
    Testing,
    Done(Result<String, String>),
}

/// The line shown next to the Test button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionMessage {
    pub ok: bool,
    pub text: String,
}

pub fn connection_message(r: &Result<String, String>) -> ConnectionMessage {
    match r {
        Ok(version) => ConnectionMessage {
            ok: true,
            text: format!("Connected, Ollama {version}"),
        },
        Err(e) => ConnectionMessage {
            ok: false,
            text: format!("Could not connect: {e}"),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveIndicator {
    Idle,
    Saving,
    Saved { until: Instant },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CachedStatus {
    Loading,
    Loaded(Vec<CachedRow>),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Neutral,
    Ok,
    Bad,
}

/// Text and colour class of the token badge.
pub fn token_badge(status: &HfTokenStatus) -> (BadgeKind, String) {
    match status {
        HfTokenStatus::Checking => (BadgeKind::Neutral, "Checking keychain…".to_string()),
        HfTokenStatus::None => (BadgeKind::Neutral, "No token saved".to_string()),
        HfTokenStatus::Saved => (BadgeKind::Neutral, "Saved, not checked yet".to_string()),
        HfTokenStatus::Valid(name) => (BadgeKind::Ok, format!("Valid, signed in as {name}")),
        HfTokenStatus::Invalid(why) => (BadgeKind::Bad, format!("Invalid: {why}")),
        HfTokenStatus::Unreachable(why) => (BadgeKind::Neutral, format!("Could not check: {why}")),
    }
}

/// "3 folders, 4.68 GB" for the Cached files list.
pub fn cached_total(rows: &[CachedRow]) -> String {
    if rows.is_empty() {
        return "Nothing cached".to_string();
    }
    let bytes = rows.iter().fold(0u64, |a, r| a.saturating_add(r.bytes));
    let n = rows.len();
    format!(
        "{n} {}, {}",
        if n == 1 { "folder" } else { "folders" },
        format_size(bytes)
    )
}

/// Help text for the "keep partial downloads" toggle.
pub const KEEP_PARTIALS_HELP: &str = "Keeps the files of a failed download for 7 days, then removes them automatically. You can clear them sooner. Resuming after a restart is not supported yet. Cancelling always deletes the partial file.";

/// Help text under the cached files total.
pub const CACHED_HELP: &str = "Used in the staging folder right now. Failed downloads are kept for 7 days and then removed; clear them any time.";

/// True when the Clear button may be used: any listed row can be removed.
pub fn has_clearable(rows: &[CachedRow]) -> bool {
    !rows.is_empty()
}

/// The machine used for fit estimates: the override, else what was detected.
pub fn effective_machine(settings: &Settings, detected: &Machine) -> Machine {
    settings
        .hardware_override
        .clone()
        .unwrap_or_else(|| detected.clone())
}

/// "16 GB VRAM · 32 GB RAM", or an honest "VRAM not detected".
pub fn detected_line(detected: Option<&Machine>) -> String {
    let Some(m) = detected else {
        return "Hardware not detected".to_string();
    };
    let vram = match m.vram_gb {
        Some(v) => format!("{v:.0} GB VRAM"),
        None => "VRAM not detected".to_string(),
    };
    let ram = if m.ram_gb > 0.0 {
        format!("{:.0} GB RAM", m.ram_gb)
    } else {
        "RAM not detected".to_string()
    };
    format!("{vram} · {ram}")
}

/// `64`, `24.5`: no trailing zeros.
fn gb_text(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn not_saved(e: String) -> String {
    format!("Not saved. {e}")
}

/// There is deliberately no `Debug`: `token_input` holds what was typed.
pub struct SettingsState {
    pub ollama_text: String,
    pub ollama_error: Option<String>,
    pub staging_text: String,
    pub staging_error: Option<String>,
    pub hf_text: String,
    pub hf_error: Option<String>,
    pub override_on: bool,
    pub ram_text: String,
    pub vram_text: String,
    pub ram_error: Option<String>,
    pub vram_error: Option<String>,
    /// What is typed in the masked field; cleared as soon as it is sent.
    pub token_input: String,
    pub token_status: HfTokenStatus,
    pub token_busy: bool,
    /// The last legacy token import failed; shown so the user can retry.
    pub legacy_import_failed: bool,
    pub connection: ConnectionState,
    pub cached: CachedStatus,
    pub clear: Confirm,
    pub save: SaveIndicator,
    pub note: Option<(String, Instant)>,
    default_staging: PathBuf,
    detected: Option<Machine>,
    pending: Vec<BackendRequest>,
    /// The change being saved moved the staging folder.
    staging_moved: bool,
    dev_memory_store: bool,
    legacy_import_available: bool,
}

impl SettingsState {
    pub fn new(
        settings: &Settings,
        default_staging: PathBuf,
        detected: Option<Machine>,
    ) -> SettingsState {
        let m = settings.hardware_override.as_ref();
        SettingsState {
            ollama_text: settings.ollama_url.clone(),
            ollama_error: None,
            staging_text: settings
                .staging_dir
                .as_deref()
                .unwrap_or(&default_staging)
                .display()
                .to_string(),
            staging_error: None,
            hf_text: settings.hf_endpoint.clone(),
            hf_error: None,
            override_on: m.is_some(),
            ram_text: m.map(|m| gb_text(m.ram_gb)).unwrap_or_default(),
            vram_text: m.and_then(|m| m.vram_gb).map(gb_text).unwrap_or_default(),
            ram_error: None,
            vram_error: None,
            token_input: String::new(),
            token_status: HfTokenStatus::Checking,
            token_busy: true,
            legacy_import_failed: false,
            connection: ConnectionState::Idle,
            cached: CachedStatus::Loading,
            clear: Confirm::default(),
            save: SaveIndicator::Idle,
            note: None,
            default_staging,
            detected,
            pending: Vec::new(),
            staging_moved: false,
            dev_memory_store: false,
            legacy_import_available: true,
        }
    }

    pub fn detected(&self) -> Option<&Machine> {
        self.detected.as_ref()
    }

    /// Marks that the token store is in memory only (dev-hooks builds).
    #[cfg(any(test, feature = "dev-hooks"))]
    pub fn set_dev_memory_store(&mut self, on: bool) {
        self.dev_memory_store = on;
        self.legacy_import_available = !on;
    }

    /// A note shown next to the token status in dev-hooks runs.
    pub fn token_note(&self) -> Option<&'static str> {
        self.dev_memory_store.then_some("dev: token not persisted")
    }

    /// Requests queued for the backend since the last call.
    pub fn take_requests(&mut self) -> Vec<BackendRequest> {
        std::mem::take(&mut self.pending)
    }

    // ---- text fields: `edit_*` validates while typing, `commit_*` applies ----

    pub fn edit_ollama(&mut self) {
        self.ollama_error = validate_ollama_url(&self.ollama_text).err();
        if matches!(self.connection, ConnectionState::Done(_)) {
            self.connection = ConnectionState::Idle;
        }
    }

    pub fn commit_ollama(&mut self, s: &mut Settings) -> SettingsChange {
        let mut change = SettingsChange::default();
        match validate_ollama_url(&self.ollama_text) {
            Ok(url) => {
                self.ollama_error = None;
                if url != s.ollama_url {
                    s.ollama_url.clone_from(&url);
                    change.ollama_url = true;
                }
                self.ollama_text = url;
            }
            Err(e) => self.ollama_error = Some(not_saved(e)),
        }
        change
    }

    pub fn edit_hf_endpoint(&mut self) {
        self.hf_error = validate_hf_endpoint(&self.hf_text).err();
    }

    pub fn commit_hf_endpoint(&mut self, s: &mut Settings) -> SettingsChange {
        let mut change = SettingsChange::default();
        match validate_hf_endpoint(&self.hf_text) {
            Ok(url) => {
                self.hf_error = None;
                if url != s.hf_endpoint {
                    s.hf_endpoint.clone_from(&url);
                    change.hf_endpoint = true;
                }
                self.hf_text = url;
            }
            Err(e) => self.hf_error = Some(not_saved(e)),
        }
        change
    }

    /// `None` means the default folder (empty text, or the default path).
    fn staging_choice(&self) -> Option<PathBuf> {
        let text = self.staging_text.trim();
        let path = PathBuf::from(text);
        (!text.is_empty() && path != self.default_staging).then_some(path)
    }

    pub fn edit_staging(&mut self) {
        self.staging_error = match self.staging_choice() {
            Some(path) => check_staging_dir(&path).err(),
            None => None,
        };
    }

    pub fn commit_staging(&mut self, s: &mut Settings) -> SettingsChange {
        let mut change = SettingsChange::default();
        let choice = self.staging_choice();
        if choice == s.staging_dir {
            // Unchanged (the default folder may not exist yet: no check).
            self.staging_error = None;
        } else {
            let verdict = match &choice {
                Some(path) => validate_staging_dir(path),
                None => Ok(()),
            };
            match verdict {
                Ok(()) => {
                    self.staging_error = None;
                    s.staging_dir = choice;
                    change.staging_dir = true;
                }
                Err(e) => self.staging_error = Some(not_saved(e)),
            }
        }
        if self.staging_error.is_none() {
            self.staging_text = s
                .staging_dir
                .as_deref()
                .unwrap_or(&self.default_staging)
                .display()
                .to_string();
        }
        change
    }

    // ---- toggles and selects ----

    fn flag<T: PartialEq>(field: &mut T, value: T) -> bool {
        let differs = *field != value;
        *field = value;
        differs
    }

    pub fn set_delete_after_import(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        SettingsChange {
            delete_after_import: Self::flag(&mut s.delete_after_import, on),
            ..Default::default()
        }
    }

    pub fn set_keep_partials(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        SettingsChange {
            keep_partials: Self::flag(&mut s.keep_partials, on),
            ..Default::default()
        }
    }

    pub fn set_max_parallel(&mut self, s: &mut Settings, n: u8) -> SettingsChange {
        SettingsChange {
            max_parallel: Self::flag(&mut s.max_parallel, clamp_max_parallel(n)),
            ..Default::default()
        }
    }

    pub fn set_auto_dismiss(&mut self, s: &mut Settings, v: AutoDismiss) -> SettingsChange {
        SettingsChange {
            auto_dismiss: Self::flag(&mut s.auto_dismiss, v),
            ..Default::default()
        }
    }

    pub fn set_theme(&mut self, s: &mut Settings, v: ThemeMode) -> SettingsChange {
        SettingsChange {
            theme: Self::flag(&mut s.theme_mode, v),
            ..Default::default()
        }
    }

    pub fn set_theme_family(&mut self, s: &mut Settings, v: ThemeFamily) -> SettingsChange {
        SettingsChange {
            theme: Self::flag(&mut s.theme_family, v),
            ..Default::default()
        }
    }

    // ---- hardware override ----

    /// The numbers as typed: RAM at least 1 GB, VRAM may be empty (unknown).
    fn parse_hardware(&self) -> (Result<f32, String>, Result<Option<f32>, String>) {
        let ram = parse_gb(&self.ram_text).and_then(|v| {
            if v >= 1.0 {
                Ok(v)
            } else {
                Err("RAM must be at least 1 GB.".to_string())
            }
        });
        let vram = if self.vram_text.trim().is_empty() {
            Ok(None)
        } else {
            parse_gb(&self.vram_text).map(Some)
        };
        (ram, vram)
    }

    pub fn set_override(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        let mut change = SettingsChange::default();
        if on == self.override_on {
            return change;
        }
        self.override_on = on;
        self.ram_error = None;
        self.vram_error = None;
        if !on {
            if s.hardware_override.take().is_some() {
                change.hardware = true;
            }
            return change;
        }
        // Start from what was detected, so the numbers are never blank.
        if let Some(d) = self.detected.clone() {
            self.ram_text = if d.ram_gb > 0.0 {
                gb_text(d.ram_gb)
            } else {
                String::new()
            };
            self.vram_text = d.vram_gb.map(gb_text).unwrap_or_default();
            change = self.commit_hardware(s);
        }
        change
    }

    pub fn edit_hardware(&mut self) {
        let (ram, vram) = self.parse_hardware();
        self.ram_error = ram.err();
        self.vram_error = vram.err();
    }

    pub fn commit_hardware(&mut self, s: &mut Settings) -> SettingsChange {
        let mut change = SettingsChange::default();
        if !self.override_on {
            return change;
        }
        let (ram, vram) = self.parse_hardware();
        self.ram_error = ram.clone().err();
        self.vram_error = vram.clone().err();
        if let (Ok(ram_gb), Ok(vram_gb)) = (ram, vram) {
            let machine = Machine { ram_gb, vram_gb };
            if s.hardware_override.as_ref() != Some(&machine) {
                s.hardware_override = Some(machine);
                change.hardware = true;
            }
        }
        change
    }

    // ---- Hugging Face token ----

    pub fn save_token(&mut self) -> bool {
        if self.token_busy {
            return false;
        }
        let token = self.token_input.trim().to_string();
        // Gone from the field at once, whatever happens next.
        self.token_input.clear();
        if token.is_empty() {
            return false;
        }
        self.pending
            .push(BackendRequest::SetHfToken(Some(HfToken::new(token))));
        self.token_busy = true;
        true
    }

    pub fn clear_token(&mut self) {
        if self.token_busy {
            return;
        }
        self.token_input.clear();
        self.pending.push(BackendRequest::SetHfToken(None));
        self.token_busy = true;
    }

    pub fn on_token_status(&mut self, status: &HfTokenStatus) {
        self.token_status = status.clone();
        self.token_busy = false;
    }

    pub fn can_import_legacy_token(&self) -> bool {
        self.legacy_import_available && self.token_status == HfTokenStatus::None && !self.token_busy
    }

    pub fn import_legacy_token(&mut self) {
        if self.can_import_legacy_token() {
            self.legacy_import_failed = false;
            self.pending.push(BackendRequest::ImportLegacyToken);
            self.token_busy = true;
        }
    }

    pub fn on_legacy_token_imported(
        &mut self,
        result: &Result<roost_core::secrets::LegacyTokenMigration, String>,
    ) {
        self.token_busy = false;
        use roost_core::secrets::LegacyTokenMigration::{AlreadyPresent, Imported, NotFound};
        self.legacy_import_failed = result.is_err();
        match result {
            Ok(Imported | AlreadyPresent) => self.token_status = HfTokenStatus::Saved,
            Ok(NotFound) | Err(_) => {}
        }
    }

    // ---- connection test ----

    pub fn test_connection(&mut self) {
        match validate_ollama_url(&self.ollama_text) {
            Ok(url) => {
                self.ollama_error = None;
                self.connection = ConnectionState::Testing;
                self.pending.push(BackendRequest::TestConnection(url));
            }
            Err(e) => {
                self.ollama_error = Some(e);
                self.connection = ConnectionState::Idle;
            }
        }
    }

    pub fn on_connection_test(&mut self, r: &Result<String, String>) {
        self.connection = ConnectionState::Done(r.clone());
    }

    // ---- cached files ----

    pub fn refresh_cached(&mut self) {
        if matches!(self.cached, CachedStatus::Error(_)) {
            self.cached = CachedStatus::Loading;
        }
        self.pending.push(BackendRequest::ListCached);
    }

    pub fn on_cached_loaded(&mut self, r: &Result<Vec<CachedRow>, String>) {
        self.cached = match r {
            Ok(rows) => CachedStatus::Loaded(rows.clone()),
            Err(e) => CachedStatus::Error(e.clone()),
        };
    }

    pub fn click_clear_cached(&mut self, now: Instant) {
        if self.clear.click(CLEAR_KEY, now) {
            self.pending.push(BackendRequest::ClearCached);
        }
    }

    pub fn on_cached_cleared(&mut self, r: &Result<u64, String>, now: Instant) {
        let text = match r {
            Ok(bytes) => format!("Freed {}", format_size(*bytes)),
            Err(e) => format!("Could not clear cached files: {e}"),
        };
        self.note = Some((text, now + NOTE_SHOWN));
        self.pending.push(BackendRequest::ListCached);
    }

    /// The user opened the Settings view: read the cached files again, and
    /// ask Hugging Face about a saved token once.
    pub fn on_opened(&mut self) {
        self.refresh_cached();
        if self.token_status == HfTokenStatus::Saved && !self.token_busy {
            self.token_busy = true;
            self.pending.push(BackendRequest::CheckHfToken);
        }
    }

    // ---- saving ----

    /// A change was applied: the file is being written.
    pub fn note_change(&mut self, change: SettingsChange) {
        if change.any() {
            self.save = SaveIndicator::Saving;
            self.staging_moved |= change.staging_dir;
        }
    }

    pub fn on_saved(&mut self, r: &Result<(), String>, now: Instant) {
        match r {
            Ok(()) => {
                self.save = SaveIndicator::Saved {
                    until: now + SAVED_SHOWN,
                };
                if std::mem::take(&mut self.staging_moved) {
                    // The cached files live in the staging folder.
                    self.refresh_cached();
                }
            }
            Err(e) => self.save = SaveIndicator::Failed(e.clone()),
        }
    }

    /// Expires indicators; returns when the next one is due.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        if matches!(self.save, SaveIndicator::Saved { until } if now >= until) {
            self.save = SaveIndicator::Idle;
        }
        if self.note.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.note = None;
        }
        self.clear.expire(now);
        [
            match self.save {
                SaveIndicator::Saved { until } => Some(until),
                _ => None,
            },
            self.note.as_ref().map(|(_, until)| *until),
            self.clear.next_due(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_for(s: &Settings) -> SettingsState {
        SettingsState::new(
            s,
            PathBuf::from("/tmp/default-staging"),
            Some(Machine {
                ram_gb: 32.0,
                vram_gb: Some(16.0),
            }),
        )
    }

    fn fresh() -> (Settings, SettingsState) {
        let s = Settings::default();
        let st = state_for(&s);
        (s, st)
    }

    fn token_ready(st: &mut SettingsState) {
        st.on_token_status(&HfTokenStatus::None);
    }

    fn flags(c: SettingsChange) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (on, name) in [
            (c.ollama_url, "ollama_url"),
            (c.staging_dir, "staging_dir"),
            (c.delete_after_import, "delete_after_import"),
            (c.keep_partials, "keep_partials"),
            (c.max_parallel, "max_parallel"),
            (c.auto_dismiss, "auto_dismiss"),
            (c.hf_endpoint, "hf_endpoint"),
            (c.hardware, "hardware"),
            (c.theme, "theme"),
        ] {
            if on {
                v.push(name);
            }
        }
        v
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn new_state_shows_the_current_values() {
        let s = Settings {
            ollama_url: "http://box:11434".into(),
            hf_endpoint: "https://mirror.example".into(),
            staging_dir: Some(PathBuf::from("/data/stage")),
            hardware_override: Some(Machine {
                ram_gb: 64.0,
                vram_gb: Some(24.0),
            }),
            ..Settings::default()
        };
        let st = state_for(&s);
        assert_eq!(st.ollama_text, "http://box:11434");
        assert_eq!(st.hf_text, "https://mirror.example");
        assert_eq!(st.staging_text, "/data/stage");
        assert!(st.override_on);
        assert_eq!((st.ram_text.as_str(), st.vram_text.as_str()), ("64", "24"));
        let d = state_for(&Settings::default());
        assert_eq!(
            d.staging_text, "/tmp/default-staging",
            "the effective folder"
        );
        assert!(!d.override_on);
        assert_eq!(d.cached, CachedStatus::Loading);
    }

    #[test]
    fn new_state_waits_for_current_token_status_before_offering_import() {
        let (_s, st) = fresh();
        assert_eq!(st.token_status, HfTokenStatus::Checking);
        assert!(st.token_busy);
        assert!(!st.can_import_legacy_token());
    }

    #[test]
    fn toggling_delete_after_import_reports_change() {
        let (mut s, mut st) = fresh();
        assert!(s.delete_after_import);
        let c = st.set_delete_after_import(&mut s, false);
        assert!(c.delete_after_import);
        assert!(!s.delete_after_import);
        assert_eq!(flags(c), ["delete_after_import"]);
        // The same value again is not a change.
        assert!(!st.set_delete_after_import(&mut s, false).any());
        let c = st.set_delete_after_import(&mut s, true);
        assert!(c.delete_after_import && s.delete_after_import);
    }

    #[test]
    fn settings_change_flags_only_the_changed_fields() {
        let (mut s, mut st) = fresh();
        assert_eq!(
            flags(st.set_keep_partials(&mut s, false)),
            ["keep_partials"]
        );
        assert_eq!(flags(st.set_max_parallel(&mut s, 3)), ["max_parallel"]);
        assert_eq!(
            flags(st.set_auto_dismiss(&mut s, AutoDismiss::OneHour)),
            ["auto_dismiss"]
        );
        assert_eq!(flags(st.set_theme(&mut s, ThemeMode::Dark)), ["theme"]);
        assert_eq!(
            flags(st.set_theme_family(&mut s, ThemeFamily::Plum)),
            ["theme"]
        );
        assert_eq!(s.theme_family, ThemeFamily::Plum);
        assert_eq!(
            flags(st.set_theme_family(&mut s, ThemeFamily::Plum)),
            [] as [&str; 0]
        );
        st.ollama_text = "http://box:11434".into();
        assert_eq!(flags(st.commit_ollama(&mut s)), ["ollama_url"]);
        st.hf_text = "https://mirror.example/".into();
        assert_eq!(flags(st.commit_hf_endpoint(&mut s)), ["hf_endpoint"]);
        assert_eq!(s.hf_endpoint, "https://mirror.example");
        let dir = tempfile::tempdir().unwrap();
        st.staging_text = dir.path().display().to_string();
        assert_eq!(flags(st.commit_staging(&mut s)), ["staging_dir"]);
        assert_eq!(s.staging_dir.as_deref(), Some(dir.path()));
        assert_eq!(flags(st.set_override(&mut s, true)), ["hardware"]);
        // Nothing else moved.
        assert!(s.delete_after_import);
        // Committing unchanged text changes nothing.
        assert!(!st.commit_ollama(&mut s).any());
        assert!(!st.commit_hf_endpoint(&mut s).any());
        assert!(!st.commit_staging(&mut s).any());
        assert!(!st.commit_hardware(&mut s).any());
        // Merging collects flags.
        let mut all = SettingsChange::default();
        all.merge(SettingsChange {
            theme: true,
            ..Default::default()
        });
        all.merge(SettingsChange {
            hardware: true,
            ..Default::default()
        });
        assert_eq!(flags(all), ["hardware", "theme"]);
    }

    #[test]
    fn max_parallel_is_clamped_by_the_state_too() {
        let (mut s, mut st) = fresh();
        let c = st.set_max_parallel(&mut s, 9);
        assert!(c.max_parallel);
        assert_eq!(s.max_parallel, 3);
        let c = st.set_max_parallel(&mut s, 0);
        assert!(c.max_parallel);
        assert_eq!(s.max_parallel, 1);
        assert!(!st.set_max_parallel(&mut s, 1).any());
    }

    #[test]
    fn invalid_values_are_not_applied_or_saved() {
        let (mut s, mut st) = fresh();
        let before = s.clone();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();

        st.ollama_text = "ftp://nope".into();
        st.edit_ollama();
        assert!(st.ollama_error.is_some(), "shown while typing");
        assert!(!st.commit_ollama(&mut s).any());
        st.hf_text = "not a url".into();
        st.edit_hf_endpoint();
        assert!(st.hf_error.is_some());
        assert!(!st.commit_hf_endpoint(&mut s).any());
        st.staging_text = file.display().to_string();
        st.edit_staging();
        assert!(st.staging_error.is_some());
        assert!(!st.commit_staging(&mut s).any());
        st.set_override(&mut s, true);
        let with_override = s.clone();
        st.ram_text = "0".into();
        st.edit_hardware();
        assert!(st.ram_error.is_some());
        assert!(!st.commit_hardware(&mut s).any());
        st.ram_text = "32".into();
        st.vram_text = "-4".into();
        st.edit_hardware();
        assert!(st.ram_error.is_none() && st.vram_error.is_some());
        assert!(!st.commit_hardware(&mut s).any());
        assert_eq!(s, with_override, "the last valid hardware is kept");

        s.hardware_override = before.hardware_override.clone();
        assert_eq!(s, before, "the live settings never saw the bad values");
        // Typing a good value clears the message and applies it.
        st.ollama_text = "http://box:11434".into();
        st.edit_ollama();
        assert!(st.ollama_error.is_none());
        assert!(st.commit_ollama(&mut s).ollama_url);
        assert_eq!(s.ollama_url, "http://box:11434");
    }

    #[test]
    fn a_failed_commit_explains_that_nothing_was_saved() {
        let (mut s, mut st) = fresh();
        st.ollama_text = "".into();
        st.commit_ollama(&mut s);
        let e = st.ollama_error.clone().unwrap();
        assert!(e.starts_with("Not saved."), "{e}");
        assert_eq!(s.ollama_url, "http://localhost:11434");
    }

    #[test]
    fn committed_urls_are_normalized_in_the_field() {
        let (mut s, mut st) = fresh();
        st.ollama_text = "  http://box:11434/  ".into();
        let c = st.commit_ollama(&mut s);
        assert!(c.ollama_url);
        assert_eq!(st.ollama_text, "http://box:11434");
        assert_eq!(s.ollama_url, "http://box:11434");
    }

    #[test]
    fn empty_staging_text_means_the_default_folder() {
        let (mut s, mut st) = fresh();
        let dir = tempfile::tempdir().unwrap();
        st.staging_text = dir.path().display().to_string();
        assert!(st.commit_staging(&mut s).staging_dir);
        st.staging_text = "  ".into();
        st.edit_staging();
        assert!(st.staging_error.is_none());
        let c = st.commit_staging(&mut s);
        assert!(c.staging_dir);
        assert_eq!(s.staging_dir, None);
        assert_eq!(st.staging_text, "/tmp/default-staging");
        // Typing the default path itself is also "default".
        st.staging_text = "/tmp/default-staging".into();
        assert!(!st.commit_staging(&mut s).any());
    }

    #[test]
    fn hardware_override_toggle_prefills_from_detected_and_applies() {
        let (mut s, mut st) = fresh();
        let c = st.set_override(&mut s, true);
        assert!(c.hardware);
        assert!(st.override_on);
        assert_eq!(
            s.hardware_override,
            Some(Machine {
                ram_gb: 32.0,
                vram_gb: Some(16.0)
            })
        );
        assert_eq!((st.ram_text.as_str(), st.vram_text.as_str()), ("32", "16"));
        // Editing the numbers applies on commit.
        st.ram_text = "64".into();
        st.vram_text = "".into();
        st.edit_hardware();
        assert!(
            st.ram_error.is_none() && st.vram_error.is_none(),
            "empty VRAM is unknown"
        );
        let c = st.commit_hardware(&mut s);
        assert!(c.hardware);
        assert_eq!(
            s.hardware_override,
            Some(Machine {
                ram_gb: 64.0,
                vram_gb: None
            })
        );
        // Turning it off drops the override.
        let c = st.set_override(&mut s, false);
        assert!(c.hardware && !st.override_on);
        assert_eq!(s.hardware_override, None);
        assert!(!st.set_override(&mut s, false).any());
    }

    #[test]
    fn hardware_ram_must_be_at_least_one_gb() {
        let (mut s, mut st) = fresh();
        st.set_override(&mut s, true);
        st.ram_text = "0.5".into();
        st.edit_hardware();
        assert!(st.ram_error.as_deref().is_some_and(|e| e.contains("1")));
        st.ram_text = "1".into();
        st.edit_hardware();
        assert!(st.ram_error.is_none());
    }

    #[test]
    fn override_without_detection_starts_from_blank_numbers() {
        let s0 = Settings::default();
        let mut st = SettingsState::new(&s0, PathBuf::from("/d"), None);
        let mut s = s0.clone();
        let c = st.set_override(&mut s, true);
        assert!(!c.any(), "nothing valid to apply yet");
        assert!(st.override_on);
        assert_eq!(s.hardware_override, None);
        st.ram_text = "16".into();
        assert!(st.commit_hardware(&mut s).hardware);
        assert_eq!(s.hardware_override.map(|m| m.ram_gb), Some(16.0));
    }

    #[test]
    fn effective_machine_prefers_the_override() {
        let det = Machine {
            ram_gb: 32.0,
            vram_gb: Some(16.0),
        };
        let mut s = Settings::default();
        assert_eq!(effective_machine(&s, &det), det);
        s.hardware_override = Some(Machine {
            ram_gb: 8.0,
            vram_gb: None,
        });
        assert_eq!(effective_machine(&s, &det).ram_gb, 8.0);
        assert_eq!(effective_machine(&s, &det).vram_gb, None);
    }

    #[test]
    fn detected_line_is_honest() {
        let m = |ram, vram| Machine {
            ram_gb: ram,
            vram_gb: vram,
        };
        assert_eq!(
            detected_line(Some(&m(32.0, Some(16.0)))),
            "16 GB VRAM · 32 GB RAM"
        );
        assert_eq!(
            detected_line(Some(&m(32.0, None))),
            "VRAM not detected · 32 GB RAM"
        );
        assert_eq!(
            detected_line(Some(&m(0.0, None))),
            "VRAM not detected · RAM not detected"
        );
        assert_eq!(detected_line(None), "Hardware not detected");
    }

    #[test]
    fn token_save_queues_one_request_and_clears_the_field() {
        let (_s, mut st) = fresh();
        token_ready(&mut st);
        st.token_input = "  hf_secret_value \n".into();
        st.save_token();
        assert!(st.token_input.is_empty(), "the typed token is gone at once");
        assert!(st.token_busy);
        let reqs = st.take_requests();
        assert!(matches!(
            reqs.as_slice(),
            [BackendRequest::SetHfToken(Some(t))] if t.expose() == "hf_secret_value"
        ));
        assert!(!format!("{reqs:?}").contains("hf_secret_value"));
        assert!(st.take_requests().is_empty());
    }

    #[test]
    fn empty_token_input_sends_nothing() {
        let (_s, mut st) = fresh();
        token_ready(&mut st);
        st.token_input = "   ".into();
        st.save_token();
        assert!(st.take_requests().is_empty());
        assert!(!st.token_busy);
    }

    #[test]
    fn token_clear_and_status_updates() {
        let (_s, mut st) = fresh();
        token_ready(&mut st);
        st.clear_token();
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::SetHfToken(None)]
        ));
        assert!(st.token_busy);
        st.on_token_status(&HfTokenStatus::Valid("jdtobe".into()));
        assert!(!st.token_busy);
        assert_eq!(st.token_status, HfTokenStatus::Valid("jdtobe".into()));
    }

    #[test]
    fn token_actions_do_not_queue_while_an_operation_is_busy() {
        let (_s, mut st) = fresh();
        st.token_busy = true;
        st.token_input = "hf_typed".into();
        st.save_token();
        st.clear_token();
        st.import_legacy_token();
        assert_eq!(st.token_input, "hf_typed");
        assert!(st.take_requests().is_empty());
    }

    #[test]
    fn a_failed_legacy_import_can_be_retried() {
        let (_s, mut st) = fresh();
        st.on_token_status(&HfTokenStatus::None);
        st.import_legacy_token();
        st.take_requests();
        st.on_legacy_token_imported(&Err("hf_event_secret".into()));
        assert_eq!(st.token_status, HfTokenStatus::None);
        assert!(st.legacy_import_failed);
        assert!(st.can_import_legacy_token());
        assert!(!st.token_busy);
        st.import_legacy_token();
        assert!(!st.legacy_import_failed, "a new attempt clears the failure");
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::ImportLegacyToken]
        ));
    }

    #[test]
    fn an_already_present_token_shows_as_saved() {
        let (_s, mut st) = fresh();
        st.on_token_status(&HfTokenStatus::None);
        st.import_legacy_token();
        st.on_legacy_token_imported(&Ok(
            roost_core::secrets::LegacyTokenMigration::AlreadyPresent,
        ));
        assert_eq!(st.token_status, HfTokenStatus::Saved);
        assert!(!st.legacy_import_failed);
    }

    #[test]
    fn legacy_import_error_state_redacts_event_text() {
        let (_s, mut st) = fresh();
        st.on_legacy_token_imported(&Err("hf_event_secret".into()));
        assert!(!format!("{:?}", st.token_status).contains("hf_event_secret"));
    }

    #[test]
    fn token_badge_texts() {
        let t = |s| token_badge(&s);
        assert_eq!(
            t(HfTokenStatus::None),
            (BadgeKind::Neutral, "No token saved".into())
        );
        assert_eq!(t(HfTokenStatus::Saved).0, BadgeKind::Neutral);
        assert!(t(HfTokenStatus::Saved).1.contains("Saved"));
        assert_eq!(
            t(HfTokenStatus::Valid("jd".into())),
            (BadgeKind::Ok, "Valid, signed in as jd".into())
        );
        let (k, text) = t(HfTokenStatus::Invalid("expired".into()));
        assert_eq!(k, BadgeKind::Bad);
        assert!(text.contains("expired"));
        let (k, text) = t(HfTokenStatus::Unreachable("offline".into()));
        assert_eq!(k, BadgeKind::Neutral);
        assert!(text.contains("offline"));
    }

    #[test]
    fn test_connection_result_maps_to_message_ok_and_error() {
        let ok = connection_message(&Ok("0.40.8".into()));
        assert!(ok.ok);
        assert_eq!(ok.text, "Connected, Ollama 0.40.8");
        let bad = connection_message(&Err("Ollama is unreachable".into()));
        assert!(!bad.ok);
        assert!(bad.text.contains("Ollama is unreachable"), "{}", bad.text);
        assert!(bad.text.starts_with("Could not connect"), "{}", bad.text);
    }

    #[test]
    fn test_connection_tests_the_typed_address_and_validates_it_first() {
        let (_s, mut st) = fresh();
        st.ollama_text = " http://box:11434/ ".into();
        st.test_connection();
        assert_eq!(st.connection, ConnectionState::Testing);
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::TestConnection(u)] if u == "http://box:11434"
        ));
        st.on_connection_test(&Ok("0.40.8".into()));
        assert_eq!(st.connection, ConnectionState::Done(Ok("0.40.8".into())));

        st.ollama_text = "box".into();
        st.test_connection();
        assert!(
            st.take_requests().is_empty(),
            "no request for a bad address"
        );
        assert!(st.ollama_error.is_some());
        assert_eq!(st.connection, ConnectionState::Idle);
    }

    fn rows() -> Vec<CachedRow> {
        vec![
            CachedRow {
                label: "job-1".into(),
                bytes: 4_680_000_000,
                partial: false,
            },
            CachedRow {
                label: "job-2".into(),
                bytes: 320_000_000,
                partial: true,
            },
        ]
    }

    #[test]
    fn cached_total_counts_folders_and_bytes() {
        assert_eq!(cached_total(&rows()), "2 folders, 5.00 GB");
        assert_eq!(cached_total(&rows()[..1]), "1 folder, 4.68 GB");
        assert_eq!(cached_total(&[]), "Nothing cached");
    }

    #[test]
    fn clear_is_enabled_when_only_partial_rows_exist() {
        let only_partial = vec![CachedRow {
            label: "job-2".into(),
            bytes: 320_000_000,
            partial: true,
        }];
        assert!(has_clearable(&only_partial));
        assert!(has_clearable(&rows()));
        assert!(!has_clearable(&[]));
    }

    #[test]
    fn wording_no_longer_promises_resuming() {
        for text in [KEEP_PARTIALS_HELP, CACHED_HELP] {
            assert!(!text.contains("resume it later"), "{text}");
            assert!(!text.contains("kept for resuming"), "{text}");
        }
    }

    #[test]
    fn refresh_asks_for_the_list_and_loaded_replaces_it() {
        let (_s, mut st) = fresh();
        st.refresh_cached();
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::ListCached]
        ));
        st.on_cached_loaded(&Ok(rows()));
        assert_eq!(st.cached, CachedStatus::Loaded(rows()));
        st.on_cached_loaded(&Err("denied".into()));
        assert_eq!(st.cached, CachedStatus::Error("denied".into()));
    }

    #[test]
    fn clear_cached_requires_two_step_confirmation_with_min_delay() {
        let (_s, mut st) = fresh();
        let t0 = Instant::now();
        st.click_clear_cached(t0);
        assert!(st.clear.is_armed(CLEAR_KEY, t0));
        assert!(st.take_requests().is_empty(), "the first click only arms");
        // A double click inside the delay does nothing.
        st.click_clear_cached(t0 + Duration::from_millis(150));
        st.click_clear_cached(t0 + Duration::from_millis(599));
        assert!(st.take_requests().is_empty());
        assert!(st
            .clear
            .is_armed(CLEAR_KEY, t0 + Duration::from_millis(599)));
        // A deliberate second click after the delay sends exactly one.
        st.click_clear_cached(t0 + Duration::from_millis(700));
        let reqs = st.take_requests();
        assert!(
            matches!(reqs.as_slice(), [BackendRequest::ClearCached]),
            "{reqs:?}"
        );
        assert!(!st
            .clear
            .is_armed(CLEAR_KEY, t0 + Duration::from_millis(700)));
        // After the 4 s window a click arms again instead of clearing.
        let late = t0 + secs(10);
        st.click_clear_cached(late);
        st.click_clear_cached(late + secs(5));
        assert!(st.take_requests().is_empty());
    }

    #[test]
    fn clear_cached_calls_staging_and_reports_freed_bytes() {
        let (_s, mut st) = fresh();
        let now = Instant::now();
        st.on_cached_loaded(&Ok(rows()));
        st.on_cached_cleared(&Ok(4_680_000_000), now);
        let (text, until) = st.note.clone().unwrap();
        assert_eq!(text, "Freed 4.68 GB");
        assert_eq!(until, now + NOTE_SHOWN);
        assert!(
            matches!(st.take_requests().as_slice(), [BackendRequest::ListCached]),
            "the list is read again"
        );
        st.on_cached_cleared(&Err("permission denied".into()), now);
        let (text, _) = st.note.clone().unwrap();
        assert!(text.contains("permission denied"), "{text}");
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::ListCached]
        ));
    }

    #[test]
    fn settings_saved_event_shows_saved_or_error() {
        let (_s, mut st) = fresh();
        let now = Instant::now();
        assert_eq!(st.save, SaveIndicator::Idle);
        st.note_change(SettingsChange {
            theme: true,
            ..Default::default()
        });
        assert_eq!(st.save, SaveIndicator::Saving);
        st.on_saved(&Ok(()), now);
        assert_eq!(
            st.save,
            SaveIndicator::Saved {
                until: now + SAVED_SHOWN
            }
        );
        assert_eq!(st.tick(now + secs(1)), Some(now + SAVED_SHOWN));
        assert_eq!(st.tick(now + SAVED_SHOWN), None);
        assert_eq!(st.save, SaveIndicator::Idle);

        st.note_change(SettingsChange {
            theme: true,
            ..Default::default()
        });
        st.on_saved(&Err("disk full".into()), now);
        assert_eq!(st.save, SaveIndicator::Failed("disk full".into()));
        assert_eq!(st.tick(now + secs(60)), None);
        assert_eq!(st.save, SaveIndicator::Failed("disk full".into()), "stays");
        // The next successful change replaces the failure.
        st.note_change(SettingsChange {
            theme: true,
            ..Default::default()
        });
        assert_eq!(st.save, SaveIndicator::Saving);
        // An empty change leaves the indicator alone.
        st.note_change(SettingsChange::default());
        assert_eq!(st.save, SaveIndicator::Saving);
    }

    #[test]
    fn a_staging_change_that_saved_refreshes_the_cached_list() {
        let (_s, mut st) = fresh();
        st.note_change(SettingsChange {
            staging_dir: true,
            ..Default::default()
        });
        st.on_saved(&Ok(()), Instant::now());
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::ListCached]
        ));
        // Other changes do not.
        st.note_change(SettingsChange {
            theme: true,
            ..Default::default()
        });
        st.on_saved(&Ok(()), Instant::now());
        assert!(st.take_requests().is_empty());
    }

    #[test]
    fn tick_expires_the_note_and_the_armed_button() {
        let (_s, mut st) = fresh();
        let now = Instant::now();
        st.on_cached_cleared(&Ok(1_000_000_000), now);
        st.take_requests();
        st.click_clear_cached(now);
        let due = st.tick(now).unwrap();
        assert_eq!(due, now + crate::confirm::ARM_MIN_DELAY);
        st.tick(now + NOTE_SHOWN + Duration::from_millis(1));
        assert!(st.note.is_none());
        assert!(!st.clear.is_armed(CLEAR_KEY, now + secs(5)));
    }

    #[test]
    fn staging_validation_uses_the_real_folder_rules() {
        // Guards that `edit_staging` goes through `validate_staging_dir`.
        let (_s, mut st) = fresh();
        // Built from a real folder so the path is absolute on every platform
        // ("/definitely/..." has no drive letter on Windows).
        let dir = tempfile::tempdir().unwrap();
        st.staging_text = dir
            .path()
            .join("not")
            .join("here")
            .join("at-all")
            .display()
            .to_string();
        st.edit_staging();
        assert!(st
            .staging_error
            .as_deref()
            .is_some_and(|e| e.contains("does not exist")));
    }

    #[test]
    fn typing_in_the_staging_field_never_writes_to_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let (_s, mut st) = fresh();
        let target = dir.path().join("Documents").join("models").join("staging");
        let full = target.display().to_string();
        let before = std::fs::metadata(dir.path()).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        for (i, _) in full.char_indices().chain([(full.len(), ' ')]) {
            st.staging_text = full[..i].to_string();
            st.edit_staging();
            let now = std::fs::metadata(dir.path()).unwrap().modified().unwrap();
            assert_eq!(now, before, "typing {:?} touched the folder", &full[..i]);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
        // Existing but read-only folders are fine while typing; the write
        // probe belongs to the commit.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = dir.path().join("ro");
            std::fs::create_dir(&ro).unwrap();
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
            st.staging_text = ro.display().to_string();
            st.edit_staging();
            assert!(st.staging_error.is_none(), "{:?}", st.staging_error);
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn commit_staging_runs_the_write_probe_once_and_leaves_no_junk() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, mut st) = fresh();
        st.staging_text = dir.path().display().to_string();
        assert!(st.commit_staging(&mut s).staging_dir);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = dir.path().join("ro");
            std::fs::create_dir(&ro).unwrap();
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
            if std::fs::write(ro.join("p"), "x").is_err() {
                st.staging_text = ro.display().to_string();
                assert!(!st.commit_staging(&mut s).any());
                assert!(st
                    .staging_error
                    .as_deref()
                    .is_some_and(|e| e.contains("not writable")));
            }
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn dev_memory_store_shows_a_not_persisted_note() {
        let (_s, mut st) = fresh();
        assert_eq!(st.token_note(), None);
        st.set_dev_memory_store(true);
        assert_eq!(st.token_note(), Some("dev: token not persisted"));
        assert!(!st.can_import_legacy_token());
        st.set_dev_memory_store(false);
        assert_eq!(st.token_note(), None);
    }

    #[test]
    fn legacy_import_is_only_queued_for_an_empty_production_store() {
        let (_s, mut st) = fresh();
        token_ready(&mut st);
        st.import_legacy_token();
        assert!(matches!(
            st.take_requests().as_slice(),
            [BackendRequest::ImportLegacyToken]
        ));
        assert!(st.token_busy);

        st.on_token_status(&HfTokenStatus::Saved);
        st.import_legacy_token();
        assert!(st.take_requests().is_empty());
    }
}
