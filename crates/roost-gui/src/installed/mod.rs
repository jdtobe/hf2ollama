//! The Installed models view: usage bar, filter and sort tools, a table with
//! expandable details and a two-click delete.

pub mod state;

#[cfg(test)]
mod layout_tests;

use crate::app::{bad, AppState};
use crate::browse::results::format_size;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, tokens, Tokens};
use crate::widgets::{ellipsize_middle, small_button_response};
use eframe::egui::{
    self, vec2, Align, Align2, Color32, FontId, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke,
};
use state::{
    column_spans, relative_time, run_command, short_digest, thousands, visible_columns, Column,
    DetailsState, InstalledRow, InstalledState, ListStatus, SortKey, SourceFilter,
};
use std::time::{Instant, SystemTime};

/// Height of a table row.
pub const ROW_H: f32 = 40.0;
const HEADER_H: f32 = 26.0;
const BUTTON_H: f32 = 30.0;
const BUTTON_GAP: f32 = 6.0;
const TOOLS_H: f32 = 30.0;
const FILTER_W: f32 = 180.0;
const SOURCE_W: f32 = 150.0;
const SORT_W: f32 = 130.0;
const TOOL_GAP: f32 = 8.0;
const REFRESH_W: f32 = 70.0;
const TOOLS_W: f32 = FILTER_W + SOURCE_W + SORT_W + REFRESH_W + 3.0 * TOOL_GAP;
const TEXT_PX: f32 = 12.5;
const LABEL_W: f32 = 96.0;

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct InstalledProbe {
    /// The whole view body.
    pub panel: Option<Rect>,
    /// Visible area of the scrolling table rows.
    pub viewport: Option<Rect>,
    /// Columns the table painted.
    pub columns: Vec<Column>,
    pub header_cells: Vec<(Column, Rect)>,
    pub rows: Vec<RowProbe>,
    pub toast: Option<Rect>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct RowProbe {
    pub name: String,
    pub rect: Rect,
    /// Painted content (text or widget) of each column.
    pub cells: Vec<(Column, Rect)>,
    pub details_btn: Rect,
    pub delete_btn: Rect,
    pub delete_id: egui::Id,
}

/// View state that is not part of the pure model: the test probe.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InstalledUi {
    pub probe: InstalledProbe,
}

/// What the table needs from the app besides the installed state.
struct Ctx<'a> {
    host: &'a str,
    dark: bool,
    t: Tokens,
    now: Instant,
}

/// The view body.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let now = Instant::now();
    if let Some(due) = app.installed.tick(now) {
        ui.ctx()
            .request_repaint_after(due.saturating_duration_since(now));
    }
    let t = tokens(ui);
    let dark = ui.visuals().dark_mode;
    #[cfg(test)]
    {
        app.installed_ui.probe = InstalledProbe::default();
    }
    let host = app.ollama_host.clone();
    let cx = Ctx {
        host: &host,
        dark,
        t,
        now,
    };
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .inner_margin(Margin::symmetric(24, 20)),
        )
        .show(ui, |ui| {
            let panel = ui.max_rect();
            #[cfg(test)]
            {
                app.installed_ui.probe.panel = Some(panel);
            }
            header(ui, &mut app.installed, &cx, out);
            ui.add_space(10.0);
            if app.installed.status == ListStatus::Loaded && !app.installed.rows().is_empty() {
                usage(ui, &app.installed, &cx);
                ui.add_space(10.0);
            }
            body(ui, app, &cx, out);
            toast(ui, app, &cx, panel);
        });
}

fn sub_line(state: &InstalledState, host: &str) -> String {
    if state.status != ListStatus::Loaded {
        return format!("Read from Ollama at {host}");
    }
    let n = state.rows().len();
    let total = state
        .rows()
        .iter()
        .fold(0u64, |t, r| t.saturating_add(r.model.size));
    format!(
        "{n} model{} · {} · read from Ollama at {host}",
        if n == 1 { "" } else { "s" },
        format_size(total)
    )
}

