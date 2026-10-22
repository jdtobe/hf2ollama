//! Everything the app decides before a window exists: where its files live,
//! which settings apply, and the one-time clean-up of abandoned job folders.
//! Kept free of any window or egui types so it can be tested directly.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use hf2ollama_core::config::Settings;
use hf2ollama_core::paths::{OsDirs, Paths};
use hf2ollama_core::staging::Staging;

/// How long a paused or failed download keeps its partial files.
pub const PARTIAL_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What `main` needs to open the window and start the backend.
#[derive(Debug, Clone)]
pub struct Startup {
    pub paths: Paths,
    /// Settings as the rest of the app should see them (an unusable saved
    /// staging folder is already dropped).
    pub settings: Settings,
    pub staging_dir: PathBuf,
    /// One line for the status bar and the log, when something was ignored.
    pub notice: Option<String>,
}

/// The staging folder to use and, when the saved one was unusable, a notice.
/// A saved folder must be an absolute path without `..` or a leading `~`.
pub fn effective_staging_dir(settings: &Settings, paths: &Paths) -> (PathBuf, Option<String>) {
    let _ = (settings, paths);
    todo!()
}

/// Resolves paths and settings. Reads files, never writes or creates any.
pub fn startup(exe_dir: &Path, os: OsDirs) -> Startup {
    let _ = (exe_dir, os);
    todo!()
}

/// Removes abandoned job folders from `staging_dir` and returns the report
/// line, which is also appended to `<log_dir>/startup.log`. Never fails.
pub fn sweep_staging(staging_dir: &Path, log_dir: &Path, now: SystemTime) -> String {
    let _ = (staging_dir, log_dir, now);
    todo!()
}

