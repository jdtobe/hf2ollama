//! Pause, resume and cancel handle shared between a job and its controller.

use crate::error::{Error, Result};
use std::sync::Arc;
use tokio::sync::watch;

#[allow(dead_code)] // RED stub
#[derive(Clone, Copy, Default)]
struct State {
    paused: bool,
    cancelled: bool,
}

/// Cloneable control handle; all clones share one state.
#[derive(Clone)]
#[allow(dead_code)] // RED stub
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
        todo!()
    }
    pub fn pause(&self) {
        todo!()
    }
    pub fn resume(&self) {
        todo!()
    }
    pub fn cancel(&self) {
        todo!()
    }
    pub fn is_paused(&self) -> bool {
        todo!()
    }
    pub fn is_cancelled(&self) -> bool {
        todo!()
    }
    /// `Err(Cancelled)` once cancelled; waits (without polling) while paused.
    pub async fn checkpoint(&self) -> Result<()> {
        let _ = Error::Cancelled;
        todo!()
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