fn header(ui: &mut egui::Ui, state: &mut InstalledState, cx: &Ctx, out: &mut Vec<BackendRequest>) {
    let sub = sub_line(state, cx.host);
    let sub_w = ui
        .painter()
        .layout_no_wrap(sub.clone(), FontId::proportional(13.0), cx.t.fg3)
        .size()
        .x;
    let title_w = sub_w.max(190.0);
    let side_by_side = ui.available_width() >= title_w + 16.0 + TOOLS_W;
    let title = |ui: &mut egui::Ui| {
        ui.vertical(|ui| {
            ui.heading("Installed models");
            ui.label(RichText::new(&sub).size(13.0).color(cx.t.fg3));
        });
    };
    if side_by_side {
        ui.horizontal(|ui| {
            title(ui);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.allocate_ui_with_layout(
                    vec2(TOOLS_W, TOOLS_H),
                    Layout::left_to_right(Align::Center),
                    |ui| tools(ui, state, out),
                );
            });
        });
    } else {
        title(ui);
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width().min(TOOLS_W), TOOLS_H),
            Layout::left_to_right(Align::Center),
            |ui| tools(ui, state, out),
        );
    }
}

fn tools(ui: &mut egui::Ui, state: &mut InstalledState, out: &mut Vec<BackendRequest>) {
    ui.spacing_mut().item_spacing.x = TOOL_GAP;
    ui.add_sized(
        vec2(FILTER_W, 28.0),
        egui::TextEdit::singleline(&mut state.filter_text)
            .hint_text("Filter installed")
            .margin(Margin::symmetric(10, 6)),
    );
    let source_label = match state.source {
        SourceFilter::All => "All sources",
        SourceFilter::Imported => "Imported",
        SourceFilter::Library => "Ollama library",
    };
    egui::ComboBox::from_id_salt("installed-source")
        .selected_text(source_label)
        .width(SOURCE_W - 20.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.source, SourceFilter::All, "All sources");
            ui.selectable_value(
                &mut state.source,
                SourceFilter::Imported,
                "Imported with hf2ollama",
            );
            ui.selectable_value(
                &mut state.source,
                SourceFilter::Library,
                "Pulled from Ollama",
            );
        });
    let sort_label = match state.sort {
        SortKey::Size => "Largest first",
        SortKey::Name => "Name",
        SortKey::Modified => "Recently modified",
    };
    egui::ComboBox::from_id_salt("installed-sort")
        .selected_text(sort_label)
        .width(SORT_W - 20.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.sort, SortKey::Size, "Largest first");
            ui.selectable_value(&mut state.sort, SortKey::Name, "Name");
            ui.selectable_value(&mut state.sort, SortKey::Modified, "Recently modified");
        });
    let r = ui.add_sized(
        vec2(REFRESH_W, 28.0),
        egui::Button::new(RichText::new("Refresh").size(12.0)),
    );
    if r.clicked() {
        state.begin_load();
        out.push(BackendRequest::ListInstalled);
    }
}

fn usage(ui: &mut egui::Ui, state: &InstalledState, cx: &Ctx) {
    let split = state.usage();
    let total = split.imported.saturating_add(split.library);
    let width = ui.available_width();
    let (bar, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(bar, 4.0, cx.t.line);
    if total > 0 {
        let cut = bar.left() + width * (split.imported as f32 / total as f32);
        let mut left = bar;
        left.set_right(cut);
        let mut right = bar;
        right.set_left(cut);
        if split.imported > 0 {
            p.rect_filled(left, 4.0, cx.t.accent);
        }
        if split.library > 0 {
            p.rect_filled(right, 4.0, cx.t.fg3);
        }
    }
    ui.add_space(6.0);
    let imported = state
        .rows()
        .iter()
        .filter(|r| r.source_repo.is_some())
        .count();
    let library = state.rows().len() - imported;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (colour, label, bytes, n) in [
            (
                cx.t.accent,
                "Imported by hf2ollama",
                split.imported,
                imported,
            ),
            (cx.t.fg3, "Pulled from Ollama", split.library, library),
        ] {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, colour);
            ui.add_space(-12.0);
            ui.label(
                RichText::new(format!("{label}  {} ({n})", format_size(bytes)))
                    .size(12.0)
                    .color(cx.t.fg2),
            );
        }
    });
}

fn message(ui: &mut egui::Ui, cx: &Ctx, text: &str) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).size(14.0).color(cx.t.fg3));
    });
}

fn body(ui: &mut egui::Ui, app: &mut AppState, cx: &Ctx, out: &mut Vec<BackendRequest>) {
    match app.installed.status.clone() {
        ListStatus::Loading => message(ui, cx, "Loading installed models…"),
        ListStatus::Error(e) => {
            message(ui, cx, &format!("Could not read the installed models. {e}"));
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                if small_button_response(ui, "Retry").clicked() {
                    app.installed.begin_load();
                    out.push(BackendRequest::ListInstalled);
                }
            });
        }
        ListStatus::Loaded if app.installed.rows().is_empty() => {
            message(ui, cx, "No models installed yet. Import one from Browse.");
        }
        ListStatus::Loaded => {
            if app.installed.visible().is_empty() {
                message(ui, cx, "No installed models match this filter.");
            } else {
                table(ui, app, cx, out);
            }
        }
    }
}

