//! The Settings view: validated fields that apply and persist immediately.

pub mod state;
pub mod validate;

#[cfg(test)]
mod layout_tests;

use crate::app::{bad, AppState};
use crate::browse::results::format_size;
use crate::runtime::{BackendRequest, CachedRow, HfTokenStatus};
use crate::theme::{mono, semibold, tokens, Tokens};
use crate::widgets::toggle;
use eframe::egui::{
    self, vec2, Align, Align2, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke,
};
use roost_core::config::{AutoDismiss, Settings, ThemeChoice};
use roost_core::fit::Machine;
use state::{
    cached_total, connection_message, detected_line, has_clearable, token_badge, BadgeKind,
    CachedStatus, ConnectionState, SaveIndicator, SettingsChange, SettingsState, CACHED_HELP,
    CLEAR_KEY, KEEP_PARTIALS_HELP,
};
use std::path::PathBuf;
use std::time::Instant;

/// Widest a card grows; the mockup keeps settings readable on big windows.
const CARD_MAX_W: f32 = 860.0;
const CARD_RADIUS: f32 = 10.0;
const HEAD_H: f32 = 34.0;
const ROW_PAD_X: f32 = 16.0;
const ROW_PAD_Y: f32 = 14.0;
const COL_GAP: f32 = 24.0;
/// The control column of a row (the mockup's `minmax(200px, 340px)`).
const CTRL_W: f32 = 340.0;
const FIELD_H: f32 = 30.0;
const BTN_W: f32 = 72.0;
const CLEAR_W: f32 = 150.0;
const SELECT_W: f32 = 170.0;
const CARDS_GAP: f32 = 22.0;
/// Space kept free at the right edge for the scroll bar.
const SCROLL_GUTTER: f32 = 14.0;
/// How many cached folders are listed before "and N more".
const CACHED_LISTED: usize = 8;
/// What the card text may take next to the control column, at least.
const LABEL_MIN_W: f32 = 200.0;

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct SettingsProbe {
    pub panel: Option<Rect>,
    /// Visible area of the scrolling cards.
    pub viewport: Option<Rect>,
    /// Height of everything inside the scroll area.
    pub content_height: f32,
    pub max_offset: f32,
    pub cards: Vec<(String, Rect)>,
    pub controls: Vec<(String, Rect)>,
    pub help: Vec<HelpProbe>,
    /// Widget id of the Clear cached files button.
    pub clear_id: Option<egui::Id>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct HelpProbe {
    pub title: String,
    /// The painted help label.
    pub rect: Rect,
    /// Right edge of the text column (the control column starts after it).
    pub column_right: f32,
    pub galley: std::sync::Arc<egui::Galley>,
}

/// The Settings view: its pure state plus what it painted (for tests).
pub struct SettingsView {
    pub state: SettingsState,
    #[cfg(test)]
    pub probe: SettingsProbe,
    /// Test hook: scroll the cards to this offset.
    #[cfg(test)]
    pub scroll_to: Option<f32>,
}

struct Cx<'a> {
    t: Tokens,
    dark: bool,
    now: Instant,
    #[cfg(test)]
    probe: &'a mut SettingsProbe,
    #[cfg(not(test))]
    _probe: std::marker::PhantomData<&'a ()>,
}

impl Cx<'_> {
    #[cfg(test)]
    fn rec(&mut self, name: &str, rect: Rect) {
        self.probe.controls.push((name.to_string(), rect));
    }

    #[cfg(not(test))]
    fn rec(&mut self, _name: &str, _rect: Rect) {}
}

impl SettingsView {
    pub fn new(
        settings: &Settings,
        default_staging: PathBuf,
        detected: Option<Machine>,
    ) -> SettingsView {
        SettingsView {
            state: SettingsState::new(settings, default_staging, detected),
            #[cfg(test)]
            probe: SettingsProbe::default(),
            #[cfg(test)]
            scroll_to: None,
        }
    }

