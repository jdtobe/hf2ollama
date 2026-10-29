//! The chrome around a conversation, kept to a few quiet controls: a title
//! row (title, folder chip for agents, an overflow menu), a status line, a
//! model chip under the input, and tool steps folded into one summary.

use super::view::{composer_id, same_model, secondary, tool_card, truncate_chars, unload_note};
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{semibold, Tokens};
use crate::widgets::tag;
use eframe::egui::{
    self,
    containers::menu::{MenuButton, MenuConfig},
    Align, Key, Layout, PopupCloseBehavior, RichText,
};
use roost_core::agent::registry;
use roost_core::chat::ChatCommand;
use roost_core::session::{
    Message, MessageContent, Permission, Session, SessionLifetime, SessionMode, ToolStatus,
};
use std::time::Instant;

/// A menu that stays open while its controls are used.
fn sticky() -> MenuConfig {
    MenuConfig::default().close_behavior(PopupCloseBehavior::CloseOnClickOutside)
}

/// The folder's own name, for the chip.
fn folder_name(s: &Session) -> String {
    s.workspace
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or_else(
            || "folder".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
}

pub(super) fn header(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    now: Instant,
) {
    let row = app.chat.rows().iter().find(|r| r.id == s.id).cloned();
    let title = row.as_ref().map_or(s.title.clone(), |r| r.title.clone());
    let shown = row
        .as_ref()
        .map_or(s.title.clone(), super::state::display_title);
    let pinned = row.as_ref().is_some_and(|r| r.pinned);
    let incognito = s.lifetime == SessionLifetime::Incognito;
    #[cfg(test)]
    let top = ui.cursor().min.y;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            more_menu(ui, app, out, t, s, &title, pinned, incognito, now);
            if s.mode == SessionMode::Agent {
                agent_chip(ui, app, out, t, s);
            }
            if incognito {
                tag(ui, "Incognito", t.accent, t.accent_soft);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                title_or_rename(ui, app, out, t, s, &shown);
            });
        });
    });
    #[cfg(test)]
    {
        app.chat_ui.probe.header = Some(egui::Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), top),
            egui::pos2(ui.max_rect().right(), ui.cursor().min.y),
        ));
    }
    ui.add_space(6.0);
}

fn title_or_rename(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    shown: &str,
) {
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
            ui.add(
                egui::Label::new(RichText::new(shown).font(semibold(16.0)).color(t.fg)).truncate(),
            );
        }
    }
}

/// Rename, pin and delete: the actions needed rarely.
#[allow(clippy::too_many_arguments)]
fn more_menu(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    title: &str,
    pinned: bool,
    incognito: bool,
    now: Instant,
) {
    let (button, _) = MenuButton::new(RichText::new("···").size(14.0))
        .config(sticky())
        .ui(ui, |ui| {
            ui.set_min_width(150.0);
            if ui.button("Rename").clicked() {
                app.chat_ui.renaming = Some((s.id, title.to_string()));
                ui.close();
            }
            if !incognito && ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
                out.push(BackendRequest::Chat(ChatCommand::SetPinned(s.id, !pinned)));
                ui.close();
            }
            ui.separator();
            let key = s.id.to_string();
            let armed = app.chat_ui.confirm.is_armed(&key, now);
            let label = if armed {
                "Click again to delete"
            } else {
                "Delete…"
            };
            let del = ui.button(RichText::new(label).color(t.bad));
            #[cfg(test)]
            {
                app.chat_ui.probe.delete_button = Some((del.rect, label.to_string()));
            }
            if armed {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            if del.clicked() && app.chat_ui.confirm.click(&key, now) {
                out.push(BackendRequest::Chat(ChatCommand::Delete(s.id)));
                ui.close();
            }
        });
    #[cfg(test)]
    {
        app.chat_ui.probe.more_button = Some(button.rect);
    }
    let _ = button;
}

