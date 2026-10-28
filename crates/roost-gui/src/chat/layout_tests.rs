//! Headless layout tests for the Chat view: real painted rectangles at the
//! window sizes the layout is designed for, in every theme family.

use crate::app::{App, View};
use crate::browse::test_util::Harness;
use crate::runtime::BackendRequest;
use eframe::egui::{self, vec2, Pos2, Rect};
use roost_core::chat::{ChatCommand, ChatEvent};
use roost_core::config::{Settings, ThemeFamily};
use roost_core::fit::Machine;
use roost_core::session::{AssistantStatus, RunId, Session, SessionLifetime, TurnId};
use roost_core::store::{IndexRow, RowStatus};
use std::sync::mpsc::Receiver;

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];
const LONG_TITLE: &str =
    "A very long conversation title that keeps going well past what any rail could hold on one line";
const LONG_MODEL: &str =
    "registry.example.com/community/an-extremely-long-model-name-for-testing:70b";

fn row_for(s: &Session, title: &str) -> IndexRow {
    IndexRow {
        id: s.id,
        title: title.into(),
        mode: s.mode,
        lifetime: s.lifetime,
        pinned: false,
        model: s.model.clone(),
        parent: None,
        created_at_ms: s.created_at_ms,
        updated_at_ms: s.updated_at_ms,
        active: false,
        status: RowStatus::Ok,
        file_mtime_ms: 0,
        file_len: 0,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub(super) struct Fixture {
    pub(super) app: App,
    sent: Receiver<BackendRequest>,
    open: Session,
    bad: IndexRow,
}

fn fixture() -> Fixture {
    fixture_with(12)
}

/// `chats` ordinary saved chats besides the open one, the incognito one and
/// the damaged one; a short list keeps every group inside the viewport.
pub(super) fn fixture_with(chats: usize) -> Fixture {
    let (tx, sent) = std::sync::mpsc::channel();
    let (_, rx) = std::sync::mpsc::channel();
    let mut app = App::new(
        &Settings::default(),
        std::path::PathBuf::from("/tmp/staging"),
        Machine {
            ram_gb: 64.0,
            vram_gb: Some(24.0),
        },
        tx,
        rx,
    );
    let now = now_ms();
    let mut open = Session::new_chat(LONG_MODEL, SessionLifetime::Persisted, now);
    open.push_user(&"Please explain every detail of this. ".repeat(12), now);
    let a = open.begin_assistant(RunId::new(), TurnId::new(), now);
    open.append_assistant(
        a,
        &"A long and careful answer. ".repeat(30),
        "Let me think about it.",
    );
    open.finish_assistant(a, AssistantStatus::Complete, None, now);
    open.push_user("And again?", now);
    let f = open.begin_assistant(RunId::new(), TurnId::new(), now);
    open.append_assistant(f, "partial", "");
    open.finish_assistant(
        f,
        AssistantStatus::Failed,
        Some("Ollama stopped responding.".into()),
        now,
    );

    let mut rows = vec![row_for(&open, LONG_TITLE)];
    for i in 0..chats as u64 {
        let s = Session::new_chat(
            "llama3.2:3b",
            SessionLifetime::Persisted,
            now - i * 3_600_000,
        );
        let mut r = row_for(&s, &format!("Chat number {i}"));
        r.pinned = i == 1;
        rows.push(r);
    }
    let inc = Session::new_chat("qwen2.5:7b", SessionLifetime::Incognito, now);
    rows.push(row_for(&inc, "Quiet thoughts"));
    let damaged = Session::new_chat("m", SessionLifetime::Persisted, now - 5 * 86_400_000);
    let mut bad = row_for(&damaged, "Unreadable session");
    bad.status = RowStatus::Quarantined;
    rows.push(bad.clone());

    let st = app.state_mut();
    st.ollama = Some(Some("0.9".into()));
    st.view = View::Chat;
    st.chat.fold(ChatEvent::Rows(rows));
    st.chat.fold(ChatEvent::Opened(Box::new(open.clone())));
    Fixture {
        app,
        sent,
        open,
        bad,
    }
}

pub(super) fn run(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), frames: usize) {
    for _ in 0..frames {
        h.frame(vec2(w, hh), vec![], |ui| app.show(ui));
    }
}

fn click(app: &mut App, h: &mut Harness, size: (f32, f32), at: Pos2) {
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::PointerMoved(at)],
        |ui| app.show(ui),
    );
    for pressed in [true, false] {
        h.frame(
            vec2(size.0, size.1),
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }],
            |ui| app.show(ui),
        );
    }
    run(app, h, size, 2);
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.top() >= outer.top() - 0.5
        && inner.bottom() <= outer.bottom() + 0.5
        && inner.left() >= outer.left() - 0.5
        && inner.right() <= outer.right() + 0.5
}