    /// Draws the cards; returns which settings the user changed this frame.
    pub fn show(&mut self, ui: &mut egui::Ui, settings: &mut Settings) -> SettingsChange {
        let t = tokens(ui);
        let now = Instant::now();
        if let Some(due) = self.state.tick(now) {
            ui.ctx()
                .request_repaint_after(due.saturating_duration_since(now));
        }
        #[cfg(test)]
        {
            self.probe = SettingsProbe {
                panel: Some(ui.max_rect()),
                ..Default::default()
            };
        }
        let mut change = SettingsChange::default();
        let state = &mut self.state;
        #[cfg(test)]
        let probe = &mut self.probe;
        let mut cx = Cx {
            t,
            dark: ui.visuals().dark_mode,
            now,
            #[cfg(test)]
            probe,
            #[cfg(not(test))]
            _probe: std::marker::PhantomData,
        };
        header(ui, state, &cx);
        ui.add_space(10.0);
        let scroll = egui::ScrollArea::vertical()
            .id_salt("settings-scroll")
            .auto_shrink([false, false]);
        #[cfg(test)]
        let scroll = match self.scroll_to {
            Some(offset) => scroll.vertical_scroll_offset(offset),
            None => scroll,
        };
        let shown = scroll.show(ui, |ui| {
            // Leave room for the scroll bar so it never covers a control.
            let width = (ui.available_width() - SCROLL_GUTTER).clamp(300.0, CARD_MAX_W);
            ollama_card(ui, &mut cx, state, settings, width, &mut change);
            downloads_card(ui, &mut cx, state, settings, width, &mut change);
            cached_card(ui, &mut cx, state, width);
            hugging_face_card(ui, &mut cx, state, settings, width, &mut change);
            hardware_card(ui, &mut cx, state, settings, width, &mut change);
        });
        #[cfg(test)]
        {
            self.probe.viewport = Some(shown.inner_rect);
            self.probe.content_height = shown.content_size.y;
            self.probe.max_offset = (shown.content_size.y - shown.inner_rect.height()).max(0.0);
        }
        let _ = shown;
        change
    }
}

/// Draws the Settings view in the central panel, applies what changed and
/// queues the request that saves it.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .inner_margin(Margin::symmetric(24, 20)),
        )
        .show(ui, |ui| {
            let change = app.settings_view.show(ui, &mut app.settings);
            out.extend(app.apply_change(change, ui.ctx()));
        });
}

/// Fixed id of the token field, so its edit state can be removed.
fn token_field_id() -> egui::Id {
    egui::Id::NULL.with("hf-token-input")
}

/// Empties the typed token and drops the text edit's undo history, so
/// neither the string nor Ctrl+Z can bring it back.
pub fn forget_token_field(ctx: &egui::Context, state: &mut SettingsState) {
    state.token_input.clear();
    ctx.data_mut(|d| d.remove::<egui::text_edit::TextEditState>(token_field_id()));
}

fn header(ui: &mut egui::Ui, state: &SettingsState, cx: &Cx) {
    ui.label(RichText::new("Settings").font(semibold(20.0)));
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Changes apply immediately.")
                .size(13.0)
                .color(cx.t.fg3),
        );
        match &state.save {
            SaveIndicator::Idle => {}
            SaveIndicator::Saving => {
                ui.label(RichText::new("Saving…").size(13.0).color(cx.t.fg3));
            }
            SaveIndicator::Saved { .. } => {
                ui.label(RichText::new("Saved").size(13.0).color(cx.t.ok));
            }
            SaveIndicator::Failed(e) => {
                ui.label(
                    RichText::new(format!("Could not save settings: {e}"))
                        .size(13.0)
                        .color(bad(cx.dark)),
                );
            }
        }
    });
}

// ---- scaffold: cards and rows ----