/// An agent's folder, step budget and tools behind one chip.
fn agent_chip(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
) {
    let running = app.chat.is_running(s.id);
    let id = s.id;
    let label = format!("Folder: {}", truncate_chars(&folder_name(s), 22));
    let (chip, _) = MenuButton::new(RichText::new(label).size(12.0))
        .config(sticky())
        .ui(ui, |ui| {
            ui.set_min_width(280.0);
            ui.label(RichText::new("Folder").font(semibold(11.0)).color(t.fg3));
            let path = s
                .workspace
                .as_ref()
                .map_or(String::new(), |p| p.display().to_string());
            ui.add(egui::Label::new(RichText::new(path).size(12.0).color(t.fg2)).wrap());
            if secondary(ui, "Change folder…", !running).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose the agent's folder")
                    .pick_folder()
                {
                    out.push(BackendRequest::Chat(ChatCommand::SetWorkspace(id, path)));
                }
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!running, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Step budget").size(12.0).color(t.fg2));
                    let mut steps = s.step_budget;
                    if ui
                        .add(egui::DragValue::new(&mut steps).range(1..=100))
                        .changed()
                    {
                        out.push(BackendRequest::Chat(ChatCommand::SetStepBudget(id, steps)));
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Tools").font(semibold(11.0)).color(t.fg3));
                // Off, Ask each time, or Auto for the Files · Read tools.
                let current = s
                    .effective_tools
                    .capabilities
                    .get("files.read")
                    .copied()
                    .unwrap_or(Permission::Auto);
                #[cfg(test)]
                {
                    app.chat_ui.probe.permission_buttons.clear();
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Files · Read").size(12.5).color(t.fg2));
                    for (value, label, tip) in [
                        (Permission::Off, "Off", "The agent cannot read files"),
                        (Permission::Ask, "Ask", "Ask before each file is read"),
                        (Permission::Auto, "Auto", "Read files without asking"),
                    ] {
                        let r = ui
                            .selectable_label(current == value, RichText::new(label).size(12.5))
                            .on_hover_text(tip);
                        #[cfg(test)]
                        app.chat_ui.probe.permission_buttons.push(r.rect);
                        if r.clicked() && current != value {
                            out.push(BackendRequest::Chat(ChatCommand::SetCapability(
                                id,
                                "files.read".into(),
                                value,
                            )));
                        }
                    }
                });
                for spec in registry::specs() {
                    let mut on = registry::tool_enabled(&s.effective_tools, &spec);
                    if ui
                        .checkbox(&mut on, spec.name)
                        .on_hover_text(spec.description)
                        .changed()
                    {
                        out.push(BackendRequest::Chat(ChatCommand::SetToolEnabled(
                            id,
                            spec.name.into(),
                            on,
                        )));
                    }
                }
            });
            if running {
                ui.label(
                    RichText::new("Locked while a run is active.")
                        .size(11.5)
                        .color(t.fg3),
                );
            }
        });
    #[cfg(test)]
    {
        app.chat_ui.probe.agent_chip = Some(chip.rect);
    }
    let _ = chip;
}

/// One line above the input: progress of a running agent, then any notice.
pub(super) fn status_line(ui: &mut egui::Ui, app: &AppState, t: &Tokens, s: &Session) {
    let mut parts: Vec<String> = Vec::new();
    if s.mode == SessionMode::Agent && app.chat.is_running(s.id) {
        parts.push(format!(
            "Step {} of {}",
            app.chat.step_of(s.id).max(1),
            s.step_budget
        ));
        let waiting = app.chat.queued(s.id).len();
        if waiting > 0 {
            parts.push(format!(
                "{waiting} message{} waiting for the next step",
                if waiting == 1 { "" } else { "s" }
            ));
        }
    }
    if let Some(n) = &app.chat.notice {
        parts.push(n.clone());
    }
    if parts.is_empty() {
        return;
    }
    let notice = app.chat.notice.is_some();
    let color = if notice { t.warn } else { t.accent };
    ui.add(egui::Label::new(RichText::new(parts.join("  ·  ")).size(12.5).color(color)).wrap());
    #[cfg(test)]
    {
        let _ = ui;
    }
}

