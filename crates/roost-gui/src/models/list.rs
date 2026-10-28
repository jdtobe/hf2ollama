//! The left column of the Models tab: filter chips, the usage bar, the
//! Installed section and the Hugging Face section.

use crate::app::AppState;
use crate::browse::results::{format_size, section, section_header};
use crate::installed::state::ListStatus;
use crate::runtime::BackendRequest;
use crate::theme::{mono, tokens, Tokens};
use crate::widgets::{ellipsize_middle, small_button};
use eframe::egui::{self, vec2, Margin, Rect, RichText, Sense, Stroke};

const ROW_H: f32 = 54.0;

pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    let panel = egui::Panel::left("models_list")
        .default_size(340.0)
        .size_range(300.0..=360.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.0, t.line)),
        )
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    crate::browse::filters::chips(ui, app);
                });
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    installed_section(ui, app, &t, out);
                    section(ui, app, out);
                });
        });
    #[cfg(test)]
    {
        app.browse.probe.list = Some(panel.response.rect);
    }
    let _ = panel;
}

fn installed_section(
    ui: &mut egui::Ui,
    app: &mut AppState,
    t: &Tokens,
    out: &mut Vec<BackendRequest>,
) {
    let visible: Vec<_> = app.installed.visible().into_iter().cloned().collect();
    let total = app.installed.rows().len();
    let count = match app.installed.status {
        ListStatus::Loaded if visible.len() == total => total.to_string(),
        ListStatus::Loaded => format!("{} of {total}", visible.len()),
        _ => String::new(),
    };
    section_header(ui, "Installed", &count);
    match app.installed.status.clone() {
        ListStatus::Loading => note(ui, t, "Reading the models Ollama has…"),
        ListStatus::Error(e) => {
            note(ui, t, &format!("Could not read the installed models. {e}"));
            if small_button(ui, "Retry") {
                app.installed.begin_load();
                out.push(BackendRequest::ListInstalled);
            }
        }
        ListStatus::Loaded if total == 0 => {
            note(
                ui,
                t,
                "No models installed yet. Import one from Hugging Face below.",
            );
        }
        ListStatus::Loaded => {
            usage(ui, app, t);
            if visible.is_empty() {
                note(ui, t, "No installed model matches this search.");
            }
            let selected = app.models_installed.clone();
            for row in &visible {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let on = selected.as_deref() == Some(row.name());
                paint_row(ui, t, rect, resp.hovered(), on);
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 8.0))),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        let room = ui.available_width();
                        let name = ellipsize_middle(ui, row.name(), mono(13.0), room);
                        ui.label(RichText::new(name).font(mono(13.0)).color(t.fg));
                        let mut meta = vec![format_size(row.model.size)];
                        if let Some(q) = &row.model.quant {
                            meta.push(q.clone());
                        }
                        meta.push(match &row.source_repo {
                            Some(r) => format!("from {r}"),
                            None => "Ollama library".to_string(),
                        });
                        let line = ellipsize_middle(
                            ui,
                            &meta.join("  ·  "),
                            egui::FontId::proportional(12.0),
                            room,
                        );
                        ui.label(RichText::new(line).size(12.0).color(t.fg3));
                    },
                );
                #[cfg(test)]
                app.browse
                    .probe
                    .installed_rows
                    .push((row.name().to_string(), rect));
                if resp.clicked() {
                    app.models_installed = Some(row.name().to_string());
                    if let Some(n) = app.installed.want_details(row.name()) {
                        out.push(BackendRequest::ShowInstalled(n));
                    }
                }
            }
        }
    }
}

/// Selected / hovered background plus a divider under the row.
pub fn paint_row(ui: &egui::Ui, t: &Tokens, rect: Rect, hovered: bool, selected: bool) {
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 0.0, t.accent_soft);
        p.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            0.0,
            t.accent,
        );
    } else if hovered {
        p.rect_filled(rect, 0.0, t.surface2);
    }
    p.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.line));
}

fn note(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(t.fg2)).wrap());
        });
}

/// The two-segment bar: imported with Roost, pulled from Ollama.
fn usage(ui: &mut egui::Ui, app: &AppState, t: &Tokens) {
    let split = app.installed.usage();
    let total = split.imported.saturating_add(split.library);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(14, 8))
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.set_width(width);
            let (bar, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(bar, 3.0, t.line);
            if total > 0 {
                let cut = bar.left() + width * (split.imported as f32 / total as f32);
                let mut left = bar;
                left.set_right(cut);
                let mut right = bar;
                right.set_left(cut);
                if split.imported > 0 {
                    p.rect_filled(left, 3.0, t.accent);
                }
                if split.library > 0 {
                    p.rect_filled(right, 3.0, t.fg3);
                }
            }
            ui.add_space(4.0);
            let imported = app
                .installed
                .rows()
                .iter()
                .filter(|r| r.source_repo.is_some())
                .count();
            let library = app.installed.rows().len() - imported;
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Roost {} ({imported})  ·  Ollama {} ({library})",
                        format_size(split.imported),
                        format_size(split.library)
                    ))
                    .size(11.5)
                    .color(t.fg3),
                )
                .truncate(),
            );
        });
}