/// Runs [`sweep_staging`] on its own thread so the window never waits for it.
pub fn spawn_sweep(staging_dir: PathBuf, log_dir: PathBuf) {
    let _ = (staging_dir, log_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use hf2ollama_core::events::JobId;

    fn os(root: &Path) -> OsDirs {
        OsDirs {
            config: root.join("config"),
            cache: root.join("cache"),
            state: root.join("state"),
        }
    }

    fn paths(root: &Path) -> Paths {
        Paths::resolve(root, os(root))
    }

    #[test]
    fn a_valid_saved_staging_folder_is_used() {
        let root = tempfile::tempdir().unwrap();
        let p = paths(root.path());
        let mut s = Settings::default();
        s.staging_dir = Some(root.path().join("mine"));
        let (dir, notice) = effective_staging_dir(&s, &p);
        assert_eq!(dir, root.path().join("mine"));
        assert_eq!(notice, None);
    }

    #[test]
    fn no_saved_staging_folder_means_the_default_without_a_notice() {
        let root = tempfile::tempdir().unwrap();
        let p = paths(root.path());
        let (dir, notice) = effective_staging_dir(&Settings::default(), &p);
        assert_eq!(dir, p.default_staging);
        assert_eq!(notice, None);
    }

    #[test]
    fn unusable_saved_staging_folders_fall_back_with_a_notice() {
        let root = tempfile::tempdir().unwrap();
        let p = paths(root.path());
        let abs_parent = root.path().join("a").join("..").join("b");
        for bad in [
            PathBuf::from("relative/dir"),
            PathBuf::from("./here"),
            PathBuf::from("~/models"),
            PathBuf::from("~"),
            PathBuf::from(""),
            abs_parent,
        ] {
            let mut s = Settings::default();
            s.staging_dir = Some(bad.clone());
            let (dir, notice) = effective_staging_dir(&s, &p);
            assert_eq!(dir, p.default_staging, "{bad:?}");
            assert_eq!(
                notice.as_deref(),
                Some("ignored an invalid saved staging folder"),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn startup_drops_an_invalid_saved_folder_from_the_settings_it_returns() {
        let exe = tempfile::tempdir().unwrap();
        let osd = tempfile::tempdir().unwrap();
        let cfg = os(osd.path()).config;
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("hf2ollama.toml"), "staging_dir = \"relative/x\"\n").unwrap();
        let st = startup(exe.path(), os(osd.path()));
        assert_eq!(st.settings.staging_dir, None);
        assert_eq!(st.staging_dir, st.paths.default_staging);
        assert!(st.notice.is_some());
    }

    #[test]
    fn startup_with_portable_marker_writes_nothing_outside_exe_dir() {
        let exe = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("portable"), "").unwrap();
        let osd = tempfile::tempdir().unwrap();
        let st = startup(exe.path(), os(osd.path()));
        assert!(st.paths.portable);
        assert_eq!(st.paths.config_file, exe.path().join("hf2ollama.toml"));
        assert_eq!(st.paths.history_file, exe.path().join("history.json"));
        assert_eq!(st.paths.log_dir, exe.path().join("logs"));
        assert_eq!(st.staging_dir, exe.path().join("staging"));
        // An orphan job dir exists so the sweep has something to log about.
        let staging = Staging::new(st.staging_dir.clone());
        drop(staging.create_job(JobId(1), 0).unwrap());
        let line = sweep_staging(&st.staging_dir, &st.paths.log_dir, SystemTime::now());
        assert!(line.contains("1 orphan"), "{line}");
        assert!(st.paths.log_dir.join("startup.log").is_file());
        let outside: Vec<_> = std::fs::read_dir(osd.path()).unwrap().collect();
        assert!(outside.is_empty(), "OS dirs touched: {outside:?}");
    }

    #[test]
    fn startup_without_marker_uses_os_dirs() {
        let exe = tempfile::tempdir().unwrap();
        let osd = tempfile::tempdir().unwrap();
        let st = startup(exe.path(), os(osd.path()));
        assert!(!st.paths.portable);
        assert_eq!(st.paths.config_file, osd.path().join("config/hf2ollama.toml"));
        assert_eq!(st.staging_dir, osd.path().join("cache/staging"));
        assert_eq!(st.paths.log_dir, osd.path().join("state/logs"));
        sweep_staging(&st.staging_dir, &st.paths.log_dir, SystemTime::now());
        assert!(osd.path().join("state/logs/startup.log").is_file());
        let beside: Vec<_> = std::fs::read_dir(exe.path()).unwrap().collect();
        assert!(beside.is_empty(), "exe dir touched: {beside:?}");
    }

    #[test]
    fn startup_sweeps_orphans_but_never_foreign_files() {
        let root = tempfile::tempdir().unwrap();
        let staging_dir = root.path().join("staging");
        let staging = Staging::new(staging_dir.clone());
        drop(staging.create_job(JobId(7), 0).unwrap());
        let partial = staging.create_job(JobId(8), 0).unwrap();
        partial.mark_partial().unwrap();
        let foreign_dir = staging_dir.join("my-photos");
        std::fs::create_dir_all(&foreign_dir).unwrap();
        std::fs::write(foreign_dir.join("a.jpg"), b"x").unwrap();
        // Looks like a job dir by name but has no marker: not ours.
        let unmarked = staging_dir.join("job-9");
        std::fs::create_dir_all(&unmarked).unwrap();
        std::fs::write(staging_dir.join("notes.txt"), b"x").unwrap();

        let line = sweep_staging(&staging_dir, &root.path().join("logs"), SystemTime::now());

        assert!(line.contains("1 orphan"), "{line}");
        assert!(!staging_dir.join("job-7").exists());
        assert!(staging_dir.join("job-8").exists(), "fresh partial is kept");
        assert!(foreign_dir.join("a.jpg").exists());
        assert!(unmarked.exists());
        assert!(staging_dir.join("notes.txt").exists());
    }

    #[test]
    fn sweeping_a_missing_staging_folder_does_not_create_it_or_fail() {
        let root = tempfile::tempdir().unwrap();
        let staging_dir = root.path().join("nope");
        let line = sweep_staging(&staging_dir, &root.path().join("logs"), SystemTime::now());
        assert!(!line.is_empty());
        assert!(!staging_dir.exists());
    }
}
