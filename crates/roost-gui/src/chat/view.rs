//! Rendering for the Chat view: the history rail, the conversation pane and
//! the composer. All state lives in `ChatState` (events folded) and `ChatUi`
//! (interaction state); commands are queued as `BackendRequest::Chat`.

use super::state::{local_day, status_label, RailRow, RunBadge, SendAttempt};
use crate::app::{AppState, View};
use crate::confirm::Confirm;
use crate::runtime::BackendRequest;
use crate::theme::{semibold, tokens, Tokens};
use crate::widgets::{primary_button, small_button, tag};
use eframe::egui::{
    self, vec2, Align, Color32, FontId, Key, Layout, Margin, Modifiers, RichText, Sense, Stroke,
};
use roost_core::chat::ChatCommand;
use roost_core::session::{
    AssistantStatus, Message, MessageContent, Session, SessionId, SessionLifetime,
};
use roost_core::store::RowStatus;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const RAIL_W: f32 = 264.0;
const ROW_H: f32 = 46.0;
const COMPOSER_ROWS: usize = 3;

/// Interaction state that is not part of the conversation data.
#[derive(Default)]
pub struct ChatUi {
    /// A session being renamed in the header, with the text so far.
    pub renaming: Option<(SessionId, String)>,
    pub confirm: Confirm,
    /// A rail row that cannot be opened (damaged, newer, unreadable).
    pub inspect: Option<SessionId>,
    /// Put the cursor in the composer on the next frame.
    pub focus_composer: bool,
    #[cfg(test)]
    pub probe: ChatProbe,
}

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct ChatProbe {
    pub rail: Option<egui::Rect>,
    pub pane: Option<egui::Rect>,
    pub composer: Option<egui::Rect>,
    pub header: Option<egui::Rect>,
    pub rows: Vec<(SessionId, egui::Rect)>,
    pub send_button: Option<(egui::Rect, &'static str)>,
    /// The delete control of each rail row that shows one, and whether it is armed.
    pub rail_delete: Vec<(SessionId, egui::Rect, bool)>,
    pub header_unload: Option<egui::Rect>,
    pub loaded_footer: Option<egui::Rect>,
    pub footer_unload: Vec<egui::Rect>,
    pub delete_button: Option<(egui::Rect, String)>,
}

/// The composer text field of a session; also used to drop its editor state.
pub(super) fn composer_id(id: SessionId) -> egui::Id {
    egui::Id::unique("chat-composer").with(id)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A single-line galley cut with an ellipsis at `width`.
fn truncated(
    ui: &egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

fn secondary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let t = tokens(ui);
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).size(12.0).color(t.fg2)).min_size(vec2(0.0, 24.0)),
    )
}

pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    let now = Instant::now();
    #[cfg(test)]
    {
        app.chat_ui.probe = ChatProbe::default();
    }
    if let Some(why) = app.chat.unavailable.clone() {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(t.bg).inner_margin(Margin::same(24)))
            .show(ui, |ui| {
                ui.label(RichText::new("History is in use").font(semibold(16.0)));
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(why).color(t.fg2)).wrap());
            });
        return;
    }
    if app.chat_ui.confirm.is_armed("", now) || app.chat_ui.confirm_pending() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    // A destroyed session takes its text-editor state (undo history) too.
    for id in app.chat.take_forgotten() {
        ui.ctx()
            .data_mut(|d| d.remove::<egui::text_edit::TextEditState>(composer_id(id)));
    }
    let installed: Vec<String> = app
        .installed
        .rows()
        .iter()
        .map(|r| r.name().to_string())
        .collect();

    egui::Panel::left("chat_rail")
        .resizable(false)
        .exact_size(RAIL_W)
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.0, t.line))
                .inner_margin(Margin::same(12)),
        )
        .show(ui, |ui| {
            #[cfg(test)]
            {
                app.chat_ui.probe.rail = Some(ui.max_rect());
            }
            rail(ui, app, out, &t, &installed);
        });

    // The open session is taken out for the frame so the composer can edit
    // the draft while the transcript borrows the session.
    let open = app.chat.open.take();
    if let Some(s) = &open {
        egui::Panel::bottom("chat_composer")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(t.surface)
                    .stroke(Stroke::new(1.0, t.line))
                    .inner_margin(Margin::symmetric(20, 12)),
            )
            .show(ui, |ui| {
                #[cfg(test)]
                {
                    app.chat_ui.probe.composer = Some(ui.max_rect());
                }
                composer(ui, app, out, &t, s);
            });
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.bg)
                .inner_margin(Margin::symmetric(20, 16)),
        )
        .show(ui, |ui| {
            #[cfg(test)]
            {
                app.chat_ui.probe.pane = Some(ui.max_rect());
            }
            pane(ui, app, out, &t, open.as_ref(), &installed, now);
        });
    app.chat.open = open;
}

