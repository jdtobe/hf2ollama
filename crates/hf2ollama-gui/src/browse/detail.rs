//! The detail pane for the selected repository.

use super::options::{gb, OptionRow};
use super::plan_card;
use super::results::{compact_count, format_size, relative_age};
use super::state::{is_gated_error, PlanKind, SearchStatus, GATED_MESSAGE, GATED_REPO_WARNING};
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{caption, fit_pill_kind, fit_pill_label, fit_text, pill_colors, small_button};
use eframe::egui::{
    self, vec2, Align2, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind,
};
use hf2ollama_core::fit::{fit, Machine};
use hf2ollama_core::hf::ModelInfo;
use std::time::{SystemTime, UNIX_EPOCH};

/// Share of the spare width each metadata column gets when everything fits
/// on one row, so long architecture names get the most room.
fn meta_weight(label: &str) -> f32 {
    match label {
        "Architecture" => 0.30,
        "Base model" => 0.18,
        "License" => 0.16,
        "Prompt template" => 0.12,
        _ => 0.12,
    }
}

const META_GAP: f32 = 24.0;
const META_ROW_GAP: f32 = 10.0;

/// Draws the metadata strip (label over value, "—" for unknown values) and
/// returns the label and painted rectangle of each value. Values never wrap: columns are at least as
/// wide as their content, and when they do not all fit on one row they flow
/// onto further rows instead.
pub fn meta_strip(ui: &mut egui::Ui, items: &[(&str, String)]) -> Vec<(String, Rect)> {
    let t = tokens(ui);
    let mut cells = Vec::new();
    egui::Frame::new()
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            let avail = ui.available_width();
            let caps: Vec<_> = items
                .iter()
                .map(|(k, _)| {
                    ui.painter()
                        .layout_no_wrap(k.to_uppercase(), semibold(11.0), t.fg3)
                })
                .collect();
            let vals: Vec<_> = items
                .iter()
                .map(|(_, v)| ui.painter().layout_no_wrap(v.clone(), mono(13.0), t.fg))
                .collect();
            let natural: Vec<f32> = caps
                .iter()
                .zip(&vals)
                .map(|(c, v)| c.size().x.max(v.size().x))
                .collect();
            let n = items.len();
            let used: f32 = natural.iter().sum::<f32>() + META_GAP * n.saturating_sub(1) as f32;
            // (x offset, row, width) per column.
            let mut places: Vec<(f32, usize, f32)> = Vec::with_capacity(n);
            if used <= avail {
                let spare = avail - used;
                let total_w: f32 = items.iter().map(|(k, _)| meta_weight(k)).sum();
                let mut x = 0.0;
                for ((k, _), nat) in items.iter().zip(&natural) {
                    let w = nat + spare * meta_weight(k) / total_w;
                    places.push((x, 0, w));
                    x += w + META_GAP;
                }
            } else {
                let (mut x, mut row) = (0.0, 0);
                for nat in &natural {
                    let w = nat.min(avail);
                    if x > 0.0 && x + w > avail {
                        x = 0.0;
                        row += 1;
                    }
                    places.push((x, row, w));
                    x += w + META_GAP;
                }
            }
            let rows = places.last().map_or(1, |p| p.1 + 1);
            let row_h = caps.first().map_or(14.0, |c| c.size().y)
                + 2.0
                + vals.first().map_or(18.0, |v| v.size().y);
            let height = rows as f32 * row_h + (rows - 1) as f32 * META_ROW_GAP;
            let (area, _) = ui.allocate_exact_size(vec2(avail, height), Sense::hover());
            for (i, (x, row, w)) in places.iter().enumerate() {
                let origin = area.min + vec2(*x, *row as f32 * (row_h + META_ROW_GAP));
                let cap = caps[i].clone();
                let cap_h = cap.size().y;
                ui.painter().galley(origin, cap, t.fg3);
                let value_pos = origin + vec2(0.0, cap_h + 2.0);
                let galley = if vals[i].size().x > *w {
                    fit_text(ui, &items[i].1, mono(13.0), t.fg, *w)
                } else {
                    vals[i].clone()
                };
                let rect = Rect::from_min_size(value_pos, galley.size());
                ui.painter().galley(value_pos, galley, t.fg);
                cells.push((items[i].0.to_string(), rect));
            }
        });
    cells
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A bordered message box; `warn` picks the amber palette.
fn callout(ui: &mut egui::Ui, warn: bool, text: &str) {
    let t = tokens(ui);
    let (fill, stroke) = if warn {
        (t.warn_soft, t.warn)
    } else {
        (t.surface2, t.line)
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(13.0).color(t.fg));
        });
}