/// Width of the widest of `texts` in the 12 pt button font, plus padding.
fn button_w(ui: &egui::Ui, texts: &[&str]) -> f32 {
    texts
        .iter()
        .map(|t| {
            ui.painter()
                .layout_no_wrap(t.to_string(), FontId::proportional(12.0), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + 2.0 * ui.spacing().button_padding.x
        + 4.0
}

fn caption_text(c: Column) -> &'static str {
    match c {
        Column::Model => "MODEL",
        Column::Params => "PARAMS",
        Column::Quant => "QUANT",
        Column::Size => "SIZE",
        Column::Source => "SOURCE",
        Column::Modified => "MODIFIED",
        Column::Actions => "",
    }
}

fn table(ui: &mut egui::Ui, app: &mut AppState, cx: &Ctx, out: &mut Vec<BackendRequest>) {
    let width = ui.available_width();
    let cols = visible_columns(width);
    let spans = column_spans(&cols, width);
    let details_w = button_w(ui, &["Details", "Hide"]);
    let delete_w = button_w(ui, &["Delete", "Confirm delete"]);
    #[cfg(test)]
    {
        app.installed_ui.probe.columns = cols.clone();
    }

    // Header.
    let (head, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    for (c, l, r) in &spans {
        let galley =
            ui.painter()
                .layout_no_wrap(caption_text(*c).to_string(), semibold(11.0), cx.t.fg3);
        let x = if *c == Column::Size {
            head.left() + r - galley.size().x
        } else {
            head.left() + l
        };
        let pos = Pos2::new(x, head.center().y - galley.size().y / 2.0);
        #[cfg(test)]
        app.installed_ui
            .probe
            .header_cells
            .push((*c, Rect::from_min_size(pos, galley.size())));
        ui.painter().galley(pos, galley, cx.t.fg3);
    }
    ui.painter().line_segment(
        [head.left_bottom(), head.right_bottom()],
        Stroke::new(1.0, cx.t.line),
    );

    let rows: Vec<InstalledRow> = app.installed.visible().into_iter().cloned().collect();
    let now_sys = SystemTime::now();
    let area = egui::ScrollArea::vertical()
        .id_salt("installed-table")
        .auto_shrink([false, false])
        .max_height(ui.available_height());
    let shown = area.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &rows {
            ui.push_id(row.name(), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
                let expanded = app.installed.is_expanded(row.name());
                let armed = app.installed.is_armed(row.name(), cx.now);
                let ready = app.installed.confirm_ready(row.name(), cx.now);
                let probe = if ui.is_rect_visible(rect) {
                    paint_row(
                        ui,
                        row,
                        rect,
                        &spans,
                        (details_w, delete_w),
                        (expanded, armed, ready),
                        cx,
                        now_sys,
                    )
                } else {
                    None
                };
                if let Some(p) = probe {
                    if p.details.clicked() {
                        if let Some(n) = app.installed.toggle_expanded(row.name()) {
                            out.push(BackendRequest::ShowInstalled(n));
                        }
                    }
                    // A held Enter or Space repeats `clicked()`; only a fresh
                    // activation may arm or confirm.
                    let held = crate::confirm::activation_key_held(ui);
                    if p.delete.clicked() && !held {
                        if let Some(a) = app.installed.click_delete(row.name(), cx.now) {
                            out.push(BackendRequest::DeleteInstalled(a.name));
                        }
                    }
                    #[cfg(test)]
                    app.installed_ui.probe.rows.push(RowProbe {
                        name: row.name().to_string(),
                        rect,
                        cells: p.cells,
                        details_btn: p.details.rect,
                        delete_btn: p.delete.rect,
                        delete_id: p.delete.id,
                    });
                }
                if app.installed.is_expanded(row.name()) {
                    details(ui, app, row, width, cx);
                }
            });
        }
    });
    #[cfg(test)]
    {
        app.installed_ui.probe.viewport = Some(shown.inner_rect);
    }
    let _ = shown;
}

struct Painted {
    details: egui::Response,
    delete: egui::Response,
    #[cfg(test)]
    cells: Vec<(Column, Rect)>,
    #[cfg(not(test))]
    _unused: (),
}