impl ChatUi {
    /// A delete confirmation is showing and will time out on its own.
    fn confirm_pending(&self) -> bool {
        self.inspect.is_some() || self.renaming.is_some()
    }
}

// ---------------------------------------------------------------- the rail

fn rail(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    installed: &[String],
) {
    let now = Instant::now();
    if app.ollama.as_ref().is_some_and(|o| o.is_some()) {
        egui::Panel::bottom("chat_loaded")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 6)))
            .show(ui, |ui| {
                #[cfg(test)]
                {
                    app.chat_ui.probe.loaded_footer = Some(ui.max_rect());
                }
                loaded_footer(ui, app, out, t);
            });
    }
    let model = app.chat.default_model(installed);
    ui.horizontal(|ui| {
        let ready = model.is_some();
        let new_chat = primary_button(ui, "New chat", ready)
            .on_disabled_hover_text("Install a model first (Browse), then start a chat.");
        let incognito = secondary(ui, "New incognito", ready)
            .on_disabled_hover_text("Install a model first (Browse), then start a chat.");
        for (clicked, lifetime) in [
            (new_chat.clicked(), SessionLifetime::Persisted),
            (incognito.clicked(), SessionLifetime::Incognito),
        ] {
            if let (true, Some(model)) = (clicked, model.clone()) {
                app.chat_ui.focus_composer = true;
                app.chat_ui.inspect = None;
                out.push(BackendRequest::Chat(ChatCommand::Create {
                    lifetime,
                    model,
                }));
            }
        }
    });
    ui.add_space(8.0);
    let rail = app.chat.rail(now_ms(), local_day);
    let selected = app.chat.open.as_ref().map(|s| s.id).or(app.chat_ui.inspect);
    let mut open_id = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let groups = [
                ("Pinned", &rail.pinned),
                ("Today", &rail.today),
                ("Yesterday", &rail.yesterday),
                ("Earlier", &rail.earlier),
            ];
            for (label, rows) in groups {
                if rows.is_empty() {
                    continue;
                }
                section_label(ui, t, label);
                for row in rows {
                    if rail_row(ui, t, row, selected == Some(row.id), app, out, now) {
                        open_id = Some(row.id);
                    }
                }
            }
            if !rail.incognito.is_empty() {
                ui.horizontal(|ui| {
                    section_label(ui, t, "Incognito");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if small_button(ui, "Clear") {
                            out.push(BackendRequest::Chat(ChatCommand::ClearIncognito));
                        }
                    });
                });
                ui.add(
                    egui::Label::new(
                        RichText::new("Memory only. Gone when closed.")
                            .size(11.5)
                            .color(t.fg3),
                    )
                    .wrap(),
                );
                for row in &rail.incognito {
                    if rail_row(ui, t, row, selected == Some(row.id), app, out, now) {
                        open_id = Some(row.id);
                    }
                }
            }
            if app.chat.loaded && app.chat.rows().is_empty() {
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new("No chats yet.").color(t.fg3)).wrap());
            }
        });
    if let Some(id) = open_id {
        // Leaving a saved session lets the manager unload it (a running one
        // finishes first). Incognito sessions must never be closed here: that
        // would destroy them.
        let leaving = app
            .chat
            .open
            .as_ref()
            .filter(|s| s.id != id && s.lifetime == SessionLifetime::Persisted)
            .map(|s| s.id);
        if let Some(prev) = leaving {
            out.push(BackendRequest::Chat(ChatCommand::Close(prev)));
        }
        let bad_row = app
            .chat
            .rows()
            .iter()
            .find(|r| r.id == id)
            .is_some_and(|r| r.status != RowStatus::Ok);
        app.chat_ui.renaming = None;
        app.chat_ui.focus_composer = !bad_row;
        if bad_row {
            app.chat_ui.inspect = Some(id);
            app.chat.open = None;
        } else {
            app.chat_ui.inspect = None;
            if app.chat.open.as_ref().map(|s| s.id) != Some(id) {
                out.push(BackendRequest::Chat(ChatCommand::Open(id)));
            }
        }
    }
}

