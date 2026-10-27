use roost_core::migration::migrate_legacy_files;
use roost_core::paths::{LegacyPaths, OsDirs, Paths};
use std::path::Path;

fn os(root: &Path) -> OsDirs {
    OsDirs {
        config: root.join("config"),
        cache: root.join("cache"),
        state: root.join("state"),
    }
}

#[test]
fn portable_legacy_and_current_share_history_logs_and_staging_but_not_settings() {
    let exe = tempfile::tempdir().unwrap();
    std::fs::write(exe.path().join("portable"), "").unwrap();
    let other = tempfile::tempdir().unwrap();

    let current = Paths::resolve(exe.path(), os(other.path()));
    let legacy = LegacyPaths::resolve(exe.path(), os(other.path()));

    assert_eq!(current.config_file, exe.path().join("roost.toml"));
    assert_eq!(legacy.config_file, exe.path().join("hf2ollama.toml"));
    assert_eq!(current.import_history_file, legacy.import_history_file);
    assert_eq!(current.log_dir, legacy.log_dir);
    assert_eq!(current.default_staging, legacy.default_staging);
    assert_eq!(current.sessions_dir, exe.path().join("sessions"));
}

#[test]
fn os_legacy_paths_use_hf2ollama_metadata_directories() {
    let exe = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let legacy = LegacyPaths::resolve(exe.path(), os(root.path()));

    assert_eq!(
        legacy.config_file,
        root.path().join("config/hf2ollama.toml")
    );
    assert_eq!(
        legacy.import_history_file,
        root.path().join("config/history.json")
    );
    assert_eq!(legacy.log_dir, root.path().join("state/logs"));
    assert_eq!(legacy.default_staging, root.path().join("cache/staging"));
    assert!(!legacy.portable);
}

#[test]
fn migration_copies_missing_settings_and_import_history_without_logs_or_staging() {
    let current_root = tempfile::tempdir().unwrap();
    let legacy_root = tempfile::tempdir().unwrap();
    let exe = tempfile::tempdir().unwrap();
    let current = Paths::resolve(exe.path(), os(current_root.path()));
    let legacy = LegacyPaths::resolve(exe.path(), os(legacy_root.path()));
    std::fs::create_dir_all(legacy.config_file.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&legacy.log_dir).unwrap();
    std::fs::create_dir_all(&legacy.default_staging).unwrap();
    std::fs::write(&legacy.config_file, "max_parallel = 3\n").unwrap();
    std::fs::write(&legacy.import_history_file, "{\"imports\":{}}\n").unwrap();
    std::fs::write(legacy.log_dir.join("old.log"), "log").unwrap();
    std::fs::write(legacy.default_staging.join("old.part"), "cache").unwrap();

    let report = migrate_legacy_files(&current, &legacy);

    assert_eq!(
        std::fs::read_to_string(&current.config_file).unwrap(),
        "max_parallel = 3\n"
    );
    assert_eq!(
        std::fs::read_to_string(&current.import_history_file).unwrap(),
        "{\"imports\":{}}\n"
    );
    assert_eq!(report.migrated, vec!["settings", "import history"]);
    assert!(!current.log_dir.exists());
    assert!(!current.default_staging.exists());
    assert!(legacy.config_file.exists());
    assert!(legacy.import_history_file.exists());
}

#[test]
fn migration_preserves_current_metadata_and_is_idempotent() {
    let current_root = tempfile::tempdir().unwrap();
    let legacy_root = tempfile::tempdir().unwrap();
    let exe = tempfile::tempdir().unwrap();
    let current = Paths::resolve(exe.path(), os(current_root.path()));
    let legacy = LegacyPaths::resolve(exe.path(), os(legacy_root.path()));
    std::fs::create_dir_all(current.config_file.parent().unwrap()).unwrap();
    std::fs::create_dir_all(legacy.config_file.parent().unwrap()).unwrap();
    std::fs::write(&current.config_file, "max_parallel = 9\n").unwrap();
    std::fs::write(&legacy.config_file, "max_parallel = 3\n").unwrap();
    std::fs::write(
        &legacy.import_history_file,
        "{\"imports\":{\"old\":\"repo\"}}\n",
    )
    .unwrap();

    let first = migrate_legacy_files(&current, &legacy);
    let second = migrate_legacy_files(&current, &legacy);

    assert_eq!(
        std::fs::read_to_string(&current.config_file).unwrap(),
        "max_parallel = 9\n"
    );
    assert_eq!(first.migrated, vec!["import history"]);
    assert!(second.migrated.is_empty());
    assert!(second.warnings.is_empty());
}

#[test]
fn portable_shared_import_history_is_not_copied() {
    let exe = tempfile::tempdir().unwrap();
    std::fs::write(exe.path().join("portable"), "").unwrap();
    let other = tempfile::tempdir().unwrap();
    let current = Paths::resolve(exe.path(), os(other.path()));
    let legacy = LegacyPaths::resolve(exe.path(), os(other.path()));
    std::fs::write(
        &legacy.import_history_file,
        "{\"imports\":{\"keep\":\"repo\"}}\n",
    )
    .unwrap();

    let report = migrate_legacy_files(&current, &legacy);

    assert!(report.migrated.is_empty());
    assert_eq!(
        std::fs::read_to_string(&current.import_history_file).unwrap(),
        "{\"imports\":{\"keep\":\"repo\"}}\n"
    );
}

#[test]
fn migration_copy_failure_leaves_legacy_source_and_current_destination_intact() {
    let current_root = tempfile::tempdir().unwrap();
    let legacy_root = tempfile::tempdir().unwrap();
    let exe = tempfile::tempdir().unwrap();
    let mut current = Paths::resolve(exe.path(), os(current_root.path()));
    let legacy = LegacyPaths::resolve(exe.path(), os(legacy_root.path()));
    std::fs::create_dir_all(legacy.config_file.parent().unwrap()).unwrap();
    std::fs::write(&legacy.config_file, "max_parallel = 3\n").unwrap();
    let blocker = current_root.path().join("blocker");
    std::fs::write(&blocker, "blocker").unwrap();
    current.config_file = blocker.join("roost.toml");

    let report = migrate_legacy_files(&current, &legacy);

    assert!(report.migrated.is_empty());
    assert_eq!(
        std::fs::read_to_string(&legacy.config_file).unwrap(),
        "max_parallel = 3\n"
    );
    assert_eq!(std::fs::read_to_string(&blocker).unwrap(), "blocker");
    assert!(!current.config_file.exists());
    assert_eq!(report.warnings.len(), 1);
}
