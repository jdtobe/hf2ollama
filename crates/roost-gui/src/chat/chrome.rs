//! The chrome around a conversation, kept to a few quiet controls: a title
//! row (title, folder chip for agents, an overflow menu), a status line, a
//! model chip under the input, and tool steps folded into one summary.

use super::view::{composer_id, same_model, secondary, tool_card, truncate_chars, unload_note};
use crate::app::AppState;
use crate::icons::Icon;
use crate::runtime::BackendRequest;
use crate::theme::{mono, semibold, Tokens};
use crate::widgets::tag;
use eframe::egui::{
    self,
    containers::menu::{MenuButton, MenuConfig},
    Align, Key, Layout, PopupCloseBehavior, RichText,
};
use roost_core::agent::registry::{self, Capability, Family};
use roost_core::chat::ChatCommand;
use roost_core::session::{
    Message, MessageContent, Permission, Session, SessionLifetime, SessionMode, ToolStatus,
};
use std::time::Instant;

/// A menu that stays open while its controls are used.
fn sticky() -> MenuConfig {
    MenuConfig::default().close_behavior(PopupCloseBehavior::CloseOnClickOutside)
}

/// The folder with the home directory written as `~`.
pub(super) fn home_relative(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(home) = home
        .map(std::path::PathBuf::from)
        .filter(|h| !h.as_os_str().is_empty())
    {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display())
            };
        }
    }
    path.display().to_string()
}

/// What the status dot says: idle, working (with the step of an agent) or
/// waiting for the user's answer.
pub(super) fn status_of(app: &AppState, s: &Session) -> (StatusKind, String) {
    if app.chat.approval(s.id).is_some() {
        return (StatusKind::Waiting, "Waiting for you".to_string());
    }
    if app.chat.is_running(s.id) {
        let label = match (s.mode, app.chat.step_of(s.id)) {
            (SessionMode::Agent, n) if n > 0 => {
                format!("Working · step {n} of {}", s.step_budget)
            }
            (SessionMode::Agent, _) => "Working".to_string(),
            _ => "Replying".to_string(),
        };
        return (StatusKind::Working, label);
    }
    (StatusKind::Idle, "Idle".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusKind {
    Idle,
    Working,
    Waiting,
}

/// Side of the rename pencil beside the title.
const PENCIL_W: f32 = 24.0;

/// The folder under an agent's title: smaller than body text so it stays
/// quiet and fits more of the path.
const PATH_FONT: f32 = 11.0;

/// Below this pane width the Agent settings button is the gear alone.
const COMPACT_PANE_W: f32 = 600.0;

/// Height of the square icon buttons in the header.
const HEADER_BTN: f32 = 32.0;

/// A bordered square icon button.
fn icon_button(ui: &mut egui::Ui, t: &Tokens, icon: Icon, tip: &str) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(HEADER_BTN, HEADER_BTN), egui::Sense::click());
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    let fill = if resp.hovered() {
        t.surface2
    } else {
        t.surface
    };
    ui.painter().rect(
        rect,
        8.0,
        fill,
        egui::Stroke::new(1.0, t.line),
        egui::StrokeKind::Inside,
    );
    crate::icons::paint(ui.painter(), rect, icon, t.fg2);
    resp
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
    let shown = row
        .as_ref()
        .map_or(s.title.clone(), super::state::display_title);
    let pinned = row.as_ref().is_some_and(|r| r.pinned);
    let incognito = s.lifetime == SessionLifetime::Incognito;
    let (kind, status) = status_of(app, s);
    // The band is painted behind the content once its height is known.
    let band = ui.painter().add(egui::Shape::Noop);
    let pane = ui.max_rect();
    let start = ui.cursor().min.y;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            header_actions(ui, app, out, t, s, pinned, incognito, now);
            if s.mode == SessionMode::Agent {
                agent_settings(ui, app, out, t, s);
                capability_chips(ui, app, t, s);
            }
            if incognito {
                tag(ui, "Incognito", t.accent, t.accent_soft);
            }
            let (colour, glow) = match kind {
                StatusKind::Idle => (t.fg3, false),
                StatusKind::Working => (t.accent, true),
                StatusKind::Waiting => (t.warn, true),
            };
            let label = ui.label(RichText::new(&status).size(12.5).color(t.fg2));
            #[cfg(test)]
            {
                app.chat_ui.probe.status_rect = Some(label.rect);
            }
            let dot = egui::Rect::from_center_size(
                egui::pos2(label.rect.left() - 10.0, label.rect.center().y),
                egui::vec2(9.0, 9.0),
            );
            ui.add_space(14.0);
            if glow {
                ui.painter()
                    .circle_filled(dot.center(), 7.0, colour.gamma_multiply(0.25));
            }
            ui.painter().circle_filled(dot.center(), 4.5, colour);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                title_block(ui, app, out, t, s, &shown);
            });
        });
    });
    #[cfg(test)]
    {
        app.chat_ui.probe.status_label = Some(status.clone());
    }
    ui.add_space(10.0);
    let bottom = ui.cursor().min.y;
    // Reach over the pane's margins; with nothing above the header the band
    // starts at the top edge of the pane.
    let top = if start - pane.top() < 1.0 {
        pane.top() - super::view::PANE_MARGIN_Y
    } else {
        start - 8.0
    };
    let rect = egui::Rect::from_min_max(
        egui::pos2(pane.left() - super::view::PANE_MARGIN_X, top),
        egui::pos2(pane.right() + super::view::PANE_MARGIN_X, bottom),
    );
    ui.painter().set(
        band,
        egui::Shape::Vec(vec![
            egui::Shape::rect_filled(rect, 0.0, t.header),
            egui::Shape::hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(1.0, t.line),
            ),
        ]),
    );
    #[cfg(test)]
    {
        app.chat_ui.probe.header = Some(rect);
    }
    ui.add_space(12.0);
}