const TABLE_PAD: f32 = 14.0;
const BAR_W: f32 = 56.0;
const NOTE_MIN: f32 = 80.0;

/// The hand-painted options table. Returns the clicked row, if any. Wide
/// panes get one line per row; narrow ones put the quality and fit on a
/// second line so nothing is clipped.
pub fn options_table(
    ui: &mut egui::Ui,
    first_col: &str,
    rows: &[OptionRow],
    selected: Option<usize>,
    machine: &Machine,
    scroll_to_selected: bool,
    selected_rect: &mut Option<Rect>,
) -> Option<usize> {
    let t = tokens(ui);
    let w = ui.available_width();
    let p = ui.painter().clone();
    let measure =
        |text: &str, font: FontId| p.layout_no_wrap(text.to_string(), font, t.fg).size().x;
    let rec_w = measure("Recommended", semibold(11.0));
    let est_w = measure("est.", FontId::proportional(11.0));
    let fits: Vec<_> = rows
        .iter()
        .map(|r| fit(gb(r.size_bytes), machine))
        .collect();
    let size_text = |r: &OptionRow| format!("{:.2} GB", gb(r.size_bytes));
    let name_w = rows
        .iter()
        .map(|r| measure(&r.label, mono(13.5)) + if r.recommended { 10.0 + rec_w } else { 0.0 })
        .fold(0.0, f32::max);
    let size_w = rows
        .iter()
        .map(|r| measure(&size_text(r), mono(13.0)) + if r.estimated { 6.0 + est_w } else { 0.0 })
        .fold(0.0, f32::max);
    let pill_w = fits
        .iter()
        .map(|f| measure(fit_pill_label(*f), FontId::proportional(12.0)) + 16.0)
        .fold(measure("ON YOUR MACHINE", semibold(11.0)), f32::max);
    let quality_w = w - (TABLE_PAD * 4.0 + name_w + size_w + pill_w);
    let wide = quality_w >= BAR_W + 10.0 + NOTE_MIN;

    let mut clicked = None;
    let mut top = None;
    let mut bottom = 0.0;
    ui.spacing_mut().item_spacing.y = 0.0;
    if wide {
        let (head, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        top = Some(head.min);
        bottom = head.max.y;
        ui.painter().rect_filled(
            head,
            CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            },
            t.surface2,
        );
        let size_right = TABLE_PAD + name_w + TABLE_PAD + size_w;
        let q_x = size_right + TABLE_PAD;
        let pill_x = w - TABLE_PAD - pill_w;
        for (x, label, right) in [
            (TABLE_PAD, first_col, false),
            (size_right, "SIZE", true),
            (q_x, "QUALITY", false),
            (pill_x, "ON YOUR MACHINE", false),
        ] {
            ui.painter().text(
                egui::pos2(head.min.x + x, head.center().y),
                if right {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                label.to_uppercase(),
                semibold(11.0),
                t.fg3,
            );
        }
    }
    let row_h = if wide { 38.0 } else { 62.0 };
    for (i, r) in rows.iter().enumerate() {
        let (row, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
        if top.is_none() {
            top = Some(row.min);
        }
        bottom = row.max.y;
        let sel = selected == Some(i);
        if sel {
            *selected_rect = Some(row);
            if scroll_to_selected {
                ui.scroll_to_rect(row, Some(egui::Align::Center));
            }
            ui.painter().rect_filled(row, 0.0, t.accent_soft);
            ui.painter().rect_filled(
                Rect::from_min_size(row.min, vec2(3.0, row.height())),
                0.0,
                t.accent,
            );
        } else if resp.hovered() {
            ui.painter().rect_filled(row, 0.0, t.surface2);
        }
        if i + 1 < rows.len() {
            ui.painter()
                .hline(row.x_range(), row.bottom(), Stroke::new(1.0, t.line));
        }
        let fit_kind = fit_pill_kind(fits[i]);
        let (fg, bg) = pill_colors(&t, fit_kind);
        let pill_g = ui.painter().layout_no_wrap(
            fit_pill_label(fits[i]).to_string(),
            FontId::proportional(12.0),
            fg,
        );
        let bar_fill = if sel { t.surface } else { t.line };
        let draw_bar = |ui: &egui::Ui, at: egui::Pos2| {
            let bar = Rect::from_min_size(at, vec2(BAR_W, 6.0));
            ui.painter().rect_filled(bar, 3.0, bar_fill);
            ui.painter().rect_filled(
                Rect::from_min_size(bar.min, vec2(BAR_W * r.quality, 6.0)),
                3.0,
                t.accent,
            );
            bar
        };
        let note_text = r.note.unwrap_or("—");
        let (y1, y2) = if wide {
            (row.center().y, row.center().y)
        } else {
            (row.min.y + 20.0, row.min.y + 43.0)
        };
        // Label, recommended marker, size.
        let name_rect = ui.painter().text(
            egui::pos2(row.min.x + TABLE_PAD, y1),
            Align2::LEFT_CENTER,
            &r.label,
            mono(13.5),
            t.fg,
        );
        if r.recommended {
            ui.painter().text(
                egui::pos2(name_rect.right() + 10.0, y1),
                Align2::LEFT_CENTER,
                "Recommended",
                semibold(11.0),
                t.accent,
            );
        }
        let size_right = if wide {
            row.min.x + TABLE_PAD + name_w + TABLE_PAD + size_w
        } else {
            row.max.x - TABLE_PAD
        };
        let size_rect = ui.painter().text(
            egui::pos2(size_right, y1),
            Align2::RIGHT_CENTER,
            size_text(r),
            mono(13.0),
            t.fg,
        );
        if r.estimated {
            ui.painter().text(
                egui::pos2(size_rect.left() - 6.0, y1),
                Align2::RIGHT_CENTER,
                "est.",
                FontId::proportional(11.0),
                t.fg3,
            );
        }
        // Quality bar, note and fit pill.
        let (bar_x, pill_left) = if wide {
            (
                row.min.x + TABLE_PAD + name_w + TABLE_PAD + size_w + TABLE_PAD,
                row.max.x - TABLE_PAD - pill_w,
            )
        } else {
            (
                row.min.x + TABLE_PAD,
                row.max.x - TABLE_PAD - (pill_g.size().x + 16.0),
            )
        };
        let bar = draw_bar(ui, egui::pos2(bar_x, y2 - 3.0));
        let note_w = (pill_left - 12.0) - (bar.right() + 10.0);
        let note = fit_text(ui, note_text, FontId::proportional(12.0), t.fg2, note_w);
        ui.painter().galley(
            egui::pos2(bar.right() + 10.0, y2 - note.size().y / 2.0),
            note,
            t.fg2,
        );
        let pill = Rect::from_min_size(
            egui::pos2(pill_left, y2 - 10.0),
            vec2(pill_g.size().x + 16.0, 20.0),
        );
        ui.painter()
            .rect_filled(pill, 10.0, if sel { t.surface } else { bg });
        ui.painter()
            .galley(pill.center() - pill_g.size() / 2.0, pill_g, fg);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    if let Some(top) = top {
        let table = Rect::from_min_max(top, egui::pos2(top.x + w, bottom));
        ui.painter()
            .rect_stroke(table, 8.0, Stroke::new(1.0, t.line), StrokeKind::Inside);
    }
    ui.spacing_mut().item_spacing.y = 6.0;
    clicked
}

fn metadata(info: &ModelInfo) -> Vec<(&'static str, String)> {
    let dash = || "—".to_string();
    vec![
        ("Parameters", info.params.clone().unwrap_or_else(dash)),
        ("Architecture", info.arch.clone().unwrap_or_else(dash)),
        ("Context", info.context.map_or_else(dash, thousands)),
        ("License", info.license.clone().unwrap_or_else(dash)),
        ("Base model", info.base_model.clone().unwrap_or_else(dash)),
        // Hugging Face does not report a chat template in the model listing.
        ("Prompt template", dash()),
    ]
}

/// Smallest height the scrolling part of the pane keeps when the pinned
/// import card is tall (narrow windows wrap its chips).
const MIN_SCROLL_H: f32 = 200.0;

/// The detail pane: scrolling header, metadata and options, with the import
/// card pinned below so the primary action is always visible.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    let selected = app.browse.selected_id().map(str::to_string);
    let card_visible = selected.as_deref().is_some_and(|id| {
        matches!(app.browse.info_for(id), Some(Ok(_))) || app.browse.selected_is_gated()
    });
    if card_visible {
        let max_h = (ui.available_height() - MIN_SCROLL_H).max(120.0);
        egui::Panel::bottom("plan_card")
            .resizable(false)
            .max_size(max_h)
            .frame(
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(Stroke::new(1.0, t.line))
                    .inner_margin(Margin {
                        left: 24,
                        right: 24,
                        top: 12,
                        bottom: 12,
                    }),
            )
            .show(ui, |ui| plan_card::show(ui, app, out));
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .inner_margin(Margin::symmetric(24, 20)),
        )
        .show(ui, |ui| {
            let Some(id) = selected else {
                ui.label(
                    RichText::new("Pick a model from the list to see its files and import it.")
                        .color(t.fg3),
                );
                return;
            };
            let scroll = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let info = app.browse.info_for(&id).cloned();
                    header(
                        ui,
                        &id,
                        info.as_ref().and_then(|r| r.as_ref().ok()).map(|i| &**i),
                    );
                    match info {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading the file list…").color(t.fg3));
                            });
                        }
                        Some(Err(e)) => {
                            let text = if is_gated_error(&e) {
                                GATED_MESSAGE.to_string()
                            } else {
                                crate::browse::state::friendly_error(&e)
                            };
                            callout(ui, true, &text);
                            if small_button(ui, "Retry") {
                                app.browse.retry_info(&id);
                            }
                        }
                        Some(Ok(info)) => ready(ui, app, &info),
                    }
                });
            #[cfg(test)]
            {
                app.browse.probe.viewport = Some(scroll.inner_rect);
                app.browse.probe.scroll_offset = scroll.state.clamped_offset().y;
            }
            #[cfg(not(test))]
            let _ = scroll;
        });
}

