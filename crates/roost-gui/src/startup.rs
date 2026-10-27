//! Everything the app decides before a window exists: where its files live,
//! which settings apply, and the one-time clean-up of abandoned job folders.
//! Kept free of any window or egui types so it can be tested directly.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use roost_core::config::Settings;
use roost_core::migration::{migrate_legacy_files, MigrationReport};
use roost_core::paths::{LegacyPaths, OsDirs, Paths};
use roost_core::staging::Staging;

/// How long a paused or failed download keeps its partial files.
pub const PARTIAL_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// What `main` needs to open the window and start the backend.
#[derive(Debug, Clone)]
pub struct Startup {
    pub paths: Paths,
    pub migration: MigrationReport,
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
    match &settings.staging_dir {
        None => (paths.default_staging.clone(), None),
        Some(dir) if saved_staging_is_usable(dir) => (dir.clone(), None),
        Some(_) => (
            paths.default_staging.clone(),
            Some("ignored an invalid saved staging folder".to_string()),
        ),
    }
}

fn saved_staging_is_usable(dir: &Path) -> bool {
    dir.is_absolute()
        && !dir.to_string_lossy().starts_with('~')
        && !dir
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Moves an unreadable settings file to `<name>.bak` (replacing an older
/// backup) so the next save cannot destroy it. Returns the line for the
/// status bar and the log.
fn set_corrupt_settings_aside(config_file: &Path, why: &str) -> String {
    let mut bak = config_file.as_os_str().to_os_string();
    bak.push(".bak");
    let bak = PathBuf::from(bak);
    let bak_name = bak.file_name().map_or_else(
        || "a backup".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    // A rename does not replace an existing file on every platform.
    let _ = std::fs::remove_file(&bak);
    match std::fs::rename(config_file, &bak) {
        Ok(()) => format!(
            "the settings file could not be read ({why}); it was kept as {bak_name} and defaults are in use"
        ),
        Err(e) => format!(
            "the settings file could not be read ({why}) and could not be moved aside ({e}); defaults are in use"
        ),
    }
}

/// Resolves paths, migrates legacy metadata, and loads settings. Reads files and never creates any, except
/// that an unreadable settings file is renamed to `<name>.bak` and the
/// reason is appended to `startup.log`.
pub fn startup(exe_dir: &Path, current_os: OsDirs, legacy_os: Option<OsDirs>) -> Startup {
    let paths = Paths::resolve(exe_dir, current_os);
    let migration = legacy_os
        .map(|os| migrate_legacy_files(&paths, &LegacyPaths::resolve(exe_dir, os)))
        .unwrap_or_default();
    let mut notes = Vec::new();
    if !migration.migrated.is_empty() {
        notes.push(format!("migrated legacy {}", migration.migrated.join(", ")));
    }
    notes.extend(migration.warnings.iter().cloned());
    let mut settings = Settings::load(&paths.config_file).unwrap_or_else(|e| {
        let line = set_corrupt_settings_aside(&paths.config_file, &e.to_string());
        append_log(&paths.log_dir, &line);
        notes.push(line);
        Settings::default()
    });
    let (staging_dir, staging_notice) = effective_staging_dir(&settings, &paths);
    if staging_notice.is_some() {
        // The backend reads the same field; keep both on the same folder.
        settings.staging_dir = None;
    }
    notes.extend(staging_notice);
    let notice = (!notes.is_empty()).then(|| notes.join("; "));
    Startup {
        paths,
        migration,
        settings,
        staging_dir,
        notice,
    }
}

/// Removes abandoned job folders from `staging_dir` and returns the report
/// line, which is also appended to `<log_dir>/startup.log`. Never fails.
pub fn sweep_staging(staging_dir: &Path, log_dir: &Path, now: SystemTime) -> String {
    let line = match Staging::new(staging_dir.to_path_buf()).sweep(now, PARTIAL_TTL) {
        Ok(r) => {
            let mut line = format!(
                "staging sweep: removed {} orphan and {} expired job folders, kept {} partial, freed {} bytes",
                r.removed_orphans, r.removed_expired, r.kept_partials, r.freed_bytes
            );
            if r.failed > 0 {
                line.push_str(&format!(
                    "; {} could not be removed ({})",
                    r.failed,
                    r.errors.join("; ")
                ));
            }
            line
        }
        Err(e) => format!("staging sweep failed: {e}"),
    };
    append_log(log_dir, &line);
    line
}

/// Best effort: a log that cannot be written goes to stderr instead.
pub fn append_log(log_dir: &Path, line: &str) {
    use std::io::Write;
    let written = std::fs::create_dir_all(log_dir).and_then(|()| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("startup.log"))?;
        writeln!(f, "{line}")
    });
    if let Err(e) = written {
        eprintln!("hf2ollama: {line} (could not write the startup log: {e})");
    }
}

/// Runs [`sweep_staging`] on its own thread so the window never waits for it.
pub fn spawn_sweep(staging_dir: PathBuf, log_dir: PathBuf) {
    let spawned = std::thread::Builder::new()
        .name("hf2ollama-sweep".into())
        .spawn(move || {
            sweep_staging(&staging_dir, &log_dir, SystemTime::now());
        });
    if let Err(e) = spawned {
        eprintln!("could not start the staging sweep: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roost_core::events::JobId;

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
        let s = Settings {
            staging_dir: Some(root.path().join("mine")),
            ..Settings::default()
        };
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
            let s = Settings {
                staging_dir: Some(bad.clone()),
                ..Settings::default()
            };
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
        std::fs::write(cfg.join("roost.toml"), "staging_dir = \"relative/x\"\n").unwrap();
        let st = startup(exe.path(), os(osd.path()), None);
        assert_eq!(st.settings.staging_dir, None);
        assert_eq!(st.staging_dir, st.paths.default_staging);
        assert!(st.notice.is_some());
    }

    #[test]
    fn startup_with_portable_marker_writes_nothing_outside_exe_dir() {
        let exe = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("portable"), "").unwrap();
        let osd = tempfile::tempdir().unwrap();
        let st = startup(exe.path(), os(osd.path()), None);
        assert!(st.paths.portable);
        assert_eq!(st.paths.config_file, exe.path().join("roost.toml"));
        assert_eq!(
            st.paths.import_history_file,
            exe.path().join("history.json")
        );
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
        let st = startup(exe.path(), os(osd.path()), None);
        assert!(!st.paths.portable);
        assert_eq!(st.paths.config_file, osd.path().join("config/roost.toml"));
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

    #[test]
    fn a_corrupt_settings_file_is_kept_as_bak_before_falling_back_to_defaults() {
        let exe = tempfile::tempdir().unwrap();
        let osd = tempfile::tempdir().unwrap();
        let cfg = osd.path().join("config/roost.toml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "this is = not [valid toml").unwrap();
        // An older backup is replaced.
        let bak = osd.path().join("config/roost.toml.bak");
        std::fs::write(&bak, "old backup").unwrap();

        let st = startup(exe.path(), os(osd.path()), None);

        assert_eq!(st.settings, Settings::default());
        assert!(!cfg.exists(), "the unreadable file is moved out of the way");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "this is = not [valid toml"
        );
        let notice = st.notice.expect("a startup notice");
        assert!(notice.contains("roost.toml.bak"), "{notice}");
        let log = std::fs::read_to_string(osd.path().join("state/logs/startup.log")).unwrap();
        assert!(log.contains("roost.toml.bak"), "{log}");
    }

    #[test]
    fn a_valid_settings_file_is_left_alone() {
        let exe = tempfile::tempdir().unwrap();
        let osd = tempfile::tempdir().unwrap();
        let cfg = osd.path().join("config/roost.toml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "max_parallel = 3\n").unwrap();
        let st = startup(exe.path(), os(osd.path()), None);
        assert_eq!(st.settings.max_parallel, 3);
        assert!(cfg.exists());
        assert!(!osd.path().join("config/roost.toml.bak").exists());
        assert!(st.notice.is_none());
    }

    #[test]
    fn startup_migrates_legacy_metadata_before_loading_settings() {
        let exe = tempfile::tempdir().unwrap();
        let current_os = tempfile::tempdir().unwrap();
        let legacy_os = tempfile::tempdir().unwrap();
        let legacy_config = legacy_os.path().join("config/hf2ollama.toml");
        std::fs::create_dir_all(legacy_config.parent().unwrap()).unwrap();
        std::fs::write(&legacy_config, "max_parallel = 3\n").unwrap();
        std::fs::write(
            legacy_os.path().join("config/history.json"),
            "{\"imports\":{\"qwen\":\"org/repo\"}}",
        )
        .unwrap();

        let st = startup(
            exe.path(),
            os(current_os.path()),
            Some(os(legacy_os.path())),
        );

        assert_eq!(st.settings.max_parallel, 3);
        assert_eq!(st.migration.migrated, vec!["settings", "import history"]);
        assert_eq!(
            std::fs::read_to_string(&st.paths.import_history_file).unwrap(),
            "{\"imports\":{\"qwen\":\"org/repo\"}}"
        );
        assert!(st
            .notice
            .as_deref()
            .unwrap()
            .contains("migrated legacy settings"));
    }

    #[test]
    fn startup_keeps_current_data_and_configured_staging() {
        let exe = tempfile::tempdir().unwrap();
        let current_os = tempfile::tempdir().unwrap();
        let legacy_os = tempfile::tempdir().unwrap();
        let configured_staging = current_os.path().join("configured-staging");
        let current_config = current_os.path().join("config/roost.toml");
        std::fs::create_dir_all(current_config.parent().unwrap()).unwrap();
        std::fs::write(
            &current_config,
            format!(
                "max_parallel = 2\nstaging_dir = \"{}\"\n",
                configured_staging.display()
            ),
        )
        .unwrap();
        let legacy_config = legacy_os.path().join("config/hf2ollama.toml");
        std::fs::create_dir_all(legacy_config.parent().unwrap()).unwrap();
        std::fs::write(&legacy_config, "max_parallel = 3\n").unwrap();

        let st = startup(
            exe.path(),
            os(current_os.path()),
            Some(os(legacy_os.path())),
        );

        assert_eq!(st.settings.max_parallel, 2);
        assert_eq!(st.staging_dir, configured_staging);
        assert!(st.migration.migrated.is_empty());
        assert_eq!(
            std::fs::read_to_string(&current_config)
                .unwrap()
                .lines()
                .next(),
            Some("max_parallel = 2")
        );
    }

    #[test]
    fn portable_startup_migrates_only_the_distinct_legacy_settings_file() {
        let exe = tempfile::tempdir().unwrap();
        std::fs::write(exe.path().join("portable"), "").unwrap();
        std::fs::write(exe.path().join("hf2ollama.toml"), "max_parallel = 4\n").unwrap();
        std::fs::write(exe.path().join("history.json"), "{\"imports\":{}}\n").unwrap();
        let osd = tempfile::tempdir().unwrap();

        let st = startup(exe.path(), os(osd.path()), Some(os(osd.path())));

        assert!(st.paths.portable);
        assert_eq!(st.paths.sessions_dir, exe.path().join("sessions"));
        assert_eq!(st.migration.migrated, vec!["settings"]);
        assert_eq!(
            std::fs::read_to_string(exe.path().join("history.json")).unwrap(),
            "{\"imports\":{}}\n"
        );
        assert!(std::fs::read_dir(osd.path()).unwrap().next().is_none());
    }

    #[test]
    fn startup_surfaces_an_unreadable_legacy_source_as_a_notice() {
        let exe = tempfile::tempdir().unwrap();
        let current_os = tempfile::tempdir().unwrap();
        let legacy_os = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(legacy_os.path().join("config/hf2ollama.toml")).unwrap();

        let st = startup(
            exe.path(),
            os(current_os.path()),
            Some(os(legacy_os.path())),
        );

        assert!(st.migration.migrated.is_empty());
        assert_eq!(st.migration.warnings.len(), 1);
        assert!(st
            .notice
            .as_deref()
            .unwrap()
            .contains("could not read legacy settings"));
    }
}