#[allow(clippy::too_many_arguments)]
fn paint_row(
    ui: &mut egui::Ui,
    row: &InstalledRow,
    rect: Rect,
    spans: &[(Column, f32, f32)],
    (details_w, delete_w): (f32, f32),
    (expanded, armed, ready): (bool, bool, bool),
    cx: &Ctx,
    now: SystemTime,
) -> Option<Painted> {
    let t = &cx.t;
    if expanded {
        ui.painter().rect_filled(rect, 0.0, t.surface2);
    }
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, t.line),
    );
    let cy = rect.center().y;
    let mut cells: Vec<(Column, Rect)> = Vec::new();
    let mut buttons = None;
    for (c, l, r) in spans {
        let (left, right) = (rect.left() + l, rect.left() + r);
        let col = Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(right, rect.bottom()));
        let text_at =
            |ui: &egui::Ui, text: String, font: FontId, color: Color32, right_align: bool| {
                let galley = ui.painter().layout_no_wrap(text, font, color);
                let x = if right_align {
                    right - galley.size().x
                } else {
                    left
                };
                let pos = Pos2::new(x, cy - galley.size().y / 2.0);
                let cell = Rect::from_min_size(pos, galley.size());
                ui.painter().with_clip_rect(col).galley(pos, galley, color);
                cell
            };
        let cell = match c {
            Column::Model => {
                let shown = ellipsize_middle(ui, row.name(), mono(TEXT_PX), r - l);
                let cell = text_at(ui, shown.clone(), mono(TEXT_PX), t.fg, false);
                if shown != row.name() {
                    ui.interact(cell, ui.scope_id().with("name"), Sense::hover())
                        .on_hover_text(row.name());
                }
                cell
            }
            Column::Params => text_at(
                ui,
                row.model.params.clone().unwrap_or_else(|| "—".into()),
                FontId::proportional(TEXT_PX),
                t.fg2,
                false,
            ),
            Column::Quant => text_at(
                ui,
                row.model.quant.clone().unwrap_or_else(|| "—".into()),
                mono(11.5),
                t.fg2,
                false,
            ),
            Column::Size => text_at(ui, format_size(row.model.size), mono(TEXT_PX), t.fg, true),
            Column::Source => match (&row.source_repo, row.source_url()) {
                (Some(repo), Some(url)) => {
                    let shown = ellipsize_middle(ui, repo, FontId::proportional(TEXT_PX), r - l);
                    let galley = ui.painter().layout_no_wrap(
                        shown.clone(),
                        FontId::proportional(TEXT_PX),
                        t.accent,
                    );
                    let cell = Rect::from_min_size(
                        Pos2::new(left, cy - galley.size().y / 2.0),
                        galley.size(),
                    );
                    let resp = ui.put(
                        cell,
                        egui::Hyperlink::from_label_and_url(
                            RichText::new(shown.as_str()).size(TEXT_PX),
                            url,
                        ),
                    );
                    if shown != *repo {
                        resp.on_hover_text(repo);
                    }
                    cell
                }
                _ => text_at(
                    ui,
                    "Ollama library".to_string(),
                    FontId::proportional(TEXT_PX),
                    t.fg3,
                    false,
                ),
            },
            Column::Modified => text_at(
                ui,
                relative_time(&row.model.modified, now),
                FontId::proportional(TEXT_PX),
                t.fg2,
                false,
            ),
            Column::Actions => {
                let top = cy - BUTTON_H / 2.0;
                let d_rect = Rect::from_min_size(Pos2::new(left, top), vec2(details_w, BUTTON_H));
                let x_rect = Rect::from_min_size(
                    Pos2::new(d_rect.right() + BUTTON_GAP, top),
                    vec2(delete_w, BUTTON_H),
                );
                let d = ui.put(
                    d_rect,
                    egui::Button::new(
                        RichText::new(if expanded { "Hide" } else { "Details" })
                            .size(12.0)
                            .color(t.fg2),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .min_size(vec2(details_w, BUTTON_H)),
                );
                let x = if armed {
                    // Same rect and label size in both phases. Dimmed until
                    // `ARM_MIN_DELAY` has passed; kept enabled (not disabled)
                    // so keyboard focus survives, with the state ignoring
                    // clicks in the meantime.
                    let fill = if ready {
                        bad(cx.dark)
                    } else {
                        bad(cx.dark).gamma_multiply(0.45)
                    };
                    ui.put(
                        x_rect,
                        egui::Button::new(
                            RichText::new("Confirm delete")
                                .size(12.0)
                                .color(Color32::WHITE),
                        )
                        .fill(fill)
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .min_size(vec2(delete_w, BUTTON_H)),
                    )
                } else {
                    ui.put(
                        x_rect,
                        egui::Button::new(RichText::new("Delete").size(12.0).color(t.fg2))
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .min_size(vec2(delete_w, BUTTON_H)),
                    )
                };
                buttons = Some((d, x));
                d_rect.union(x_rect)
            }
        };
        cells.push((*c, cell));
    }
    buttons.map(|(details, delete)| Painted {
        details,
        delete,
        #[cfg(test)]
        cells,
        #[cfg(not(test))]
        _unused: (),
    })
}