/// A bordered card with an uppercase heading band; `body` adds the rows.
fn card(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    title: &str,
    width: f32,
    body: impl FnOnce(&mut egui::Ui, &mut Cx),
) {
    let r = egui::Frame::new()
        .fill(cx.t.surface)
        .stroke(Stroke::new(1.0, cx.t.line))
        .corner_radius(CARD_RADIUS)
        .show(ui, |ui| {
            ui.set_width(width - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (band, _) = ui.allocate_exact_size(vec2(width - 2.0, HEAD_H), Sense::hover());
            let top_round = CARD_RADIUS as u8 - 1;
            ui.painter().rect_filled(
                band,
                CornerRadius {
                    nw: top_round,
                    ne: top_round,
                    sw: 0,
                    se: 0,
                },
                cx.t.surface2,
            );
            ui.painter()
                .hline(band.x_range(), band.bottom(), Stroke::new(1.0, cx.t.line));
            ui.painter().text(
                egui::pos2(band.left() + ROW_PAD_X, band.center().y),
                Align2::LEFT_CENTER,
                title.to_uppercase(),
                semibold(12.0),
                cx.t.fg2,
            );
            body(ui, cx);
        })
        .response
        .rect;
    #[cfg(test)]
    cx.probe.cards.push((title.to_string(), r));
    let _ = r;
    ui.add_space(CARDS_GAP);
}

/// One settings row: label and help on the left, the control on the right.
/// `ctrl_h` is the control's single-line height, used to centre it against
/// the text; a control that grows (an error line) pushes the row taller.
fn row(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    label: &str,
    help: &str,
    ctrl_h: f32,
    last: bool,
    control: impl FnOnce(&mut egui::Ui, &mut Cx, f32),
) {
    let width = ui.available_width();
    let ctrl_w = CTRL_W.min((width - 2.0 * ROW_PAD_X - COL_GAP - LABEL_MIN_W).max(160.0));
    let left_w = width - 2.0 * ROW_PAD_X - COL_GAP - ctrl_w;
    let help_font = egui::FontId::proportional(12.0);
    // Measured with the same fonts and width the labels are drawn with.
    let label_h = ui
        .painter()
        .layout(label.to_string(), semibold(14.0), cx.t.fg, left_w)
        .size()
        .y;
    let help_galley = (!help.is_empty()).then(|| {
        ui.painter()
            .layout(help.to_string(), help_font.clone(), cx.t.fg3, left_w)
    });
    let left_h = label_h + help_galley.as_ref().map_or(0.0, |g| 2.0 + g.size().y);

    let top = ui.cursor().min;
    let mut row_ui = ui.new_child(
        egui::UiBuilder::new().max_rect(Rect::from_min_size(top, vec2(width, 100_000.0))),
    );
    let off = ((left_h - ctrl_h) / 2.0).max(0.0);
    let ctrl_rect = Rect::from_min_size(
        egui::pos2(top.x + width - ROW_PAD_X - ctrl_w, top.y + ROW_PAD_Y + off),
        vec2(ctrl_w, 100_000.0),
    );
    let used = row_ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(ctrl_rect)
                .layout(Layout::top_down(Align::Max)),
            |ui| control(ui, cx, ctrl_w),
        )
        .response
        .rect;
    let content_h = left_h.max(off + used.height());
    let text_rect = Rect::from_min_size(
        egui::pos2(
            top.x + ROW_PAD_X,
            top.y + ROW_PAD_Y + (content_h - left_h) / 2.0,
        ),
        vec2(left_w, 100_000.0),
    );
    row_ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(RichText::new(label).font(semibold(14.0)).color(cx.t.fg)).wrap());
        if let Some(galley) = help_galley {
            let r = ui
                .add(egui::Label::new(RichText::new(help).font(help_font).color(cx.t.fg3)).wrap());
            #[cfg(test)]
            cx.probe.help.push(HelpProbe {
                title: label.to_string(),
                rect: r.rect,
                column_right: text_rect.right(),
                galley,
            });
            #[cfg(not(test))]
            let _ = (r, galley);
        }
    });
    let (rect, _) =
        ui.allocate_exact_size(vec2(width, content_h + 2.0 * ROW_PAD_Y), Sense::hover());
    if !last {
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, cx.t.line));
    }
}

// ---- cards ----