/// The kind-of-session tile, the title (or the rename field) and, for an
/// agent, its folder.
fn title_block(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    shown: &str,
) {
    let agent = s.mode == SessionMode::Agent;
    let (tile, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
    ui.painter().rect_filled(tile, 9.0, t.accent);
    crate::icons::paint(
        ui.painter(),
        tile,
        if agent { Icon::Agent } else { Icon::Chat },
        t.accent_fg,
    );
    ui.add_space(4.0);
    if app
        .chat_ui
        .renaming
        .as_ref()
        .is_some_and(|(id, _)| *id == s.id)
    {
        title_or_rename(ui, app, out, t, s, shown);
        return;
    }
    let untitled = shown.trim().is_empty() || shown == roost_core::session::UNTITLED;
    let text = if untitled {
        "Untitled".to_string()
    } else {
        shown.to_string()
    };
    let font = semibold(21.0);
    let natural = ui
        .painter()
        .layout_no_wrap(text.clone(), font.clone(), t.fg)
        .size()
        .x;
    let path = s
        .workspace
        .as_ref()
        .filter(|_| agent)
        .map(|p| (home_relative(p), p.display().to_string()));
    // The title over the folder, in one column beside the tile; the folder
    // has the column's whole width because it is not beside the title.
    let mut rename = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // What is left of the row, less the pencil that follows the title.
        let room = (ui.available_width() - PENCIL_W - 8.0).max(40.0);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.set_max_width(room.min(natural));
                let mut rich =
                    RichText::new(&text)
                        .font(font)
                        .color(if untitled { t.fg3 } else { t.fg });
                if untitled {
                    rich = rich.italics();
                }
                // Double-clicking the title edits it too.
                let label = ui.add(
                    egui::Label::new(rich)
                        .truncate()
                        .sense(egui::Sense::click()),
                );
                #[cfg(test)]
                {
                    app.chat_ui.probe.title_rect = Some(label.rect);
                }
                rename |= label.double_clicked();
            });
            // A small pencil beside the title, quiet until pointed at.
            let (pencil, resp) =
                ui.allocate_exact_size(egui::vec2(PENCIL_W, PENCIL_W), egui::Sense::click());
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Rename");
            if resp.hovered() {
                ui.painter().rect_filled(pencil, 6.0, t.surface);
            }
            crate::icons::paint(
                ui.painter(),
                pencil,
                Icon::Edit,
                if resp.hovered() { t.fg } else { t.fg2 },
            );
            #[cfg(test)]
            {
                app.chat_ui.probe.rename_button = Some(pencil);
            }
            rename |= resp.clicked();
        });
        if let Some((short, full)) = path {
            let room = ui.available_width().max(0.0);
            let cut_to = crate::widgets::ellipsize_middle(ui, &short, mono(PATH_FONT), room);
            let cut = cut_to != short;
            let r = ui.label(RichText::new(cut_to).font(mono(PATH_FONT)).color(t.fg2));
            #[cfg(test)]
            {
                app.chat_ui.probe.path_rect = Some(r.rect);
            }
            // Always say where it is on hover; the full path is the point
            // when it was cut.
            r.on_hover_text(if cut {
                full
            } else {
                format!("The agent works in {full}")
            });
        }
    });
    if rename {
        app.chat_ui.renaming = Some((s.id, shown.to_string()));
    }
}

