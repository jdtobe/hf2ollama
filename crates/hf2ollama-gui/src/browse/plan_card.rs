//! The import plan card: model name, pipeline step chips, sizes and the
//! primary button.

use super::options::gb;
use super::results::format_size;
use super::state::PlanKind;
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{caption, primary_button};
use eframe::egui::{self, vec2, Margin, RichText, Sense, Stroke};
use hf2ollama_core::events::Command;

/// Pipeline steps shown as chips for the chosen import.
pub fn steps(is_gguf: bool, quantize: bool, delete_after: bool) -> Vec<&'static str> {
    let mut v = vec![
        "Download",
        "Verify sha256",
        if is_gguf {
            "Upload blob(s)"
        } else {
            "Upload blobs"
        },
    ];
    if !is_gguf && quantize {
        v.push("Quantize");
    }
    v.push("Create model");
    v.push(if delete_after {
        "Delete staging"
    } else {
        "Keep files"
    });
    v
}

/// Where one chip goes: its row and x offset from the left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipSlot {
    pub row: usize,
    pub x: f32,
}

/// Flows chips of the given widths into rows no wider than `avail`. Chips on
/// the same row are separated by `gap + sep_w + gap`.
pub fn layout_chips(widths: &[f32], gap: f32, sep_w: f32, avail: f32) -> Vec<ChipSlot> {
    let mut slots = Vec::with_capacity(widths.len());
    let (mut row, mut end) = (0usize, None::<f32>);
    for &w in widths {
        let x = match end {
            None => 0.0,
            Some(e) => {
                let next = e + gap + sep_w + gap;
                if next + w > avail {
                    row += 1;
                    0.0
                } else {
                    next
                }
            }
        };
        slots.push(ChipSlot { row, x });
        end = Some(x + w);
    }
    slots
}

/// Horizontal padding added to a chip label (7 px each side).
pub const CHIP_PAD: f32 = 14.0;
const CHIP_H: f32 = 20.0;
const CHIP_ROW_GAP: f32 = 6.0;
const CHIP_GAP: f32 = 4.0;
const CHIP_SEP_W: f32 = 8.0;

/// Draws the chips (wrapping to as many rows as needed) and returns their
/// rectangles. Chip widths are measured from the labels, so no label wraps.
pub fn step_chips(ui: &mut egui::Ui, labels: &[&str]) -> Vec<egui::Rect> {
    let t = tokens(ui);
    let galleys: Vec<_> = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap((*l).to_string(), mono(11.0), t.fg2)
        })
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + CHIP_PAD).collect();
    let avail = ui.available_width();
    let slots = layout_chips(&widths, CHIP_GAP, CHIP_SEP_W, avail);
    let rows = slots.last().map_or(0, |s| s.row + 1);
    if rows == 0 {
        return Vec::new();
    }
    let height = rows as f32 * CHIP_H + (rows - 1) as f32 * CHIP_ROW_GAP;
    let (area, _) = ui.allocate_exact_size(vec2(avail, height), Sense::hover());
    let mut rects: Vec<egui::Rect> = Vec::with_capacity(labels.len());
    for ((slot, w), galley) in slots.iter().zip(&widths).zip(galleys) {
        let min = area.min + vec2(slot.x, slot.row as f32 * (CHIP_H + CHIP_ROW_GAP));
        let rect = egui::Rect::from_min_size(min, vec2(*w, CHIP_H));
        if let Some(prev) = rects.last() {
            if (prev.min.y - rect.min.y).abs() < 0.5 {
                let sep = egui::Rect::from_min_max(
                    egui::pos2(prev.max.x, rect.min.y),
                    egui::pos2(rect.min.x, rect.max.y),
                );
                ui.painter().text(
                    sep.center(),
                    egui::Align2::CENTER_CENTER,
                    "›",
                    egui::FontId::proportional(12.0),
                    t.fg3,
                );
            }
        }
        ui.painter().rect_filled(rect, 4.0, t.surface);
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, t.line),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, t.fg2);
        rects.push(rect);
    }
    rects
}

