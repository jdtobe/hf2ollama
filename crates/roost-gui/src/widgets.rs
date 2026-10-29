//! Custom-drawn widgets ported from the approved egui spike.

use crate::theme::{mono, semibold, tokens};
use eframe::egui::{
    self, vec2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, StrokeKind,
};
use roost_core::fit::Fit;

/// Side of the clear (✕) button inside a search field.
const CLEAR_SIZE: f32 = 18.0;

/// What `search_field` painted, for tests and callers that need to focus it.
pub struct SearchField {
    pub response: egui::Response,
    /// Where the clear (✕) button sits; `None` while the field is empty.
    pub clear: Option<egui::Rect>,
}

/// A one-line search box as wide as the space left, with a clear (✕) button
/// that appears once there is text. Every search box in the app is this one.
pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str) -> SearchField {
    let t = tokens(ui);
    let has_text = !text.is_empty();
    let response = ui.add_sized(
        vec2(ui.available_width(), 32.0),
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .margin(Margin {
                left: 10,
                right: if has_text { 10 + CLEAR_SIZE as i8 } else { 10 },
                top: 7,
                bottom: 7,
            }),
    );
    let mut clear = None;
    if has_text {
        let rect = egui::Rect::from_center_size(
            egui::pos2(
                response.rect.right() - 6.0 - CLEAR_SIZE / 2.0,
                response.rect.center().y,
            ),
            vec2(CLEAR_SIZE, CLEAR_SIZE),
        );
        clear = Some(rect);
        let hit = ui
            .interact(rect, response.id.with("clear"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Clear search");
        let fg = if hit.hovered() { t.fg } else { t.fg3 };
        if hit.hovered() {
            ui.painter()
                .circle_filled(rect.center(), CLEAR_SIZE / 2.0, t.surface2);
        }
        let c = rect.center();
        let arm = 3.5;
        let stroke = Stroke::new(1.5, fg);
        ui.painter()
            .line_segment([c + vec2(-arm, -arm), c + vec2(arm, arm)], stroke);
        ui.painter()
            .line_segment([c + vec2(-arm, arm), c + vec2(arm, -arm)], stroke);
        if hit.clicked() {
            text.clear();
            response.request_focus();
        }
    }
    SearchField { response, clear }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillKind {
    Ok,
    Warn,
    Neutral,
}

/// Only ok and warn colour tokens exist, so every negative outcome uses warn.
pub fn fit_pill_kind(fit: Fit) -> PillKind {
    match fit {
        Fit::Fits => PillKind::Ok,
        Fit::Tight | Fit::PartialOffload | Fit::TooLarge => PillKind::Warn,
        Fit::Unknown => PillKind::Neutral,
    }
}

pub fn fit_pill_label(fit: Fit) -> &'static str {
    match fit {
        Fit::Fits => "Fits in VRAM",
        Fit::Tight => "Tight fit",
        Fit::PartialOffload => "Partial CPU offload",
        Fit::TooLarge => "Too large",
        Fit::Unknown => "Fit unknown",
    }
}

/// Single-row text galley that ends in an ellipsis instead of wrapping.
pub fn fit_text(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// Shortens `text` in the middle with an ellipsis until it measures at most
/// `max_width` in `font`; the tail keeps the larger share.
pub fn ellipsize_middle(ui: &egui::Ui, text: &str, font: FontId, max_width: f32) -> String {
    let width = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| {
        let head = keep / 3;
        let tail = keep - head;
        let mut out: String = chars[..head].iter().collect();
        out.push('…');
        out.extend(&chars[chars.len() - tail..]);
        out
    };
    // Largest `keep` that still fits; width grows with it.
    let (mut lo, mut hi) = (0, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(&build(mid)) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    build(lo)
}

/// Foreground and background colours of a fit pill.
pub fn pill_colors(t: &crate::theme::Tokens, kind: PillKind) -> (Color32, Color32) {
    match kind {
        PillKind::Ok => (t.ok, t.ok_soft),
        PillKind::Warn => (t.warn, t.warn_soft),
        PillKind::Neutral => (t.fg2, t.surface2),
    }
}

/// Uppercase section caption.
pub fn caption(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = tokens(ui);
    ui.label(
        RichText::new(text.to_uppercase())
            .font(semibold(11.0))
            .color(t.fg3),
    )
}

/// Small monospace chip.
pub fn tag(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(4.0)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).font(mono(11.0)).color(fg))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
}

/// Accent-filled primary action.
pub fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let t = tokens(ui);
    let b = egui::Button::new(RichText::new(text).font(semibold(14.0)).color(if enabled {
        t.accent_fg
    } else {
        t.fg3
    }))
    .fill(if enabled { t.accent } else { t.line })
    .stroke(Stroke::NONE)
    .min_size(vec2(0.0, 32.0));
    ui.add_enabled(enabled, b)
}

/// The mockup's switch: 38 x 22, accent track when on. Flips `on` when
/// clicked (or activated from the keyboard) and marks the response changed.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let t = tokens(ui);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let value = *on;
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, ui.is_enabled(), value, ""));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, 11.0, if value { t.accent } else { t.line });
        let x = rect.left() + if value { 27.0 } else { 11.0 };
        p.circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
        if resp.has_focus() {
            p.rect_stroke(
                rect.expand(2.0),
                13.0,
                Stroke::new(2.0, t.accent),
                StrokeKind::Outside,
            );
        }
    }
    resp
}