fn title_or_rename(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    _t: &Tokens,
    s: &Session,
    _shown: &str,
) {
    if let Some((id, mut text)) = app.chat_ui.renaming.clone().filter(|(id, _)| *id == s.id) {
        let edit = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(ui.available_width())
                .font(semibold(18.0)),
        );
        if !edit.has_focus() && !edit.lost_focus() {
            edit.request_focus();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let escape = ui.input(|i| i.key_pressed(Key::Escape));
        if escape {
            app.chat_ui.renaming = None;
        } else if enter || edit.lost_focus() {
            // Enter, or clicking away, keeps the new name.
            if !text.trim().is_empty() {
                out.push(BackendRequest::Chat(ChatCommand::Rename(id, text.clone())));
            }
            app.chat_ui.renaming = None;
        } else {
            app.chat_ui.renaming = Some((id, text));
        }
    }
}

/// Pin and delete, as icons: a pin that fills when the session is pinned, and
/// a red trash can that asks twice (the second time it says so).
#[allow(clippy::too_many_arguments)]
fn header_actions(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    pinned: bool,
    incognito: bool,
    now: Instant,
) {
    // Right to left: the trash can sits at the far edge, pin next to it.
    let key = s.id.to_string();
    let armed = app.chat_ui.confirm.is_armed(&key, now);
    let del = if armed {
        let label = "Click again to delete";
        let galley = ui.painter().layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(12.5),
            egui::Color32::WHITE,
        );
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(galley.size().x + 24.0, HEADER_BTN),
            egui::Sense::click(),
        );
        ui.painter().rect_filled(rect, 8.0, t.bad);
        ui.painter().galley(
            rect.center() - galley.size() / 2.0,
            galley,
            egui::Color32::WHITE,
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
        resp
    } else {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(HEADER_BTN, HEADER_BTN), egui::Sense::click());
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Delete this session…");
        ui.painter().rect(
            rect,
            8.0,
            if resp.hovered() {
                t.bad_soft
            } else {
                t.surface
            },
            egui::Stroke::new(1.0, t.bad.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        crate::icons::paint(ui.painter(), rect, Icon::Trash, t.bad);
        resp
    };
    #[cfg(test)]
    {
        let label = if armed {
            "Click again to delete"
        } else {
            "Delete…"
        };
        app.chat_ui.probe.delete_button = Some((del.rect, label.to_string()));
    }
    if del.clicked() && app.chat_ui.confirm.click(&key, now) {
        out.push(BackendRequest::Chat(ChatCommand::Delete(s.id)));
    }
    if !incognito {
        let pin = icon_button(
            ui,
            t,
            Icon::Pin,
            if pinned {
                "Unpin"
            } else {
                "Pin to the top of the list"
            },
        );
        if pinned {
            // Filled and in the accent while pinned.
            ui.painter()
                .rect_filled(pin.rect.shrink(1.0), 7.0, t.accent_soft);
            crate::icons::paint(ui.painter(), pin.rect, Icon::Pin, t.accent);
        }
        #[cfg(test)]
        {
            app.chat_ui.probe.pin_button = Some(pin.rect);
        }
        if pin.clicked() {
            out.push(BackendRequest::Chat(ChatCommand::SetPinned(s.id, !pinned)));
        }
    }
}

