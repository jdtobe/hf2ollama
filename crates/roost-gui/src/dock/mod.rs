//! The downloads dock: one row per job with progress, controls and a log.

pub mod jobs;

#[cfg(test)]
mod layout_tests;

use crate::app::bad;
use crate::browse::results::format_size;
use crate::theme::{mono, tokens, Tokens};
use crate::widgets::{ellipsize_middle, fit_text};
use eframe::egui::{self, vec2, Align, Layout, Margin, RichText, Sense, Stroke};
use jobs::{JobRow, JobsState, RowStatus};
use roost_core::events::{JobId, Stage};

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
    /// Progress bars and stage pills painted this frame.
    pub bars: Vec<egui::Rect>,
    pub pills: Vec<egui::Rect>,
    /// The floating log drawer, when open.
    pub log_panel: Option<egui::Rect>,
    /// The scrolling log text area inside the drawer.
    pub log_viewport: Option<egui::Rect>,
    /// The drawer's close button.
    pub log_close: Option<egui::Rect>,
    /// The drawer's Copy button.
    pub log_copy: Option<egui::Rect>,
}

/// View state of the dock (which logs are expanded).
#[derive(Debug, Default)]
pub struct DockUi {
    /// The one job whose log is open in the drawer.
    pub open_log: Option<JobId>,
    /// Panel height of the last frame; a change needs one more frame so the
    /// panels above it (laid out from this frame's size) catch up.
    last_height: f32,
    #[cfg(test)]
    pub probe: DockProbe,
}

/// A job is two lines: name, size, pill and buttons; then bar and numbers.
const LINE1_H: f32 = 24.0;
const LINE2_H: f32 = 14.0;
const LINE_GAP: f32 = 4.0;
const ROW_H: f32 = LINE1_H + LINE_GAP + LINE2_H;
/// Every job's bar is this wide, so the bars line up down the dock.
const BAR_W: f32 = 300.0;
/// The bar never gets narrower than this, whatever the window.
const BAR_MIN_W: f32 = 120.0;
/// Room the numbers keep before the bar starts giving way.
const DETAIL_MIN_W: f32 = 330.0;
const PILL_H: f32 = 18.0;
const COL_GAP: f32 = 14.0;
const FRAME_V: f32 = 10.0;
const HEADER_H: f32 = 24.0;
/// The log drawer is exactly this many lines tall.
const LOG_LINES: usize = 5;
const LOG_HEADER_H: f32 = 22.0;
const LOG_PAD: f32 = 8.0;
/// Gap between the drawer and the top of the dock.
const LOG_GAP: f32 = 6.0;
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
        st.probe.bars.clear();
        st.probe.pills.clear();
        st.probe.viewport = None;
        st.probe.log_panel = None;
        st.probe.log_viewport = None;
        st.probe.log_close = None;
        st.probe.log_copy = None;
    }
    let resp = egui::Panel::bottom("dock")
        .frame(crate::app::panel_frame(
            t.surface2,
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
                ui.spacing_mut().item_spacing.y = 2.0;
                header(ui, &t, jobs, st, &mut actions);
                rows(ui, cap, &t, jobs, st, &mut actions);
            }
        });
    let height = resp.response.rect.height();
    if height_changed(st.last_height, height) {
        st.last_height = height;
        ui.ctx().request_repaint();
    }
    #[cfg(test)]
    {
        st.probe.panel = Some(resp.response.rect);
    }
    if st
        .open_log
        .is_some_and(|id| !jobs.rows().iter().any(|r| r.id == id))
    {
        st.open_log = None;
    }
    if let Some(id) = st.open_log {
        if let Some(row) = jobs.rows().iter().find(|r| r.id == id) {
            log_drawer(ui, resp.response.rect, &t, row, st);
        }
    }
    actions
}