fn header(ui: &mut egui::Ui, id: &str, info: Option<&ModelInfo>) {
    let t = tokens(ui);
    let (owner, name) = id.split_once('/').unwrap_or(("", id));
    ui.heading(name);
    ui.horizontal_wrapped(|ui| {
        let mut line = format!("by {owner}");
        if let Some(i) = info {
            line.push_str(&format!(
                " · {} downloads · updated {}",
                compact_count(i.downloads),
                relative_age(&i.updated, now_unix())
            ));
        }
        ui.label(RichText::new(line).color(t.fg3));
        ui.hyperlink_to(
            RichText::new("Open on huggingface.co ↗").size(12.0),
            format!("https://huggingface.co/{id}"),
        );
    });
    ui.add_space(10.0);
}

fn ready(ui: &mut egui::Ui, app: &mut AppState, info: &ModelInfo) {
    if app.browse.selected_info_gated() {
        callout(ui, true, GATED_REPO_WARNING);
        ui.hyperlink_to(
            RichText::new("Open on huggingface.co ↗").size(12.0),
            format!("https://huggingface.co/{}", info.id),
        );
        ui.add_space(6.0);
    }
    meta_strip(ui, &metadata(info));
    ui.add_space(12.0);

    let Some(plan) = app.browse.plan().cloned() else {
        return;
    };
    let selected = app.browse.selected_option();
    let scroll = app.browse.take_scroll_request();
    let mut selected_rect = None;
    let mut clicked = None;
    match &plan.kind {
        PlanKind::Gguf(_) => {
            caption(ui, "Choose a file");
            clicked = options_table(
                ui,
                "File",
                &plan.rows,
                selected,
                &app.machine,
                scroll,
                &mut selected_rect,
            );
        }
        PlanKind::Safetensors(_) => {
            let source = plan.rows.first().map_or(0, |r| r.size_bytes);
            callout(
                ui,
                true,
                &format!(
                    "Safetensors, supported by Ollama. Weights download in full precision ({}) and Ollama converts and quantizes them when it creates the model.",
                    format_size(source)
                ),
            );
            ui.add_space(6.0);
            caption(ui, "Choose a quantization");
            clicked = options_table(
                ui,
                "Quantization",
                &plan.rows,
                selected,
                &app.machine,
                scroll,
                &mut selected_rect,
            );
        }
        PlanKind::Unsupported(reason) => {
            callout(
                ui,
                true,
                &format!("Ollama cannot import this repository. {reason}"),
            );
            ui.add_space(6.0);
            related(ui, app);
        }
    }
    #[cfg(test)]
    {
        app.browse.probe.selected_row = selected_rect;
    }
    #[cfg(not(test))]
    let _ = selected_rect;
    if let Some(i) = clicked {
        app.browse.select_option(i);
    }
}

