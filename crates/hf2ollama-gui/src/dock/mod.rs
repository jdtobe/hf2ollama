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
use jobs::{JobRow, JobsState, RowStatus};
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

pub fn show(ui: &mut egui::Ui, jobs: &JobsState, st: &mut DockUi) -> Vec<DockAction> {
    let t = tokens(ui);
    let cap = dock_cap(ui.ctx().content_rect().height());
    let mut actions = Vec::new();
    #[cfg(test)]
    {
        st.probe.buttons.clear();
        st.probe.clear_button = None;
        st.probe.rows.clear();
        st.probe.viewport = None;
    }
    let resp = egui::Panel::bottom("dock")
        .frame(crate::app::panel_frame(
            t.surface2,
            t.line,
            Margin::symmetric(16, FRAME_V as i8),
        ))
        .resizable(false)
        .max_size(cap)
        .show(ui, |ui| {
            // Let content grow to the cap; the scroll area bounds it.
            ui.set_max_height(cap - 2.0 * FRAME_V);
            if jobs.rows().is_empty() {
                idle_line(ui, &t, jobs);
            } else {
                header(ui, &t, jobs, st, &mut actions);
                rows(ui, cap, &t, jobs, st, &mut actions);
            }
        });
    #[cfg(test)]
    {
        st.probe.panel = Some(resp.response.rect);
    }
    #[cfg(not(test))]
    let _ = resp;
    actions
}

fn idle_line(ui: &mut egui::Ui, t: &Tokens, jobs: &JobsState) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
        ui.label(
            RichText::new("Nothing running. Pick a model above to start.")
                .size(13.0)
                .color(t.fg3),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(jobs.header_text()).size(12.0).color(t.fg3));
        });
    });
}

fn header(
    ui: &mut egui::Ui,
    t: &Tokens,
    jobs: &JobsState,
    st: &mut DockUi,
    actions: &mut Vec<DockAction>,
) {
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_H);
        ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if jobs.has_settled() {
                let r = clear_button(ui);
                #[cfg(test)]
                {
                    st.probe.clear_button = Some(r.rect);
                }
                if r.clicked() {
                    actions.push(DockAction::ClearFinished);
                }
            }
            ui.label(RichText::new(jobs.header_text()).size(12.0).color(t.fg3));
        });
    });
    let _ = st;
}

fn clear_button(ui: &mut egui::Ui) -> egui::Response {
    let t = tokens(ui);
    ui.spacing_mut().button_padding = vec2(12.0, 4.0);
    ui.add(
        egui::Button::new(RichText::new("Clear finished").size(12.0).color(t.fg2))
            .min_size(vec2(0.0, 24.0)),
    )
}

fn rows(
    ui: &mut egui::Ui,
    cap: f32,
    t: &Tokens,
    jobs: &JobsState,
    st: &mut DockUi,
    actions: &mut Vec<DockAction>,
) {
    // Everything the panel may use, less its margins, the header and the gap.
    let avail = (cap - 2.0 * FRAME_V - HEADER_H - ui.spacing().item_spacing.y).max(ROW_H);
    let out = egui::ScrollArea::vertical()
        .id_salt("dock_rows")
        .max_height(avail)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            // Keep the buttons clear of the floating scroll bar.
            let w = ui.available_width() - 10.0;
            for row in jobs.rows() {
                ui.allocate_ui(vec2(w, 0.0), |ui| row_ui(ui, t, row, st, actions));
            }
        });
    #[cfg(test)]
    {
        st.probe.viewport = Some(out.inner_rect);
    }
    #[cfg(not(test))]
    let _ = out;
}

fn row_button(
    ui: &mut egui::Ui,
    text: &str,
    color: egui::Color32,
    fill: Option<egui::Color32>,
    st: &mut DockUi,
) -> egui::Response {
    let mut b =
        egui::Button::new(RichText::new(text).size(12.0).color(color)).min_size(vec2(0.0, 24.0));
    if let Some(f) = fill {
        b = b.fill(f);
    }
    let r = ui.add(b);
    #[cfg(test)]
    st.probe.buttons.push(r.rect);
    let _ = st;
    r
}