fn ollama_card(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    state: &mut SettingsState,
    settings: &mut Settings,
    width: f32,
    change: &mut SettingsChange,
) {
    card(ui, cx, "Ollama", width, |ui, cx| {
        row(
            ui,
            cx,
            "Server address",
            "The app talks to Ollama only through its HTTP API.",
            FIELD_H,
            false,
            |ui, cx, w| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let fw = w - BTN_W - 8.0;
                        let r = text_field(
                            ui,
                            cx,
                            &mut state.ollama_text,
                            fw,
                            "http://localhost:11434",
                            state.ollama_error.is_some(),
                        );
                        cx.rec("ollama-url", r.rect);
                        if r.changed() {
                            state.edit_ollama();
                        }
                        if r.lost_focus() {
                            change.merge(state.commit_ollama(settings));
                        }
                        let test = ui.add_sized(
                            vec2(BTN_W, FIELD_H),
                            egui::Button::new(RichText::new("Test").size(12.0).color(cx.t.fg2)),
                        );
                        cx.rec("ollama-test", test.rect);
                        if test.clicked() {
                            state.test_connection();
                        }
                    });
                    error_line(ui, cx, state.ollama_error.as_deref(), w);
                    match &state.connection {
                        ConnectionState::Idle => {}
                        ConnectionState::Testing => {
                            ui.label(RichText::new("Testing…").size(12.0).color(cx.t.fg3));
                        }
                        ConnectionState::Done(r) => {
                            let m = connection_message(r);
                            let color = if m.ok { cx.t.ok } else { bad(cx.dark) };
                            ui.add(
                                egui::Label::new(RichText::new(m.text).size(12.0).color(color))
                                    .wrap(),
                            );
                        }
                    }
                });
            },
        );
        row(
            ui,
            cx,
            "Models directory",
            "Ollama decides where models are stored. To change it, set OLLAMA_MODELS and restart Ollama.",
            20.0,
            true,
            |ui, cx, _| {
                ui.label(
                    RichText::new("Set by the Ollama server")
                        .font(mono(13.0))
                        .color(cx.t.fg2),
                );
            },
        );
    });
}

fn downloads_card(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    state: &mut SettingsState,
    settings: &mut Settings,
    width: f32,
    change: &mut SettingsChange,
) {
    card(ui, cx, "Downloads", width, |ui, cx| {
        row(
            ui,
            cx,
            "Staging folder",
            "Where files are downloaded before they are sent to Ollama. Needs room for one model at a time. Leave it empty for the default folder.",
            FIELD_H,
            false,
            |ui, cx, w| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let fw = w - BTN_W - 8.0;
                        let r = text_field(
                            ui,
                            cx,
                            &mut state.staging_text,
                            fw,
                            "Staging folder",
                            state.staging_error.is_some(),
                        );
                        cx.rec("staging", r.rect);
                        if r.changed() {
                            state.edit_staging();
                        }
                        if r.lost_focus() {
                            change.merge(state.commit_staging(settings));
                        }
                        let browse = ui.add_sized(
                            vec2(BTN_W, FIELD_H),
                            egui::Button::new(RichText::new("Browse").size(12.0).color(cx.t.fg2)),
                        );
                        cx.rec("staging-browse", browse.rect);
                        if browse.clicked() {
                            if let Some(dir) = pick_folder(&state.staging_text) {
                                state.staging_text = dir;
                                state.edit_staging();
                                change.merge(state.commit_staging(settings));
                            }
                        }
                    });
                    error_line(ui, cx, state.staging_error.as_deref(), w);
                });
            },
        );
        row(
            ui,
            cx,
            "Delete downloaded files after import",
            "Removes the staging copy once Ollama has the model. Turn off to keep GGUF and safetensors files for reuse.",
            22.0,
            false,
            |ui, cx, _| {
                let mut on = settings.delete_after_import;
                let r = toggle(ui, &mut on);
                cx.rec("toggle-delete", r.rect);
                if r.changed() {
                    change.merge(state.set_delete_after_import(settings, on));
                }
            },
        );
        row(
            ui,
            cx,
            "Keep partial downloads for 7 days",
            KEEP_PARTIALS_HELP,
            22.0,
            false,
            |ui, cx, _| {
                let mut on = settings.keep_partials;
                let r = toggle(ui, &mut on);
                cx.rec("toggle-partials", r.rect);
                if r.changed() {
                    change.merge(state.set_keep_partials(settings, on));
                }
            },
        );
        row(
            ui,
            cx,
            "Auto-dismiss finished and cancelled jobs",
            "Rows stay in the downloads list, with their log, until you dismiss them or this timer runs out.",
            FIELD_H,
            false,
            |ui, cx, _| {
                let options = [
                    (AutoDismiss::Never, "Never"),
                    (AutoDismiss::OneMinute, "After 1 minute"),
                    (AutoDismiss::FiveMinutes, "After 5 minutes"),
                    (AutoDismiss::OneHour, "After 1 hour"),
                ];
                if let Some(v) = select(ui, cx, "select-dismiss", settings.auto_dismiss, &options)
                {
                    change.merge(state.set_auto_dismiss(settings, v));
                }
            },
        );
        row(
            ui,
            cx,
            "Simultaneous downloads",
            "How many models download at the same time.",
            FIELD_H,
            true,
            |ui, cx, _| {
                let options = [(1u8, "1"), (2, "2"), (3, "3")];
                if let Some(v) = select(ui, cx, "select-parallel", settings.max_parallel, &options)
                {
                    change.merge(state.set_max_parallel(settings, v));
                }
            },
        );
    });
}

