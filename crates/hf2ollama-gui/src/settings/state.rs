//! Pure state of the Settings view: text buffers with their validation
//! errors, the change flags, token and connection status, the cached files
//! list and the two-step clear. No egui in here.

#![allow(dead_code)] // stubs until the behavior commit

use super::validate::{
    clamp_max_parallel, parse_gb, validate_hf_endpoint, validate_ollama_url, validate_staging_dir,
};
use crate::browse::results::format_size;
use crate::confirm::Confirm;
use crate::runtime::{BackendRequest, CachedRow, HfToken, HfTokenStatus};
use hf2ollama_core::config::{AutoDismiss, Settings, ThemeChoice};
use hf2ollama_core::fit::Machine;
use std::path::{Path, PathBuf};
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
        let _ = other;
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
    let _ = r;
    ConnectionMessage {
        ok: false,
        text: String::new(),
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
    let _ = status;
    (BadgeKind::Ok, String::new())
}

/// "3 folders, 4.68 GB" for the Cached files list.
pub fn cached_total(rows: &[CachedRow]) -> String {
    let _ = rows;
    String::new()
}

/// The machine used for fit estimates: the override, else what was detected.
pub fn effective_machine(settings: &Settings, detected: &Machine) -> Machine {
    let _ = (settings, detected);
    Machine {
        ram_gb: 0.0,
        vram_gb: None,
    }
}

/// "16 GB VRAM · 32 GB RAM", or an honest "VRAM not detected".
pub fn detected_line(detected: Option<&Machine>) -> String {
    let _ = detected;
    String::new()
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
    pub connection: ConnectionState,
    pub cached: CachedStatus,
    pub clear: Confirm,
    pub save: SaveIndicator,
    pub note: Option<(String, Instant)>,
    default_staging: PathBuf,
    detected: Option<Machine>,
    pending: Vec<BackendRequest>,
}

impl SettingsState {
    pub fn new(
        settings: &Settings,
        default_staging: PathBuf,
        detected: Option<Machine>,
    ) -> SettingsState {
        let _ = settings;
        SettingsState {
            ollama_text: String::new(),
            ollama_error: None,
            staging_text: String::new(),
            staging_error: None,
            hf_text: String::new(),
            hf_error: None,
            override_on: false,
            ram_text: String::new(),
            vram_text: String::new(),
            ram_error: None,
            vram_error: None,
            token_input: String::new(),
            token_status: HfTokenStatus::None,
            token_busy: false,
            connection: ConnectionState::Idle,
            cached: CachedStatus::Loading,
            clear: Confirm::default(),
            save: SaveIndicator::Idle,
            note: None,
            default_staging,
            detected,
            pending: Vec::new(),
        }
    }

    pub fn detected(&self) -> Option<&Machine> {
        self.detected.as_ref()
    }

    /// Requests queued for the backend since the last call.
    pub fn take_requests(&mut self) -> Vec<BackendRequest> {
        std::mem::take(&mut self.pending)
    }

    // ---- text fields: `edit_*` validates while typing, `commit_*` applies ----

    pub fn edit_ollama(&mut self) {}
    pub fn commit_ollama(&mut self, s: &mut Settings) -> SettingsChange {
        let _ = s;
        SettingsChange::default()
    }
    pub fn edit_hf_endpoint(&mut self) {}
    pub fn commit_hf_endpoint(&mut self, s: &mut Settings) -> SettingsChange {
        let _ = s;
        SettingsChange::default()
    }
    pub fn edit_staging(&mut self) {}
    pub fn commit_staging(&mut self, s: &mut Settings) -> SettingsChange {
        let _ = s;
        SettingsChange::default()
    }

    // ---- toggles and selects ----

    pub fn set_delete_after_import(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        let _ = (s, on);
        SettingsChange::default()
    }
    pub fn set_keep_partials(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        let _ = (s, on);
        SettingsChange::default()
    }
    pub fn set_max_parallel(&mut self, s: &mut Settings, n: u8) -> SettingsChange {
        let _ = (s, n);
        SettingsChange::default()
    }
    pub fn set_auto_dismiss(&mut self, s: &mut Settings, v: AutoDismiss) -> SettingsChange {
        let _ = (s, v);
        SettingsChange::default()
    }
    pub fn set_theme(&mut self, s: &mut Settings, v: ThemeChoice) -> SettingsChange {
        let _ = (s, v);
        SettingsChange::default()
    }

    // ---- hardware override ----

    pub fn set_override(&mut self, s: &mut Settings, on: bool) -> SettingsChange {
        let _ = (s, on);
        SettingsChange::default()
    }
    pub fn edit_hardware(&mut self) {}
    pub fn commit_hardware(&mut self, s: &mut Settings) -> SettingsChange {
        let _ = s;
        SettingsChange::default()
    }

    // ---- Hugging Face token ----

    pub fn save_token(&mut self) {}
    pub fn clear_token(&mut self) {}
    pub fn on_token_status(&mut self, status: &HfTokenStatus) {
        let _ = status;
    }

    // ---- connection test ----

    pub fn test_connection(&mut self) {}
    pub fn on_connection_test(&mut self, r: &Result<String, String>) {
        let _ = r;
    }

    // ---- cached files ----

    pub fn refresh_cached(&mut self) {}
    pub fn on_cached_loaded(&mut self, r: &Result<Vec<CachedRow>, String>) {
        let _ = r;
    }
    pub fn click_clear_cached(&mut self, now: Instant) {
        let _ = now;
    }
    pub fn on_cached_cleared(&mut self, r: &Result<u64, String>, now: Instant) {
        let _ = (r, now);
    }

    // ---- saving ----

    /// A change was applied: the file is being written.
    pub fn note_change(&mut self, change: SettingsChange) {
        let _ = change;
    }
    pub fn on_saved(&mut self, r: &Result<(), String>, now: Instant) {
        let _ = (r, now);
    }

    /// Expires indicators; returns when the next one is due.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        let _ = now;
        None
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
        assert_eq!(flags(st.set_theme(&mut s, ThemeChoice::Dark)), ["theme"]);
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
        st.token_input = "   ".into();
        st.save_token();
        assert!(st.take_requests().is_empty());
        assert!(!st.token_busy);
    }

    #[test]
    fn token_clear_and_status_updates() {
        let (_s, mut st) = fresh();
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
        st.staging_text = "/definitely/not/here/at/all".into();
        st.edit_staging();
        assert!(st
            .staging_error
            .as_deref()
            .is_some_and(|e| e.contains("does not exist")));
        let _ = Path::new("");
    }
}