/// The model in use, whether it is in memory, and a way to change it.
pub(super) fn model_chip(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    installed: &[String],
) {
    let running = app.chat.is_running(s.id);
    let model = app
        .chat
        .rows()
        .iter()
        .find(|r| r.id == s.id)
        .map_or_else(|| s.model.clone(), |r| r.model.clone());
    let loaded = app
        .ollama
        .as_ref()
        .is_some_and(|o| o.is_some())
        .then(|| {
            app.running
                .iter()
                .find(|m| same_model(&m.name, &model))
                .cloned()
        })
        .flatten();
    let dot = if loaded.is_some() { "● " } else { "○ " };
    let label = format!("{dot}{}", truncate_chars(&model, 28));
    let (chip, _) = MenuButton::new(RichText::new(label).size(12.0))
        .config(sticky())
        .ui(ui, |ui| {
            ui.set_min_width(240.0);
            ui.label(RichText::new("Model").font(semibold(11.0)).color(t.fg3));
            ui.add_enabled_ui(!running, |ui| {
                let mut names: Vec<&str> = installed.iter().map(String::as_str).collect();
                if !names.contains(&model.as_str()) {
                    names.insert(0, &model);
                }
                for name in names {
                    if ui.selectable_label(name == model, name).clicked() && name != model {
                        out.push(BackendRequest::Chat(ChatCommand::SetModel(
                            s.id,
                            name.into(),
                        )));
                        ui.close();
                    }
                }
            });
            ui.separator();
            match &loaded {
                Some(m) => {
                    let note = m
                        .expires_at
                        .as_deref()
                        .and_then(|e| unload_note(e, chrono::Utc::now()))
                        .unwrap_or_else(|| "loaded".to_string());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("In memory · {note}"))
                                .size(12.0)
                                .color(t.ok),
                        );
                        let unload = secondary(ui, "Unload", true);
                        #[cfg(test)]
                        {
                            app.chat_ui.probe.model_unload = Some(unload.rect);
                        }
                        if unload.clicked() {
                            out.push(BackendRequest::UnloadModel(m.name.clone()));
                            app.running.retain(|x| x.name != m.name);
                            ui.close();
                        }
                    });
                }
                None => {
                    ui.label(
                        RichText::new("Not in memory. Ollama loads it when you send a message.")
                            .size(12.0)
                            .color(t.fg3),
                    );
                }
            }
        });
    #[cfg(test)]
    {
        app.chat_ui.probe.model_chip = Some(chip.rect);
    }
    let _ = chip;
}

/// Consecutive tool steps folded into one line; open it for the cards.
pub(super) fn tool_group(ui: &mut egui::Ui, t: &Tokens, msgs: &[Message], app: &mut AppState) {
    let mut names: Vec<(String, usize)> = Vec::new();
    let (mut running, mut failed) = (None, 0usize);
    for m in msgs {
        if let MessageContent::Tool { name, status, .. } = &m.content {
            match names.iter_mut().find(|(n, _)| n == name) {
                Some((_, c)) => *c += 1,
                None => names.push((name.clone(), 1)),
            }
            match status {
                ToolStatus::Running => running = Some(name.clone()),
                ToolStatus::Failed => failed += 1,
                _ => {}
            }
        }
    }
    let list = names
        .iter()
        .map(|(n, c)| {
            if *c > 1 {
                format!("{n} ×{c}")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let count = msgs.len();
    let mut head = format!(
        "Used {count} tool{}: {list}",
        if count == 1 { "" } else { "s" }
    );
    if failed > 0 {
        head.push_str(&format!("  ·  {failed} failed"));
    }
    if let Some(n) = &running {
        head.push_str(&format!("  ·  {n} running…"));
    }
    let color = if running.is_some() {
        t.accent
    } else if failed > 0 {
        t.warn
    } else {
        t.fg2
    };
    #[cfg(test)]
    let top = ui.cursor().min.y;
    let first = msgs.first().map(|m| m.id);
    let header = egui::CollapsingHeader::new(RichText::new(head).size(12.5).color(color))
        .id_salt(("tool-group", first))
        .show(ui, |ui| {
            for m in msgs {
                tool_card(ui, t, m, app);
            }
        });
    #[cfg(test)]
    {
        app.chat_ui.probe.tool_groups.push(egui::Rect::from_min_max(
            egui::pos2(header.header_response.rect.left(), top),
            egui::pos2(
                header.header_response.rect.right(),
                header.header_response.rect.bottom(),
            ),
        ));
    }
    let _ = header;
    ui.add_space(6.0);
}

#[allow(dead_code)]
fn _id(id: roost_core::session::SessionId) -> egui::Id {
    composer_id(id)
}
