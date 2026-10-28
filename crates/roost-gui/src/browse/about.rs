//! The "About this model" tab: chips from the repository metadata and the
//! rendered model card.

use super::detail::callout;
use super::markdown::{is_safe_link, Block, Span};
use crate::app::AppState;
use crate::theme::{mono, semibold, tokens, Tokens};
use crate::widgets::{small_button_response, tag};
use eframe::egui::{
    self, vec2, Align2, Color32, FontId, Frame, Margin, Rect, RichText, ScrollArea, Sense, Stroke,
};
use roost_core::hf::ModelInfo;

/// Tags that say nothing useful to a person choosing a model.
pub const NOISY_TAGS: &[&str] = &[
    "region:",
    "endpoints_compatible",
    "license:",
    "autotrain_compatible",
    "text-generation-inference",
];

/// Most plain tags shown as chips.
pub const MAX_TAG_CHIPS: usize = 6;

/// Most languages shown before "+N".
pub const MAX_LANGUAGE_CHIPS: usize = 5;

/// Chip labels: pipeline tag, languages, then up to `MAX_TAG_CHIPS` tags.
pub fn chips(info: &ModelInfo) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = &info.pipeline_tag {
        out.push(p.clone());
    }
    out.extend(info.languages.iter().take(MAX_LANGUAGE_CHIPS).cloned());
    if info.languages.len() > MAX_LANGUAGE_CHIPS {
        out.push(format!("+{}", info.languages.len() - MAX_LANGUAGE_CHIPS));
    }
    let mut seen: Vec<&str> = Vec::new();
    for t in &info.tags {
        if seen.len() == MAX_TAG_CHIPS {
            break;
        }
        let repeats = info.pipeline_tag.as_deref() == Some(t) || info.languages.contains(t);
        if is_noisy(t) || repeats || seen.contains(&t.as_str()) {
            continue;
        }
        seen.push(t);
    }
    out.extend(seen.into_iter().map(str::to_string));
    out
}

fn is_noisy(tag: &str) -> bool {
    NOISY_TAGS.iter().any(|n| {
        if n.ends_with(':') {
            tag.starts_with(n)
        } else {
            tag == *n
        }
    })
}

/// What one frame of drawing collected: ids for scroll areas and, in
/// tests, the painted rectangles.
#[derive(Default)]
struct Cx {
    next_id: u32,
    #[cfg(test)]
    rects: Vec<Rect>,
    #[cfg(test)]
    links: Vec<(Rect, String)>,
    #[cfg(test)]
    tables: Vec<(Rect, f32)>,
}

impl Cx {
    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn rect(&mut self, _r: Rect) {
        #[cfg(test)]
        self.rects.push(_r);
    }
}

/// Draws parsed Markdown blocks in `color`; the chat view reuses this.
pub(crate) fn render(ui: &mut egui::Ui, t: &Tokens, list: &[Block], color: Color32) {
    blocks(ui, &mut Cx::default(), t, list, color);
}

const BODY: f32 = 13.5;
const CODE: f32 = 12.5;

/// Text style handed down while drawing blocks.
#[derive(Clone, Copy)]
struct Look {
    size: f32,
    color: Color32,
    bold: bool,
}

/// One span as a label, or a hyperlink for a safe link.
fn one_span(ui: &mut egui::Ui, cx: &mut Cx, t: &Tokens, span: &Span, look: Look, wrap: bool) {
    let mut text = RichText::new(&span.text).color(look.color);
    text = if span.code {
        text.font(mono(look.size - 1.0))
            .background_color(t.surface2)
    } else if span.strong || look.bold {
        text.font(semibold(look.size))
    } else {
        text.size(look.size)
    };
    if span.emph {
        text = text.italics();
    }
    let safe = span.link.as_deref().filter(|u| is_safe_link(u));
    if let Some(url) = safe {
        let r = ui.add(egui::Hyperlink::from_label_and_url(
            text.color(t.accent),
            url,
        ));
        let r = r.on_hover_text(url);
        if wrap {
            cx.rect(r.rect);
        }
        #[cfg(test)]
        cx.links.push((r.rect, url.to_string()));
    } else {
        let mut label = egui::Label::new(text);
        if !wrap {
            label = label.wrap_mode(egui::TextWrapMode::Extend);
        }
        let r = ui.add(label);
        if wrap {
            cx.rect(r.rect);
        }
    }
}