fn sent_chat_commands(rx: &Receiver<BackendRequest>) -> Vec<ChatCommand> {
    rx.try_iter()
        .filter_map(|r| match r {
            BackendRequest::Chat(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn every_theme_family_and_mode_keeps_chat_usable_at_minimum_size() {
    for family in [
        ThemeFamily::Meadow,
        ThemeFamily::Ember,
        ThemeFamily::Plum,
        ThemeFamily::Tide,
    ] {
        for dark in [false, true] {
            for size in SIZES {
                let mut f = fixture();
                let mut h = Harness::themed(family, dark);
                run(&mut f.app, &mut h, size, 4);
                let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
                let p = f.app.state_mut().chat_ui.probe.clone();
                let tag = format!("{family:?} dark={dark} {size:?}");
                let (rail, pane, composer, header) = (
                    p.rail.expect("rail"),
                    p.pane.expect("pane"),
                    p.composer.expect("composer"),
                    p.header.expect("header"),
                );
                assert!(inside(rail, window), "{tag}: rail {rail:?}");
                assert!(inside(pane, window), "{tag}: pane {pane:?}");
                assert!(inside(composer, window), "{tag}: composer {composer:?}");
                assert!(
                    rail.right() <= pane.left() + 0.5,
                    "{tag}: rail overlaps pane"
                );
                assert!(
                    inside(header, pane),
                    "{tag}: header {header:?} pane {pane:?}"
                );
                let (send, label) = p.send_button.expect("send button");
                assert_eq!(label, "Send", "{tag}");
                assert!(
                    inside(send, composer),
                    "{tag}: send {send:?} composer {composer:?}"
                );
                let (delete, _) = p.delete_button.clone().expect("delete");
                assert!(inside(delete, pane), "{tag}: delete {delete:?}");
                assert!(!p.rows.is_empty(), "{tag}: no rail rows");
                let mut last_bottom = f32::MIN;
                for (_, r) in &p.rows {
                    assert!(r.top() >= last_bottom - 0.5, "{tag}: rows overlap");
                    assert!(
                        r.left() >= rail.left() - 0.5 && r.right() <= rail.right() + 0.5,
                        "{tag}: row {r:?} outside rail {rail:?}"
                    );
                    last_bottom = r.bottom();
                }
            }
        }
    }
}

#[test]
fn chat_is_the_first_tab() {
    assert_eq!(View::ALL[0], View::Chat);
}

#[test]
fn unavailable_history_moves_the_app_to_browse() {
    let mut f = fixture();
    let st = f.app.state_mut();
    st.on_event(&crate::runtime::UiEvent::ChatUnavailable(
        "Another Roost".into(),
    ));
    assert_eq!(st.view, View::Browse);
}

#[test]
fn the_nav_shows_activity_while_a_background_run_is_active() {
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    let width = |app: &mut App| {
        app.state_mut()
            .top_bar
            .iter()
            .find(|(n, _)| *n == "nav")
            .unwrap()
            .1
            .width()
    };
    let idle = width(&mut f.app);
    let id = f.open.id;
    let turn = TurnId::new();
    f.app
        .state_mut()
        .chat
        .fold(ChatEvent::TurnStarted { session: id, turn });
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    assert!(width(&mut f.app) > idle, "badge should widen the nav");
}

#[test]
fn send_becomes_stop_during_a_run() {
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    assert_eq!(
        f.app.state_mut().chat_ui.probe.send_button.unwrap().1,
        "Send"
    );
    f.app.state_mut().chat.fold(ChatEvent::TurnStarted {
        session: f.open.id,
        turn: TurnId::new(),
    });
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    assert_eq!(
        f.app.state_mut().chat_ui.probe.send_button.unwrap().1,
        "Stop"
    );
}

fn press_enter() -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: Some(egui::Key::Enter),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

#[test]
fn enter_sends_when_idle_and_does_nothing_during_a_run() {
    let size = (1180.0, 820.0);
    let mut f = fixture();
    let mut h = Harness::new(false);
    let id = f.open.id;
    *f.app.state_mut().chat.draft_mut(id) = "hello there".into();
    f.app.state_mut().chat_ui.focus_composer = true;
    run(&mut f.app, &mut h, size, 3);
    h.frame(vec2(size.0, size.1), press_enter(), |ui| f.app.show(ui));
    run(&mut f.app, &mut h, size, 2);
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Send { text, .. }] if text == "hello there"),
        "{cmds:?}"
    );
    assert_eq!(
        f.app.state_mut().chat.draft(id),
        "hello there",
        "kept until confirmed"
    );

    // A run is in progress: Enter must not send, and must keep the draft.
    let mut f = fixture();
    let id = f.open.id;
    *f.app.state_mut().chat.draft_mut(id) = "next".into();
    f.app.state_mut().chat.fold(ChatEvent::TurnStarted {
        session: id,
        turn: TurnId::new(),
    });
    f.app.state_mut().chat_ui.focus_composer = true;
    run(&mut f.app, &mut h, size, 3);
    h.frame(vec2(size.0, size.1), press_enter(), |ui| f.app.show(ui));
    run(&mut f.app, &mut h, size, 2);
    assert!(sent_chat_commands(&f.sent).is_empty());
    assert_eq!(f.app.state_mut().chat.draft(id), "next");
}

#[test]
fn delete_needs_the_confirm_pause() {
    let size = (1180.0, 820.0);
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let (rect, label) = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .delete_button
        .clone()
        .unwrap();
    assert_eq!(label, "Delete");
    click(&mut f.app, &mut h, size, rect.center());
    let (rect, label) = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .delete_button
        .clone()
        .unwrap();
    assert_eq!(label, "Confirm delete");
    // An immediate second click (inside the pause) must not delete.
    click(&mut f.app, &mut h, size, rect.center());
    assert!(
        !sent_chat_commands(&f.sent)
            .iter()
            .any(|c| matches!(c, ChatCommand::Delete(_))),
        "a quick double click must not delete"
    );
}

#[test]
fn incognito_group_shows_its_warning_and_clear() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 2);
    let texts = h.frame_texts(vec2(size.0, size.1), vec![], |ui| f.app.show(ui));
    let all = texts.join("\n");
    assert!(all.contains("Memory only"), "{all}");
    assert!(all.contains("Clear"), "{all}");
    assert!(all.contains("Quiet thoughts"), "{all}");
}