fn section_label(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .font(semibold(11.0))
            .color(t.fg3),
    );
}

fn row_note(row: &RailRow) -> Option<(&'static str, bool)> {
    match (row.status, row.badge) {
        (RowStatus::Quarantined, _) => Some(("Damaged, set aside", true)),
        (RowStatus::NewerSchema, _) => Some(("Made by a newer Roost", true)),
        (RowStatus::Unreadable, _) => Some(("Could not be read", true)),
        (_, RunBadge::Waiting) => Some(("Waiting for model", false)),
        (_, RunBadge::Running) => Some(("Replying…", false)),
        _ => None,
    }
}

/// One rail row. Clicking it opens the session; the delete control at its
/// right (shown on hover, on the selected row and while armed) takes a click
/// and a confirming second click after a short pause.
fn rail_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    row: &RailRow,
    selected: bool,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    now: Instant,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    #[cfg(test)]
    app.chat_ui.probe.rows.push((row.id, rect));
    let key = row.id.to_string();
    let armed = app.chat_ui.confirm.is_armed(&key, now);
    let show_delete = selected || armed || ui.rect_contains_pointer(rect);
    if selected {
        ui.painter().rect_filled(rect, 8.0, t.accent_soft);
    } else if show_delete {
        ui.painter().rect_filled(rect, 8.0, t.surface2);
    }
    let reserved = match (show_delete, armed) {
        (false, _) => 0.0,
        (true, false) => 30.0,
        (true, true) => 76.0,
    };
    let left = rect.left() + 10.0 + if row.pinned { 10.0 } else { 0.0 };
    if row.pinned {
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 10.0, rect.top() + 15.0),
            3.0,
            t.accent,
        );
    }
    let width = rect.right() - left - 10.0 - reserved;
    let title = truncated(ui, &row.title, FontId::proportional(13.5), t.fg, width);
    ui.painter()
        .galley(egui::pos2(left, rect.top() + 7.0), title, t.fg);
    let (sub, color) = match row_note(row) {
        Some((text, bad)) => (text.to_string(), if bad { t.bad } else { t.accent }),
        None => (row.model.clone(), t.fg3),
    };
    let sub_width = rect.right() - left - 10.0;
    let sub = truncated(ui, &sub, FontId::proportional(11.5), color, sub_width);
    ui.painter()
        .galley(egui::pos2(left, rect.top() + 26.0), sub, color);

    let mut delete_clicked = false;
    if show_delete {
        let del_rect = if armed {
            egui::Rect::from_min_max(
                egui::pos2(rect.right() - 8.0 - 62.0, rect.top() + 6.0),
                egui::pos2(rect.right() - 8.0, rect.top() + 28.0),
            )
        } else {
            egui::Rect::from_center_size(
                egui::pos2(rect.right() - 17.0, rect.top() + 17.0),
                vec2(22.0, 22.0),
            )
        };
        #[cfg(test)]
        app.chat_ui
            .probe
            .rail_delete
            .push((row.id, del_rect, armed));
        let hit = ui
            .interact(del_rect, resp.id.with("delete"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if armed {
                "Click again to delete this chat"
            } else {
                "Delete chat"
            });
        if armed {
            ui.painter().rect_filled(del_rect, 6.0, t.bad_soft);
            ui.painter().text(
                del_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Delete?",
                semibold(12.0),
                t.bad,
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            if hit.hovered() {
                ui.painter()
                    .circle_filled(del_rect.center(), 11.0, t.surface);
            }
            let c = del_rect.center();
            let arm = 3.5;
            let stroke = Stroke::new(1.5, if hit.hovered() { t.bad } else { t.fg3 });
            ui.painter()
                .line_segment([c + vec2(-arm, -arm), c + vec2(arm, arm)], stroke);
            ui.painter()
                .line_segment([c + vec2(-arm, arm), c + vec2(arm, -arm)], stroke);
        }
        if hit.clicked() {
            delete_clicked = true;
            if app.chat_ui.confirm.click(&key, now) {
                out.push(BackendRequest::Chat(ChatCommand::Delete(row.id)));
            }
        }
    }
    resp.clicked() && !delete_clicked
}

// ---------------------------------------------------------- the conversation

fn banner(ui: &mut egui::Ui, fg: Color32, bg: Color32, text: &str) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(fg)).wrap());
        });
    ui.add_space(8.0);
}

