//! The left column shared by the Chat, Work and Models tabs: one width on
//! every tab, draggable, and remembered between runs.

use crate::app::AppState;
use eframe::egui;

/// Narrowest the sidebar may be dragged.
pub const MIN_W: f32 = 280.0;
/// Widest it may be dragged.
pub const MAX_W: f32 = 520.0;
/// Room the content beside the sidebar always keeps, so a narrow window
/// limits the sidebar instead of squeezing the page.
pub const MIN_CONTENT: f32 = 540.0;

/// The widest the sidebar may be in a window `avail` points wide.
pub fn max_width(avail: f32) -> f32 {
    (avail - MIN_CONTENT).clamp(MIN_W, MAX_W)
}

/// Draws the sidebar with `frame` around `add` and a divider of colour `edge`
/// on its right, starting at `wanted` wide (the saved width). The panel has one id for all tabs, so a width chosen
/// on one is the width on the others. Returns what `add` returned and the
/// width drawn; hand that to `finish`.
pub fn show<R>(
    ui: &mut egui::Ui,
    wanted: f32,
    frame: egui::Frame,
    edge: egui::Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, f32) {
    let max = max_width(ui.available_width());
    let panel = egui::Panel::left("sidebar")
        .resizable(true)
        .default_size(wanted.clamp(MIN_W, max))
        .size_range(MIN_W..=max)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, add);
    // One divider on the right edge, in a darker tint than the sidebar.
    let r = panel.response.rect;
    ui.painter()
        .vline(r.right() - 0.5, r.y_range(), egui::Stroke::new(1.0, edge));
    (panel.inner, r.width())
}

/// Height of the tinted band across the top of the sidebar, and of the page
/// header beside it, so the two read as one strip.
pub const BAND_H: f32 = 56.0;

/// How far the search field sits below the band's own padding, so it is
/// centred in the band.
pub const SEARCH_DROP: f32 = (BAND_H - 32.0) / 2.0 - 12.0;

/// Call after `show` with the width it returned.
pub fn finish(ui: &egui::Ui, app: &mut AppState, width: f32) {
    remember(app, width, ui.input(|i| i.pointer.any_down()));
}

/// Notes a drag in progress and, when the pointer is let go after one, keeps
/// the width. A window that is merely made smaller changes the width without
/// the pointer being down, so it never overwrites the chosen width.
fn remember(app: &mut AppState, width: f32, pointer_down: bool) {
    if pointer_down && (width - app.sidebar_seen).abs() > 0.5 && app.sidebar_seen > 0.0 {
        app.sidebar_moved = true;
    }
    app.sidebar_seen = width;
    if !pointer_down && app.sidebar_moved {
        app.sidebar_moved = false;
        if (width - app.settings.sidebar_width).abs() > 0.5 {
            app.settings.sidebar_width = width;
            app.sidebar_dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_window_limits_the_sidebar_and_a_wide_one_caps_it() {
        assert_eq!(max_width(900.0), 360.0);
        assert_eq!(max_width(700.0), MIN_W);
        assert_eq!(max_width(3000.0), MAX_W);
    }

    #[test]
    fn only_a_drag_changes_the_remembered_width() {
        let mut app = AppState::new();
        app.settings.sidebar_width = 340.0;
        // The window shrinks: the width follows but nothing is remembered.
        remember(&mut app, 340.0, false);
        remember(&mut app, 300.0, false);
        assert_eq!(app.settings.sidebar_width, 340.0);
        assert!(!app.sidebar_dirty);
        // A drag: the width changes while the pointer is down, then it is let go.
        remember(&mut app, 300.0, true);
        remember(&mut app, 380.0, true);
        remember(&mut app, 380.0, false);
        assert_eq!(app.settings.sidebar_width, 380.0);
        assert!(app.sidebar_dirty);
    }
}