/// A full-width, five-line log viewer floating just above the dock. It is a
/// drawer rather than part of the row so the minimum window keeps both the
/// five lines and Browse's protected height.
fn log_drawer(ui: &egui::Ui, dock: egui::Rect, t: &Tokens, row: &JobRow, st: &mut DockUi) {
    let line_h = ui
        .painter()
        .layout_no_wrap("Ag".into(), mono(11.5), t.fg2)
        .size()
        .y;
    let drawer_h = 2.0 * LOG_PAD + LOG_HEADER_H + 4.0 + LOG_LINES as f32 * line_h;
    let pos = egui::pos2(dock.left(), (dock.top() - LOG_GAP - drawer_h).max(0.0));
    let width = dock.width();
    let inner_w = (width - 2.0 * LOG_PAD - 2.0).max(160.0);
    let mut close = false;
    let area = egui::Area::new(egui::Id::unique("dock_log_drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            let frame = egui::Frame::new()
                .fill(t.surface2)
                .stroke(Stroke::new(1.0, t.line))
                .corner_radius(8.0)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(40),
                })
                .inner_margin(Margin::symmetric(LOG_PAD as i8, LOG_PAD as i8))
                .show(ui, |ui| {
                    ui.set_width(inner_w);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.horizontal(|ui| {
                        ui.set_min_height(LOG_HEADER_H);
                        let name = if row.name.is_empty() {
                            "Starting…"
                        } else {
                            &row.name
                        };
                        ui.label(RichText::new(name).font(mono(12.0)).color(t.fg));
                        ui.label(
                            RichText::new(row.status_text())
                                .font(mono(11.0))
                                .color(t.fg3),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                            let resp = resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Close log");
                            let fg = if resp.hovered() { t.fg } else { t.fg3 };
                            if resp.hovered() {
                                ui.painter().circle_filled(rect.center(), 10.0, t.surface);
                            }
                            let c = rect.center();
                            let arm = 3.5;
                            let stroke = Stroke::new(1.5, fg);
                            ui.painter()
                                .line_segment([c + vec2(-arm, -arm), c + vec2(arm, arm)], stroke);
                            ui.painter()
                                .line_segment([c + vec2(-arm, arm), c + vec2(arm, -arm)], stroke);
                            #[cfg(test)]
                            {
                                st.probe.log_close = Some(rect);
                            }
                            if resp.clicked() {
                                close = true;
                            }
                            ui.spacing_mut().button_padding = vec2(8.0, 1.0);
                            let copy = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Copy").size(11.5).color(t.fg2),
                                    )
                                    .min_size(vec2(0.0, 20.0)),
                                )
                                .on_hover_text("Copy the whole log");
                            #[cfg(test)]
                            {
                                st.probe.log_copy = Some(copy.rect);
                            }
                            if copy.clicked() {
                                ui.ctx().copy_text(row.log.join("\n"));
                            }
                        });
                    });
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                    let text_h = LOG_LINES as f32 * line_h;
                    if row.log.is_empty() {
                        ui.allocate_ui(vec2(inner_w, text_h), |ui| {
                            ui.label(RichText::new("No log lines yet.").size(11.5).color(t.fg3));
                        });
                    } else {
                        let out = egui::ScrollArea::vertical()
                            .id_salt(("dock_log", row.id.0))
                            .max_height(text_h)
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show_rows(ui, line_h, row.log.len(), |ui, range| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for line in &row.log[range] {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(line).font(mono(11.5)).color(t.fg2),
                                        )
                                        .selectable(true)
                                        .truncate(),
                                    );
                                }
                            });
                        #[cfg(test)]
                        {
                            st.probe.log_viewport = Some(out.inner_rect);
                        }
                        #[cfg(not(test))]
                        let _ = out;
                    }
                });
            frame.response.rect
        });
    #[cfg(test)]
    {
        st.probe.log_panel = Some(area.inner);
    }
    #[cfg(not(test))]
    let _ = area;
    if close {
        st.open_log = None;
    }
}