fn pane(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    open: Option<&Session>,
    installed: &[String],
    now: Instant,
) {
    if !app.chat.warnings.is_empty() {
        banner(ui, t.warn, t.warn_soft, &app.chat.warnings.join("  "));
        if small_button(ui, "Dismiss") {
            app.chat.dismiss_warnings();
        }
        ui.add_space(4.0);
    }
    if app.ollama == Some(None) {
        banner(
            ui,
            t.warn,
            t.warn_soft,
            "Ollama is not reachable. Start it and Roost will reconnect; your chats stay readable.",
        );
    } else if app.installed.status == crate::installed::state::ListStatus::Loaded
        && installed.is_empty()
    {
        banner(
            ui,
            t.warn,
            t.warn_soft,
            "No models are installed yet. Open Browse to import one, then come back to chat.",
        );
        if small_button(ui, "Open Browse") {
            app.select(View::Browse);
        }
    }
    let Some(s) = open else {
        if let Some(id) = app.chat_ui.inspect {
            inspect(ui, app, out, t, id, now);
        } else {
            ui.add_space(24.0);
            ui.label(RichText::new("Start a chat").font(semibold(18.0)));
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Choose a conversation on the left, or start a new one. Incognito chats are never saved.",
                    )
                    .color(t.fg2),
                )
                .wrap(),
            );
        }
        return;
    };
    header(ui, app, out, t, s, installed, now);
    if s.lifetime == SessionLifetime::Incognito {
        banner(
            ui,
            t.warn,
            t.warn_soft,
            "Incognito: this chat exists only in memory. It disappears when you close it or quit Roost.",
        );
    }
    if let Some((id, model)) = app.chat.model_missing.clone() {
        if id == s.id {
            banner(
                ui,
                t.bad,
                t.bad_soft,
                &format!("The model {model} is not installed. Choose another model above."),
            );
        }
    }
    let omitted = app.chat.omitted(s.id);
    if omitted > 0 {
        ui.label(
            RichText::new(format!(
                "{omitted} earlier messages were left out of the last request to fit the model's context."
            ))
            .size(12.0)
            .color(t.fg3),
        );
    }
    let badge = app.chat.badge(s.id);
    let model = row_model(app, s);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            let last = s.messages.len().saturating_sub(1);
            for (i, m) in s.messages.iter().enumerate() {
                message_card(ui, t, m, &model, badge);
                if i == last {
                    retry_row(ui, app, out, s, m);
                }
            }
        });
}

/// The model shown for a session: the rail row is the live source.
fn row_model(app: &AppState, s: &Session) -> String {
    app.chat
        .rows()
        .iter()
        .find(|r| r.id == s.id)
        .map_or_else(|| s.model.clone(), |r| r.model.clone())
}

fn retry_row(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    s: &Session,
    last: &Message,
) {
    let retryable = matches!(
        &last.content,
        MessageContent::Assistant {
            status: AssistantStatus::Failed
                | AssistantStatus::Cancelled
                | AssistantStatus::Interrupted,
            ..
        }
    );
    if retryable && !app.chat.is_running(s.id) && small_button(ui, "Retry") {
        if let SendAttempt::Sent(cmd) = app.chat.try_retry(s.id) {
            out.push(BackendRequest::Chat(cmd));
        }
    }
}