fn row_ui(
    ui: &mut egui::Ui,
    t: &Tokens,
    row: &JobRow,
    st: &mut DockUi,
    actions: &mut Vec<DockAction>,
) {
    let dark = ui.visuals().dark_mode;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
    #[cfg(test)]
    st.probe.rows.push(rect);

    // Buttons, rightmost first.
    let open = st.open_logs.contains(&row.id);
    let mut bui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    bui.spacing_mut().item_spacing.x = 6.0;
    bui.spacing_mut().button_padding = vec2(12.0, 4.0);
    let log_fill = open.then_some(t.accent_soft);
    if row_button(&mut bui, "Log", t.fg2, log_fill, st).clicked() {
        if open {
            st.open_logs.remove(&row.id);
        } else {
            st.open_logs.insert(row.id);
        }
    }
    if row.status.is_settled() {
        if row_button(&mut bui, "Dismiss", t.fg2, None, st).clicked() {
            actions.push(DockAction::Dismiss(row.id));
        }
    } else {
        if row_button(&mut bui, "Cancel", bad(dark), None, st).clicked() {
            actions.push(DockAction::Cancel(row.id));
        }
        let toggle: Option<(&str, DockAction)> = match row.status {
            RowStatus::Paused => Some(("Resume", DockAction::Resume(row.id))),
            RowStatus::Running => Some(("Pause", DockAction::Pause(row.id))),
            _ => None,
        };
        if let Some((label, action)) = toggle {
            if row_button(&mut bui, label, t.fg2, None, st).clicked() {
                actions.push(action);
            }
        }
    }
    let buttons_left = bui.min_rect().left();

    // Name, progress bar and status in the space left of the buttons.
    let region = (buttons_left - COL_GAP - rect.left()).max(0.0);
    let bar_min = 40.0;
    let name_w = (region * 0.34)
        .clamp(120.0, 300.0)
        .min((region - bar_min - STATUS_W - 2.0 * COL_GAP).max(60.0));
    let bar_w = (region - name_w - STATUS_W - 2.0 * COL_GAP).max(bar_min);
    let cy = rect.center().y;

    let size_text = (row.total > 0).then(|| format_size(row.total));
    let size_g = size_text.map(|s| {
        ui.painter()
            .layout_no_wrap(s, egui::FontId::proportional(12.0), t.fg3)
    });
    let size_w = size_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let name = if row.name.is_empty() {
        "Starting…"
    } else {
        &row.name
    };
    let name_g = fit_text(ui, name, mono(12.5), t.fg, (name_w - size_w).max(30.0));
    let name_end = rect.left() + name_g.size().x;
    ui.painter().galley(
        egui::pos2(rect.left(), cy - name_g.size().y / 2.0),
        name_g,
        t.fg,
    );
    if let Some(g) = size_g {
        ui.painter()
            .galley(egui::pos2(name_end + 8.0, cy - g.size().y / 2.0), g, t.fg3);
    }

    let bar = egui::Rect::from_min_size(
        egui::pos2(rect.left() + name_w + COL_GAP, cy - 3.0),
        vec2(bar_w, 6.0),
    );
    let (fill, text_color) = match row.status {
        RowStatus::Finished => (t.ok, t.ok),
        RowStatus::Failed => (bad(dark), bad(dark)),
        RowStatus::Paused => (t.warn, t.warn),
        RowStatus::Cancelled => (t.fg3, t.fg3),
        RowStatus::Queued | RowStatus::Running => (t.accent, t.fg2),
    };
    ui.painter().rect_filled(bar, 3.0, t.line);
    let mut done = bar;
    done.set_width(bar.width() * row.fraction());
    ui.painter().rect_filled(done, 3.0, fill);

    let status_g = fit_text(ui, &row.status_text(), mono(12.0), text_color, STATUS_W);
    ui.painter().galley(
        egui::pos2(bar.right() + COL_GAP, cy - status_g.size().y / 2.0),
        status_g,
        text_color,
    );

    if let Some(err) = &row.error {
        ui.add(
            egui::Label::new(RichText::new(err).size(12.0).color(bad(dark)))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
    if open {
        log_view(ui, t, row);
    }
}

fn log_view(ui: &mut egui::Ui, t: &Tokens, row: &JobRow) {
    egui::Frame::new()
        .fill(t.bg)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(6.0)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if row.log.is_empty() {
                ui.label(RichText::new("No log lines yet.").size(11.5).color(t.fg3));
                return;
            }
            let line_h = ui
                .painter()
                .layout_no_wrap("Ag".into(), mono(11.5), t.fg2)
                .size()
                .y;
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt(("dock_log", row.id.0))
                .max_height(LOG_MAX_H)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show_rows(ui, line_h, row.log.len(), |ui, range| {
                    for line in &row.log[range] {
                        ui.add(
                            egui::Label::new(RichText::new(line).font(mono(11.5)).color(t.fg2))
                                .selectable(true)
                                .truncate(),
                        );
                    }
                });
        });
}
