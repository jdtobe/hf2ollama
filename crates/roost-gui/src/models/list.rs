//! The left column of the Models tab: filter chips, the usage bar, the
//! Installed section and the Hugging Face section.

use crate::app::AppState;
use crate::browse::results::{format_size, section, section_header};
use crate::installed::state::ListStatus;
use crate::runtime::BackendRequest;
use crate::theme::{mono, tokens, Tokens};
use crate::widgets::{ellipsize_middle, small_button, tag};
use eframe::egui::{self, vec2, Margin, Rect, RichText, Sense, Stroke};

const ROW_H: f32 = 82.0;

pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    // Flat, like the Chat and Work sidebar: the list in the half tint, the
    // search and filters on a band of the full tint, one divider on the right.
    let frame = egui::Frame::new().fill(t.side_list());
    let wanted = app.settings.sidebar_width;
    let (rect, width) = crate::sidebar::show(ui, wanted, frame, t.side_edge(), |ui| {
        egui::Frame::new()
            .fill(t.side_band())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let search = crate::widgets::search_field(
                    ui,
                    &mut app.query,
                    "Search installed models and Hugging Face",
                );
                app.search_rect = Some(search.response.rect);
                app.search_clear_rect = search.clear;
                ui.add_space(8.0);
                crate::browse::filters::filter_bar(ui, app);
            });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                installed_section(ui, app, &t, out);
                section(ui, app, out);
            });
        ui.min_rect()
    });
    crate::sidebar::finish(ui, app, width);
    #[cfg(test)]
    {
        app.browse.probe.list = Some(rect);
    }
    let _ = rect;
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
                    egui::UiBuilder::new().max_rect(rect.shrink2(vec2(14.0, 12.0))),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let room = ui.available_width();
                        let name = ellipsize_middle(ui, row.name(), mono(13.0), room);
                        ui.label(RichText::new(name).font(mono(13.0)).color(t.fg));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for pill in [
                                row.model.params.clone(),
                                row.model.quant.clone(),
                                Some(format_size(row.model.size)),
                            ]
                            .into_iter()
                            .flatten()
                            {
                                tag(ui, &pill, t.fg2, t.surface2);
                            }
                        });
                        let from = match &row.source_repo {
                            Some(r) => format!("from {r}"),
                            None => "Pulled from the Ollama library".to_string(),
                        };
                        let from =
                            ellipsize_middle(ui, &from, egui::FontId::proportional(12.0), room);
                        ui.label(RichText::new(from).size(12.0).color(t.fg3));
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
        p.rect_filled(rect, 0.0, t.surface);
        p.rect_filled(
            Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
            0.0,
            t.accent,
        );
    } else if hovered {
        p.rect_filled(rect, 0.0, t.side_band());
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

/// Disk used by the installed models, as a titled card: the total, a bar split
/// into what Roost imported and what was pulled from Ollama, and a legend
/// with a colour swatch for each.
fn usage(ui: &mut egui::Ui, app: &AppState, t: &Tokens) {
    let split = app.installed.usage();
    let total = split.imported.saturating_add(split.library);
    let imported = app
        .installed
        .rows()
        .iter()
        .filter(|r| r.source_repo.is_some())
        .count();
    let library = app.installed.rows().len() - imported;
    // Two clearly different shades of the theme's accent.
    let (c_roost, c_ollama) = (t.accent, t.accent.gamma_multiply(0.35));
    egui::Frame::new()
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            let outer = ui.available_width();
            ui.set_width(outer);
            egui::Frame::new()
                .fill(t.surface2)
                .stroke(Stroke::new(1.0, t.line))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    let width = ui.available_width();
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Disk used by models").size(12.0).color(t.fg2));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_size(total))
                                    .font(mono(12.5))
                                    .color(t.fg),
                            );
                        });
                    });
                    let (bar, _) = ui.allocate_exact_size(vec2(width, 10.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(bar, 5.0, t.line);
                    if total > 0 {
                        let cut = bar.left() + width * (split.imported as f32 / total as f32);
                        let mut left = bar;
                        left.set_right(cut);
                        let mut right = bar;
                        right.set_left(cut);
                        if split.library > 0 {
                            p.rect_filled(right, 5.0, c_ollama);
                        }
                        if split.imported > 0 {
                            p.rect_filled(left, 5.0, c_roost);
                        }
                        // A hairline keeps the two shades apart.
                        if split.imported > 0 && split.library > 0 {
                            p.vline(cut, bar.y_range(), Stroke::new(1.5, t.surface2));
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = vec2(14.0, 4.0);
                        for (colour, label, bytes, n) in [
                            (c_roost, "Imported with Roost", split.imported, imported),
                            (c_ollama, "Pulled from Ollama", split.library, library),
                        ] {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let (dot, _) =
                                    ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
                                ui.painter().rect_filled(dot, 2.5, colour);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{label} {} ({n})",
                                            format_size(bytes)
                                        ))
                                        .size(11.5)
                                        .color(t.fg2),
                                    )
                                    .truncate(),
                                );
                            });
                        }
                    });
                });
        });
}