fn header(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    installed: &[String],
    now: Instant,
) {
    let row = app.chat.rows().iter().find(|r| r.id == s.id).cloned();
    let title = row.as_ref().map_or(s.title.clone(), |r| r.title.clone());
    let shown_title = row
        .as_ref()
        .map_or(s.title.clone(), super::state::display_title);
    let pinned = row.as_ref().is_some_and(|r| r.pinned);
    let model = row.as_ref().map_or(s.model.clone(), |r| r.model.clone());
    let running = app.chat.is_running(s.id);
    let incognito = s.lifetime == SessionLifetime::Incognito;
    let top = ui.cursor().min.y;
    ui.horizontal(|ui| {
        // Right-aligned controls first so the title takes what is left.
        let mut title_room = ui.available_width();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let delete_key = s.id.to_string();
            let armed = app.chat_ui.confirm.is_armed(&delete_key, now);
            let delete_label = if armed { "Confirm delete" } else { "Delete" };
            let del = secondary(ui, delete_label, true);
            #[cfg(test)]
            {
                app.chat_ui.probe.delete_button = Some((del.rect, delete_label.to_string()));
            }
            if del.clicked() && app.chat_ui.confirm.click(&delete_key, now) {
                out.push(BackendRequest::Chat(ChatCommand::Delete(s.id)));
            }
            if !incognito && secondary(ui, if pinned { "Unpin" } else { "Pin" }, true).clicked() {
                out.push(BackendRequest::Chat(ChatCommand::SetPinned(s.id, !pinned)));
            }
            if secondary(ui, "Rename", true).clicked() {
                app.chat_ui.renaming = Some((s.id, title.clone()));
            }
            let current = model.clone();
            let mut choice = None;
            ui.add_enabled_ui(!running, |ui| {
                egui::ComboBox::from_id_salt(("chat-model", s.id))
                    .width(180.0)
                    .selected_text(truncate_chars(&current, 26))
                    .show_ui(ui, |ui| {
                        let mut names: Vec<&str> = installed.iter().map(String::as_str).collect();
                        if !names.contains(&current.as_str()) {
                            names.insert(0, &current);
                        }
                        for name in names {
                            if ui.selectable_label(name == current, name).clicked() {
                                choice = Some(name.to_string());
                            }
                        }
                    });
            });
            if let Some(m) = choice.filter(|m| *m != current) {
                out.push(BackendRequest::Chat(ChatCommand::SetModel(s.id, m)));
            }
            title_room = ui.available_width();
        });
        let _ = title_room;
    });
    // The title gets its own line so nothing can overlap it.
    ui.add_space(2.0);
    match app.chat_ui.renaming.clone() {
        Some((id, mut text)) if id == s.id => {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(ui.available_width())
                    .font(semibold(16.0)),
            );
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let escape = ui.input(|i| i.key_pressed(Key::Escape));
            if enter {
                out.push(BackendRequest::Chat(ChatCommand::Rename(id, text.clone())));
                app.chat_ui.renaming = None;
            } else if escape || edit.lost_focus() {
                app.chat_ui.renaming = None;
            } else {
                app.chat_ui.renaming = Some((id, text));
            }
        }
        _ => {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    loaded_status(ui, app, out, t, &model);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&shown_title).font(semibold(16.0)).color(t.fg),
                            )
                            .truncate(),
                        );
                    });
                });
            });
        }
    }
    #[cfg(test)]
    {
        let bottom = ui.cursor().min.y;
        app.chat_ui.probe.header = Some(egui::Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), top),
            egui::pos2(ui.max_rect().right(), bottom),
        ));
    }
    let _ = top;
    ui.add_space(8.0);
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn message_card(ui: &mut egui::Ui, t: &Tokens, m: &Message, model: &str, badge: RunBadge) {
    let (who, fill, body, status, error, thinking) = match &m.content {
        MessageContent::User { text, .. } => ("You", t.accent_soft, text.clone(), None, None, None),
        MessageContent::Assistant {
            text,
            status,
            error,
            thinking,
            ..
        } => (
            model,
            t.surface,
            text.clone(),
            Some(*status),
            error.clone(),
            thinking.clone(),
        ),
        MessageContent::Tool { name, output, .. } => (
            name.as_str(),
            t.surface2,
            output.clone().unwrap_or_default(),
            None,
            None,
            None,
        ),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, t.line))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(who).font(semibold(12.0)).color(t.fg2));
                if let Some(label) = status.and_then(status_label) {
                    let (fg, bg) = match status {
                        Some(AssistantStatus::Failed) => (t.bad, t.bad_soft),
                        _ => (t.warn, t.warn_soft),
                    };
                    tag(ui, label, fg, bg);
                }
            });
            if let Some(th) = thinking.filter(|x| !x.is_empty()) {
                egui::CollapsingHeader::new(RichText::new("Thinking").size(12.0).color(t.fg3))
                    .id_salt(("thinking", m.id))
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(th).size(12.5).color(t.fg2))
                                .selectable(true)
                                .wrap(),
                        );
                    });
            }
            if body.is_empty() && status == Some(AssistantStatus::Streaming) {
                let wait = if badge == RunBadge::Waiting {
                    "Waiting for model…"
                } else {
                    "Loading model…"
                };
                ui.label(RichText::new(wait).color(t.fg3));
            } else if !body.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(body).size(14.0).color(t.fg))
                        .selectable(true)
                        .wrap(),
                );
            }
            if let Some(e) = error {
                ui.add(egui::Label::new(RichText::new(e).size(12.5).color(t.bad)).wrap());
            }
        });
    ui.add_space(8.0);
}