fn cached_card(ui: &mut egui::Ui, cx: &mut Cx, state: &mut SettingsState, width: f32) {
    card(ui, cx, "Cached files", width, |ui, cx| {
        match &state.cached {
            CachedStatus::Loading => {
                ui.label(
                    RichText::new("Reading the staging folder…")
                        .size(13.0)
                        .color(cx.t.fg3),
                );
            }
            CachedStatus::Error(e) => {
                ui.label(
                    RichText::new(format!("Could not read the cached files. {e}"))
                        .size(13.0)
                        .color(bad(cx.dark)),
                );
            }
            CachedStatus::Loaded(rows) => {
                for r in rows.iter().take(CACHED_LISTED) {
                    cached_row(ui, cx, r);
                }
                if rows.len() > CACHED_LISTED {
                    ui.label(
                        RichText::new(format!("and {} more", rows.len() - CACHED_LISTED))
                            .size(12.0)
                            .color(cx.t.fg3),
                    );
                }
            }
        }
        let (total, clearable) = match &state.cached {
            CachedStatus::Loaded(rows) => (cached_total(rows), has_clearable(rows)),
            _ => ("Cached files".to_string(), false),
        };
        row(ui, cx, &total, CACHED_HELP, FIELD_H, true, |ui, cx, _| {
            ui.vertical(|ui| {
                let now = cx.now;
                let armed = state.clear.is_armed(CLEAR_KEY, now);
                let ready = state.clear.ready(CLEAR_KEY, now);
                let held = crate::confirm::activation_key_held(ui);
                let (text, fill) = if armed {
                    let red = bad(cx.dark);
                    (
                        "Confirm clear",
                        if ready { red } else { red.gamma_multiply(0.45) },
                    )
                } else {
                    (
                        "Clear cached files",
                        ui.visuals().widgets.inactive.weak_bg_fill,
                    )
                };
                let color = if armed {
                    egui::Color32::WHITE
                } else {
                    cx.t.fg2
                };
                let b = ui.add_enabled(
                    clearable || armed,
                    egui::Button::new(RichText::new(text).size(12.0).color(color))
                        .fill(fill)
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .min_size(vec2(CLEAR_W, FIELD_H)),
                );
                cx.rec("clear-cached", b.rect);
                #[cfg(test)]
                {
                    cx.probe.clear_id = Some(b.id);
                }
                if b.clicked() && !held {
                    state.click_clear_cached(now);
                }
                if let Some((note, _)) = &state.note {
                    ui.label(RichText::new(note.as_str()).size(12.0).color(cx.t.fg2));
                }
            });
        });
    });
}

fn cached_row(ui: &mut egui::Ui, cx: &mut Cx, r: &CachedRow) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&r.label).font(mono(12.5)).color(cx.t.fg));
        ui.label(
            RichText::new(format_size(r.bytes))
                .font(mono(12.5))
                .color(cx.t.fg2),
        );
        if r.partial {
            ui.label(RichText::new("partial").size(11.0).color(cx.t.warn));
        }
    });
}

