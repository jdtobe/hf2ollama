use crate::error::{Error, Result};
use crate::fit::Machine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long finished and cancelled jobs stay in the dock before dismissal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoDismiss {
    #[serde(alias = "Never")]
    Never,
    #[serde(alias = "OneMinute")]
    OneMinute,
    #[serde(alias = "FiveMinutes")]
    FiveMinutes,
    #[serde(alias = "OneHour")]
    OneHour,
}

impl AutoDismiss {
    pub fn duration(&self) -> Option<Duration> {
        match self {
            AutoDismiss::Never => None,
            AutoDismiss::OneMinute => Some(Duration::from_secs(60)),
            AutoDismiss::FiveMinutes => Some(Duration::from_secs(300)),
            AutoDismiss::OneHour => Some(Duration::from_secs(3600)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    #[serde(alias = "System")]
    System,
    #[serde(alias = "Light")]
    Light,
    #[serde(alias = "Dark")]
    Dark,
}

/// The colour family; each has a light and a dark variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeFamily {
    Meadow,
    Ember,
    Plum,
    Tide,
}

/// How long Ollama keeps a model in memory after a chat request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeepAlive {
    /// Send nothing; Ollama uses its own default (five minutes).
    #[default]
    OllamaDefault,
    FifteenMinutes,
    OneHour,
    /// Until Ollama itself stops or the model is unloaded.
    UntilQuit,
}

/// When Roost tells you something needs you (an approval, a run that ended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Notifications {
    Off,
    /// Only while the Roost window is not the one in front.
    #[default]
    Background,
    /// Also while it is in front.
    Always,
}

impl KeepAlive {
    /// The `keep_alive` value for a request, or `None` to leave it out.
    pub fn wire(self) -> Option<serde_json::Value> {
        match self {
            KeepAlive::OllamaDefault => None,
            KeepAlive::FifteenMinutes => Some(serde_json::json!("15m")),
            KeepAlive::OneHour => Some(serde_json::json!("1h")),
            KeepAlive::UntilQuit => Some(serde_json::json!(-1)),
        }
    }
}

/// User settings, persisted as TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ollama_url: String,
    pub staging_dir: Option<PathBuf>,
    pub delete_after_import: bool,
    pub keep_partials: bool,
    pub max_parallel: u8,
    pub auto_dismiss: AutoDismiss,
    pub hf_endpoint: String,
    pub hardware_override: Option<Machine>,
    /// Saved as `theme` by earlier versions.
    #[serde(alias = "theme")]
    pub theme_mode: ThemeMode,
    pub theme_family: ThemeFamily,
    pub keep_alive: KeepAlive,
    /// When to nudge you about approvals and finished runs.
    pub notifications: Notifications,
    /// Width in points of the sidebar shared by the Chat, Work and Models tabs.
    pub sidebar_width: f32,
    /// May agents write files at all. Off makes every agent read-only.
    pub agents_can_write: bool,
    /// May agents run shell commands at all. Off until the user turns it on
    /// (after a warning); an agent still has to be set to Ask or Allow.
    pub agents_can_run: bool,
    /// Limits for the agent tools, saved under `[tools]`.
    pub tools: crate::agent::tools::Limits,
}

/// The sidebar's width until the user drags it.
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 340.0;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ollama_url: "http://localhost:11434".into(),
            staging_dir: None,
            delete_after_import: true,
            keep_partials: true,
            max_parallel: 2,
            auto_dismiss: AutoDismiss::Never,
            hf_endpoint: "https://huggingface.co".into(),
            hardware_override: None,
            theme_mode: ThemeMode::System,
            theme_family: ThemeFamily::Meadow,
            keep_alive: KeepAlive::OllamaDefault,
            notifications: Notifications::Background,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            agents_can_write: true,
            agents_can_run: false,
            tools: crate::agent::tools::Limits::default(),
        }
    }
}

