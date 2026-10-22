//! Custom-drawn widgets ported from the approved egui spike.

use crate::theme::{mono, semibold, tokens};
use eframe::egui::{
    self, vec2, Color32, CornerRadius, FontId, Margin, RichText, Sense, Stroke, StrokeKind,
};
use hf2ollama_core::fit::Fit;

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

// TODO(task 16 Installed): used by the Installed table.
#[allow(dead_code)]
/// Thin horizontal quality bar; `frac` is clamped to 0..=1.
pub fn quality_bar(ui: &mut egui::Ui, frac: f32, width: f32) {
    let t = tokens(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, t.line);
    let mut fill = rect;
    fill.set_width(width * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, 3.0, t.accent);
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

/// Compact secondary button; returns true when clicked.
pub fn small_button(ui: &mut egui::Ui, text: &str) -> bool {
    let t = tokens(ui);
    ui.add(egui::Button::new(RichText::new(text).size(12.0).color(t.fg2)).min_size(vec2(0.0, 24.0)))
        .clicked()
}

/// Segmented control. `badges[i]`, when present, is drawn after label `i`.
/// Returns the clicked index, if any.
pub fn segmented_nav(
    ui: &mut egui::Ui,
    labels: &[&str],
    badges: &[Option<String>],
    selected: usize,
) -> Option<usize> {
    let t = tokens(ui);
    let mut clicked = None;
    egui::Frame::new()
        .fill(t.surface2)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(8.0)
        .inner_margin(2.0)
        .show(ui, |ui| {
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
                let badge = badges
                    .get(i)
                    .and_then(|b| b.as_deref())
                    .map(|b| ui.painter().layout_no_wrap(b.into(), mono(11.0), t.fg2));
                let badge_w = badge.as_ref().map_or(0.0, |g| g.size().x + 14.0);
                let (rect, resp) = ui.allocate_exact_size(
                    vec2(galley.size().x + 26.0 + badge_w, 28.0),
                    Sense::click(),
                );
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
                let text_pos =
                    egui::pos2(rect.min.x + 13.0, rect.center().y - galley.size().y / 2.0);
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
    clicked
}

// TODO(task 16 Installed): used by the Installed table.
#[allow(dead_code)]
/// Rounded status pill showing the fit estimate.
pub fn fit_pill(ui: &mut egui::Ui, fit: Fit) {
    let t = tokens(ui);
    let (fg, bg) = match fit_pill_kind(fit) {
        PillKind::Ok => (t.ok, t.ok_soft),
        PillKind::Warn => (t.warn, t.warn_soft),
        PillKind::Neutral => (t.fg2, t.surface2),
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(fit_pill_label(fit)).size(12.0).color(fg))
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
        });
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
