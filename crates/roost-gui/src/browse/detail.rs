//! The detail pane for the selected repository.

use super::about;
use super::options::{gb, OptionRow};
use super::plan_card;
use super::results::{compact_count, format_size, relative_age};
use super::state::{is_gated_error, PlanKind, SearchStatus, GATED_MESSAGE, GATED_REPO_WARNING};
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{
    band, caption, fit_pill_kind, fit_pill_label, fit_text, pill_colors, small_button,
};
use eframe::egui::{
    self, vec2, Align2, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind,
};
use roost_core::fit::{fit, Machine};
use roost_core::hf::ModelInfo;
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
                        .layout_no_wrap(k.to_uppercase(), semibold(11.0), t.band_ink2())
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
pub(super) fn callout(ui: &mut egui::Ui, warn: bool, text: &str) -> Rect {
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
        })
        .response
        .rect
}

/// What the options table shows and how it behaves.
#[derive(Clone, Copy)]
pub struct TableSpec<'a> {
    /// Shown in the sticky bar when the header has scrolled away.
    pub title: &'a str,
    pub first_col: &'a str,
    /// The section caption above the table.
    pub label: Option<Rect>,
    pub selected: Option<usize>,
    pub scroll_to_selected: bool,
}

/// Where the options table painted its parts (read by the layout tests).
#[derive(Default)]
pub struct TableOut {
    pub selected: Option<Rect>,
    pub header: Option<Rect>,
}

/// Height of the header bar that sticks to the top of the scroll area.
const STICKY_H: f32 = 30.0;
const TABLE_PAD: f32 = 14.0;
const BAR_W: f32 = 56.0;
const NOTE_MIN: f32 = 80.0;