fn details(ui: &mut egui::Ui, app: &mut AppState, row: &InstalledRow, width: f32, cx: &Ctx) {
    let t = &cx.t;
    egui::Frame::new()
        .fill(t.surface2)
        .inner_margin(Margin::symmetric(16, 10))
        .show(ui, |ui| {
            ui.set_width(width - 32.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            let line = |ui: &mut egui::Ui, label: &str, value: &mut dyn FnMut(&mut egui::Ui)| {
                ui.horizontal(|ui| {
                    let (lr, _) = ui.allocate_exact_size(vec2(LABEL_W, 18.0), Sense::hover());
                    ui.painter().text(
                        lr.left_center(),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(12.0),
                        t.fg3,
                    );
                    value(ui);
                });
            };
            match app.installed.details_of(row.name()) {
                None | Some(DetailsState::Loading) => {
                    ui.label(RichText::new("Loading details…").size(12.5).color(t.fg3));
                }
                Some(DetailsState::Error(e)) => {
                    ui.label(
                        RichText::new(format!("Could not read the details. {e}"))
                            .size(12.5)
                            .color(bad(cx.dark)),
                    );
                }
                Some(DetailsState::Loaded(d)) => {
                    let family = if d.family.is_empty() {
                        "—".to_string()
                    } else {
                        d.family.clone()
                    };
                    line(ui, "Family", &mut |ui| {
                        ui.label(RichText::new(&family).size(12.5));
                    });
                    let context = d
                        .context
                        .map_or("—".to_string(), |c| format!("{} tokens", thousands(c)));
                    line(ui, "Context", &mut |ui| {
                        ui.label(RichText::new(&context).size(12.5));
                    });
                    let caps = if d.capabilities.is_empty() {
                        "—".to_string()
                    } else {
                        d.capabilities.join(", ")
                    };
                    line(ui, "Capabilities", &mut |ui| {
                        ui.label(RichText::new(&caps).size(12.5));
                    });
                    let full = if d.digest.is_empty() {
                        row.model.digest.clone()
                    } else {
                        d.digest.clone()
                    };
                    line(ui, "Digest", &mut |ui| {
                        ui.label(RichText::new(short_digest(&full)).font(mono(11.5)))
                            .on_hover_text(&full);
                    });
                }
            }
            let cmd = run_command(row.name());
            let mut copy = false;
            line(ui, "Run", &mut |ui| {
                let room = (ui.available_width() - 70.0).max(60.0);
                let shown = ellipsize_middle(ui, &cmd, mono(11.5), room);
                let r = ui.label(RichText::new(&shown).font(mono(11.5)));
                if shown != cmd {
                    r.on_hover_text(&cmd);
                }
                copy = small_button_response(ui, "Copy").clicked();
            });
            if copy {
                ui.ctx().copy_text(cmd.clone());
            }
            line(
                ui,
                "Source",
                &mut |ui| match (&row.source_repo, row.source_url()) {
                    (Some(repo), Some(url)) => {
                        let room = ui.available_width().max(60.0);
                        let shown = ellipsize_middle(ui, repo, FontId::proportional(12.5), room);
                        ui.hyperlink_to(RichText::new(shown).size(12.5), url);
                    }
                    _ => {
                        ui.label(RichText::new("Ollama library").size(12.5).color(t.fg3));
                    }
                },
            );
        });
}

fn toast(ui: &mut egui::Ui, app: &mut AppState, cx: &Ctx, panel: Rect) {
    let Some(toast) = app.installed.toast(cx.now) else {
        return;
    };
    let is_error = toast.text.starts_with("Could not");
    let (fill, fg) = if is_error {
        (bad(cx.dark), Color32::WHITE)
    } else {
        (cx.t.fg, cx.t.surface)
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
        app.installed_ui.probe.toast = Some(rect);
    }
}
