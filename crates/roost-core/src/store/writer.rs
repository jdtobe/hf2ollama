//! All session disk work runs on one dedicated thread so a slow disk never
//! stalls the chat manager. Deleting a session tombstones its id, so a late
//! save can never bring the file back.

use super::{IndexRow, LoadError, Store, StoreWarning};
use crate::session::{Session, SessionId};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

enum Job {
    Save(Box<Session>),
    Delete(SessionId),
    Restore(SessionId, oneshot::Sender<bool>),
    Purge(SessionId),
    Snapshot(oneshot::Sender<Vec<IndexRow>>),
    Load(SessionId, oneshot::Sender<Result<Session, LoadError>>),
    Flush(oneshot::Sender<()>),
}

#[derive(Default)]
struct Reports {
    warnings: Vec<StoreWarning>,
    errors: Vec<String>,
}

/// Cloneable handle. The thread exits, after finishing queued work, once the
/// last handle is dropped.
#[derive(Clone)]
pub struct Writer {
    tx: Sender<Job>,
    reports: Arc<Mutex<Reports>>,
}

impl Writer {
    pub fn spawn(mut store: Store, startup_warnings: Vec<StoreWarning>) -> Writer {
        let (tx, rx) = channel::<Job>();
        let reports = Arc::new(Mutex::new(Reports {
            warnings: startup_warnings,
            errors: Vec::new(),
        }));
        let thread_reports = Arc::clone(&reports);
        let started = std::thread::Builder::new()
            .name("roost-store".into())
            .spawn(move || {
                let mut tombstones: HashSet<SessionId> = HashSet::new();
                while let Ok(job) = rx.recv() {
                    match job {
                        Job::Save(s) => {
                            if tombstones.contains(&s.id) {
                                continue;
                            }
                            if let Err(e) = store.save(&s) {
                                if let Ok(mut r) = thread_reports.lock() {
                                    r.errors.push(e.to_string());
                                }
                            }
                        }
                        Job::Delete(id) => {
                            tombstones.insert(id);
                            if let Err(e) = store.delete(id) {
                                if let Ok(mut r) = thread_reports.lock() {
                                    r.errors.push(e.to_string());
                                }
                            }
                        }
                        Job::Restore(id, reply) => {
                            let ok = store.restore(id).is_ok();
                            if ok {
                                tombstones.remove(&id);
                            }
                            let _ = reply.send(ok);
                        }
                        Job::Purge(id) => store.purge(id),
                        Job::Snapshot(reply) => {
                            let _ = reply.send(store.rows());
                        }
                        Job::Load(id, reply) => {
                            let _ = reply.send(store.load(id));
                        }
                        Job::Flush(reply) => {
                            let _ = reply.send(());
                        }
                    }
                }
            });
        if let Err(e) = started {
            eprintln!("could not start the history writer: {e}");
        }
        Writer { tx, reports }
    }

    /// Queues a save; dropped if the session was deleted.
    pub fn save(&self, s: Session) {
        let _ = self.tx.send(Job::Save(Box::new(s)));
    }

    /// Moves the session to the trash; it stays recoverable until `purge`.
    pub fn delete(&self, id: SessionId) {
        let _ = self.tx.send(Job::Delete(id));
    }

    /// Brings a deleted session back. False when it is no longer in the trash.
    pub async fn restore(&self, id: SessionId) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Job::Restore(id, tx)).is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    /// Removes a deleted session for good.
    pub fn purge(&self, id: SessionId) {
        let _ = self.tx.send(Job::Purge(id));
    }

    pub async fn snapshot(&self) -> Vec<IndexRow> {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Job::Snapshot(tx)).is_err() {
            return Vec::new();
        }
        rx.await.unwrap_or_default()
    }

    pub async fn load(&self, id: SessionId) -> Result<Session, LoadError> {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Job::Load(id, tx)).is_err() {
            return Err(LoadError::Unreadable("history writer stopped".into()));
        }
        rx.await
            .unwrap_or_else(|_| Err(LoadError::Unreadable("history writer stopped".into())))
    }

    /// Returns once every job queued before this call has been applied.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Job::Flush(tx)).is_ok() {
            let _ = rx.await;
        }
    }

    pub fn take_warnings(&self) -> Vec<StoreWarning> {
        self.reports
            .lock()
            .map(|mut r| std::mem::take(&mut r.warnings))
            .unwrap_or_default()
    }

    /// Readable messages for saves that failed.
    pub fn take_errors(&self) -> Vec<String> {
        self.reports
            .lock()
            .map(|mut r| std::mem::take(&mut r.errors))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionLifetime;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn chat(title: &str) -> Session {
        let mut s = Session::new_chat("m", SessionLifetime::Persisted, 1_000);
        s.rename(title).unwrap();
        s
    }

    fn writer(dir: &std::path::Path) -> Writer {
        let (store, w) = Store::open(dir).unwrap();
        Writer::spawn(store, w)
    }

    #[tokio::test]
    async fn saves_are_applied_in_order_and_flush_waits() {
        let d = TempDir::new().unwrap();
        let w = writer(d.path());
        let mut s = chat("v1");
        w.save(s.clone());
        s.rename("v2").unwrap();
        w.save(s.clone());
        w.flush().await;
        assert_eq!(w.load(s.id).await.unwrap().title, "v2");
    }

    #[tokio::test]
    async fn a_save_after_delete_does_not_recreate_the_file() {
        let d = TempDir::new().unwrap();
        let w = writer(d.path());
        let s = chat("doomed");
        w.save(s.clone());
        w.delete(s.id);
        w.save(s.clone());
        w.flush().await;
        assert!(w.snapshot().await.is_empty());
        assert!(!d.path().join(format!("{}.json", s.id)).exists());
    }

    #[tokio::test]
    async fn snapshot_reflects_queued_saves() {
        let d = TempDir::new().unwrap();
        let w = writer(d.path());
        w.save(chat("a"));
        w.save(chat("b"));
        assert_eq!(w.snapshot().await.len(), 2);
    }

    #[tokio::test]
    async fn dropping_all_handles_drains_the_queue_and_releases_the_lock() {
        let d = TempDir::new().unwrap();
        let w = writer(d.path());
        let s = chat("last words");
        w.save(s.clone());
        drop(w);
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut store, _) = loop {
            match Store::open(d.path()) {
                Ok(v) => break v,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(e) => panic!("lock never released: {e}"),
            }
        };
        assert_eq!(store.load(s.id).unwrap().title, "last words");
    }

    #[tokio::test]
    async fn startup_warnings_are_reported_once() {
        let d = TempDir::new().unwrap();
        std::fs::write(d.path().join("index-v1.json"), b"junk").unwrap();
        let w = writer(d.path());
        assert_eq!(w.take_warnings(), vec![StoreWarning::IndexRebuilt]);
        assert!(w.take_warnings().is_empty());
    }
}
