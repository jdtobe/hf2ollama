use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{Error, Result};
use crate::events::JobId;

const JOB_MARKER: &str = ".hf2ollama-job";
const PARTIAL_MARKER: &str = ".partial";
const KEPT_MARKER: &str = ".kept";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEntry {
    pub label: String,
    pub bytes: u64,
    pub partial: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub removed_orphans: u32,
    pub removed_expired: u32,
    pub kept_partials: u32,
    pub freed_bytes: u64,
    /// Directories that could not be removed.
    pub failed: u32,
    /// One message per failed directory.
    pub errors: Vec<String>,
}

/// What `clear_cached_report` did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClearReport {
    pub freed_bytes: u64,
    pub removed: u32,
    pub failed: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Staging {
    root: PathBuf,
}

#[derive(Debug)]
pub struct JobDir {
    path: PathBuf,
}

fn io_err(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

/// True only for a real (non-symlink) directory named `job-<digits>` that
/// carries our creation marker. Every deletion goes through this check.
fn is_our_job_dir(path: &Path) -> bool {
    let name_ok = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("job-"))
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
    if !name_ok {
        return false;
    }
    let is_real_dir = std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false);
    let marked = std::fs::symlink_metadata(path.join(JOB_MARKER))
        .map(|m| m.file_type().is_file())
        .unwrap_or(false);
    is_real_dir && marked
}

fn remove_job_dir(path: &Path) -> Result<()> {
    if !is_our_job_dir(path) {
        return Err(Error::UnsafePath(path.display().to_string()));
    }
    std::fs::remove_dir_all(path).map_err(io_err)
}

fn has_marker(dir: &Path, name: &str) -> bool {
    std::fs::symlink_metadata(dir.join(name)).is_ok()
}

/// Total size of regular files below `path`, never following symlinks.
fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        })
        .fold(0u64, u64::saturating_add)
}

impl Staging {
    pub fn new(root: PathBuf) -> Staging {
        Staging { root }
    }

    pub fn create_job(&self, id: JobId, needed_bytes: u64) -> Result<JobDir> {
        std::fs::create_dir_all(&self.root).map_err(io_err)?;
        let available = fs4::available_space(&self.root).map_err(io_err)?;
        if needed_bytes > available {
            return Err(Error::InsufficientSpace {
                needed: needed_bytes,
                available,
            });
        }
        let path = self.root.join(format!("job-{}", id.0));
        // create_dir fails if it already exists, so a foreign directory is never adopted.
        std::fs::create_dir(&path).map_err(io_err)?;
        std::fs::write(path.join(JOB_MARKER), b"").map_err(io_err)?;
        Ok(JobDir { path })
    }

    /// Job directories created by this app: direct children of the root only.
    fn jobs(&self) -> Result<Vec<PathBuf>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(e)),
        };
        let mut jobs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_our_job_dir(p))
            .collect();
        jobs.sort();
        Ok(jobs)
    }

    pub fn sweep(&self, now: SystemTime, partial_ttl: Duration) -> Result<SweepReport> {
        let mut report = SweepReport::default();
        for job in self.jobs()? {
            if has_marker(&job, KEPT_MARKER) {
                continue;
            }
            let expired = if has_marker(&job, PARTIAL_MARKER) {
                let age = std::fs::symlink_metadata(job.join(PARTIAL_MARKER))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .unwrap_or(Duration::ZERO);
                if age <= partial_ttl {
                    report.kept_partials += 1;
                    continue;
                }
                true
            } else {
                false
            };
            let bytes = dir_size(&job);
            remove_job_dir(&job)?;
            report.freed_bytes = report.freed_bytes.saturating_add(bytes);
            if expired {
                report.removed_expired += 1;
            } else {
                report.removed_orphans += 1;
            }
        }
        Ok(report)
    }

    pub fn list_cached(&self) -> Result<Vec<CachedEntry>> {
        Ok(self
            .jobs()?
            .into_iter()
            .filter_map(|job| {
                let kept = has_marker(&job, KEPT_MARKER);
                let partial = has_marker(&job, PARTIAL_MARKER);
                if !kept && !partial {
                    return None;
                }
                Some(CachedEntry {
                    label: job.file_name()?.to_string_lossy().into_owned(),
                    bytes: dir_size(&job),
                    partial: partial && !kept,
                })
            })
            .collect())
    }

    /// Removes kept (finished with keep) and failed (`.partial`) job directories and
    /// returns the bytes freed. A directory with neither marker may belong to a running
    /// job and is never touched.
    pub fn clear_cached_report(&self) -> Result<ClearReport> {
        let mut report = ClearReport::default();
        for job in self.jobs()? {
            if has_marker(&job, KEPT_MARKER) || has_marker(&job, PARTIAL_MARKER) {
                let bytes = dir_size(&job);
                remove_job_dir(&job)?;
                report.freed_bytes = report.freed_bytes.saturating_add(bytes);
                report.removed += 1;
            }
        }
        Ok(report)
    }

    pub fn clear_cached(&self) -> Result<u64> {
        let mut freed = 0u64;
        for job in self.jobs()? {
            if has_marker(&job, KEPT_MARKER) || has_marker(&job, PARTIAL_MARKER) {
                let bytes = dir_size(&job);
                remove_job_dir(&job)?;
                freed = freed.saturating_add(bytes);
            }
        }
        Ok(freed)
    }
}