/// Compact secondary button; returns true when clicked.
pub fn small_button(ui: &mut egui::Ui, text: &str) -> bool {
    small_button_response(ui, text).clicked()
}

/// `small_button` for callers that need the painted rectangle.
pub fn small_button_response(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = tokens(ui);
    ui.add(egui::Button::new(RichText::new(text).size(12.0).color(t.fg2)).min_size(vec2(0.0, 24.0)))
}

/// Like `segmented_nav`, also returning the rectangle it occupies.
pub fn segmented_nav_rect(
    ui: &mut egui::Ui,
    labels: &[&str],
    badges: &[Option<String>],
    selected: usize,
) -> (Option<usize>, egui::Rect) {
    let t = tokens(ui);
    let mut clicked = None;
    let frame = egui::Frame::new()
        .fill(t.surface2)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(8.0)
        .inner_margin(2.0)
        .show(ui, |ui| {
            // Always one row, whatever layout the caller is in.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, name) in labels.iter().enumerate() {
                    let on = i == selected;
                    let font = if on {
                        semibold(14.0)
                    } else {
                        FontId::proportional(14.0)
                    };
                    let color = if on { t.fg } else { t.fg2 };
                    let galley = ui.painter().layout_no_wrap((*name).into(), font, color);
                    // Size by the bolder face so selecting a segment never resizes it.
                    let widest = ui
                        .painter()
                        .layout_no_wrap((*name).into(), semibold(14.0), color)
                        .size()
                        .x
                        .max(galley.size().x);
                    let badge = badges
                        .get(i)
                        .and_then(|b| b.as_deref())
                        .map(|b| ui.painter().layout_no_wrap(b.into(), mono(11.0), t.fg2));
                    let badge_w = badge.as_ref().map_or(0.0, |g| g.size().x + 14.0);
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(widest + 26.0 + badge_w, 28.0), Sense::click());
                    if on {
                        ui.painter().rect_filled(rect, 6.0, t.surface);
                        ui.painter().rect_stroke(
                            rect,
                            6.0,
                            Stroke::new(1.0, t.line),
                            StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 6.0, t.surface);
                    }
                    let text_pos = egui::pos2(
                        rect.min.x + 13.0 + (widest - galley.size().x) / 2.0,
                        rect.center().y - galley.size().y / 2.0,
                    );
                    let text_right = text_pos.x + galley.size().x;
                    ui.painter().galley(text_pos, galley, color);
                    if let Some(g) = badge {
                        let r = egui::Rect::from_min_size(
                            egui::pos2(text_right + 6.0, rect.center().y - 9.0),
                            vec2(g.size().x + 10.0, 18.0),
                        );
                        ui.painter().rect_filled(r, CornerRadius::same(9), t.line);
                        ui.painter().galley(r.center() - g.size() / 2.0, g, t.fg2);
                    }
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    (clicked, frame.response.rect)
}

/// One full-width band of a detail page. The bands stack: the header
/// (tinted with the theme's accent), the details (the second surface) and
/// the description (the page surface), so every theme keeps its own look.
pub fn band<R>(
    ui: &mut egui::Ui,
    fill: Color32,
    line: Option<Color32>,
    min_height: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, egui::Rect) {
    let out = egui::Frame::new()
        .fill(fill)
        .inner_margin(Margin::symmetric(24, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(min_height);
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui)
        });
    let rect = out.response.rect;
    if let Some(c) = line {
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, c));
    }
    (out.inner, rect)
}

/// A row of choices where the chosen one is filled with the accent.
/// Returns the index clicked, if any, and records each segment's rectangle.
pub fn segmented(
    ui: &mut egui::Ui,
    t: &crate::theme_tokens::Tokens,
    labels: &[(&str, &str)],
    chosen: usize,
    rects: &mut Vec<egui::Rect>,
) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, (label, tip)) in labels.iter().enumerate() {
            let on = i == chosen;
            let galley = ui.painter().layout_no_wrap(
                (*label).to_string(),
                egui::FontId::proportional(12.5),
                if on { t.accent_fg } else { t.fg2 },
            );
            let size = egui::vec2(galley.size().x + 24.0, 26.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(*tip);
            let radius = egui::CornerRadius {
                nw: if i == 0 { 7 } else { 0 },
                sw: if i == 0 { 7 } else { 0 },
                ne: if i + 1 == labels.len() { 7 } else { 0 },
                se: if i + 1 == labels.len() { 7 } else { 0 },
            };
            let fill = if on {
                t.accent
            } else if resp.hovered() {
                t.line
            } else {
                t.surface
            };
            ui.painter().rect(
                rect,
                radius,
                fill,
                egui::Stroke::new(1.0, if on { t.accent } else { t.line }),
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                rect.center() - galley.size() / 2.0,
                galley,
                if on { t.accent_fg } else { t.fg2 },
            );
            rects.push(rect);
            if resp.clicked() && !on {
                clicked = Some(i);
            }
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_pill_kind_maps_every_fit_variant() {
        let cases = [
            (Fit::Fits, "Fits in VRAM", PillKind::Ok),
            (Fit::Tight, "Tight fit", PillKind::Warn),
            (Fit::PartialOffload, "Partial CPU offload", PillKind::Warn),
            (Fit::TooLarge, "Too large", PillKind::Warn),
            (Fit::Unknown, "Fit unknown", PillKind::Neutral),
        ];
        for (fit, label, kind) in cases {
            assert_eq!(fit_pill_kind(fit), kind, "{fit:?}");
            assert_eq!(fit_pill_label(fit), label, "{fit:?}");
        }
    }
}
