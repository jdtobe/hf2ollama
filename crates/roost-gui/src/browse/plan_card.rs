//! The import plan card: model name, pipeline step chips, sizes and the
//! primary button.

use super::options::gb;
use super::results::format_size;
use super::state::PlanKind;
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{caption, primary_button};
use eframe::egui::{self, vec2, Margin, RichText, Stroke};
use roost_core::events::Command;

/// Pipeline steps for the chosen import, in order. One word each: the whole
/// line has to fit the narrowest card on one row.
pub fn steps(is_gguf: bool, quantize: bool) -> Vec<&'static str> {
    let mut v = vec!["Download", "Verify", "Upload"];
    if !is_gguf && quantize {
        v.push("Quantize");
    }
    v.push("Create");
    v
}

/// What the steps do, for the hover text.
pub fn steps_detail(is_gguf: bool, quantize: bool) -> String {
    let mut v = vec![
        "Download the files",
        "Verify their sha256",
        if is_gguf {
            "Upload the blob to Ollama"
        } else {
            "Upload the blobs to Ollama"
        },
    ];
    if !is_gguf && quantize {
        v.push("Quantize");
    }
    v.push("Create the model");
    v.join("\n")
}

/// Draws the steps as one line, `Download › Verify › Upload › Create`. The
/// line never wraps; if the card is too narrow it is cut with an ellipsis and
/// the hover text lists every step. Returns the line's rectangle.
pub fn step_line(ui: &mut egui::Ui, labels: &[&str], detail: &str) -> egui::Rect {
    let t = tokens(ui);
    let mut job = egui::text::LayoutJob::default();
    for (i, l) in labels.iter().enumerate() {
        if i > 0 {
            job.append(
                "  ›  ",
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(11.0), t.fg3),
            );
        }
        job.append(
            l,
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(11.5), t.fg2),
        );
    }
    ui.add(egui::Label::new(job).truncate())
        .on_hover_text(detail)
        .rect
}

/// Draws the card for the selected repository and sends the import when the
/// primary button is pressed.
///
/// `budget` is the tallest the card may be. The name field and the button
/// are always shown in full; the step chips, size lines and the disabled
/// reason scroll inside whatever height is left.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>, budget: f32) {
    let t = tokens(ui);
    let reason = app.import_block_reason();

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
            if clicked && reason.is_none() {
                if let Some(req) = app.browse.import_request() {
                    app.browse.mark_import_pending(&req.model_name);
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
                                let labels = steps(is_gguf, quantize);
                                let _line =
                                    step_line(ui, &labels, &steps_detail(is_gguf, quantize));
                                #[cfg(test)]
                                {
                                    app.browse.probe.chips = vec![_line];
                                }
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

    #[test]
    fn steps_follow_the_import_kind() {
        assert_eq!(
            steps(true, false),
            ["Download", "Verify", "Upload", "Create"]
        );
        assert_eq!(
            steps(false, true),
            ["Download", "Verify", "Upload", "Quantize", "Create"]
        );
        assert_eq!(steps(false, false), steps(true, false));
    }
}
