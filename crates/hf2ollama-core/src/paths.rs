use std::path::{Path, PathBuf};

/// Per-OS base directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsDirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

impl OsDirs {
    pub fn detect() -> OsDirs {
        todo!()
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
        let _ = (exe_dir, os);
        todo!()
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