fn inspect(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    id: SessionId,
    now: Instant,
) {
    let Some(row) = app.chat.rows().iter().find(|r| r.id == id).cloned() else {
        app.chat_ui.inspect = None;
        return;
    };
    let why = match row.status {
        RowStatus::Quarantined => {
            "This session file was damaged. Roost set it aside and kept it for diagnosis."
        }
        RowStatus::NewerSchema => {
            "This session was made by a newer version of Roost. It is left untouched so it still works there."
        }
        RowStatus::Unreadable => "This session file could not be read right now.",
        RowStatus::Ok => "This session is available.",
    };
    ui.label(RichText::new(&row.title).font(semibold(16.0)));
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(why).color(t.fg2)).wrap());
    ui.add_space(8.0);
    let key = id.to_string();
    let label = if app.chat_ui.confirm.is_armed(&key, now) {
        "Confirm delete"
    } else {
        "Delete"
    };
    let r = secondary(ui, label, true);
    #[cfg(test)]
    {
        app.chat_ui.probe.delete_button = Some((r.rect, label.to_string()));
    }
    if r.clicked() && app.chat_ui.confirm.click(&key, now) {
        out.push(BackendRequest::Chat(ChatCommand::Delete(id)));
        app.chat_ui.inspect = None;
    }
}

// ------------------------------------------------------------ loaded models

/// Ollama lists `llama3` and `llama3:latest` for the same model.
fn same_model(a: &str, b: &str) -> bool {
    a.strip_suffix(":latest").unwrap_or(a) == b.strip_suffix(":latest").unwrap_or(b)
}

/// "unloads in 5 min", "stays loaded" and so on, from Ollama's `expires_at`.
pub(super) fn unload_note(expires_at: &str, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(expires_at)
        .ok()?
        .to_utc();
    let left = at - now;
    Some(if left > chrono::Duration::hours(24) {
        "stays loaded".to_string()
    } else if left <= chrono::Duration::zero() {
        "unloading".to_string()
    } else if left < chrono::Duration::minutes(1) {
        "unloads in under a minute".to_string()
    } else {
        format!(
            "unloads in {} min",
            (left.num_milliseconds() + 59_999) / 60_000
        )
    })
}

