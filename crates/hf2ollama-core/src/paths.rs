use std::path::{Path, PathBuf};

/// Per-OS base directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsDirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl OsDirs {
    /// Detects the OS directories; falls back to a `hf2ollama-data` folder
    /// beside the executable (or in the current directory) when unavailable.
    pub fn detect() -> OsDirs {
        match directories::ProjectDirs::from("", "", "hf2ollama") {
            Some(d) => OsDirs {
                config: d.config_dir().to_path_buf(),
                cache: d.cache_dir().to_path_buf(),
                state: d
                    .state_dir()
                    .unwrap_or_else(|| d.data_local_dir())
                    .to_path_buf(),
            },
            None => {
                let base = std::env::current_exe()
                    .ok()
                    .and_then(|e| e.parent().map(Path::to_path_buf))
                    .unwrap_or_default()
                    .join("hf2ollama-data");
                OsDirs {
                    config: base.clone(),
                    cache: base.clone(),
                    state: base,
                }
            }
        }
    }
}

/// Resolved file locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub history_file: PathBuf,
    pub log_dir: PathBuf,
    pub default_staging: PathBuf,
    pub portable: bool,
}

impl Paths {
    pub fn resolve(exe_dir: &Path, os: OsDirs) -> Paths {
        if exe_dir.join("portable").exists() {
            Paths {
                config_file: exe_dir.join("hf2ollama.toml"),
                history_file: exe_dir.join("history.json"),
                log_dir: exe_dir.join("logs"),
                default_staging: exe_dir.join("staging"),
                portable: true,
            }
        } else {
            Paths {
                config_file: os.config.join("hf2ollama.toml"),
                history_file: os.config.join("history.json"),
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
        assert_eq!(p.config_file, exe.path().join("hf2ollama.toml"));
        assert_eq!(p.history_file, exe.path().join("history.json"));
        assert_eq!(p.log_dir, exe.path().join("logs"));
        assert_eq!(p.default_staging, exe.path().join("staging"));
    }

    #[test]
    fn no_marker_uses_os_dirs() {
        let exe = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let p = Paths::resolve(exe.path(), os(root.path()));
        assert!(!p.portable);
        assert_eq!(p.config_file, root.path().join("config/hf2ollama.toml"));
        assert_eq!(p.history_file, root.path().join("config/history.json"));
        assert_eq!(p.log_dir, root.path().join("state/logs"));
        assert_eq!(p.default_staging, root.path().join("cache/staging"));
    }
}