/// A run of spans. Wrapping runs flow like a paragraph; the others stay on
/// one line (table cells).
fn inline(ui: &mut egui::Ui, cx: &mut Cx, t: &Tokens, spans: &[Span], look: Look, wrap: bool) {
    let draw = |ui: &mut egui::Ui, cx: &mut Cx| {
        ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
        for s in spans {
            one_span(ui, cx, t, s, look, wrap);
        }
    };
    if wrap {
        ui.horizontal_wrapped(|ui| draw(ui, cx));
    } else {
        ui.horizontal(|ui| draw(ui, cx));
    }
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 20.0,
        2 => 17.0,
        3 => 15.0,
        _ => 14.0,
    }
}

fn blocks(ui: &mut egui::Ui, cx: &mut Cx, t: &Tokens, list: &[Block], color: Color32) {
    let body = Look {
        size: BODY,
        color,
        bold: false,
    };
    for (n, b) in list.iter().enumerate() {
        match b {
            Block::Heading(level, spans) => {
                if n > 0 {
                    ui.add_space(6.0);
                }
                let look = Look {
                    size: heading_size(*level),
                    color: t.fg,
                    bold: true,
                };
                inline(ui, cx, t, spans, look, true);
            }
            Block::Paragraph(spans) => inline(ui, cx, t, spans, body, true),
            Block::List { ordered, items } => {
                for (i, item) in items.iter().enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let marker = if *ordered {
                            format!("{}.", i + 1)
                        } else {
                            "•".to_string()
                        };
                        let (r, _) = ui.allocate_exact_size(vec2(24.0, 18.0), Sense::hover());
                        cx.rect(r);
                        ui.painter().text(
                            r.right_top() + vec2(0.0, 1.0),
                            Align2::RIGHT_TOP,
                            marker,
                            FontId::proportional(BODY),
                            t.fg3,
                        );
                        ui.vertical(|ui| blocks(ui, cx, t, item, color));
                    });
                }
            }
            Block::Quote(inner) => {
                let resp = Frame::new()
                    .inner_margin(Margin {
                        left: 14,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| blocks(ui, cx, t, inner, t.fg2))
                    .response;
                cx.rect(resp.rect);
                let rule = Rect::from_min_size(
                    resp.rect.left_top() + vec2(2.0, 0.0),
                    vec2(3.0, resp.rect.height()),
                );
                ui.painter().rect_filled(rule, 1.5, t.line);
            }
            Block::CodeBlock(code) => {
                let id = cx.id();
                let resp = Frame::new()
                    .fill(t.surface2)
                    .stroke(Stroke::new(1.0, t.line))
                    .corner_radius(6.0)
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::horizontal()
                            .id_salt(("about_code", id))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(code.trim_end_matches('\n'))
                                            .font(mono(CODE))
                                            .color(t.fg),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                    })
                    .response;
                cx.rect(resp.rect);
            }
            Block::Table(rows) => table(ui, cx, t, rows),
            Block::Rule => {
                ui.add_space(2.0);
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(r, 0.0, t.line);
                ui.add_space(2.0);
            }
        }
    }
}

