//! Closing the window while downloads are running: ask first, then cancel
//! every job, wait a bounded time for them to settle and close.

#![allow(dead_code)] // stubs until the implementation lands

use crate::app::AppState;
use crate::runtime::BackendRequest;
use eframe::egui;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// The longest the window waits for cancelled jobs before it closes anyway.
pub const CANCEL_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitState {
    Idle,
    /// The dialog is open.
    Confirming,
    /// "Quit and cancel" was chosen; waiting for the jobs to settle.
    Cancelling {
        since: Instant,
    },
    /// The window has been told to close.
    Closing,
}

/// What to do with a close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Let the window close.
    Allow,
    /// Keep the window open (the dialog is or stays up).
    Hold,
}

impl QuitState {
    pub fn on_close_requested(&mut self, _unsettled: usize) -> CloseDecision {
        CloseDecision::Allow
    }

    pub fn keep_running(&mut self) {}

    pub fn quit_and_cancel(&mut self, _now: Instant) {}

    /// True once, when the window should be closed now.
    pub fn tick(&mut self, _now: Instant, _unsettled: usize) -> bool {
        false
    }
}

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub struct QuitProbe {
    pub dialog: Option<egui::Rect>,
    pub keep: Option<egui::Rect>,
    pub quit: Option<egui::Rect>,
    pub quit_id: Option<egui::Id>,
    pub keep_has_focus: bool,
}

pub fn process_close(
    _st: &mut AppState,
    _close_requested: bool,
    _now: Instant,
) -> Vec<egui::ViewportCommand> {
    Vec::new()
}

pub fn show(_ui: &mut egui::Ui, _st: &mut AppState, _tx: &Sender<BackendRequest>, _now: Instant) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn closing_with_no_jobs_closes_immediately() {
        let mut q = QuitState::Idle;
        assert_eq!(q.on_close_requested(0), CloseDecision::Allow);
        assert_eq!(q, QuitState::Idle);
    }

    #[test]
    fn closing_with_running_jobs_asks_first() {
        let mut q = QuitState::Idle;
        assert_eq!(q.on_close_requested(2), CloseDecision::Hold);
        assert_eq!(q, QuitState::Confirming);
        // A second close request while the dialog is up keeps it up.
        assert_eq!(q.on_close_requested(2), CloseDecision::Hold);
        assert_eq!(q, QuitState::Confirming);
    }

    #[test]
    fn keep_running_returns_to_idle() {
        let mut q = QuitState::Confirming;
        q.keep_running();
        assert_eq!(q, QuitState::Idle);
    }

    #[test]
    fn cancelling_state_forces_close_after_three_seconds() {
        let start = t0();
        let mut q = QuitState::Idle;
        q.on_close_requested(1);
        q.quit_and_cancel(start);
        assert_eq!(q, QuitState::Cancelling { since: start });
        assert!(!q.tick(start + Duration::from_millis(2900), 1));
        assert!(q.tick(start + CANCEL_WAIT, 1));
        assert_eq!(q, QuitState::Closing);
        // The close we asked for must not be held back again.
        assert_eq!(q.on_close_requested(1), CloseDecision::Allow);
        assert!(!q.tick(start + Duration::from_secs(9), 1), "only once");
    }

    #[test]
    fn cancelling_closes_as_soon_as_nothing_is_unsettled() {
        let start = t0();
        let mut q = QuitState::Cancelling { since: start };
        assert!(!q.tick(start + Duration::from_millis(10), 2));
        assert!(q.tick(start + Duration::from_millis(20), 0));
    }

    #[test]
    fn a_close_request_while_cancelling_keeps_waiting() {
        let start = t0();
        let mut q = QuitState::Cancelling { since: start };
        assert_eq!(q.on_close_requested(1), CloseDecision::Hold);
        assert_eq!(q, QuitState::Cancelling { since: start });
    }
}