/// "+3 −1" in the colours of the diff.
fn counts(ui: &mut egui::Ui, t: &Tokens, added: u32, removed: u32) {
    ui.label(
        RichText::new(format!("+{added}"))
            .font(mono(12.0))
            .color(t.ok),
    );
    ui.label(
        RichText::new(format!("−{removed}"))
            .font(mono(12.0))
            .color(crate::app::bad(ui.visuals().dark_mode)),
    );
}

/// Under a group of tool calls: one row for each file those calls changed,
/// with its diff on request and a way to take the change back.
pub(super) fn change_rows(
    ui: &mut egui::Ui,
    t: &Tokens,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    s: &Session,
    msgs: &[Message],
) {
    let running = app.chat.is_running(s.id);
    for rec in s
        .changes
        .iter()
        .filter(|c| msgs.iter().any(|m| m.id == c.message))
    {
        let key = (s.id, rec.id);
        let open = app.chat_ui.diffs_open.contains(&key);
        let mut toggle = false;
        let mut undo = false;
        let frame = egui::Frame::new()
            .fill(t.surface2)
            .stroke(egui::Stroke::new(1.0, t.line))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                ui.horizontal_wrapped(|ui| {
                    let what = if rec.created { "Created" } else { "Changed" };
                    ui.label(RichText::new(what).color(t.fg2));
                    ui.label(RichText::new(&rec.path).font(mono(12.5)).color(t.fg));
                    counts(ui, t, rec.added, rec.removed);
                });
                ui.label(
                    RichText::new(if rec.undone {
                        "Undone: the original was put back."
                    } else {
                        "The original was kept before the change."
                    })
                    .size(11.5)
                    .color(t.fg3),
                );
                ui.horizontal_wrapped(|ui| {
                    let show = secondary(ui, if open { "Hide diff" } else { "Show diff" }, true);
                    toggle = show.clicked();
                    #[cfg(test)]
                    {
                        app.chat_ui
                            .probe
                            .show_diff_buttons
                            .push((rec.id, show.rect));
                    }
                    if !rec.undone {
                        let b = secondary(ui, "Undo this change", !running);
                        undo = b.clicked();
                        #[cfg(test)]
                        {
                            app.chat_ui.probe.undo_buttons.push((rec.id, b.rect));
                        }
                    }
                });
                if open {
                    super::view::diff_box(ui, t, &rec.preview);
                }
            });
        #[cfg(test)]
        {
            app.chat_ui
                .probe
                .change_rows
                .push((rec.id, frame.response.rect));
        }
        let _ = frame;
        if toggle {
            if open {
                app.chat_ui.diffs_open.remove(&key);
            } else {
                app.chat_ui.diffs_open.insert(key);
            }
        }
        if undo {
            out.push(BackendRequest::Chat(ChatCommand::UndoChange {
                session: s.id,
                change: rec.id,
            }));
        }
        ui.add_space(6.0);
    }
}

/// The Changes made card of the agent settings: every file touched, and one
/// button that takes them all back, newest first.
#[cfg_attr(not(test), allow(unused_variables))]
fn changes_made(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    running: bool,
) {
    let open_count = s.changes.iter().filter(|c| !c.undone).count();
    settings_card(
        ui,
        t,
        "Changes made",
        &format!("{open_count} of {} still in place", s.changes.len()),
        |ui| {
            for rec in &s.changes {
                ui.horizontal(|ui| {
                    let room = (ui.available_width() - 110.0).max(60.0);
                    let shown = crate::widgets::ellipsize_middle(ui, &rec.path, mono(12.0), room);
                    let colour = if rec.undone { t.fg3 } else { t.fg };
                    ui.label(RichText::new(shown).font(mono(12.0)).color(colour));
                    if rec.undone {
                        ui.label(RichText::new("undone").size(11.5).color(t.fg3));
                    } else {
                        counts(ui, t, rec.added, rec.removed);
                    }
                });
            }
            let all = secondary(ui, "Undo all", open_count > 0 && !running);
            #[cfg(test)]
            {
                app.chat_ui.probe.undo_all_button = Some(all.rect);
            }
            if all.clicked() {
                out.push(BackendRequest::Chat(ChatCommand::UndoAll(s.id)));
            }
        },
    );
}

