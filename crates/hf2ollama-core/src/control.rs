//! Pause, resume and cancel handle shared between a job and its controller.

use crate::error::{Error, Result};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Clone, Copy, Default)]
struct State {
    paused: bool,
    cancelled: bool,
}

/// Cloneable control handle; all clones share one state.
#[derive(Clone)]
pub struct JobControl {
    tx: Arc<watch::Sender<State>>,
}

impl Default for JobControl {
    fn default() -> Self {
        Self::new()
    }
}

impl JobControl {
    pub fn new() -> JobControl {
        JobControl {
            tx: Arc::new(watch::channel(State::default()).0),
        }
    }
    pub fn pause(&self) {
        self.tx.send_modify(|s| s.paused = true);
    }
    pub fn resume(&self) {
        self.tx.send_modify(|s| s.paused = false);
    }
    pub fn cancel(&self) {
        self.tx.send_modify(|s| s.cancelled = true);
    }
    pub fn is_paused(&self) -> bool {
        self.tx.borrow().paused
    }
    pub fn is_cancelled(&self) -> bool {
        self.tx.borrow().cancelled
    }
    /// Resolves once cancelled; ignores pause. For racing against slow I/O.
    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        loop {
            if rx.borrow_and_update().cancelled {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
    /// `Err(Cancelled)` once cancelled; waits (without polling) while paused.
    pub async fn checkpoint(&self) -> Result<()> {
        let mut rx = self.tx.subscribe();
        loop {
            let state = *rx.borrow_and_update();
            if state.cancelled {
                return Err(Error::Cancelled);
            }
            if !state.paused {
                return Ok(());
            }
            // The sender lives in `self`, so `changed` cannot fail here.
            if rx.changed().await.is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn checkpoint_passes_when_idle() {
        let c = JobControl::new();
        assert!(!c.is_paused() && !c.is_cancelled());
        assert_eq!(c.checkpoint().await, Ok(()));
    }

    #[tokio::test]
    async fn cancel_makes_checkpoint_fail_for_all_clones() {
        let c = JobControl::new();
        let d = c.clone();
        c.cancel();
        assert!(d.is_cancelled());
        assert_eq!(d.checkpoint().await, Err(Error::Cancelled));
    }

    #[tokio::test]
    async fn pause_blocks_checkpoint_until_resume() {
        let c = JobControl::new();
        c.pause();
        assert!(c.is_paused());
        let waiter = c.clone();
        let h = tokio::spawn(async move { waiter.checkpoint().await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!h.is_finished());
        c.resume();
        assert_eq!(
            timeout(Duration::from_secs(5), h).await.unwrap().unwrap(),
            Ok(())
        );
        assert!(!c.is_paused());
    }

    #[tokio::test]
    async fn cancel_wakes_a_paused_checkpoint() {
        let c = JobControl::new();
        c.pause();
        let waiter = c.clone();
        let h = tokio::spawn(async move { waiter.checkpoint().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.cancel();
        assert_eq!(
            timeout(Duration::from_secs(5), h).await.unwrap().unwrap(),
            Err(Error::Cancelled)
        );
    }
}
