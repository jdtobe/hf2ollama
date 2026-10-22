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
}

#[derive(Debug, Clone)]
pub struct Staging {
    root: PathBuf,
}

#[derive(Debug)]
pub struct JobDir {
    path: PathBuf,
}

impl Staging {
    pub fn new(root: PathBuf) -> Staging {
        todo!()
    }

    pub fn create_job(&self, id: JobId, needed_bytes: u64) -> Result<JobDir> {
        todo!()
    }

    pub fn sweep(&self, now: SystemTime, partial_ttl: Duration) -> Result<SweepReport> {
        todo!()
    }

    pub fn list_cached(&self) -> Result<Vec<CachedEntry>> {
        todo!()
    }

    pub fn clear_cached(&self) -> Result<u64> {
        todo!()
    }
}

impl JobDir {
    pub fn path(&self) -> &Path {
        todo!()
    }

    pub fn finish(self, keep: bool) -> Result<()> {
        todo!()
    }

    pub fn mark_partial(&self) -> Result<()> {
        todo!()
    }

    pub fn discard(self) -> Result<()> {
        todo!()
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
        assert_eq!(s.list_cached().unwrap()[0].partial, true);

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