fn hugging_face_card(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    state: &mut SettingsState,
    settings: &mut Settings,
    width: f32,
    change: &mut SettingsChange,
) {
    card(ui, cx, "Hugging Face", width, |ui, cx| {
        row(
            ui,
            cx,
            "Access token",
            "Needed for gated models such as Llama. Stored in the system keychain, never in a config file.",
            FIELD_H,
            false,
            |ui, cx, w| {
                ui.vertical(|ui| {
                    let r = ui.add_sized(
                        vec2(w, FIELD_H),
                        egui::TextEdit::singleline(&mut state.token_input)
                            .id(token_field_id())
                            .password(true)
                            .font(mono(13.0))
                            .hint_text("Paste a token to save it")
                            .margin(Margin::symmetric(10, 6)),
                    );
                    cx.rec("token", r.rect);
                    if r.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && state.save_token()
                    {
                        forget_token_field(ui.ctx(), state);
                    }
                    ui.horizontal(|ui| {
                        let can_save = !state.token_input.trim().is_empty() && !state.token_busy;
                        let save = ui.add_enabled(
                            can_save,
                            egui::Button::new(RichText::new("Save").size(12.0).color(cx.t.fg2))
                                .min_size(vec2(BTN_W, FIELD_H)),
                        );
                        cx.rec("token-save", save.rect);
                        if save.clicked() && state.save_token() {
                            forget_token_field(ui.ctx(), state);
                        }
                        let can_clear = state.token_status != HfTokenStatus::None && !state.token_busy;
                        let clear = ui.add_enabled(
                            can_clear,
                            egui::Button::new(RichText::new("Clear").size(12.0).color(cx.t.fg2))
                                .min_size(vec2(BTN_W, FIELD_H)),
                        );
                        cx.rec("token-clear", clear.rect);
                        if clear.clicked() {
                            state.clear_token();
                            forget_token_field(ui.ctx(), state);
                        }
                        if state.can_import_legacy_token() {
                            let import = ui.add_sized(
                                vec2(170.0, FIELD_H),
                                egui::Button::new(
                                    RichText::new("Import hf2ollama token").size(12.0).color(cx.t.fg2),
                                ),
                            );
                            cx.rec("token-import-legacy", import.rect);
                            if import.clicked() {
                                state.import_legacy_token();
                            }
                        }
                    });
                    if state.can_import_legacy_token() {
                        ui.add(
                            egui::Label::new(
                                RichText::new("macOS may display a Keychain access prompt.")
                                    .size(11.5)
                                    .color(cx.t.warn),
                            )
                            .wrap(),
                        );
                    }
                    token_badge_ui(ui, cx, state, w);
                    if let Some(note) = state.token_note() {
                        ui.add(
                            egui::Label::new(RichText::new(note).size(11.5).color(cx.t.warn))
                                .wrap(),
                        );
                    }
                });
            },
        );
        row(
            ui,
            cx,
            "Endpoint",
            "Change this to use a mirror.",
            FIELD_H,
            true,
            |ui, cx, w| {
                ui.vertical(|ui| {
                    let r = text_field(
                        ui,
                        cx,
                        &mut state.hf_text,
                        w,
                        "https://huggingface.co",
                        state.hf_error.is_some(),
                    );
                    cx.rec("hf-endpoint", r.rect);
                    if r.changed() {
                        state.edit_hf_endpoint();
                    }
                    if r.lost_focus() {
                        change.merge(state.commit_hf_endpoint(settings));
                    }
                    error_line(ui, cx, state.hf_error.as_deref(), w);
                });
            },
        );
    });
}

fn token_badge_ui(ui: &mut egui::Ui, cx: &mut Cx, state: &SettingsState, w: f32) {
    let (kind, text) = if state.token_busy {
        (BadgeKind::Neutral, "Checking…".to_string())
    } else {
        token_badge(&state.token_status)
    };
    let (fg, bg) = match kind {
        BadgeKind::Ok => (cx.t.ok, cx.t.ok_soft),
        BadgeKind::Bad => (bad(cx.dark), bad(cx.dark).gamma_multiply(0.14)),
        BadgeKind::Neutral => (cx.t.fg2, cx.t.surface2),
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(4.0)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.set_max_width(w - 14.0);
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(fg)).wrap());
        });
}