/// What a session has a capability set to, never above its maximum.
fn cap_permission(cfg: &roost_core::session::EffectiveToolConfig, cap: Capability) -> Permission {
    registry::specs()
        .iter()
        .find(|sp| sp.capability == cap)
        .map_or(Permission::Off, |sp| registry::permission_of(cfg, sp))
}

/// The R and W chips beside the Agent settings button: what the agent may do,
/// at a glance. Read is accent when allowed, write is amber when it asks;
/// anything off is dimmed. No W when writing is off in Settings.
fn capability_chips(ui: &mut egui::Ui, app: &mut AppState, t: &Tokens, s: &Session) {
    let dark = ui.visuals().dark_mode;
    #[cfg(test)]
    {
        app.chat_ui.probe.chips.clear();
    }
    // Right-to-left: X is placed first, so W and then R end up on its left.
    let mut chips = Vec::new();
    if app.settings.agents_can_run {
        chips.push((Capability::Run, 'X', "Running commands"));
    }
    if app.settings.agents_can_write {
        chips.push((Capability::Write, 'W', "Writing files"));
    }
    chips.push((Capability::Read, 'R', "Reading files"));
    for (cap, letter, what) in chips {
        let perm = cap_permission(&s.effective_tools, cap);
        // Running commands is never drawn in the calm accent colour.
        let danger = cap == Capability::Run && perm == Permission::Auto;
        let (fg, fill, stroke) = match perm {
            Permission::Auto if danger => {
                (crate::app::bad(dark), t.surface2, crate::app::bad(dark))
            }
            Permission::Auto => (t.accent, t.accent_soft, t.accent),
            Permission::Ask => (t.warn, t.warn_soft, t.warn),
            Permission::Off => (t.fg3, t.surface2, t.line),
        };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::hover());
        ui.painter().rect(
            rect,
            6.0,
            fill,
            egui::Stroke::new(1.0, stroke.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            semibold(12.0),
            fg,
        );
        let state = match perm {
            Permission::Auto => "allowed without asking",
            Permission::Ask => "asks first",
            Permission::Off => "off",
        };
        resp.on_hover_text(format!("{what}: {state}"));
        #[cfg(test)]
        {
            app.chat_ui.probe.chips.push((letter, rect, perm));
        }
    }
    #[cfg(test)]
    {
        app.chat_ui
            .probe
            .chips
            .sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    }
}

/// A card inside the agent settings: a small heading over a rounded panel.
fn settings_card<R>(
    ui: &mut egui::Ui,
    t: &Tokens,
    title: &str,
    note: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(t.surface2)
        .stroke(egui::Stroke::new(1.0, t.line))
        .corner_radius(9.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(title.to_uppercase())
                        .font(semibold(11.0))
                        .color(t.fg3),
                );
                if !note.is_empty() {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(note).size(11.5).color(t.fg3));
                    });
                }
            });
            add(ui)
        })
        .inner
}

