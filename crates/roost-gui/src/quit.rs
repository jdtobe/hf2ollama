//! Closing the window while downloads are running: ask first, then cancel
//! every job, wait a bounded time for them to settle and close.

use crate::app::bad;
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{semibold, tokens};
use eframe::egui::{self, vec2, Color32, RichText, Stroke};
use roost_core::events::Command;
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
    /// A close request arrived with `unsettled` jobs queued, running or paused.
    pub fn on_close_requested(&mut self, unsettled: usize) -> CloseDecision {
        match *self {
            QuitState::Closing => CloseDecision::Allow,
            QuitState::Cancelling { .. } => CloseDecision::Hold,
            QuitState::Idle | QuitState::Confirming if unsettled == 0 => {
                *self = QuitState::Idle;
                CloseDecision::Allow
            }
            QuitState::Idle | QuitState::Confirming => {
                *self = QuitState::Confirming;
                CloseDecision::Hold
            }
        }
    }

    pub fn keep_running(&mut self) {
        if *self == QuitState::Confirming {
            *self = QuitState::Idle;
        }
    }

    pub fn quit_and_cancel(&mut self, now: Instant) {
        if *self == QuitState::Confirming {
            *self = QuitState::Cancelling { since: now };
        }
    }

    /// True once, when the window should be closed now: every job settled, or
    /// the wait ran out.
    pub fn tick(&mut self, now: Instant, unsettled: usize) -> bool {
        match *self {
            QuitState::Cancelling { since }
                if unsettled == 0 || now.saturating_duration_since(since) >= CANCEL_WAIT =>
            {
                *self = QuitState::Closing;
                true
            }
            _ => false,
        }
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

/// What the window should do about a close request and about a quit that
/// is waiting for its jobs to cancel.
pub fn process_close(
    st: &mut AppState,
    close_requested: bool,
    now: Instant,
) -> Vec<egui::ViewportCommand> {
    let unsettled = st.jobs.unsettled_ids().len();
    // Chat and agent runs also make quitting a decision. They do not hold up
    // the close once chosen: the shutdown at exit ends them and saves what
    // they have so far.
    let pending = unsettled + running_chats(st);
    let mut cmds = Vec::new();
    if close_requested && st.quit.on_close_requested(pending) == CloseDecision::Hold {
        cmds.push(egui::ViewportCommand::CancelClose);
    }
    // Work that ended by itself while the dialog was up: nothing to ask.
    if st.quit == QuitState::Confirming && pending == 0 {
        st.quit = QuitState::Idle;
    }
    if st.quit.tick(now, unsettled) {
        cmds.push(egui::ViewportCommand::Close);
    }
    cmds
}

/// Chats and agents with a run in progress, shown or not.
fn running_chats(st: &AppState) -> usize {
    use roost_core::session::SessionMode;
    st.chat.active_runs_for(SessionMode::Chat) + st.chat.active_runs_for(SessionMode::Agent)
}

fn running_text(downloads: usize, runs: usize) -> String {
    let mut parts = Vec::new();
    if downloads > 0 {
        parts.push(match downloads {
            1 => "1 download".to_string(),
            n => format!("{n} downloads"),
        });
    }
    if runs > 0 {
        parts.push(match runs {
            1 => "1 chat or agent run".to_string(),
            n => format!("{n} chat or agent runs"),
        });
    }
    let verb = if downloads + runs == 1 { "is" } else { "are" };
    let mut text = format!("{} {verb} still running. Quit anyway?", parts.join(" and "));
    if downloads > 0 {
        text.push_str(" Running downloads will be cancelled and their files removed.");
    }
    if runs > 0 {
        text.push_str(
            " Running chats and agents will be stopped; what they have written so far is kept.",
        );
    }
    text
}

/// Draws the confirmation while a quit is pending.
pub fn show(ui: &mut egui::Ui, st: &mut AppState, tx: &Sender<BackendRequest>, now: Instant) {
    #[cfg(test)]
    {
        st.quit_probe = QuitProbe::default();
    }
    let cancelling = matches!(st.quit, QuitState::Cancelling { .. });
    if st.quit != QuitState::Confirming && !cancelling {
        return;
    }
    let t = tokens(ui);
    let dark = ui.visuals().dark_mode;
    let width = (ui.ctx().content_rect().width() - 64.0).clamp(200.0, 440.0);
    let unsettled = st.jobs.unsettled_ids();
    let runs = running_chats(st);
    let frame = egui::Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(10.0)
        .inner_margin(20.0);
    let held = crate::confirm::activation_key_held(ui);
    let (mut keep, mut quit) = (false, false);
    let _modal = egui::Modal::new(egui::Id::NULL.with("quit-confirm"))
        .frame(frame)
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
            ui.label(
                RichText::new("Quit Roost?")
                    .font(semibold(16.0))
                    .color(t.fg),
            );
            let body = if cancelling {
                if unsettled.is_empty() {
                    "Stopping…".to_string()
                } else {
                    "Cancelling downloads…".to_string()
                }
            } else {
                running_text(unsettled.len(), runs)
            };
            ui.add(egui::Label::new(RichText::new(body).size(13.0).color(t.fg2)).wrap());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let keep_r = ui.add_enabled(
                    !cancelling,
                    egui::Button::new(RichText::new("Keep running").size(13.0).color(t.accent_fg))
                        .fill(t.accent)
                        .min_size(vec2(0.0, 32.0)),
                );
                if !cancelling && ui.memory(|m| m.focused()).is_none() {
                    keep_r.request_focus();
                }
                let quit_r = ui.add_enabled(
                    !cancelling,
                    egui::Button::new(
                        RichText::new("Quit and cancel")
                            .size(13.0)
                            .color(Color32::WHITE),
                    )
                    .fill(bad(dark))
                    .min_size(vec2(0.0, 32.0)),
                );
                keep = keep_r.clicked();
                // A held Enter or Space repeats `clicked()` on the focused button.
                quit = quit_r.clicked() && !held;
                #[cfg(test)]
                {
                    st.quit_probe.keep = Some(keep_r.rect);
                    st.quit_probe.quit = Some(quit_r.rect);
                    st.quit_probe.quit_id = Some(quit_r.id);
                    st.quit_probe.keep_has_focus = keep_r.has_focus();
                }
            });
        });
    #[cfg(test)]
    {
        st.quit_probe.dialog = Some(_modal.response.rect);
    }
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if !cancelling && (keep || escape) {
        st.quit.keep_running();
    } else if !cancelling && quit {
        for id in unsettled {
            let _ = tx.send(BackendRequest::Core(Command::Cancel(id)));
        }
        st.quit.quit_and_cancel(now);
    }
}

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