#[test]
fn errored_rows_open_an_explanation_with_delete_only() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let target = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .rows
        .iter()
        .find(|(id, _)| *id == f.bad.id)
        .map(|(_, r)| *r)
        .expect("bad row painted");
    click(&mut f.app, &mut h, size, target.center());
    assert!(
        !sent_chat_commands(&f.sent)
            .iter()
            .any(|c| matches!(c, ChatCommand::Open(_))),
        "an errored row is not opened"
    );
    let texts = h.frame_texts(vec2(size.0, size.1), vec![], |ui| f.app.show(ui));
    let all = texts.join("\n");
    assert!(all.contains("damaged"), "{all}");
    assert!(all.contains("Delete"), "{all}");
    assert!(!all.contains("Rename") && !all.contains("Send"), "{all}");
}

#[test]
fn switching_sessions_closes_the_previous_saved_one_so_it_can_be_evicted() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let previous = f.open.id;
    let target = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .rows
        .iter()
        .find(|(id, _)| *id != previous && *id != f.bad.id)
        .map(|(id, r)| (*id, *r))
        .expect("another row");
    click(&mut f.app, &mut h, size, target.1.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Close(a), ChatCommand::Open(b)] if *a == previous && *b == target.0),
        "{cmds:?}"
    );
}

fn rail_row_of(app: &mut App, id: roost_core::session::SessionId) -> Rect {
    app.state_mut()
        .chat_ui
        .probe
        .rows
        .iter()
        .find(|(r, _)| *r == id)
        .map(|(_, r)| *r)
        .expect("row painted")
}

fn rail_delete_of(app: &mut App, id: roost_core::session::SessionId) -> Option<(Rect, bool)> {
    app.state_mut()
        .chat_ui
        .probe
        .rail_delete
        .iter()
        .find(|(r, _, _)| *r == id)
        .map(|(_, rect, armed)| (*rect, *armed))
}

#[test]
fn hovering_a_rail_row_shows_a_delete_control_inside_the_row() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(3);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let target = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .rows
        .iter()
        .map(|(id, _)| *id)
        .find(|id| *id != f.open.id && *id != f.bad.id)
        .unwrap();
    assert!(
        rail_delete_of(&mut f.app, target).is_none(),
        "hidden until hovered"
    );
    let row = rail_row_of(&mut f.app, target);
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::PointerMoved(row.center())],
        |ui| f.app.show(ui),
    );
    run(&mut f.app, &mut h, size, 2);
    let (del, armed) = rail_delete_of(&mut f.app, target).expect("delete control on hover");
    assert!(!armed);
    assert!(inside(del, rail_row_of(&mut f.app, target)), "{del:?}");
    // The selected row always shows it, so it is one click away.
    assert!(rail_delete_of(&mut f.app, f.open.id).is_some());
}

#[test]
fn rail_delete_takes_a_click_and_a_confirm_after_a_short_pause() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(3);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let target = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .rows
        .iter()
        .map(|(id, _)| *id)
        .find(|id| *id != f.open.id && *id != f.bad.id)
        .unwrap();
    let row = rail_row_of(&mut f.app, target);
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::PointerMoved(row.center())],
        |ui| f.app.show(ui),
    );
    run(&mut f.app, &mut h, size, 2);
    let (del, _) = rail_delete_of(&mut f.app, target).unwrap();
    click(&mut f.app, &mut h, size, del.center());
    let (del, armed) = rail_delete_of(&mut f.app, target).expect("still shown while armed");
    assert!(armed, "the first click arms");
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, ChatCommand::Delete(_) | ChatCommand::Open(_))),
        "{cmds:?}"
    );
    // An immediate second click is inside the pause and must not delete.
    click(&mut f.app, &mut h, size, del.center());
    assert!(!sent_chat_commands(&f.sent)
        .iter()
        .any(|c| matches!(c, ChatCommand::Delete(_))));
    std::thread::sleep(std::time::Duration::from_millis(650));
    let (del, _) = rail_delete_of(&mut f.app, target).unwrap();
    click(&mut f.app, &mut h, size, del.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Delete(d)] if *d == target),
        "{cmds:?}"
    );
}