/// Draws the card for the selected repository and sends the import when the
/// primary button is pressed.
///
/// `budget` is the tallest the card may be. The name field and the button
/// are always shown in full; the step chips, size lines and the disabled
/// reason scroll inside whatever height is left.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>, budget: f32) {
    let t = tokens(ui);
    let reachable = app.ollama.as_ref().map(|o| o.is_some());
    let reason = app.browse.import_block_reason(reachable);
    let delete_after = app.delete_after_import;

    // (is_gguf, quantize chosen, download bytes, final bytes, final is an estimate)
    let mut facts = None;
    if let (Some(plan), Some(i)) = (app.browse.plan(), app.browse.selected_option()) {
        if let Some(row) = plan.rows.get(i) {
            let is_gguf = matches!(plan.kind, PlanKind::Gguf(_));
            let download = match plan.kind {
                PlanKind::Safetensors(_) => {
                    plan.rows.first().map_or(row.size_bytes, |r| r.size_bytes)
                }
                _ => row.size_bytes,
            };
            facts = Some((
                is_gguf,
                row.quantize.is_some(),
                download,
                row.size_bytes,
                row.estimated,
            ));
        }
    }
    let label = match facts {
        Some((_, _, download, _, _)) => format!("Download & import · {}", format_size(download)),
        None => "Download & import".to_string(),
    };

    let card = egui::Frame::new()
        .fill(t.surface2)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(8.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            let start = ui.cursor().top();
            caption(ui, "Model name in Ollama");
            let btn_w = ui
                .painter()
                .layout_no_wrap(label.clone(), semibold(14.0), t.fg)
                .size()
                .x
                + 28.0;
            let avail = ui.available_width();
            let side_by_side = avail >= btn_w + 14.0 + 220.0;
            let mut clicked = false;
            let edit = |ui: &mut egui::Ui, w: f32, app: &mut AppState| {
                let r = ui.add_sized(
                    vec2(w, 32.0),
                    egui::TextEdit::singleline(&mut app.browse.name)
                        .font(mono(13.0))
                        .margin(Margin::symmetric(10, 7)),
                );
                if r.changed() {
                    app.browse.name_edited = true;
                }
                #[cfg(test)]
                {
                    app.browse.probe.name_field = Some(r.rect);
                }
            };
            if side_by_side {
                ui.horizontal(|ui| {
                    edit(ui, avail - btn_w - 14.0, app);
                    let b = primary_button(ui, &label, reason.is_none());
                    let b = match &reason {
                        Some(r) => b.on_disabled_hover_text(r),
                        None => b,
                    };
                    #[cfg(test)]
                    {
                        app.browse.probe.button = Some(b.rect);
                    }
                    clicked = b.clicked();
                });
            } else {
                edit(ui, avail, app);
                let b = primary_button(ui, &label, reason.is_none());
                let b = match &reason {
                    Some(r) => b.on_disabled_hover_text(r),
                    None => b,
                };
                #[cfg(test)]
                {
                    app.browse.probe.button = Some(b.rect);
                }
                clicked = b.clicked();
            }
            if clicked {
                if let Some(req) = app.browse.import_request() {
                    out.push(BackendRequest::Core(Command::StartImport(req)));
                }
            }
            let used = ui.cursor().top() - start;
            let secondary_max = (budget - 22.0 - used).max(0.0);
            // The panel's own height lags a frame behind, so the region is
            // sized explicitly: its content height from last frame, capped.
            let nat_id = egui::Id::NULL.with("card_secondary_natural");
            let natural: f32 = ui
                .ctx()
                .data(|d| d.get_temp(nat_id))
                .unwrap_or(secondary_max);
            let height = natural.min(secondary_max);
            let width = ui.available_width();
            let region = ui
                .allocate_ui(vec2(width, height), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("card_secondary")
                        .min_scrolled_height(0.0)
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if let Some(r) = &reason {
                                ui.label(RichText::new(r).size(12.0).color(t.warn));
                            }
                            if let Some((is_gguf, quantize, download, final_bytes, estimated)) =
                                facts
                            {
                                ui.add_space(2.0);
                                let labels = steps(is_gguf, quantize, delete_after);
                                step_chips(ui, &labels);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    let dim = |s: &str| {
                                        RichText::new(s.to_string()).size(12.0).color(t.fg3)
                                    };
                                    let val = |s: String| RichText::new(s).font(mono(12.0));
                                    ui.label(dim("Temporary space"));
                                    ui.label(val(format!("{:.2} GB", gb(download))));
                                    ui.add_space(12.0);
                                    ui.label(dim("Final in Ollama"));
                                    let est = if estimated { "~" } else { "" };
                                    ui.label(val(format!("{est}{:.2} GB", gb(final_bytes))));
                                });
                            }
                        })
                })
                .inner;
            let content_h = region.content_size.y;
            if (content_h - natural).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(nat_id, content_h));
                ui.ctx().request_repaint();
            }
        })
        .response
        .rect;
    #[cfg(test)]
    {
        app.browse.probe.card = Some(card);
    }
    #[cfg(not(test))]
    let _ = card;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::test_util::headless;
    use crate::theme::mono;

    #[test]
    fn steps_follow_the_import_kind() {
        assert_eq!(
            steps(true, false, true),
            [
                "Download",
                "Verify sha256",
                "Upload blob(s)",
                "Create model",
                "Delete staging"
            ]
        );
        assert_eq!(steps(true, false, false).last(), Some(&"Keep files"));
        assert_eq!(
            steps(false, true, true),
            [
                "Download",
                "Verify sha256",
                "Upload blobs",
                "Quantize",
                "Create model",
                "Delete staging"
            ]
        );
        assert_eq!(
            steps(false, false, false),
            [
                "Download",
                "Verify sha256",
                "Upload blobs",
                "Create model",
                "Keep files"
            ]
        );
    }

    #[test]
    fn layout_chips_wraps_when_the_row_is_full() {
        // 60 + (6+14+6) + 80 = 166 fits in 170; the third chip needs 26 + 70 more.
        let slots = layout_chips(&[60.0, 80.0, 70.0], 6.0, 14.0, 170.0);
        assert_eq!(slots[0], ChipSlot { row: 0, x: 0.0 });
        assert_eq!(slots[1], ChipSlot { row: 0, x: 86.0 });
        assert_eq!(slots[2], ChipSlot { row: 1, x: 0.0 });
        // Everything fits in one row when wide enough.
        let one = layout_chips(&[60.0, 80.0, 70.0], 6.0, 14.0, 400.0);
        assert!(one.iter().all(|s| s.row == 0));
        // A chip wider than the row still gets a row of its own.
        let wide = layout_chips(&[300.0, 20.0], 6.0, 14.0, 100.0);
        assert_eq!(wide[0], ChipSlot { row: 0, x: 0.0 });
        assert_eq!(wide[1].row, 1);
    }

    #[test]
    fn step_chips_wrap_onto_rows_without_widening_any_chip() {
        let labels = steps(false, true, true);
        for dark in [false, true] {
            // 700 px window: the card is far narrower than the window.
            let (rects, widths) = headless(dark, 300.0, |ui| {
                let rects = step_chips(ui, &labels);
                let widths: Vec<f32> = labels
                    .iter()
                    .map(|l| {
                        ui.painter()
                            .layout_no_wrap((*l).to_string(), mono(11.0), egui::Color32::WHITE)
                            .size()
                            .x
                    })
                    .collect();
                (rects, widths)
            });
            assert_eq!(rects.len(), labels.len(), "dark={dark}");
            let mut ys: Vec<i32> = rects.iter().map(|r| r.min.y.round() as i32).collect();
            ys.dedup();
            assert!(ys.len() >= 2, "dark={dark}: chips should wrap, rows {ys:?}");
            for (r, w) in rects.iter().zip(&widths) {
                assert!(
                    (r.width() - (w + CHIP_PAD)).abs() < 0.6,
                    "dark={dark}: chip {} wide vs content {}",
                    r.width(),
                    w
                );
                assert!(r.height() < 26.0, "dark={dark}: chip wrapped its label");
            }
            let left = rects.iter().map(|r| r.min.x).fold(f32::MAX, f32::min);
            let right = rects.iter().map(|r| r.max.x).fold(0.0, f32::max);
            assert!(
                right - left <= 300.5,
                "dark={dark}: overflow {}",
                right - left
            );
        }
    }
}
