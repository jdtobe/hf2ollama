#![allow(dead_code, unused_imports)] // RED stub, removed in the GREEN commit
//! The downloads dock: one row per job with progress, controls and a log.

pub mod jobs;

#[cfg(test)]
mod layout_tests;

use crate::app::bad;
use crate::browse::results::format_size;
use crate::theme::{mono, tokens, Tokens};
use crate::widgets::fit_text;
use eframe::egui::{self, vec2, Align, Layout, Margin, RichText, Sense, Stroke};
use hf2ollama_core::events::JobId;
use jobs::JobsState;
use std::collections::HashSet;

/// What the user asked for; the app applies it (backend commands or local).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAction {
    Pause(JobId),
    Resume(JobId),
    Cancel(JobId),
    Dismiss(JobId),
    ClearFinished,
}

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct DockProbe {
    /// The whole dock panel.
    pub panel: Option<egui::Rect>,
    /// Visible area of the scrolling rows.
    pub viewport: Option<egui::Rect>,
    /// Every row button painted this frame (Pause, Resume, Cancel, Dismiss, Log).
    pub buttons: Vec<egui::Rect>,
    /// The `Clear finished` button in the header, when shown.
    pub clear_button: Option<egui::Rect>,
    /// Row content rectangles painted this frame.
    pub rows: Vec<egui::Rect>,
}

/// View state of the dock (which logs are expanded).
#[derive(Debug, Default)]
pub struct DockUi {
    pub open_logs: HashSet<JobId>,
    #[cfg(test)]
    pub probe: DockProbe,
}

const ROW_H: f32 = 30.0;
const STATUS_W: f32 = 150.0;
const COL_GAP: f32 = 14.0;
const LOG_MAX_H: f32 = 120.0;
const FRAME_V: f32 = 10.0;
const HEADER_H: f32 = 24.0;
/// The dock never takes more than this share of the window height.
const MAX_SHARE: f32 = 0.35;
/// Top bar and status bar together.
const CHROME_H: f32 = 86.0;
/// Height the Browse view keeps for itself (header, notices, scroll area
/// and import card at the minimum window size).
const BROWSE_MIN_H: f32 = 420.0;
/// Room for the header and a single row, whatever the window height.
const DOCK_FLOOR: f32 = 2.0 * FRAME_V + HEADER_H + ROW_H + 8.0;

/// Tallest the dock panel (margins included) may be in a window of `window_h`.
pub fn dock_cap(window_h: f32) -> f32 {
    let share = window_h * MAX_SHARE;
    let leaves_browse = window_h - CHROME_H - BROWSE_MIN_H;
    share.min(leaves_browse).max(DOCK_FLOOR)
}

#[cfg_attr(not(test), allow(unused_variables))]
pub fn show(ui: &mut egui::Ui, jobs: &JobsState, st: &mut DockUi) -> Vec<DockAction> {
    let t = tokens(ui);
    let mut actions = Vec::new();
    #[cfg(test)]
    {
        st.probe.buttons.clear();
        st.probe.rows.clear();
        st.probe.clear_button = None;
    }
    let resp = egui::Panel::bottom("dock")
        .frame(crate::app::panel_frame(
            t.surface2,
            t.line,
            Margin::symmetric(16, FRAME_V as i8),
        ))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
                if jobs.rows().is_empty() {
                    ui.label(
                        RichText::new("Nothing running. Pick a model above to start.")
                            .size(13.0)
                            .color(t.fg3),
                    );
                }
            });
            // Placeholder rows: unbounded, buttons left-aligned after the name.
            for row in jobs.rows() {
                let r = ui.horizontal(|ui| {
                    ui.label(&row.name);
                    let b = ui.button("Log");
                    #[cfg(test)]
                    st.probe.buttons.push(b.rect);
                    let _ = b;
                });
                #[cfg(test)]
                st.probe.rows.push(r.response.rect);
                let _ = r;
            }
            // Placeholder: the clear button sits below all the rows.
            if jobs.has_settled() {
                let b = ui.button("Clear finished");
                #[cfg(test)]
                {
                    st.probe.clear_button = Some(b.rect);
                }
                let _ = b;
            }
        });
    #[cfg(test)]
    {
        st.probe.panel = Some(resp.response.rect);
        st.probe.viewport =
            (!jobs.rows().is_empty()).then(|| resp.response.rect.shrink2(vec2(16.0, FRAME_V)));
    }
    let _ = resp;
    actions.clear();
    actions
}