/// Right end of the conversation title line: whether this chat's model is in
/// memory, and a button to unload it.
fn loaded_status(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    model: &str,
) {
    if !app.ollama.as_ref().is_some_and(|o| o.is_some()) {
        return;
    }
    let found = app
        .running
        .iter()
        .find(|m| same_model(&m.name, model))
        .cloned();
    match found {
        Some(m) => {
            let note = m
                .expires_at
                .as_deref()
                .and_then(|e| unload_note(e, chrono::Utc::now()));
            let r = secondary(ui, "Unload", true).on_hover_text("Free this model's memory now");
            #[cfg(test)]
            {
                app.chat_ui.probe.header_unload = Some(r.rect);
            }
            if r.clicked() {
                out.push(BackendRequest::UnloadModel(m.name.clone()));
                app.running.retain(|x| x.name != m.name);
            }
            let text = match note {
                Some(n) => format!("● Loaded · {n}"),
                None => "● Loaded".to_string(),
            };
            ui.label(RichText::new(text).size(12.0).color(t.ok));
        }
        None => {
            ui.label(RichText::new("○ Not loaded").size(12.0).color(t.fg3))
                .on_hover_text("Ollama loads it when you send a message");
        }
    }
}

/// "In memory" at the bottom of the rail: what Ollama holds, with Unload.
fn loaded_footer(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>, t: &Tokens) {
    section_label(ui, t, "In memory");
    if app.running.is_empty() {
        ui.label(RichText::new("Nothing loaded").size(12.0).color(t.fg3));
        return;
    }
    let shown: Vec<_> = app.running.iter().take(3).cloned().collect();
    for m in &shown {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let r = secondary(ui, "Unload", true);
                #[cfg(test)]
                app.chat_ui.probe.footer_unload.push(r.rect);
                if r.clicked() {
                    out.push(BackendRequest::UnloadModel(m.name.clone()));
                    app.running.retain(|x| x.name != m.name);
                }
                ui.label(
                    RichText::new(crate::browse::results::format_size(m.size))
                        .size(11.5)
                        .color(t.fg3),
                );
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&m.name).size(12.0).color(t.fg2)).truncate(),
                    );
                });
            });
        });
    }
    if app.running.len() > shown.len() {
        ui.label(
            RichText::new(format!("and {} more", app.running.len() - shown.len()))
                .size(11.5)
                .color(t.fg3),
        );
    }
}

// ------------------------------------------------------------- the composer

fn composer(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
) {
    let id = s.id;
    let running = app.chat.is_running(id);
    if let Some(n) = app.chat.notice.clone() {
        ui.label(RichText::new(n).size(12.5).color(t.warn));
        ui.add_space(4.0);
    }
    let eid = composer_id(id);
    if std::mem::take(&mut app.chat_ui.focus_composer) {
        ui.memory_mut(|m| m.request_focus(eid));
    }
    // Plain Enter never types a newline here; it sends, or does nothing
    // while a reply is running. Shift+Enter still adds a line.
    let focused = ui.memory(|m| m.has_focus(eid));
    let enter = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let mut send = enter;
    let mut stop = false;
    ui.horizontal(|ui| {
        let button_w = 76.0;
        let edit_w = (ui.available_width() - button_w - 10.0).max(80.0);
        ui.add(
            egui::TextEdit::multiline(app.chat.draft_mut(id))
                .id(eid)
                .desired_rows(COMPOSER_ROWS)
                .desired_width(edit_w)
                .hint_text("Message (Enter to send, Shift+Enter for a new line)"),
        );
        ui.vertical(|ui| {
            ui.set_width(button_w);
            let has_text = !app.chat.draft(id).trim().is_empty();
            if running {
                let r = secondary(ui, "Stop", true);
                #[cfg(test)]
                {
                    app.chat_ui.probe.send_button = Some((r.rect, "Stop"));
                }
                stop = r.clicked();
            } else {
                let r = primary_button(ui, "Send", has_text);
                #[cfg(test)]
                {
                    app.chat_ui.probe.send_button = Some((r.rect, "Send"));
                }
                send |= r.clicked();
            }
        });
    });
    if stop {
        out.push(BackendRequest::Chat(ChatCommand::Stop(id)));
    }
    if send {
        if let SendAttempt::Sent(cmd) = app.chat.try_send(id) {
            out.push(BackendRequest::Chat(cmd));
        }
    }
}