impl JobDir {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn finish(self, keep: bool) -> Result<()> {
        if keep {
            std::fs::write(self.path.join(KEPT_MARKER), b"").map_err(io_err)?;
            match std::fs::remove_file(self.path.join(PARTIAL_MARKER)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err(e)),
                _ => Ok(()),
            }
        } else {
            remove_job_dir(&self.path)
        }
    }

    pub fn mark_partial(&self) -> Result<()> {
        std::fs::write(self.path.join(PARTIAL_MARKER), b"").map_err(io_err)
    }

    pub fn discard(self) -> Result<()> {
        remove_job_dir(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn staging(dir: &tempfile::TempDir) -> Staging {
        Staging::new(dir.path().join("staging"))
    }

    fn write(path: &Path, bytes: usize) {
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn create_job_makes_unique_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let a = s.create_job(JobId(1), 0).unwrap();
        let b = s.create_job(JobId(2), 0).unwrap();
        assert_ne!(a.path(), b.path());
        assert!(a.path().is_dir());
        assert!(b.path().is_dir());
        assert!(a.path().starts_with(tmp.path().join("staging")));
    }

    #[test]
    fn job_dir_name_is_job_dash_id() {
        let tmp = tempfile::tempdir().unwrap();
        let job = staging(&tmp).create_job(JobId(42), 0).unwrap();
        assert_eq!(job.path().file_name().unwrap(), "job-42");
        assert!(job.path().join(".hf2ollama-job").is_file());
    }

    #[test]
    fn insufficient_space_errors_before_creating() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let err = s.create_job(JobId(1), u64::MAX).unwrap_err();
        assert!(matches!(err, Error::InsufficientSpace { needed, .. } if needed == u64::MAX));
        assert!(!tmp.path().join("staging").join("job-1").exists());
    }

    #[test]
    fn finish_without_keep_removes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let job = staging(&tmp).create_job(JobId(1), 0).unwrap();
        let p = job.path().to_path_buf();
        write(&p.join("a.bin"), 10);
        job.finish(false).unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn finish_with_keep_leaves_files_and_lists_as_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let job = s.create_job(JobId(1), 0).unwrap();
        let p = job.path().to_path_buf();
        write(&p.join("a.bin"), 100);
        job.finish(true).unwrap();
        assert!(p.join("a.bin").is_file());
        let cached = s.list_cached().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].label, "job-1");
        assert!(cached[0].bytes >= 100);
        assert!(!cached[0].partial);
    }

    #[test]
    fn discard_removes_even_partial() {
        let tmp = tempfile::tempdir().unwrap();
        let job = staging(&tmp).create_job(JobId(1), 0).unwrap();
        let p = job.path().to_path_buf();
        job.mark_partial().unwrap();
        assert!(p.join(".partial").is_file());
        job.discard().unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn sweep_removes_expired_partials_keeps_fresh_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let job = s.create_job(JobId(1), 0).unwrap();
        write(&job.path().join("a.bin"), 50);
        job.mark_partial().unwrap();
        let p = job.path().to_path_buf();
        let ttl = Duration::from_secs(3600);

        let fresh = s.sweep(SystemTime::now(), ttl).unwrap();
        assert_eq!(fresh.kept_partials, 1);
        assert_eq!(fresh.removed_expired, 0);
        assert!(p.exists());
        assert!(s.list_cached().unwrap()[0].partial);

        let later = SystemTime::now() + Duration::from_secs(7200);
        let report = s.sweep(later, ttl).unwrap();
        assert_eq!(report.removed_expired, 1);
        assert_eq!(report.kept_partials, 0);
        assert!(report.freed_bytes >= 50);
        assert!(!p.exists());
    }

    #[test]
    fn sweep_removes_orphan_dirs_without_marker_and_not_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        // Simulate a crash: job dir left behind, never finished or marked.
        let orphan = s.create_job(JobId(1), 0).unwrap();
        write(&orphan.path().join("a.bin"), 10);
        let orphan_path = orphan.path().to_path_buf();
        std::mem::forget(orphan);
        let kept = s.create_job(JobId(2), 0).unwrap();
        let kept_path = kept.path().to_path_buf();
        kept.finish(true).unwrap();

        let report = s
            .sweep(SystemTime::now(), Duration::from_secs(3600))
            .unwrap();
        assert_eq!(report.removed_orphans, 1);
        assert!(report.freed_bytes >= 10);
        assert!(!orphan_path.exists());
        assert!(kept_path.exists());
    }

    #[test]
    fn sweep_and_clear_never_touch_foreign_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let root = tmp.path().join("staging");
        fs::create_dir_all(root.join("photos")).unwrap();
        write(&root.join("photos").join("a.jpg"), 10);
        write(&root.join("notes.txt"), 10);
        fs::create_dir_all(root.join("job-9")).unwrap();
        write(&root.join("job-9").join("mine.txt"), 10);
        let kept = s.create_job(JobId(1), 0).unwrap();
        kept.finish(true).unwrap();

        let report = s
            .sweep(
                SystemTime::now() + Duration::from_secs(10_000),
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(report, SweepReport::default());
        let labels: Vec<String> = s
            .list_cached()
            .unwrap()
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["job-1".to_string()]);
        s.clear_cached().unwrap();

        assert!(root.join("notes.txt").is_file());
        assert!(root.join("photos").join("a.jpg").is_file());
        assert!(root.join("job-9").join("mine.txt").is_file());
        assert!(!root.join("job-1").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_job_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let root = tmp.path().join("staging");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        write(&outside.join("precious.txt"), 10);
        write(&outside.join(".hf2ollama-job"), 0);
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("job-3")).unwrap();

        let report = s.sweep(SystemTime::now(), Duration::ZERO).unwrap();
        assert_eq!(report, SweepReport::default());
        assert!(s.list_cached().unwrap().is_empty());
        assert_eq!(s.clear_cached().unwrap(), 0);
        assert!(outside.join("precious.txt").is_file());
        assert!(root.join("job-3").exists());
    }

    #[test]
    fn paths_with_spaces_and_unicode_work() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Staging::new(tmp.path().join("My Staging ü 模型"));
        let job = s.create_job(JobId(7), 0).unwrap();
        write(&job.path().join("a b.bin"), 5);
        let p = job.path().to_path_buf();
        job.finish(true).unwrap();
        assert!(p.join("a b.bin").is_file());
        assert_eq!(s.list_cached().unwrap().len(), 1);
        s.clear_cached().unwrap();
        assert!(!p.exists());
    }

    /// Makes `job-1` undeletable (read-only with a file inside). Returns false
    /// when permissions are not enforced (running as root).
    #[cfg(unix)]
    fn make_undeletable(dir: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        write(&dir.join("stuck.bin"), 50);
        fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).unwrap();
        fs::write(dir.join("probe"), b"x").is_err()
    }

    #[cfg(unix)]
    fn restore(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    }

    #[cfg(unix)]
    #[test]
    fn clear_continues_past_a_directory_that_cannot_be_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let stuck = s.create_job(JobId(1), 0).unwrap();
        let stuck_path = stuck.path().to_path_buf();
        stuck.finish(true).unwrap();
        let ok = s.create_job(JobId(2), 0).unwrap();
        write(&ok.path().join("a.bin"), 100);
        let ok_path = ok.path().to_path_buf();
        ok.mark_partial().unwrap();
        if !make_undeletable(&stuck_path) {
            restore(&stuck_path);
            return;
        }
        let report = s.clear_cached_report().unwrap();
        let single = s.clear_cached();
        restore(&stuck_path);
        assert!(!ok_path.exists(), "the other directory is still removed");
        assert_eq!(report.removed, 1);
        assert_eq!(report.failed, 1, "{report:?}");
        assert_eq!(report.errors.len(), 1);
        assert!(report.freed_bytes >= 100);
        // Nothing removable is left, so the plain call reports the failure.
        assert!(single.is_err(), "{single:?}");
        assert!(stuck_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_continues_past_a_directory_that_cannot_be_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let stuck = s.create_job(JobId(1), 0).unwrap();
        let stuck_path = stuck.path().to_path_buf();
        std::mem::forget(stuck); // orphan: no markers
        let orphan = s.create_job(JobId(2), 0).unwrap();
        let orphan_path = orphan.path().to_path_buf();
        std::mem::forget(orphan);
        if !make_undeletable(&stuck_path) {
            restore(&stuck_path);
            return;
        }
        let report = s
            .sweep(SystemTime::now(), Duration::from_secs(3600))
            .unwrap();
        restore(&stuck_path);
        assert!(!orphan_path.exists());
        assert_eq!(report.removed_orphans, 1);
        assert_eq!(report.failed, 1, "{report:?}");
        assert_eq!(report.errors.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn clear_cached_removes_partial_dirs_but_never_active_or_foreign_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        let root = tmp.path().join("staging");
        // foreign directory
        fs::create_dir_all(root.join("photos")).unwrap();
        write(&root.join("photos").join("a.jpg"), 11);
        // unmarked job-9 (not ours)
        fs::create_dir_all(root.join("job-9")).unwrap();
        write(&root.join("job-9").join("mine.txt"), 12);
        // symlinked job dir pointing at a marked directory outside
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        write(&outside.join("precious.txt"), 13);
        write(&outside.join(".hf2ollama-job"), 0);
        write(&outside.join(".partial"), 0);
        std::os::unix::fs::symlink(&outside, root.join("job-3")).unwrap();
        // active job: our marker only, no .kept/.partial
        let active = s.create_job(JobId(4), 0).unwrap();
        write(&active.path().join("a.part"), 14);
        // kept and partial
        let kept = s.create_job(JobId(5), 0).unwrap();
        write(&kept.path().join("k.bin"), 100);
        let kept_path = kept.path().to_path_buf();
        kept.finish(true).unwrap();
        let partial = s.create_job(JobId(6), 0).unwrap();
        write(&partial.path().join("p.part"), 200);
        partial.mark_partial().unwrap();
        let partial_path = partial.path().to_path_buf();

        let freed = s.clear_cached().unwrap();
        assert_eq!(freed, 300);
        assert!(!kept_path.exists());
        assert!(!partial_path.exists());
        assert!(root.join("photos").join("a.jpg").is_file());
        assert!(root.join("job-9").join("mine.txt").is_file());
        assert!(outside.join("precious.txt").is_file());
        assert!(root.join("job-3").exists());
        assert!(active.path().join("a.part").is_file());
    }

    #[test]
    fn clear_cached_returns_freed_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let s = staging(&tmp);
        for i in 1..=2 {
            let job = s.create_job(JobId(i), 0).unwrap();
            write(&job.path().join("a.bin"), 100);
            job.finish(true).unwrap();
        }
        assert!(s.clear_cached().unwrap() >= 200);
        assert!(s.list_cached().unwrap().is_empty());
    }
}