/// The hand-painted options table. Returns the clicked row, if any. Wide
/// panes get one line per row; narrow ones put the quality and fit on a
/// second line so nothing is clipped.
pub fn options_table(
    ui: &mut egui::Ui,
    spec: &TableSpec,
    rows: &[OptionRow],
    machine: &Machine,
    out: &mut TableOut,
) -> Option<usize> {
    let TableSpec {
        title,
        first_col,
        label,
        selected,
        scroll_to_selected,
    } = *spec;
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

    let paint_header_labels = |ui: &egui::Ui, left: f32, cy: f32| {
        let size_right = TABLE_PAD + name_w + TABLE_PAD + size_w;
        let q_x = size_right + TABLE_PAD;
        let pill_x = w - TABLE_PAD - pill_w;
        for (x, text, right) in [
            (TABLE_PAD, first_col, false),
            (size_right, "SIZE", true),
            (q_x, "QUALITY", false),
            (pill_x, "ON YOUR MACHINE", false),
        ] {
            ui.painter().text(
                egui::pos2(left + x, cy),
                if right {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                text.to_uppercase(),
                semibold(11.0),
                t.fg3,
            );
        }
    };

    let mut clicked = None;
    let mut top = None;
    let mut bottom = 0.0;
    ui.spacing_mut().item_spacing.y = 0.0;
    if wide {
        let (head, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        top = Some(head.min);
        bottom = head.max.y;
        out.header = Some(head);
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
        paint_header_labels(ui, head.min.x, head.center().y);
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
            out.selected = Some(row);
            if scroll_to_selected {
                // Show the section label, header and the row together when
                // they fit the viewport; otherwise centre the row (the
                // header is then kept on screen by the sticky bar).
                let view = ui.clip_rect();
                let from = label.map_or(top.map_or(row.top(), |p| p.y), |l| l.top());
                let block = Rect::from_min_max(egui::pos2(row.left(), from), row.right_bottom());
                if block.height() <= view.height() {
                    if !view.contains_rect(block) {
                        ui.scroll_to_rect(block, Some(egui::Align::Center));
                    }
                } else if !view.contains_rect(row) {
                    ui.scroll_to_rect(row, Some(egui::Align::Center));
                }
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
        // Keep the header on screen while its rows scroll under it.
        let view = ui.clip_rect();
        if top.y < view.top() - 0.5 && bottom > view.top() + STICKY_H {
            let bar = Rect::from_min_size(egui::pos2(top.x, view.top()), vec2(w, STICKY_H));
            ui.painter().rect_filled(bar, 0.0, t.surface2);
            ui.painter()
                .hline(bar.x_range(), bar.bottom(), Stroke::new(1.0, t.line));
            if wide {
                paint_header_labels(ui, bar.min.x, bar.center().y);
            } else {
                ui.painter().text(
                    egui::pos2(bar.min.x + TABLE_PAD, bar.center().y),
                    Align2::LEFT_CENTER,
                    title.to_uppercase(),
                    semibold(11.0),
                    t.fg3,
                );
            }
            out.header = Some(bar);
        }
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

/// Smallest height the scrolling page keeps at any window size the app
/// allows (see `MIN_WINDOW` in main.rs).
#[cfg(test)]
pub(crate) const MIN_SCROLL_H: f32 = 108.0;
/// Smallest the pinned card may get: caption, name field and button.
const CARD_MIN_H: f32 = 132.0;
/// Most of the pane the pinned card may take.
const CARD_MAX_SHARE: f32 = 0.6;

/// The page for the selected repository: three stacked bands (the header with
/// its notices, the files to choose from, the description) in one scrolling
/// column. The import card is pinned below, and only once a file has been
/// chosen, so the primary action is always in view while you read.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    let selected = app.browse.selected_id().map(str::to_string);
    let card_visible = selected.as_deref().is_some_and(|id| {
        matches!(app.browse.info_for(id), Some(Ok(_))) && app.browse.selected_option().is_some()
    });
    let pane_h = ui.available_height();
    if card_visible {
        let max_h = (pane_h * CARD_MAX_SHARE).max(CARD_MIN_H);
        let card_panel = egui::Panel::bottom("plan_card")
            .resizable(false)
            .max_size(max_h)
            .frame(
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(Stroke::new(1.0, t.line))
                    .inner_margin(Margin {
                        left: 24,
                        right: 24,
                        top: 8,
                        bottom: 8,
                    }),
            )
            .show(ui, |ui| plan_card::show(ui, app, out, max_h - 18.0));
        #[cfg(test)]
        {
            app.browse.probe.card_panel = Some(card_panel.response.rect);
        }
        #[cfg(not(test))]
        let _ = card_panel;
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.surface))
        .show(ui, |ui| {
            let Some(id) = selected else {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Pick a model from the list to see its files and import it.")
                            .color(t.fg3),
                    );
                });
                return;
            };
            #[cfg(test)]
            {
                app.browse.probe.pane = Some(ui.max_rect());
                app.browse.probe.notices.clear();
            }
            // Measured outside the scroll area, where it is the real height.
            let view_h = ui.available_height();
            let scroll = egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let info = app.browse.info_for(&id).cloned();
                    let (_, top) = band(ui, t.head_band(), Some(t.line), 0.0, |ui| {
                        header(
                            ui,
                            &id,
                            info.as_ref().and_then(|r| r.as_ref().ok()).map(|i| &**i),
                        );
                        notices(ui, app, &id, info.as_ref());
                        if let Some(Ok(i)) = &info {
                            #[cfg(test)]
                            let strip_top = ui.cursor().top();
                            meta_strip(ui, &metadata(i));
                            #[cfg(test)]
                            {
                                app.browse.probe.meta_strip = Some(Rect::from_min_max(
                                    egui::pos2(ui.max_rect().left(), strip_top),
                                    egui::pos2(ui.max_rect().right(), ui.cursor().top()),
                                ));
                            }
                        }
                    });
                    let (_, mid) = band(ui, t.surface2, Some(t.line), 0.0, |ui| match &info {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading the file list…").color(t.fg3));
                            });
                        }
                        Some(Err(_)) => {
                            ui.label(RichText::new("No files to show.").color(t.fg3));
                        }
                        Some(Ok(i)) => files(ui, app, i),
                    });
                    let rest = (view_h - top.height() - mid.height() - 32.0).max(0.0);
                    band(ui, t.surface, None, rest, |ui| {
                        if let Some(Ok(i)) = &info {
                            about::show(ui, app, i, &i.id);
                        }
                    });
                    #[cfg(test)]
                    {
                        app.browse.probe.bands = vec![top, mid];
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

/// Messages that carry the key instruction for a state (gated repo, token
/// error, unsupported reason), inside the header band.
fn notices(
    ui: &mut egui::Ui,
    app: &mut AppState,
    id: &str,
    info: Option<&Result<std::sync::Arc<ModelInfo>, String>>,
) {
    let unsupported = match app.browse.plan().map(|p| &p.kind) {
        Some(PlanKind::Unsupported(reason)) => Some(reason.clone()),
        _ => None,
    };
    let mut shown = Vec::new();
    match info {
        Some(Err(e)) => {
            let text = if is_gated_error(e) {
                GATED_MESSAGE.to_string()
            } else {
                crate::browse::state::friendly_error(e)
            };
            shown.push(callout(ui, true, &text));
            if small_button(ui, "Retry") {
                app.browse.retry_info(id);
            }
        }
        Some(Ok(_)) => {
            if app.browse.selected_info_gated() {
                shown.push(callout(ui, true, GATED_REPO_WARNING));
            }
            if let Some(reason) = &unsupported {
                shown.push(callout(
                    ui,
                    true,
                    &format!("Ollama cannot import this repository. {reason}"),
                ));
            }
        }
        None => {}
    }
    #[cfg(test)]
    {
        app.browse.probe.notices = shown;
    }
    #[cfg(not(test))]
    let _ = shown;
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
        ui.label(RichText::new(line).color(t.band_ink2()));
        ui.hyperlink_to(
            RichText::new("Open on huggingface.co ↗").size(12.0),
            format!("https://huggingface.co/{id}"),
        );
    });
    ui.add_space(2.0);
}