fn hardware_card(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    state: &mut SettingsState,
    settings: &mut Settings,
    width: f32,
    change: &mut SettingsChange,
) {
    card(ui, cx, "Hardware and appearance", width, |ui, cx| {
        row(
            ui,
            cx,
            "Detected hardware",
            "Used for the fit badges. Override it if Ollama runs on another computer.",
            20.0,
            false,
            |ui, cx, _| {
                ui.label(
                    RichText::new(detected_line(state.detected()))
                        .font(mono(13.0))
                        .color(cx.t.fg2),
                );
            },
        );
        row(
            ui,
            cx,
            "Override hardware",
            "Turn on to type the memory yourself.",
            22.0,
            false,
            |ui, cx, _| {
                let mut on = state.override_on;
                let r = toggle(ui, &mut on);
                cx.rec("toggle-override", r.rect);
                if r.changed() {
                    change.merge(state.set_override(settings, on));
                }
            },
        );
        if state.override_on {
            row(
                ui,
                cx,
                "Memory to use",
                "RAM is at least 1 GB. Leave VRAM empty when there is no GPU or it is unknown.",
                FIELD_H,
                false,
                |ui, cx, w| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let fw = (w - 8.0) / 2.0;
                            for (id, caption, text, err) in [
                                ("ram", "RAM (GB)", 0, state.ram_error.is_some()),
                                ("vram", "VRAM (GB)", 1, state.vram_error.is_some()),
                            ] {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(caption).size(11.5).color(cx.t.fg3));
                                    let buf = if text == 0 {
                                        &mut state.ram_text
                                    } else {
                                        &mut state.vram_text
                                    };
                                    let r = text_field(ui, cx, buf, fw, "", err);
                                    cx.rec(id, r.rect);
                                    if r.changed() {
                                        state.edit_hardware();
                                    }
                                    if r.lost_focus() {
                                        change.merge(state.commit_hardware(settings));
                                    }
                                });
                            }
                        });
                        error_line(ui, cx, state.ram_error.as_deref(), w);
                        error_line(ui, cx, state.vram_error.as_deref(), w);
                    });
                },
            );
        }
        row(ui, cx, "Theme", "", FIELD_H, true, |ui, cx, _| {
            let options = [
                (ThemeChoice::System, "Match system"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Dark, "Dark"),
            ];
            if let Some(v) = select(ui, cx, "select-theme", settings.theme, &options) {
                change.merge(state.set_theme(settings, v));
            }
        });
    });
}

// ---- controls ----

/// A monospace single-line field; a red outline marks an invalid value.
fn text_field(
    ui: &mut egui::Ui,
    cx: &Cx,
    text: &mut String,
    width: f32,
    hint: &str,
    invalid: bool,
) -> egui::Response {
    let r = ui.add_sized(
        vec2(width, FIELD_H),
        egui::TextEdit::singleline(text)
            .font(mono(13.0))
            .hint_text(hint)
            .margin(Margin::symmetric(10, 6)),
    );
    if invalid {
        ui.painter().rect_stroke(
            r.rect,
            4.0,
            Stroke::new(1.5, bad(cx.dark)),
            egui::StrokeKind::Inside,
        );
    }
    r
}

fn error_line(ui: &mut egui::Ui, cx: &Cx, error: Option<&str>, width: f32) {
    if let Some(e) = error {
        ui.add_sized(
            vec2(width, 0.0),
            egui::Label::new(RichText::new(e).size(12.0).color(bad(cx.dark))).wrap(),
        );
    }
}

/// A drop-down; returns the newly picked value.
fn select<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    cx: &mut Cx,
    id: &str,
    current: T,
    options: &[(T, &str)],
) -> Option<T> {
    let label = options
        .iter()
        .find(|(v, _)| *v == current)
        .map_or("", |(_, l)| *l);
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .width(SELECT_W)
        .show_ui(ui, |ui| {
            for (v, l) in options {
                if ui.selectable_label(*v == current, *l).clicked() {
                    picked = Some(*v);
                }
            }
        });
    cx.rec(id, r.response.rect);
    picked.filter(|v| *v != current)
}

/// Native folder chooser; `None` when cancelled. Never opened in tests.
#[cfg(not(test))]
fn pick_folder(current: &str) -> Option<String> {
    let mut dialog = rfd::FileDialog::new();
    let start = std::path::Path::new(current.trim());
    if start.is_dir() {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_folder().map(|p| p.display().to_string())
}

#[cfg(test)]
fn pick_folder(_current: &str) -> Option<String> {
    None
}
