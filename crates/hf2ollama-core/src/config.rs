use crate::error::{Error, Result};
use crate::fit::Machine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long finished and cancelled jobs stay in the dock before dismissal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoDismiss {
    Never,
    OneMinute,
    FiveMinutes,
    OneHour,
}

impl AutoDismiss {
    pub fn duration(&self) -> Option<Duration> {
        todo!()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
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
    pub theme: ThemeChoice,
}

impl Default for Settings {
    fn default() -> Self {
        todo!()
    }
}

impl Settings {
    /// Loads settings; a missing file yields defaults, a corrupt one `Error::Parse`.
    pub fn load(path: &Path) -> Result<Settings> {
        let _ = (path, Error::Cancelled);
        todo!()
    }

    /// Saves atomically: temp file in the same directory, then rename.
    pub fn save(&self, path: &Path) -> Result<()> {
        let _ = path;
        todo!()
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(s.theme, ThemeChoice::System);
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
            theme: ThemeChoice::Dark,
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
        assert_eq!(s.theme, ThemeChoice::Light);
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
}
