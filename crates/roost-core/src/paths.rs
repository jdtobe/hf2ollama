use std::path::{Path, PathBuf};

/// Per-OS base directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsDirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl OsDirs {
    /// Detects the Roost OS directories; falls back to a `roost-data` folder
    /// beside the executable (or in the current directory) when unavailable.
    pub fn detect() -> OsDirs {
        match Self::project("roost") {
            Some(dirs) => dirs,
            None => Self::fallback("roost-data"),
        }
    }

    /// Detects the former hf2ollama OS directories. Absence deliberately does
    /// not fall back: migration must never invent a legacy location.
    pub fn detect_legacy() -> Option<OsDirs> {
        Self::project("hf2ollama")
    }

    fn project(application: &str) -> Option<OsDirs> {
        directories::ProjectDirs::from("", "", application).map(|d| OsDirs {
            config: d.config_dir().to_path_buf(),
            cache: d.cache_dir().to_path_buf(),
            state: d
                .state_dir()
                .unwrap_or_else(|| d.data_local_dir())
                .to_path_buf(),
        })
    }

    fn fallback(name: &str) -> OsDirs {
        let base = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_default()
            .join(name);
        OsDirs {
            config: base.clone(),
            cache: base.clone(),
            state: base,
        }
    }
}

/// Resolved file locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub import_history_file: PathBuf,
    /// Directory reserved for one session file per session.
    pub sessions_dir: PathBuf,
    pub log_dir: PathBuf,
    pub default_staging: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve(exe_dir: &Path, os: OsDirs) -> Paths {
        if exe_dir.join("portable").exists() {
            Paths {
                config_file: exe_dir.join("roost.toml"),
                import_history_file: exe_dir.join("history.json"),
                sessions_dir: exe_dir.join("sessions"),
                log_dir: exe_dir.join("logs"),
                default_staging: exe_dir.join("staging"),
                portable: true,
            }
        } else {
            Paths {
                config_file: os.config.join("roost.toml"),
                import_history_file: os.config.join("history.json"),
                sessions_dir: os.state.join("sessions"),
                log_dir: os.state.join("logs"),
                default_staging: os.cache.join("staging"),
                portable: false,
            }
        }
    }
}

/// Locations used by the preceding hf2ollama application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyPaths {
    pub config_file: PathBuf,
    pub import_history_file: PathBuf,
    pub log_dir: PathBuf,
    pub default_staging: PathBuf,
    pub portable: bool,
}

impl LegacyPaths {
    pub fn resolve(exe_dir: &Path, os: OsDirs) -> LegacyPaths {
        if exe_dir.join("portable").exists() {
            LegacyPaths {
                config_file: exe_dir.join("hf2ollama.toml"),
                import_history_file: exe_dir.join("history.json"),
                log_dir: exe_dir.join("logs"),
                default_staging: exe_dir.join("staging"),
                portable: true,
            }
        } else {
            LegacyPaths {
                config_file: os.config.join("hf2ollama.toml"),
                import_history_file: os.config.join("history.json"),
                log_dir: os.state.join("logs"),
                default_staging: os.cache.join("staging"),
                portable: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(root: &Path) -> OsDirs {
        OsDirs {
            config: root.join("config"),
            cache: root.join("cache"),
            state: root.join("state"),
        }
    }

    #[test]
    fn portable_marker_redirects_all_paths_beside_exe() {
        let exe = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("portable"), "").unwrap();
        let other = tempfile::tempdir().unwrap();
        let p = Paths::resolve(exe.path(), os(other.path()));
        assert!(p.portable);
        assert_eq!(p.config_file, exe.path().join("roost.toml"));
        assert_eq!(p.import_history_file, exe.path().join("history.json"));
        assert_eq!(p.sessions_dir, exe.path().join("sessions"));
        assert_eq!(p.log_dir, exe.path().join("logs"));
        assert_eq!(p.default_staging, exe.path().join("staging"));
    }

    #[test]
    fn no_marker_uses_os_dirs() {
        let exe = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let p = Paths::resolve(exe.path(), os(root.path()));
        assert!(!p.portable);
        assert_eq!(p.config_file, root.path().join("config/roost.toml"));
        assert_eq!(
            p.import_history_file,
            root.path().join("config/history.json")
        );
        assert_eq!(p.sessions_dir, root.path().join("state/sessions"));
        assert_eq!(p.log_dir, root.path().join("state/logs"));
        assert_eq!(p.default_staging, root.path().join("cache/staging"));
    }
}
