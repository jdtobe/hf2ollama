//! The Models tab: installed models and Hugging Face in one view, like an
//! extensions list. One search box, one list with two sections, one detail
//! pane. See `docs/superpowers/specs/2026-10-29-roost-models-tab-design.md`.

mod detail;
mod list;

#[cfg(test)]
mod layout_tests;

use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::widgets::ellipsize_middle;
use eframe::egui::{self, vec2, Color32, FontId, Pos2, Rect};
use std::time::Instant;

/// Draws the whole tab and collects the requests it wants sent.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let now = Instant::now();
    #[cfg(test)]
    {
        let p = &mut app.browse.probe;
        p.installed_rows.clear();
        p.result_rows.clear();
        p.list = None;
        p.detail = None;
        p.filters_chip = None;
        p.chat_button = None;
        p.work_button = None;
        p.delete_button = None;
        p.toast = None;
    }
    if let Some(q) = app.browse.tick(&app.query, now) {
        out.push(BackendRequest::Search(q));
    }
    if let Some(wait) = app.browse.debounce_remaining(now) {
        ui.ctx().request_repaint_after(wait);
    }
    if let Some(due) = app.installed.tick(now) {
        ui.ctx()
            .request_repaint_after(due.saturating_duration_since(now));
    }
    // The search box narrows the installed list too.
    if app.installed.filter_text != app.query {
        app.installed.filter_text = app.query.clone();
    }
    list::show(ui, app, out);
    detail::show(ui, app, out);
    toast(ui, app, now);
    if let Some(q) = app.browse.next_related_request() {
        out.push(BackendRequest::Search(q));
    }
    for id in app.browse.next_info_requests() {
        out.push(BackendRequest::ModelInfo(id));
    }
    for id in app.browse.next_card_requests() {
        out.push(BackendRequest::ModelCard(id));
    }
}

fn toast(ui: &mut egui::Ui, app: &mut AppState, now: Instant) {
    let panel = ui.max_rect();
    let t = crate::theme::tokens(ui);
    let dark = ui.visuals().dark_mode;
    let Some(toast) = app.installed.toast(now) else {
        return;
    };
    let is_error = toast.text.starts_with("Could not");
    let (fill, fg) = if is_error {
        (crate::app::bad(dark), Color32::WHITE)
    } else {
        (t.fg, t.surface)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(toast.text.clone(), FontId::proportional(13.0), fg);
    let max_w = panel.width() - 32.0;
    let galley = if galley.size().x > max_w {
        let shown = ellipsize_middle(ui, &toast.text, FontId::proportional(13.0), max_w - 28.0);
        ui.painter()
            .layout_no_wrap(shown, FontId::proportional(13.0), fg)
    } else {
        galley
    };
    let size = galley.size() + vec2(28.0, 18.0);
    let rect = Rect::from_center_size(
        Pos2::new(panel.center().x, panel.bottom() - 12.0 - size.y / 2.0),
        size,
    );
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().galley(
        Pos2::new(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    #[cfg(test)]
    {
        app.browse.probe.toast = Some(rect);
    }
}