impl Settings {
    /// Loads settings; a missing file yields defaults, a corrupt one `Error::Parse`.
    pub fn load(path: &Path) -> Result<Settings> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        let mut table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| Error::Parse(e.to_string()))?;
        // The old `theme` key is only an alias; with both present the new one wins.
        if table.contains_key("theme_mode") {
            table.remove("theme");
        }
        let mut settings: Settings = toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| Error::Parse(e.to_string()))?;
        settings.max_parallel = settings.max_parallel.clamp(1, 3);
        settings.tools = settings.tools.clamped();
        Ok(settings)
    }

    /// Saves atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).map_err(|e| Error::Parse(e.to_string()))?;
        crate::persistence::atomic_write(path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_limits_default_to_the_built_in_ones_and_load_from_a_tools_table() {
        assert_eq!(Settings::default().tools, Default::default());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        std::fs::write(
            &path,
            "[tools]\nsearch_matches = 50\nrun_timeout_secs = 600\n",
        )
        .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.tools.search_matches, 50);
        assert_eq!(s.tools.run_timeout, Duration::from_secs(600));
        assert_eq!(
            s.tools.list_entries, 200,
            "unnamed limits keep their default"
        );
        // A file written before the table existed still loads.
        std::fs::write(&path, "max_parallel = 2\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().tools, Default::default());
    }

    #[test]
    fn tool_limits_out_of_range_in_the_file_are_moved_into_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        std::fs::write(
            &path,
            "[tools]\nlist_entries = 0\nrun_timeout_secs = 99999999\n",
        )
        .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.tools.list_entries, 1);
        assert_eq!(s.tools.run_timeout, Duration::from_secs(3600));
    }

    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.ollama_url, "http://localhost:11434");
        assert_eq!(s.staging_dir, None);
        assert!(s.delete_after_import);
        assert!(s.keep_partials);
        assert_eq!(s.max_parallel, 2);
        assert_eq!(s.auto_dismiss, AutoDismiss::Never);
        assert_eq!(s.hf_endpoint, "https://huggingface.co");
        assert_eq!(s.hardware_override, None);
        assert_eq!(s.theme_mode, ThemeMode::System);
        assert_eq!(s.theme_family, ThemeFamily::Meadow);
        assert_eq!(s.keep_alive, KeepAlive::OllamaDefault);
        assert_eq!(s.sidebar_width, 340.0);
        assert!(s.agents_can_write);
        assert!(!s.agents_can_run, "running commands is opt-in");
    }

    #[test]
    fn the_write_switch_is_saved_and_older_files_allow_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        let s = Settings {
            agents_can_write: false,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert!(!Settings::load(&path).unwrap().agents_can_write);
        std::fs::write(&path, "max_parallel = 2\n").unwrap();
        assert!(Settings::load(&path).unwrap().agents_can_write);
    }

    #[test]
    fn the_command_switch_is_saved_and_older_files_keep_commands_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        let s = Settings {
            agents_can_run: true,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert!(Settings::load(&path).unwrap().agents_can_run);
        std::fs::write(&path, "max_parallel = 2\n").unwrap();
        assert!(!Settings::load(&path).unwrap().agents_can_run);
    }

    #[test]
    fn the_sidebar_width_is_saved_and_older_files_get_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        let s = Settings {
            sidebar_width: 412.0,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().sidebar_width, 412.0);
        std::fs::write(&path, "max_parallel = 3\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().sidebar_width, 340.0);
    }

    #[test]
    fn notifications_default_to_background_and_round_trip_in_kebab_case() {
        assert_eq!(Settings::default().notifications, Notifications::Background);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        std::fs::write(&path, "max_parallel = 3\n").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().notifications,
            Notifications::Background,
            "an older file gets the default"
        );
        let s = Settings {
            notifications: Notifications::Always,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("notifications = \"always\""), "{text}");
        assert_eq!(
            Settings::load(&path).unwrap().notifications,
            Notifications::Always
        );
    }

    #[test]
    fn keep_alive_maps_to_what_ollama_expects() {
        assert_eq!(KeepAlive::OllamaDefault.wire(), None);
        assert_eq!(
            KeepAlive::FifteenMinutes.wire(),
            Some(serde_json::json!("15m"))
        );
        assert_eq!(KeepAlive::OneHour.wire(), Some(serde_json::json!("1h")));
        assert_eq!(KeepAlive::UntilQuit.wire(), Some(serde_json::json!(-1)));
    }

    #[test]
    fn keep_alive_survives_a_save_and_an_old_file_without_it_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roost.toml");
        let s = Settings {
            keep_alive: KeepAlive::OneHour,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().keep_alive,
            KeepAlive::OneHour
        );
        std::fs::write(&path, "max_parallel = 3\n").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().keep_alive,
            KeepAlive::OllamaDefault
        );
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("hf2ollama.toml");
        let s = Settings {
            ollama_url: "http://box:11434".into(),
            staging_dir: Some(PathBuf::from("/data/staging")),
            delete_after_import: false,
            keep_partials: false,
            max_parallel: 3,
            auto_dismiss: AutoDismiss::FiveMinutes,
            hf_endpoint: "https://mirror.example".into(),
            hardware_override: Some(Machine {
                ram_gb: 64.0,
                vram_gb: Some(24.0),
            }),
            theme_mode: ThemeMode::Dark,
            theme_family: ThemeFamily::Tide,
            keep_alive: KeepAlive::FifteenMinutes,
            notifications: Notifications::Off,
            sidebar_width: 377.0,
            agents_can_write: false,
            agents_can_run: true,
            tools: crate::agent::tools::Limits {
                search_matches: 77,
                ..Default::default()
            },
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::load(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn load_corrupt_file_errors_with_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is = not [valid toml").unwrap();
        assert!(matches!(Settings::load(&path), Err(Error::Parse(_))));
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        std::fs::write(&path, "theme = \"Light\"\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.theme_mode, ThemeMode::Light);
        assert_eq!(s.theme_family, ThemeFamily::Meadow);
        assert_eq!(s.max_parallel, 2);
    }

    #[test]
    fn max_parallel_is_clamped_to_1_to_3_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "max_parallel = 9\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().max_parallel, 3);
        std::fs::write(&path, "max_parallel = 0\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().max_parallel, 1);
    }

    #[test]
    fn auto_dismiss_durations() {
        assert_eq!(AutoDismiss::Never.duration(), None);
        assert_eq!(
            AutoDismiss::OneMinute.duration(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            AutoDismiss::FiveMinutes.duration(),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            AutoDismiss::OneHour.duration(),
            Some(Duration::from_secs(3600))
        );
    }

    #[test]
    fn settings_toml_uses_kebab_case_enum_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.toml");
        let s = Settings {
            auto_dismiss: AutoDismiss::FiveMinutes,
            theme_mode: ThemeMode::Dark,
            theme_family: ThemeFamily::Plum,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("auto_dismiss = \"five-minutes\""), "{text}");
        assert!(text.contains("theme_mode = \"dark\""), "{text}");
        assert!(text.contains("theme_family = \"plum\""), "{text}");
        assert!(
            !text.contains("FiveMinutes") && !text.contains("Dark"),
            "{text}"
        );
    }

    #[test]
    fn legacy_pascal_case_enum_names_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.toml");
        for (old, dismiss, theme) in [
            ("Never", AutoDismiss::Never, "System"),
            ("OneMinute", AutoDismiss::OneMinute, "Light"),
            ("FiveMinutes", AutoDismiss::FiveMinutes, "Dark"),
            ("OneHour", AutoDismiss::OneHour, "Dark"),
        ] {
            std::fs::write(
                &path,
                format!("auto_dismiss = \"{old}\"\ntheme = \"{theme}\"\n"),
            )
            .unwrap();
            let s = Settings::load(&path).unwrap();
            assert_eq!(s.auto_dismiss, dismiss, "{old}");
            let want = match theme {
                "System" => ThemeMode::System,
                "Light" => ThemeMode::Light,
                _ => ThemeMode::Dark,
            };
            assert_eq!(s.theme_mode, want, "{theme}");
        }
    }

    #[test]
    fn new_theme_field_names_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.toml");
        std::fs::write(&path, "theme_mode = \"dark\"\ntheme_family = \"ember\"\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.theme_mode, ThemeMode::Dark);
        assert_eq!(s.theme_family, ThemeFamily::Ember);
    }

    #[test]
    fn old_lowercase_theme_field_loads_with_default_family() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.toml");
        std::fs::write(&path, "theme = \"dark\"\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.theme_mode, ThemeMode::Dark);
        assert_eq!(s.theme_family, ThemeFamily::Meadow);
    }

    #[test]
    fn a_file_with_both_old_and_new_theme_keys_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.toml");
        std::fs::write(&path, "theme = \"light\"\ntheme_mode = \"dark\"\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.theme_mode, ThemeMode::Dark, "the new key wins");
    }

    #[test]
    fn roundtrip_all_variants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.toml");
        for d in [
            AutoDismiss::Never,
            AutoDismiss::OneMinute,
            AutoDismiss::FiveMinutes,
            AutoDismiss::OneHour,
        ] {
            for t in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
                for f in [
                    ThemeFamily::Meadow,
                    ThemeFamily::Ember,
                    ThemeFamily::Plum,
                    ThemeFamily::Tide,
                ] {
                    let s = Settings {
                        auto_dismiss: d,
                        theme_mode: t,
                        theme_family: f,
                        ..Settings::default()
                    };
                    s.save(&path).unwrap();
                    assert_eq!(Settings::load(&path).unwrap(), s);
                }
            }
        }
    }
}