/// The files (or quantizations) to choose from.
fn files(ui: &mut egui::Ui, app: &mut AppState, _info: &ModelInfo) {
    let Some(plan) = app.browse.plan().cloned() else {
        return;
    };
    let selected = app.browse.selected_option();
    let mut table_out = TableOut::default();
    let mut clicked = None;
    match &plan.kind {
        PlanKind::Gguf(_) => {
            let label = caption(ui, "Choose a file");
            let label_rect = label.rect;
            #[cfg(test)]
            {
                app.browse.probe.table_label = Some(label.rect);
            }
            #[cfg(not(test))]
            let _ = label;
            clicked = options_table(
                ui,
                &TableSpec {
                    title: "Choose a file",
                    first_col: "File",
                    label: Some(label_rect),
                    selected,
                    scroll_to_selected: false,
                },
                &plan.rows,
                &app.machine,
                &mut table_out,
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
            let label = caption(ui, "Choose a quantization");
            let label_rect = label.rect;
            #[cfg(test)]
            {
                app.browse.probe.table_label = Some(label.rect);
            }
            #[cfg(not(test))]
            let _ = label;
            clicked = options_table(
                ui,
                &TableSpec {
                    title: "Choose a quantization",
                    first_col: "Quantization",
                    label: Some(label_rect),
                    selected,
                    scroll_to_selected: false,
                },
                &plan.rows,
                &app.machine,
                &mut table_out,
            );
        }
        PlanKind::Unsupported(reason) => {
            let _ = reason;
            related(ui, app);
        }
    }
    #[cfg(test)]
    {
        app.browse.probe.selected_row = table_out.selected;
        app.browse.probe.table_header = table_out.header;
    }
    #[cfg(not(test))]
    let _ = table_out;
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

    fn many_options_app(gated: bool) -> AppState {
        use roost_core::fit::Machine;
        use roost_core::model::RepoFile;
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
                gated,
                pipeline_tag: None,
                languages: Vec::new(),
                tags: Vec::new(),
            }),
        );
        app.browse.select_repo(id);
        app.browse.choose_recommended();
        app
    }

    #[test]
    fn no_file_is_chosen_and_there_is_no_import_card_until_a_row_is_clicked() {
        use crate::browse::test_util::Harness;
        for dark in [false, true] {
            let mut app = many_options_app(false);
            app.browse.unchoose();
            let mut h = Harness::new(dark);
            for _ in 0..6 {
                h.frame(vec2(1180.0, 820.0), vec![], |ui| {
                    show(ui, &mut app, &mut Vec::new())
                });
            }
            assert!(app.browse.selected_option().is_none());
            assert!(
                app.browse.probe.card_panel.is_none(),
                "dark={dark}: card shown"
            );
            assert!(app.browse.probe.card.is_none());
            app.browse.choose_recommended();
            for _ in 0..6 {
                h.frame(vec2(1180.0, 820.0), vec![], |ui| {
                    show(ui, &mut app, &mut Vec::new())
                });
            }
            assert!(
                app.browse.probe.card_panel.is_some(),
                "dark={dark}: card missing"
            );
        }
    }

    #[test]
    fn import_card_is_pinned_and_does_not_move_when_scrolling() {
        use crate::browse::test_util::Harness;
        for dark in [false, true] {
            let mut app = many_options_app(false);
            let mut h = Harness::new(dark);
            let size = vec2(1180.0, 820.0);
            for _ in 0..6 {
                h.frame(size, vec![], |ui| show(ui, &mut app, &mut Vec::new()));
            }
            let before = app.browse.probe.card.expect("card painted");
            let start = app.browse.probe.scroll_offset;
            assert!(
                (820.0 - before.bottom() - 9.0).abs() < 2.5,
                "dark={dark}: card bottom {} not pinned near the pane bottom",
                before.bottom()
            );
            let wheel = vec![
                egui::Event::PointerMoved(egui::pos2(600.0, 300.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    // Content moves up (the page scrolls down) from the top.
                    delta: vec2(0.0, if start > 5.0 { 400.0 } else { -400.0 }),
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

    #[test]
    fn a_gated_warning_sits_in_the_header_band_at_the_top_of_the_page() {
        use crate::browse::test_util::Harness;
        for dark in [false, true] {
            let mut app = many_options_app(true);
            let mut h = Harness::new(dark);
            let size = vec2(1180.0, 820.0);
            for _ in 0..8 {
                h.frame(size, vec![], |ui| show(ui, &mut app, &mut Vec::new()));
            }
            let notices = app.browse.probe.notices.clone();
            let pane = app.browse.probe.pane.expect("pane recorded");
            let header = app.browse.probe.bands[0];
            assert!(!notices.is_empty(), "dark={dark}: no notice painted");
            for n in &notices {
                assert!(
                    n.top() >= pane.top() - 0.5 && n.bottom() <= header.bottom() + 0.5,
                    "dark={dark}: notice {n:?} not inside the header band {header:?}"
                );
            }
        }
    }

    // ---- full-window layout tests (top bar, bars and dock included) ----

    use crate::app::App;
    use crate::browse::test_util::Harness;
    use roost_core::config::Settings;
    use roost_core::model::RepoFile;

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        ManyGated,
        Many,
        Unsupported,
        Forbidden,
        /// Safetensors with quantize chips and Ollama unreachable (longest card).
        SafetensorsUnreachable,
    }

    const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

    fn rf(path: &str, size: u64) -> RepoFile {
        RepoFile {
            path: path.into(),
            size,
            sha256: Some(format!("sha-{path}")),
            blob_id: None,
        }
    }

    fn info_for(id: &str, files: Vec<RepoFile>, arch: Option<&str>, gated: bool) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            files,
            arch: arch.map(str::to_string),
            params: Some("7B".into()),
            context: Some(32768),
            license: Some("apache-2.0".into()),
            base_model: Some("mistralai/Mistral-7B-v0.3".into()),
            downloads: 1_000,
            likes: 10,
            updated: "2026-01-01T00:00:00.000Z".into(),
            gated,
            pipeline_tag: None,
            languages: Vec::new(),
            tags: Vec::new(),
        }
    }

    fn app_for(kind: Kind) -> App {
        use roost_core::fit::Machine;
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let machine = Machine {
            ram_gb: 64.0,
            vram_gb: Some(24.0),
        };
        let mut app = App::new(
            &Settings::default(),
            std::path::PathBuf::from("/tmp/staging"),
            machine.clone(),
            tx,
            rx,
        );
        let st = app.state_mut();
        st.ollama = Some(Some("0.9".into()));
        let id = "o/repo";
        let many = || {
            let mut files: Vec<RepoFile> = (1..=25)
                .map(|i| {
                    rf(
                        &format!("m-IQ{i}_XS.gguf"),
                        10_000_000_000 + i as u64 * 100_000_000,
                    )
                })
                .collect();
            files.push(rf("m-Q4_K_M.gguf", 4_680_000_000));
            files
        };
        match kind {
            Kind::ManyGated => st
                .browse
                .on_model_info(id, Ok(info_for(id, many(), Some("llama"), true))),
            Kind::Many => st
                .browse
                .on_model_info(id, Ok(info_for(id, many(), Some("llama"), false))),
            Kind::Unsupported => st.browse.on_model_info(
                id,
                Ok(info_for(
                    id,
                    vec![rf("pytorch_model.bin", 1_000_000_000)],
                    None,
                    false,
                )),
            ),
            Kind::Forbidden => st
                .browse
                .on_model_info(id, Err("HTTP 403: Forbidden".into())),
            Kind::SafetensorsUnreachable => {
                st.ollama = Some(None);
                st.browse.on_model_info(
                    id,
                    Ok(info_for(
                        id,
                        vec![
                            rf("config.json", 600),
                            rf("tokenizer.json", 3_000_000),
                            rf("model.safetensors", 14_000_000_000),
                        ],
                        Some("MistralForCausalLM"),
                        false,
                    )),
                );
            }
        }
        st.browse.select_repo(id);
        st.browse.choose_recommended();
        app
    }

    fn laid_out(kind: Kind, dark: bool, (w, h): (f32, f32)) -> App {
        let mut app = app_for(kind);
        let mut harness = Harness::new(dark);
        for _ in 0..10 {
            harness.frame(vec2(w, h), vec![], |ui| app.show(ui));
        }
        app
    }

    fn inside(inner: Rect, outer: Rect) -> bool {
        inner.top() >= outer.top() - 0.5
            && inner.bottom() <= outer.bottom() + 0.5
            && inner.left() >= outer.left() - 0.5
            && inner.right() <= outer.right() + 0.5
    }

    const ALL_KINDS: [Kind; 5] = [
        Kind::ManyGated,
        Kind::Many,
        Kind::Unsupported,
        Kind::Forbidden,
        Kind::SafetensorsUnreachable,
    ];

    #[test]
    fn scroll_area_keeps_min_height_at_minimum_window_size() {
        for kind in ALL_KINDS {
            for size in SIZES {
                for dark in [false, true] {
                    let mut app = laid_out(kind, dark, size);
                    let view = app.state_mut().browse.probe.viewport.expect("viewport");
                    assert!(
                        view.height() >= MIN_SCROLL_H - 0.5,
                        "{kind:?} {size:?} dark={dark}: scroll area only {} px",
                        view.height()
                    );
                }
            }
        }
    }

    #[test]
    fn card_name_field_and_button_are_fully_visible() {
        for kind in [Kind::SafetensorsUnreachable, Kind::ManyGated, Kind::Many] {
            for size in SIZES {
                for dark in [false, true] {
                    let mut app = laid_out(kind, dark, size);
                    let p = &app.state_mut().browse.probe;
                    let panel = p.card_panel.expect("card panel");
                    let field = p.name_field.expect("name field");
                    let button = p.button.expect("button");
                    let window = Rect::from_min_size(egui::Pos2::ZERO, vec2(size.0, size.1));
                    assert!(
                        inside(field, panel) && inside(button, panel) && inside(panel, window),
                        "{kind:?} {size:?} dark={dark}: field {field:?} button {button:?} panel {panel:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn import_steps_are_one_short_line_at_every_window_size() {
        for size in SIZES {
            for dark in [false, true] {
                let mut app = laid_out(Kind::SafetensorsUnreachable, dark, size);
                let line = app.state_mut().browse.probe.chips.clone();
                assert_eq!(line.len(), 1, "{size:?} dark={dark}");
                assert!(
                    line[0].height() < 20.0,
                    "{size:?} dark={dark}: wrapped {line:?}"
                );
            }
        }
    }

    #[test]
    fn card_shows_chips_and_sizes_when_the_window_has_room() {
        for dark in [false, true] {
            let mut app = laid_out(Kind::SafetensorsUnreachable, dark, (1180.0, 820.0));
            let card = app.state_mut().browse.probe.card.expect("card");
            // Name field and button alone are about 80 px; chips, the size
            // line and the disabled reason add another ~70 px.
            assert!(
                card.height() > 125.0,
                "dark={dark}: card only {} px tall",
                card.height()
            );
        }
    }
}