fn table(ui: &mut egui::Ui, cx: &mut Cx, t: &Tokens, rows: &[Vec<Vec<Span>>]) {
    let id = cx.id();
    let frame = Frame::new()
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(6.0)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let out = ScrollArea::horizontal()
                .id_salt(("about_table", id))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Grid::new(("about_grid", id))
                        .striped(true)
                        .spacing(vec2(18.0, 6.0))
                        .show(ui, |ui| {
                            for (ri, row) in rows.iter().enumerate() {
                                for cell in row {
                                    let look = Look {
                                        size: BODY,
                                        color: t.fg,
                                        bold: ri == 0,
                                    };
                                    inline(ui, cx, t, cell, look, false);
                                }
                                ui.end_row();
                            }
                        });
                });
            #[cfg(test)]
            cx.tables.push((out.inner_rect, out.content_size.x));
            #[cfg(not(test))]
            let _ = out;
        })
        .response;
    cx.rect(frame.rect);
}

/// Draws the About tab content for the selected repository.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, info: &ModelInfo, id: &str) {
    let t = tokens(ui);
    let mut cx = Cx::default();
    ui.spacing_mut().item_spacing.y = 8.0;
    let chip_list = chips(info);
    if !chip_list.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for c in &chip_list {
                tag(ui, c, t.fg2, t.surface2);
            }
        });
    }
    #[allow(unused_assignments, unused_mut)]
    let mut state = "loading";
    #[allow(unused_assignments, unused_mut)]
    let mut retry: Option<Rect> = None;
    #[allow(unused_assignments, unused_mut)]
    let mut toggle: Option<Rect> = None;
    #[allow(unused_assignments, unused_mut)]
    let mut notice: Option<Rect> = None;
    let repo_url = format!("https://huggingface.co/{id}");
    match app.browse.card_for(id).cloned() {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading description…").color(t.fg3));
            });
        }
        Some(Err(msg)) => {
            state = "error";
            callout(ui, true, &msg);
            let r = small_button_response(ui, "Retry");
            retry = Some(r.rect);
            if r.clicked() {
                app.browse.retry_card(id);
            }
        }
        Some(Ok(None)) => {
            state = "empty";
            ui.label(RichText::new("This repo has no description.").color(t.fg3));
        }
        Some(Ok(Some(view))) => {
            state = "card";
            let expanded = app.browse.card_expanded();
            let shown = if expanded || !view.more {
                &view.full
            } else {
                &view.preview
            };
            blocks(ui, &mut cx, &t, shown, t.fg);
            let at_end = expanded || !view.more;
            if view.more {
                let label = if expanded { "Show less" } else { "Show more" };
                let r = small_button_response(ui, label);
                toggle = Some(r.rect);
                if r.clicked() {
                    app.browse.set_card_expanded(!expanded);
                }
            }
            if view.truncated && at_end && is_safe_link(&repo_url) {
                let r = ui
                    .horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new("Description truncated. ").color(t.fg3));
                        let l = ui.add(egui::Hyperlink::from_label_and_url(
                            RichText::new("Read the full model card on huggingface.co")
                                .color(t.accent),
                            &repo_url,
                        ));
                        let l = l.on_hover_text(&repo_url);
                        ui.label(RichText::new(".").color(t.fg3));
                        #[cfg(test)]
                        cx.links.push((l.rect, repo_url.clone()));
                        l.rect
                    })
                    .response;
                notice = Some(r.rect);
                cx.rect(r.rect);
            } else if at_end && is_safe_link(&repo_url) {
                let l = ui.add(egui::Hyperlink::from_label_and_url(
                    RichText::new("Read the full model card on huggingface.co ↗")
                        .size(12.0)
                        .color(t.accent),
                    &repo_url,
                ));
                let l = l.on_hover_text(&repo_url);
                cx.rect(l.rect);
                #[cfg(test)]
                cx.links.push((l.rect, repo_url.clone()));
            }
        }
    }
    #[cfg(test)]
    {
        let p = &mut app.browse.probe;
        p.about_state = Some(state);
        p.about_rects = std::mem::take(&mut cx.rects);
        p.about_links = std::mem::take(&mut cx.links);
        p.about_tables = std::mem::take(&mut cx.tables);
        p.about_retry = retry;
        p.about_toggle = toggle;
        p.about_notice = notice;
    }
    #[cfg(not(test))]
    let _ = (state, retry, toggle, notice, cx);
}