/// Clickable related GGUF repositories for an unsupported one.
fn related(ui: &mut egui::Ui, app: &mut AppState) {
    let t = tokens(ui);
    caption(ui, "Related GGUF repos");
    match app.browse.related.clone() {
        SearchStatus::Idle | SearchStatus::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Looking for GGUF versions…").color(t.fg3));
            });
        }
        SearchStatus::Failed(msg) => {
            ui.label(RichText::new(msg).color(t.fg3));
            if small_button(ui, "Retry") {
                app.browse.retry_related();
            }
        }
        SearchStatus::Done => {
            let repos = app.browse.related_repos().to_vec();
            if repos.is_empty() {
                ui.label(RichText::new("No GGUF versions found on Hugging Face.").color(t.fg3));
            }
            let mut pick = None;
            for r in &repos {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, t.surface2);
                }
                ui.painter()
                    .rect_stroke(rect, 6.0, Stroke::new(1.0, t.line), StrokeKind::Inside);
                let stats = format!("↓ {}", compact_count(r.downloads));
                let stats_g = ui
                    .painter()
                    .layout_no_wrap(stats, FontId::proportional(12.0), t.fg3);
                let name_w = rect.width() - 28.0 - stats_g.size().x;
                let name = fit_text(ui, &r.id, semibold(13.0), t.fg, name_w);
                ui.painter().galley(
                    egui::pos2(rect.min.x + 12.0, rect.center().y - name.size().y / 2.0),
                    name,
                    t.fg,
                );
                ui.painter().galley(
                    egui::pos2(
                        rect.max.x - 12.0 - stats_g.size().x,
                        rect.center().y - stats_g.size().y / 2.0,
                    ),
                    stats_g,
                    t.fg3,
                );
                if resp.clicked() {
                    pick = Some(r.id.clone());
                }
            }
            if let Some(id) = pick {
                app.browse.select_repo(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::test_util::headless;
    use crate::theme::mono;

    fn items() -> Vec<(&'static str, String)> {
        vec![
            ("Parameters", "7.25B".to_string()),
            ("Architecture", "MistralForCausalLM".to_string()),
            ("Context", "32,768".to_string()),
            ("License", "apache-2.0".to_string()),
            ("Base model", "—".to_string()),
            ("Prompt template", "—".to_string()),
        ]
    }

    #[test]
    fn metadata_strip_never_wraps_a_long_architecture_name() {
        for dark in [false, true] {
            for width in [900.0, 520.0, 260.0] {
                let (cells, natural) = headless(dark, width, |ui| {
                    let cells = meta_strip(ui, &items());
                    let natural = ui
                        .painter()
                        .layout_no_wrap(
                            "MistralForCausalLM".to_string(),
                            mono(13.0),
                            egui::Color32::WHITE,
                        )
                        .size();
                    (cells, natural)
                });
                assert_eq!(cells.len(), 6, "dark={dark} width={width}");
                let arch = cells
                    .iter()
                    .find(|c| c.0 == "Architecture")
                    .expect("architecture cell");
                assert!(
                    (arch.1.width() - natural.x).abs() < 1.0,
                    "dark={dark} width={width}: arch {} vs natural {}",
                    arch.1.width(),
                    natural.x
                );
                assert!(
                    arch.1.height() < natural.y * 1.5,
                    "dark={dark} width={width}: arch wrapped, height {}",
                    arch.1.height()
                );
                // Nothing may spill past the available width.
                let right = cells.iter().map(|c| c.1.max.x).fold(0.0, f32::max);
                let left = cells.iter().map(|c| c.1.min.x).fold(f32::MAX, f32::min);
                assert!(right - left <= width + 0.5, "dark={dark} width={width}");
            }
        }
    }

    fn many_options_app() -> AppState {
        use hf2ollama_core::fit::Machine;
        use hf2ollama_core::model::RepoFile;
        let id = "o/many-GGUF";
        let mut files: Vec<RepoFile> = (1..=25)
            .map(|i| RepoFile {
                path: format!("many-IQ{i}_XS.gguf"),
                size: 10_000_000_000 + i as u64 * 100_000_000,
                sha256: Some(format!("s{i}")),
                blob_id: None,
            })
            .collect();
        files.push(RepoFile {
            path: "many-Q4_K_M.gguf".into(),
            size: 4_680_000_000,
            sha256: Some("q4".into()),
            blob_id: None,
        });
        let mut app = AppState::new();
        app.machine = Machine {
            ram_gb: 64.0,
            vram_gb: Some(24.0),
        };
        app.browse = crate::browse::state::BrowseState::new(app.machine.clone());
        app.ollama = Some(Some("0.9".into()));
        app.browse.on_model_info(
            id,
            Ok(ModelInfo {
                id: id.into(),
                files,
                arch: Some("llama".into()),
                params: Some("7B".into()),
                context: None,
                license: None,
                base_model: None,
                downloads: 1,
                likes: 1,
                updated: String::new(),
                gated: false,
            }),
        );
        app.browse.select_repo(id);
        app
    }

    #[test]
    fn recommended_row_is_scrolled_into_view_after_the_first_frames() {
        use crate::browse::test_util::Harness;
        for dark in [false, true] {
            let mut app = many_options_app();
            let mut h = Harness::new(dark);
            for _ in 0..6 {
                h.frame(vec2(1180.0, 820.0), vec![], |ui| {
                    show(ui, &mut app, &mut Vec::new())
                });
            }
            let row = app.browse.probe.selected_row.expect("selected row painted");
            let view = app.browse.probe.viewport.expect("viewport recorded");
            assert!(
                row.top() >= view.top() - 0.5 && row.bottom() <= view.bottom() + 0.5,
                "dark={dark}: row {row:?} outside viewport {view:?}"
            );
        }
    }

    #[test]
    fn import_card_is_pinned_and_does_not_move_when_scrolling() {
        use crate::browse::test_util::Harness;
        for dark in [false, true] {
            let mut app = many_options_app();
            let mut h = Harness::new(dark);
            let size = vec2(1180.0, 820.0);
            for _ in 0..6 {
                h.frame(size, vec![], |ui| show(ui, &mut app, &mut Vec::new()));
            }
            let before = app.browse.probe.card.expect("card painted");
            let start = app.browse.probe.scroll_offset;
            assert!(
                (820.0 - before.bottom() - 13.0).abs() < 2.5,
                "dark={dark}: card bottom {} not pinned near the pane bottom",
                before.bottom()
            );
            let wheel = vec![
                egui::Event::PointerMoved(egui::pos2(600.0, 300.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 400.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                },
            ];
            h.frame(size, wheel, |ui| show(ui, &mut app, &mut Vec::new()));
            for _ in 0..12 {
                h.frame(size, vec![], |ui| show(ui, &mut app, &mut Vec::new()));
            }
            let after = app.browse.probe.card.expect("card painted");
            assert!(
                (app.browse.probe.scroll_offset - start).abs() > 5.0,
                "dark={dark}: scroll offset did not change ({start})"
            );
            assert_eq!(before, after, "dark={dark}: card moved while scrolling");
        }
    }
}