/// True when the dock size moved enough that another frame is needed.
fn height_changed(prev: f32, now: f32) -> bool {
    (now - prev).abs() > 0.5
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
    let line1 = egui::Rect::from_min_size(rect.min, vec2(w, LINE1_H));

    // Buttons, rightmost first.
    let open = st.open_log == Some(row.id);
    let mut bui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(line1)
            .layout(Layout::right_to_left(Align::Center)),
    );
    bui.spacing_mut().item_spacing.x = 6.0;
    bui.spacing_mut().button_padding = vec2(12.0, 4.0);
    let log_fill = open.then_some(t.accent_soft);
    if row_button(&mut bui, "Log", t.fg2, log_fill, st).clicked() {
        st.open_log = if open { None } else { Some(row.id) };
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

    // Line 1: name, size and stage pill in the space left of the buttons.
    let region = (buttons_left - COL_GAP - rect.left()).max(0.0);
    let line1_cy = rect.top() + LINE1_H / 2.0;
    let size_g = (row.total > 0).then(|| {
        ui.painter().layout_no_wrap(
            format_size(row.total),
            egui::FontId::proportional(12.0),
            t.fg3,
        )
    });
    let size_w = size_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let (pill_fg, pill_bg) = match row.status {
        RowStatus::Finished => (t.ok, t.accent_soft),
        RowStatus::Failed => (bad(dark), bad(dark).gamma_multiply(0.16)),
        RowStatus::Paused | RowStatus::Cancelled => (t.fg2, t.line),
        RowStatus::Queued => (t.fg2, t.line),
        RowStatus::Running if row.stage == Stage::Downloading => (t.accent, t.accent_soft),
        RowStatus::Running => (t.warn, t.warn.gamma_multiply(0.18)),
    };
    let pill_g = ui
        .painter()
        .layout_no_wrap(row.pill_text().into(), mono(11.0), pill_fg);
    let pill_w = pill_g.size().x + 16.0;
    let name_w = (region - size_w - pill_w - 8.0).max(60.0);
    let name = if row.name.is_empty() {
        "Starting…".to_string()
    } else {
        // Cut in the middle so a model's tag, such as `:32b`, stays visible.
        ellipsize_middle(ui, &row.name, mono(12.5), name_w)
    };
    let name_g = ui.painter().layout_no_wrap(name, mono(12.5), t.fg);
    let name_end = rect.left() + name_g.size().x;
    ui.painter().galley(
        egui::pos2(rect.left(), line1_cy - name_g.size().y / 2.0),
        name_g,
        t.fg,
    );
    let mut x = name_end + 8.0;
    if let Some(g) = size_g {
        let w = g.size().x;
        ui.painter()
            .galley(egui::pos2(x, line1_cy - g.size().y / 2.0), g, t.fg3);
        x += w + 8.0;
    }
    let pill =
        egui::Rect::from_min_size(egui::pos2(x, line1_cy - PILL_H / 2.0), vec2(pill_w, PILL_H));
    ui.painter().rect_filled(pill, PILL_H / 2.0, pill_bg);
    ui.painter()
        .galley(pill.center() - pill_g.size() / 2.0, pill_g, pill_fg);
    #[cfg(test)]
    st.probe.pills.push(pill);

    // Line 2: a bar of the same width for every job, then the numbers.
    let line2_cy = rect.top() + LINE1_H + LINE_GAP + LINE2_H / 2.0;
    let bar_w = BAR_W.min((rect.width() - DETAIL_MIN_W - COL_GAP).max(BAR_MIN_W));
    let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), line2_cy - 3.0), vec2(bar_w, 6.0));
    let (fill, text_color) = match row.status {
        RowStatus::Finished => (t.ok, t.fg2),
        RowStatus::Failed => (bad(dark), bad(dark)),
        RowStatus::Paused | RowStatus::Cancelled => (t.fg3, t.fg2),
        RowStatus::Queued => (t.accent, t.fg2),
        RowStatus::Running if row.stage == Stage::Downloading => (t.accent, t.fg2),
        RowStatus::Running => (t.warn, t.fg2),
    };
    ui.painter().rect_filled(bar, 3.0, t.line);
    let mut done = bar;
    done.set_width(bar.width() * row.fraction());
    ui.painter().rect_filled(done, 3.0, fill);
    #[cfg(test)]
    st.probe.bars.push(bar);

    let detail_x = bar.right() + COL_GAP;
    let detail_g = fit_text(
        ui,
        &row.detail_text(),
        mono(12.0),
        text_color,
        rect.right() - detail_x,
    );
    ui.painter().galley(
        egui::pos2(detail_x, line2_cy - detail_g.size().y / 2.0),
        detail_g,
        text_color,
    );

    if let Some(err) = &row.error {
        ui.add(
            egui::Label::new(RichText::new(err).size(12.0).color(bad(dark)))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_change_needs_another_frame_only_when_it_moved() {
        assert!(height_changed(38.0, 290.0));
        assert!(height_changed(290.0, 38.0));
        assert!(!height_changed(290.0, 290.2));
        assert!(!height_changed(38.0, 38.0));
    }
}