/// The agent's folder, step budget and tools, behind one button that says
/// so.
fn agent_settings(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
) {
    let running = app.chat.is_running(s.id);
    let id = s.id;
    let label = "Agent settings";
    // A narrow pane keeps the room for the title and the folder: the button
    // is the gear alone, and its hover text says what it is.
    let compact = ui.max_rect().width() < COMPACT_PANE_W;
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), t.fg);
    let size = if compact {
        egui::vec2(HEADER_BTN + 4.0, HEADER_BTN)
    } else {
        egui::vec2(galley.size().x + 44.0, HEADER_BTN)
    };
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if compact {
            "Agent settings: the folder this agent works in, its step budget and its tools"
        } else {
            "The folder this agent works in, its step budget and its tools"
        });
    ui.painter().rect(
        rect,
        8.0,
        if resp.hovered() {
            t.surface2
        } else {
            t.surface
        },
        egui::Stroke::new(1.0, t.line),
        egui::StrokeKind::Inside,
    );
    let gear_at = if compact {
        rect.center()
    } else {
        egui::pos2(rect.left() + 18.0, rect.center().y)
    };
    crate::icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(gear_at, egui::vec2(20.0, 20.0)),
        Icon::Gear,
        t.fg2,
    );
    if !compact {
        ui.painter().galley(
            egui::pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0),
            galley,
            t.fg,
        );
    }
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(340.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            // The cards can be taller than the room below the button, so the
            // popover scrolls instead of running off the bottom of the window.
            let room = (ui.ctx().content_rect().bottom() - rect.bottom() - 24.0).max(200.0);
            ui.set_max_height(room);
            egui::ScrollArea::vertical()
                .max_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    settings_card(ui, t, "Folder", "the agent works here", |ui| {
                        let path = s
                            .workspace
                            .as_ref()
                            .map_or(String::new(), |p| p.display().to_string());
                        ui.add(
                            egui::Label::new(RichText::new(path).font(mono(12.0)).color(t.fg2))
                                .wrap(),
                        );
                        if secondary(ui, "Change…", !running).clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_title("Choose the agent's folder")
                                .pick_folder()
                            {
                                out.push(BackendRequest::Chat(ChatCommand::SetWorkspace(id, path)));
                            }
                        }
                    });
                    settings_card(ui, t, "Run", "", |ui| {
                        ui.add_enabled_ui(!running, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Step budget").size(12.5).color(t.fg2));
                                let mut steps = s.step_budget;
                                if ui
                                    .add(egui::DragValue::new(&mut steps).range(1..=100))
                                    .changed()
                                {
                                    out.push(BackendRequest::Chat(ChatCommand::SetStepBudget(
                                        id, steps,
                                    )));
                                }
                            });
                        });
                    });
                    #[cfg(test)]
                    {
                        app.chat_ui.probe.permission_buttons.clear();
                        app.chat_ui.probe.write_buttons.clear();
                        app.chat_ui.probe.run_buttons.clear();
                        app.chat_ui.probe.run_confirm = None;
                        app.chat_ui.probe.tools_heads.clear();
                    }
                    for family in [Family::Files, Family::Shell] {
                        if family == Family::Shell && !app.settings.agents_can_run {
                            continue;
                        }
                        family_card(ui, app, out, t, s, family, running);
                    }
                    if !s.changes.is_empty() {
                        changes_made(ui, app, out, t, s, running);
                    }
                    if running {
                        ui.label(
                            RichText::new("Locked while a run is active.")
                                .size(11.5)
                                .color(t.fg3),
                        );
                    }
                });
        });
    #[cfg(test)]
    {
        app.chat_ui.probe.agent_chip = Some(rect);
    }
}

