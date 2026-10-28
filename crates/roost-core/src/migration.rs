//! One-way, non-destructive import of hf2ollama metadata.

use crate::paths::{LegacyPaths, Paths};

/// The outcome of a legacy metadata migration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub migrated: Vec<&'static str>,
    pub warnings: Vec<String>,
}

/// Copies missing settings and import history into Roost's locations.
///
/// Existing Roost paths always win. Legacy files are only read and are never
/// removed. Logs and staging are intentionally excluded: in portable mode
/// they are shared, while OS-mode staging is disposable cache.
pub fn migrate_legacy_files(current: &Paths, legacy: &LegacyPaths) -> MigrationReport {
    let mut report = MigrationReport::default();
    migrate_one(
        "settings",
        &current.config_file,
        &legacy.config_file,
        &mut report,
    );
    migrate_one(
        "import history",
        &current.import_history_file,
        &legacy.import_history_file,
        &mut report,
    );
    report
}

fn migrate_one(
    label: &'static str,
    destination: &std::path::Path,
    source: &std::path::Path,
    report: &mut MigrationReport,
) {
    if destination == source || destination.exists() || !source.exists() {
        return;
    }
    let bytes = match std::fs::read(source) {
        Ok(bytes) => bytes,
        Err(error) => {
            report
                .warnings
                .push(format!("could not read legacy {label}: {error}"));
            return;
        }
    };
    match crate::persistence::atomic_write_if_absent(destination, &bytes) {
        Ok(true) => report.migrated.push(label),
        Ok(false) => {}
        Err(error) => report
            .warnings
            .push(format!("could not migrate legacy {label}: {error}")),
    }
}