/// One card per family of tools: a Off / Ask / Allow row for each ability the
/// family has, then its tools, each with a switch. The Shell family is only
/// drawn when commands are switched on in Settings.
fn family_card(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    s: &Session,
    family: Family,
    running: bool,
) {
    let id = s.id;
    let writes = app.settings.agents_can_write;
    let specs: Vec<_> = registry::specs()
        .into_iter()
        .filter(|sp| sp.family == family && (writes || sp.capability != Capability::Write))
        .collect();
    let on_count = specs
        .iter()
        .filter(|sp| registry::tool_enabled(&s.effective_tools, sp))
        .count();
    let note = format!("{on_count} of {} tools on", specs.len());
    settings_card(ui, t, family.label(), &note, |ui| {
        ui.add_enabled_ui(!running, |ui| {
            for cap in [Capability::Read, Capability::Write, Capability::Run] {
                if !specs.iter().any(|sp| sp.capability == cap) {
                    continue;
                }
                let (title, sub, tips): (&str, &str, &[(&str, &str)]) = match cap {
                    Capability::Read => (
                        "Read files",
                        "list, find, search, read",
                        &[
                            ("Off", "The agent cannot read files"),
                            ("Ask", "Ask before each file is read"),
                            ("Allow", "Read files without asking"),
                        ],
                    ),
                    Capability::Write => (
                        "Write files",
                        "create and change files in the folder",
                        &[
                            ("Off", "The agent cannot change files"),
                            ("Ask", "Ask before each change, with the changes shown"),
                            (
                                "Allow",
                                "Change files without asking. Each change is kept so you can undo it",
                            ),
                        ],
                    ),
                    Capability::Run => (
                        "Run commands",
                        "runs as you, not limited to the folder",
                        &[
                            ("Off", "The agent cannot run commands"),
                            ("Ask", "Ask before each command, showing exactly what will run"),
                            (
                                "Allow",
                                "Run commands without asking. They run as you and can change anything you can",
                            ),
                        ],
                    ),
                };
                ui.label(RichText::new(title).font(semibold(13.0)).color(t.fg));
                ui.label(RichText::new(sub).size(11.5).color(t.fg3));
                let current = cap_permission(&s.effective_tools, cap);
                let chosen = match current {
                    Permission::Off => 0,
                    Permission::Ask => 1,
                    Permission::Auto => 2,
                };
                let mut rects = Vec::new();
                let picked = crate::widgets::segmented(ui, t, tips, chosen, &mut rects);
                #[cfg(test)]
                {
                    match cap {
                        Capability::Read => app.chat_ui.probe.permission_buttons = rects.clone(),
                        Capability::Write => app.chat_ui.probe.write_buttons = rects.clone(),
                        Capability::Run => app.chat_ui.probe.run_buttons = rects.clone(),
                    }
                }
                let _ = rects;
                if let Some(i) = picked {
                    let value = [Permission::Off, Permission::Ask, Permission::Auto][i];
                    let key = registry::capability_key(family, cap);
                    if cap == Capability::Run && value == Permission::Auto {
                        // Running commands without asking is confirmed once.
                        app.chat_ui.run_allow_pending = Some(id);
                    } else {
                        if cap == Capability::Run {
                            app.chat_ui.run_allow_pending = None;
                        }
                        out.push(BackendRequest::Chat(ChatCommand::SetCapability(
                            id, key, value,
                        )));
                    }
                }
                if cap == Capability::Run && app.chat_ui.run_allow_pending == Some(id) {
                    run_allow_confirm(ui, app, out, t, id, family);
                }
            }
            let slot = (id, family as u8);
            let open = app.chat_ui.tools_open.contains(&slot);
            let arrow = if open { "▾" } else { "▸" };
            let head = ui
                .add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{arrow}  Individual tools · {on_count} of {} on",
                            specs.len()
                        ))
                        .size(11.5)
                        .color(t.fg3),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            #[cfg(test)]
            {
                app.chat_ui.probe.tools_heads.push(head.rect);
            }
            if head.clicked() {
                if open {
                    app.chat_ui.tools_open.remove(&slot);
                } else {
                    app.chat_ui.tools_open.insert(slot);
                }
            }
            if open {
                for spec in specs {
                    let mut on = registry::tool_enabled(&s.effective_tools, &spec);
                    let tag = match spec.capability {
                        Capability::Read => 'R',
                        Capability::Write => 'W',
                        Capability::Run => 'X',
                    };
                    if ui
                        .checkbox(&mut on, format!("{tag}   {}", spec.name))
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
            }
        });
    });
}

/// The question after choosing Allow for Run. Cancel is the quiet default.
fn run_allow_confirm(
    ui: &mut egui::Ui,
    app: &mut AppState,
    out: &mut Vec<BackendRequest>,
    t: &Tokens,
    id: roost_core::session::SessionId,
    family: Family,
) {
    let dark = ui.visuals().dark_mode;
    ui.add(
        egui::Label::new(
            RichText::new(
                "Commands will run without asking. They run as you and can change or delete anything you can.",
            )
            .size(12.0)
            .color(crate::app::bad(dark)),
        )
        .wrap(),
    );
    ui.horizontal(|ui| {
        let cancel = ui.add(egui::Button::new(RichText::new("Cancel").size(12.5)));
        let confirm = ui.add(
            egui::Button::new(
                RichText::new("Run without asking")
                    .size(12.5)
                    .color(crate::app::bad(dark)),
            )
            .stroke(egui::Stroke::new(1.0, crate::app::bad(dark))),
        );
        #[cfg(test)]
        {
            app.chat_ui.probe.run_confirm = Some((confirm.rect, cancel.rect));
        }
        if confirm.clicked() {
            app.chat_ui.run_allow_pending = None;
            out.push(BackendRequest::Chat(ChatCommand::SetCapability(
                id,
                registry::capability_key(family, Capability::Run),
                Permission::Auto,
            )));
        } else if cancel.clicked() {
            app.chat_ui.run_allow_pending = None;
        }
    });
    let _ = t;
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
