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
        preview: Some(String::new()),
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
                // The header band reaches over the pane's margins on purpose.
                let reach =
                    pane.expand2(vec2(super::view::PANE_MARGIN_X, super::view::PANE_MARGIN_Y));
                assert!(
                    inside(header, reach),
                    "{tag}: header {header:?} pane {pane:?}"
                );
                let (send, label) = p.send_button.expect("send button");
                assert_eq!(label, "Send", "{tag}");
                assert!(
                    inside(send, composer),
                    "{tag}: send {send:?} composer {composer:?}"
                );
                let trash = p.delete_button.clone().expect("trash can").0;
                assert!(inside(trash, header), "{tag}: {trash:?} not in {header:?}");
                let chip = p.model_chip.expect("model chip");
                assert!(
                    inside(chip, composer),
                    "{tag}: {chip:?} not in {composer:?}"
                );
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
    assert_eq!(st.view, View::Models);
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
    press_enter_with(egui::Modifiers::NONE)
}

fn press_enter_with(modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: Some(egui::Key::Enter),
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn shift_enter_adds_a_line_instead_of_sending() {
    let size = (1180.0, 820.0);
    let mut f = fixture();
    let mut h = Harness::new(false);
    let id = f.open.id;
    *f.app.state_mut().chat.draft_mut(id) = "hello".into();
    f.app.state_mut().chat_ui.focus_composer = true;
    run(&mut f.app, &mut h, size, 3);
    h.frame(
        vec2(size.0, size.1),
        press_enter_with(egui::Modifiers::SHIFT),
        |ui| f.app.show(ui),
    );
    run(&mut f.app, &mut h, size, 2);
    assert!(sent_chat_commands(&f.sent).is_empty());
    assert_eq!(f.app.state_mut().chat.draft(id), "hello\n");
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

fn loaded(name: &str, expires: &str) -> roost_core::ollama::RunningModel {
    roost_core::ollama::RunningModel {
        name: name.into(),
        size: 4_700_000_000,
        size_vram: 4_700_000_000,
        expires_at: Some(expires.into()),
    }
}

fn unload_requests(rx: &Receiver<BackendRequest>) -> Vec<String> {
    rx.try_iter()
        .filter_map(|r| match r {
            BackendRequest::UnloadModel(n) => Some(n),
            _ => None,
        })
        .collect()
}

#[test]
fn unload_notes_read_naturally() {
    use super::view::unload_note;
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-27T12:00:00Z")
        .unwrap()
        .to_utc();
    assert_eq!(
        unload_note("2026-10-27T12:05:00Z", now).as_deref(),
        Some("unloads in 5 min")
    );
    assert_eq!(
        unload_note("2026-10-27T12:00:20Z", now).as_deref(),
        Some("unloads in under a minute")
    );
    assert_eq!(
        unload_note("2026-10-27T11:59:00Z", now).as_deref(),
        Some("unloading")
    );
    assert_eq!(
        unload_note("2318-01-01T00:00:00Z", now).as_deref(),
        Some("stays loaded")
    );
    assert_eq!(unload_note("not a time", now), None);
    assert_eq!(
        unload_note("2026-10-27T05:10:00.123456-07:00", now).as_deref(),
        Some("unloads in 11 min")
    );
}

// ------------------------------------------------------------- the Work tab

use roost_core::session::{SessionMode, ToolStatus};

fn agent_fixture() -> (App, Receiver<BackendRequest>, Session) {
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
    let folder =
        std::env::temp_dir().join("a-project-with-a-rather-long-folder-name/and/some/nested/path");
    let mut s = Session::new_agent(LONG_MODEL, SessionLifetime::Persisted, folder, now).unwrap();
    s.push_user("Find where the config is loaded and explain it.", now);
    let run = RunId::new();
    let a = s.begin_assistant(run, TurnId::new(), now);
    s.append_assistant(a, "I will search the sources first.", "");
    s.finish_assistant(a, AssistantStatus::Complete, None, now);
    for (i, status, out) in [
        (
            1,
            ToolStatus::Complete,
            "src/config.rs:12: fn load() {}".repeat(20),
        ),
        (
            2,
            ToolStatus::Failed,
            "Error: access denied: .env is protected and cannot be read".to_string(),
        ),
        (3, ToolStatus::Running, String::new()),
    ] {
        let id = s.push_tool_call(
            run,
            &format!("call_{i}"),
            "search_text",
            serde_json::json!({"pattern": "load_config", "path": "src/some/very/long/path/to/search/in"}),
            now,
        );
        if status != ToolStatus::Running {
            s.finish_tool(id, &out, status, now);
        } else {
            s.start_tool(id);
        }
    }
    let mut agent_row = row_for(&s, "Config walkthrough");
    agent_row.mode = SessionMode::Agent;
    let chat = Session::new_chat("llama3.2:3b", SessionLifetime::Persisted, now);
    let chat_row = row_for(&chat, "Just a chat");
    let st = app.state_mut();
    st.ollama = Some(Some("0.9".into()));
    st.view = View::Work;
    st.chat.fold(ChatEvent::Rows(vec![agent_row, chat_row]));
    st.chat.fold(ChatEvent::Opened(Box::new(s.clone())));
    (app, sent, s)
}

#[test]
fn work_lists_only_agents_and_chat_lists_only_chats() {
    let size = (1180.0, 820.0);
    let (mut app, _sent, _s) = agent_fixture();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 3);
    let texts = h
        .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n");
    assert!(
        texts.contains("Config walkthrough") && !texts.contains("Just a chat"),
        "{texts}"
    );
    assert!(texts.contains("+ New"), "{texts}");
    app.state_mut().view = View::Chat;
    run(&mut app, &mut h, size, 3);
    let texts = h
        .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n");
    assert!(
        texts.contains("Just a chat") && !texts.contains("Config walkthrough"),
        "{texts}"
    );
    assert!(
        texts.contains("Start a chat"),
        "an agent open on Work is not shown on Chat: {texts}"
    );
}

#[test]
fn the_new_agent_form_cannot_start_until_the_model_supports_tools() {
    let size = (1180.0, 820.0);
    let (tx, sent) = std::sync::mpsc::channel();
    let (_, rx) = std::sync::mpsc::channel();
    let mut app = App::new(
        &Settings::default(),
        std::path::PathBuf::new(),
        Machine {
            ram_gb: 8.0,
            vram_gb: None,
        },
        tx,
        rx,
    );
    {
        let st = app.state_mut();
        st.ollama = Some(Some("0.9".into()));
        st.view = View::Work;
        st.installed
            .on_installed_loaded(&Ok(vec![crate::runtime::InstalledEntry {
                model: roost_core::ollama::InstalledModel {
                    name: "qwen:7b".into(),
                    size: 1,
                    params: None,
                    quant: None,
                    family: None,
                    modified: String::new(),
                    digest: String::new(),
                },
                source_repo: None,
            }]));
        st.chat.fold(ChatEvent::Rows(vec![]));
        st.chat_ui.new_agent.open = true;
        st.chat_ui.new_agent.model = Some("qwen:7b".into());
        st.chat_ui.new_agent.folder = Some(std::env::temp_dir());
    }
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 3);
    let start = app
        .state_mut()
        .chat_ui
        .probe
        .start_agent
        .expect("Start button");
    click(&mut app, &mut h, size, start.center());
    assert!(
        sent_chat_commands(&sent).is_empty(),
        "disabled while tool support is unknown"
    );

    app.state_mut().chat.fold(ChatEvent::ModelCapabilities {
        model: "qwen:7b".into(),
        tools: false,
    });
    run(&mut app, &mut h, size, 3);
    let texts = h
        .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n");
    assert!(texts.contains("does not support tools"), "{texts}");
    click(&mut app, &mut h, size, start.center());
    assert!(
        sent_chat_commands(&sent).is_empty(),
        "still disabled for a model without tools"
    );

    app.state_mut().chat.fold(ChatEvent::ModelCapabilities {
        model: "qwen:7b".into(),
        tools: true,
    });
    run(&mut app, &mut h, size, 3);
    let start = app.state_mut().chat_ui.probe.start_agent.unwrap();
    click(&mut app, &mut h, size, start.center());
    let cmds = sent_chat_commands(&sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::CreateAgent { lifetime: SessionLifetime::Persisted, model, workspace }]
            if model == "qwen:7b" && *workspace == std::env::temp_dir()),
        "{cmds:?}"
    );
    assert!(!app.state_mut().chat_ui.new_agent.open);
}

#[test]
fn enter_steers_a_running_agent() {
    let size = (1180.0, 820.0);
    let (mut app, sent, s) = agent_fixture();
    let mut h = Harness::new(false);
    app.state_mut().chat.fold(ChatEvent::TurnStarted {
        session: s.id,
        turn: TurnId::new(),
    });
    *app.state_mut().chat.draft_mut(s.id) = "look in src instead".into();
    app.state_mut().chat_ui.focus_composer = true;
    run(&mut app, &mut h, size, 3);
    h.frame(vec2(size.0, size.1), press_enter(), |ui| app.show(ui));
    run(&mut app, &mut h, size, 2);
    let cmds = sent_chat_commands(&sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Send { text, .. }] if text == "look in src instead"),
        "{cmds:?}"
    );
}

#[test]
fn the_work_nav_item_and_menu_exist() {
    assert_eq!(View::ALL[..2], [View::Chat, View::Work]);
    let model =
        crate::menu::model::build(&crate::app::AppState::new(), crate::menu::Platform::Linux);
    let labels: Vec<_> = model
        .action_items()
        .iter()
        .map(|a| a.label.clone())
        .collect();
    assert!(labels.contains(&"Work".to_string()));
    assert!(labels.contains(&"New Agent…".to_string()));
}

// ----------------------------------------------------- the quieter chrome

fn open_menu(app: &mut App, h: &mut Harness, size: (f32, f32), at: Rect) {
    click(app, h, size, at.center());
    run(app, h, size, 2);
}

fn texts_of(app: &mut App, h: &mut Harness, size: (f32, f32)) -> String {
    h.frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n")
}

#[test]
fn the_title_row_has_a_pencil_a_pin_and_a_trash_can_and_deleting_takes_two_clicks() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let p = f.app.state_mut().chat_ui.probe.clone();
    let header = p.header.unwrap();
    let (pencil, pin) = (p.rename_button.expect("pencil"), p.pin_button.expect("pin"));
    let (del, label) = p.delete_button.clone().expect("trash can");
    assert_eq!(label, "Delete…");
    for r in [pencil, pin, del] {
        assert!(inside(r, header), "{r:?} not in {header:?}");
    }
    assert!(pin.right() <= del.left(), "pin is left of the trash can");
    // No overflow menu is left.
    let shown = texts_of(&mut f.app, &mut h, size);
    assert!(
        !shown.lines().any(|l| l == "Rename" || l == "Delete"),
        "{shown}"
    );

    click(&mut f.app, &mut h, size, del.center());
    let (del, label) = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .delete_button
        .clone()
        .unwrap();
    assert_eq!(label, "Click again to delete");
    click(&mut f.app, &mut h, size, del.center());
    assert!(
        !sent_chat_commands(&f.sent)
            .iter()
            .any(|c| matches!(c, ChatCommand::Delete(_))),
        "a quick second click must not delete"
    );
}

#[test]
fn the_pin_toggles_and_an_incognito_chat_has_none() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let id = f.open.id;
    let pin = f.app.state_mut().chat_ui.probe.pin_button.unwrap();
    let _ = sent_chat_commands(&f.sent);
    click(&mut f.app, &mut h, size, pin.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::SetPinned(i, true)] if *i == id),
        "{cmds:?}"
    );
    // An incognito chat is never saved, so it cannot be pinned.
    let inc = Session::new_chat("qwen2.5:7b", SessionLifetime::Incognito, now_ms());
    f.app
        .state_mut()
        .chat
        .fold(ChatEvent::Opened(Box::new(inc)));
    run(&mut f.app, &mut h, size, 3);
    assert!(f.app.state_mut().chat_ui.probe.pin_button.is_none());
    assert!(f.app.state_mut().chat_ui.probe.delete_button.is_some());
}

#[test]
fn the_pencil_and_a_double_click_make_the_title_editable() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let id = f.open.id;
    let pencil = f.app.state_mut().chat_ui.probe.rename_button.unwrap();
    assert!(f.app.state_mut().chat_ui.renaming.is_none());
    click(&mut f.app, &mut h, size, pencil.center());
    assert_eq!(
        f.app.state_mut().chat_ui.renaming.as_ref().map(|r| r.0),
        Some(id)
    );
    // Enter keeps the new name.
    f.app.state_mut().chat_ui.renaming = Some((id, "A better name".into()));
    run(&mut f.app, &mut h, size, 3);
    let _ = sent_chat_commands(&f.sent);
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: Some(egui::Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        |ui| f.app.show(ui),
    );
    run(&mut f.app, &mut h, size, 2);
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Rename(i, name)] if *i == id && name == "A better name"),
        "{cmds:?}"
    );
    assert!(f.app.state_mut().chat_ui.renaming.is_none());
}

#[test]
fn the_model_chip_shows_whether_the_model_is_in_memory_and_unloads_it() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(
        texts.contains("○ ") && !texts.contains("● "),
        "not loaded yet: {texts}"
    );
    f.app.state_mut().running = vec![loaded(LONG_MODEL, "2099-01-01T00:00:00Z")];
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("● "), "{texts}");
    let chip = f.app.state_mut().chat_ui.probe.model_chip.unwrap();
    open_menu(&mut f.app, &mut h, size, chip);
    let opened = texts_of(&mut f.app, &mut h, size);
    assert!(
        opened.contains("In memory") && opened.contains("stays loaded"),
        "{opened}"
    );
    let unload = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .model_unload
        .expect("Unload in the popover");
    click(&mut f.app, &mut h, size, unload.center());
    assert_eq!(unload_requests(&f.sent), vec![LONG_MODEL.to_string()]);
    assert!(f.app.state_mut().running.is_empty());
}

#[test]
fn the_status_pill_lists_what_is_in_memory_and_says_when_nothing_is() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let pill = f.app.state_mut().pill_rect.expect("pill");
    open_menu(&mut f.app, &mut h, size, pill);
    let empty = texts_of(&mut f.app, &mut h, size);
    assert!(empty.contains("Nothing loaded"), "{empty}");
    click(&mut f.app, &mut h, size, egui::pos2(5.0, 5.0)); // closes it
    f.app.state_mut().running = vec![loaded(LONG_MODEL, "2099-01-01T00:00:00Z")];
    run(&mut f.app, &mut h, size, 3);
    open_menu(&mut f.app, &mut h, size, pill);
    let listed = texts_of(&mut f.app, &mut h, size);
    assert!(
        listed.contains("In memory") && listed.contains(LONG_MODEL) && listed.contains("Unload"),
        "{listed}"
    );
}

#[test]
fn the_rail_has_one_new_button_and_no_footer() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(2);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("+ New chat"), "{texts}");
    for gone in ["New incognito", "IN MEMORY", "In memory"] {
        assert!(
            !texts
                .to_uppercase()
                .lines()
                .any(|l| l == gone.to_uppercase()),
            "{gone}: {texts}"
        );
    }
    let new = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .new_button
        .expect("+ New button");
    let rail = f.app.state_mut().chat_ui.probe.rail.unwrap();
    assert!(inside(new, rail), "{new:?} {rail:?}");
}

#[test]
fn the_rail_search_filters_the_list_and_stays_inside_the_rail() {
    let size = (900.0, 640.0);
    let mut f = fixture_with(3);
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let search = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .rail_search
        .expect("search field");
    let rail = f.app.state_mut().chat_ui.probe.rail.unwrap();
    assert!(inside(search, rail), "{search:?} {rail:?}");
    f.app.state_mut().chat_ui.rail_query = "no-such-session-anywhere".into();
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("No matches."), "{texts}");
}

#[test]
fn an_agent_run_shows_progress_in_one_status_line_and_folds_tool_steps() {
    for family in [
        ThemeFamily::Meadow,
        ThemeFamily::Ember,
        ThemeFamily::Plum,
        ThemeFamily::Tide,
    ] {
        for dark in [false, true] {
            for size in SIZES {
                let (mut app, _sent, s) = agent_fixture();
                app.state_mut().chat.fold(ChatEvent::TurnStarted {
                    session: s.id,
                    turn: TurnId::new(),
                });
                app.state_mut().chat.fold(ChatEvent::SteeringQueued {
                    session: s.id,
                    text: "Only look in the src folder please, and ignore the tests entirely"
                        .into(),
                });
                let mut h = Harness::themed(family, dark);
                run(&mut app, &mut h, size, 4);
                let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
                let p = app.state_mut().chat_ui.probe.clone();
                let tag = format!("{family:?} dark={dark} {size:?}");
                let (pane, composer, header) =
                    (p.pane.unwrap(), p.composer.unwrap(), p.header.unwrap());
                assert!(inside(pane, window) && inside(composer, window), "{tag}");
                let chip = p.agent_chip.expect("agent chip");
                let more = p.delete_button.clone().expect("trash can").0;
                assert!(
                    inside(chip, header) && inside(more, header),
                    "{tag}: {chip:?} {more:?} {header:?}"
                );
                assert!(
                    chip.right() <= more.left() + 0.5,
                    "{tag}: chip overlaps the menu button"
                );
                assert_eq!(
                    p.tool_groups.len(),
                    1,
                    "{tag}: three tool calls fold into one line"
                );
                assert!(
                    p.tool_cards.is_empty(),
                    "{tag}: cards stay folded until opened"
                );
                assert!(
                    p.queued_chips.is_empty(),
                    "{tag}: waiting messages live in the status line"
                );
                let texts = h
                    .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
                    .join("\n");
                assert!(
                    texts.contains("Used 3 tools: search_text ×3"),
                    "{tag}: {texts}"
                );
                assert!(
                    texts.contains("1 failed") && texts.contains("search_text running…"),
                    "{tag}: {texts}"
                );
                assert!(texts.contains("Step 1 of 20"), "{tag}: {texts}");
                assert!(
                    texts.contains("1 message waiting for the next step"),
                    "{tag}: {texts}"
                );
                assert!(texts.contains("Agent settings"), "{tag}: {texts}");
            }
        }
    }
}

#[test]
fn opening_the_tool_summary_shows_the_cards_inside_the_pane() {
    let size = (1180.0, 820.0);
    let (mut app, _sent, _s) = agent_fixture();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let group = app.state_mut().chat_ui.probe.tool_groups[0];
    click(
        &mut app,
        &mut h,
        size,
        egui::pos2(group.left() + 12.0, group.center().y),
    );
    run(&mut app, &mut h, size, 3);
    let p = app.state_mut().chat_ui.probe.clone();
    assert_eq!(p.tool_cards.len(), 3, "{:?}", p.tool_cards);
    let pane = p.pane.unwrap();
    for c in &p.tool_cards {
        assert!(
            c.left() >= pane.left() - 0.5 && c.right() <= pane.right() + 0.5,
            "{c:?} {pane:?}"
        );
    }
}

#[test]
fn the_agent_chip_holds_the_folder_steps_and_tools_and_locks_while_running() {
    let size = (1180.0, 820.0);
    let (mut app, sent, s) = agent_fixture();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let chip = app.state_mut().chat_ui.probe.agent_chip.unwrap();
    open_menu(&mut app, &mut h, size, chip);
    let texts = texts_of(&mut app, &mut h, size);
    let upper = texts.to_uppercase();
    for want in ["FOLDER", "RUN", "TOOLS"] {
        assert!(upper.contains(want), "card {want}: {texts}");
    }
    for want in [
        "Change…",
        "Step budget",
        "Files · Read",
        "list_dir",
        "read_file",
        "find_files",
        "search_text",
    ] {
        assert!(texts.contains(want), "{want}: {texts}");
    }
    assert!(!texts.contains("Locked while a run is active."));
    let _ = (sent, s);
}

fn open_markdown_reply(f: &mut Fixture, reply: &str) {
    let now = now_ms();
    let mut s = Session::new_chat("llama3.2:3b", SessionLifetime::Persisted, now);
    s.push_user("Show me", now);
    let a = s.begin_assistant(RunId::new(), TurnId::new(), now);
    s.append_assistant(a, reply, "");
    s.finish_assistant(a, AssistantStatus::Complete, None, now);
    f.app.state_mut().chat.fold(ChatEvent::Opened(Box::new(s)));
}

#[test]
fn assistant_replies_are_drawn_as_markdown_and_user_text_is_not() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(
        &mut f,
        "Here is **bold** text:\n\n- first item\n- second item\n\n```\nlet x = 1;\n```",
    );
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(!texts.contains("**"), "{texts}");
    assert!(!texts.contains("```"), "{texts}");
    assert!(
        texts.contains("bold") && texts.contains("first item"),
        "{texts}"
    );
    assert!(texts.contains("let x = 1;"), "{texts}");
}

#[test]
fn the_row_is_always_on_the_latest_message_and_on_hover_for_older_ones() {
    for size in SIZES {
        let mut f = fixture_with(1);
        let mut h = Harness::new(false);
        open_markdown_reply(&mut f, "A short answer.");
        run(&mut f.app, &mut h, size, 3);
        let probe = f.app.state_mut().chat_ui.probe.clone();
        let pane = probe.pane.expect("pane");
        assert_eq!(probe.copy_buttons.len(), 1, "{size:?}: latest only at rest");
        assert!(inside(probe.copy_buttons[0], pane), "{size:?}");
        let first = probe.cards[0];
        let latest = probe.cards[1];
        assert!(probe.copy_buttons[0].top() >= latest.bottom() - 0.5);
        // Pointer over the older bubble: its row appears too, below it.
        h.frame(
            vec2(size.0, size.1),
            vec![egui::Event::PointerMoved(first.center())],
            |ui| f.app.show(ui),
        );
        let probe = f.app.state_mut().chat_ui.probe.clone();
        assert_eq!(probe.copy_buttons.len(), 2, "{size:?}: hovered row shown");
        assert!(probe
            .copy_buttons
            .iter()
            .any(|b| b.top() >= first.bottom() - 0.5 && b.top() < latest.top()));
    }
}

#[test]
fn details_shows_tokens_and_speed_when_they_were_recorded() {
    use roost_core::session::ReplyStats;
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    {
        let st = f.app.state_mut();
        let open = st.chat.open.as_mut().expect("open");
        let id = open.messages.last().unwrap().id;
        open.set_reply_stats(
            id,
            ReplyStats {
                prompt_tokens: Some(12),
                reply_tokens: Some(40),
                total_ms: Some(2_500),
                load_ms: Some(300),
                eval_ms: Some(2_000),
            },
        );
    }
    run(&mut f.app, &mut h, size, 3);
    let details = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .details
        .expect("Details icon");
    click(&mut f.app, &mut h, size, details.center());
    let texts = texts_of(&mut f.app, &mut h, size);
    for want in ["12 tokens", "40 tokens", "20.0 tokens/s", "2.5 s"] {
        assert!(texts.contains(want), "{want:?} missing in:\n{texts}");
    }
}

#[test]
fn details_admits_when_nothing_was_recorded() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "An old reply.");
    run(&mut f.app, &mut h, size, 3);
    let details = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .details
        .expect("Details icon");
    click(&mut f.app, &mut h, size, details.center());
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("No timing was recorded"), "{texts}");
}

#[test]
fn headings_tables_quotes_and_rules_in_a_reply_are_not_shown_as_markup() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(
        &mut f,
        "# Title\n\n## Section\n\n> quoted words\n\n---\n\n| Name | Size |\n|------|------|\n| a | 1 |\n| b | 2 |\n\n1. one\n2. two",
    );
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    for markup in [
        "# Title",
        "## Section",
        "> quoted",
        "|------|",
        "| Name",
        "---",
    ] {
        assert!(!texts.contains(markup), "{markup:?} shown raw in:\n{texts}");
    }
    for words in [
        "Title",
        "Section",
        "quoted words",
        "Name",
        "Size",
        "one",
        "two",
    ] {
        assert!(texts.contains(words), "{words:?} missing in:\n{texts}");
    }
}

#[test]
fn a_finished_reply_offers_regenerate_and_clicking_it_sends_the_command() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    let id = f.app.state_mut().chat.open.as_ref().unwrap().id;
    run(&mut f.app, &mut h, size, 3);
    let _ = sent_chat_commands(&f.sent);
    let button = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .regenerate
        .expect("Regenerate button");
    click(&mut f.app, &mut h, size, button.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Regenerate(s)] if *s == id),
        "{cmds:?}"
    );
}

#[test]
fn a_failed_reply_offers_retry_and_not_regenerate() {
    let size = (1180.0, 820.0);
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    assert!(f.app.state_mut().chat_ui.probe.regenerate.is_none());
}

fn open_two_versions(
    f: &mut Fixture,
) -> (
    roost_core::session::SessionId,
    roost_core::session::MessageId,
) {
    let now = now_ms();
    let mut s = Session::new_chat("llama3.2:3b", SessionLifetime::Persisted, now);
    let u = s.push_user("Show me", now);
    let a = s.begin_assistant(RunId::new(), TurnId::new(), now);
    s.append_assistant(a, "first answer", "");
    s.finish_assistant(a, AssistantStatus::Complete, None, now);
    s.fork_at(1);
    let b = s.begin_assistant(RunId::new(), TurnId::new(), now);
    s.append_assistant(b, "second answer", "");
    s.finish_assistant(b, AssistantStatus::Complete, None, now);
    let id = s.id;
    f.app.state_mut().chat.fold(ChatEvent::Opened(Box::new(s)));
    (id, u)
}

#[test]
fn a_message_with_versions_shows_a_counter_at_the_bottom_right_and_switches() {
    for size in SIZES {
        let mut f = fixture_with(1);
        let mut h = Harness::new(false);
        let (id, user) = open_two_versions(&mut f);
        run(&mut f.app, &mut h, size, 3);
        let _ = sent_chat_commands(&f.sent);
        let probe = f.app.state_mut().chat_ui.probe.clone();
        let pane = probe.pane.expect("pane");
        let counter = probe.version_counter.expect("counter");
        let (prev, next) = (probe.version_prev.unwrap(), probe.version_next.unwrap());
        let card = *probe.cards.last().unwrap();
        assert!(inside(counter, pane), "{size:?}");
        assert!(
            counter.top() >= card.bottom() - 0.5,
            "{size:?}: below the bubble"
        );
        assert!(
            counter.right() >= card.right() - 40.0,
            "{size:?}: at the right"
        );
        assert!(prev.right() <= next.left() + 0.5);
        let texts = texts_of(&mut f.app, &mut h, size);
        assert!(texts.contains("2 / 2"), "{texts}");
        click(&mut f.app, &mut h, size, prev.center());
        let cmds = sent_chat_commands(&f.sent);
        assert!(
            matches!(cmds.as_slice(), [ChatCommand::SwitchVersion { session, after, to: 0 }]
                if *session == id && *after == Some(user)),
            "{cmds:?}"
        );
    }
}

#[test]
fn a_message_without_versions_has_no_counter() {
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    assert!(f.app.state_mut().chat_ui.probe.version_counter.is_none());
}

#[test]
fn editing_your_message_sends_the_new_text_as_an_edit() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    let (id, mid) = {
        let s = f.app.state_mut().chat.open.as_ref().unwrap();
        (s.id, s.messages[0].id)
    };
    run(&mut f.app, &mut h, size, 3);
    let first = f.app.state_mut().chat_ui.probe.cards[0];
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::PointerMoved(first.center())],
        |ui| f.app.show(ui),
    );
    let edit = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .edit_icon
        .expect("Edit icon");
    click(&mut f.app, &mut h, size, edit.center());
    assert_eq!(
        f.app.state_mut().chat_ui.editing.as_ref().map(|e| e.0),
        Some(mid)
    );
    f.app.state_mut().chat_ui.editing.as_mut().unwrap().1 = "a better question".into();
    run(&mut f.app, &mut h, size, 2);
    let send = f.app.state_mut().chat_ui.probe.edit_send.expect("Send");
    let _ = sent_chat_commands(&f.sent);
    click(&mut f.app, &mut h, size, send.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Edit { session, message, text }]
            if *session == id && *message == mid && text == "a better question"),
        "{cmds:?}"
    );
    assert!(f.app.state_mut().chat_ui.editing.is_none());
}

/// A finished chat of one exchange, and a branch of it (opened).
fn open_a_branch(f: &mut Fixture) -> (Session, Session) {
    let now = now_ms();
    let mut parent = Session::new_chat("llama3.2:3b", SessionLifetime::Persisted, now);
    parent.title = "Lisbon trip".into();
    parent.push_user("Plan a trip", now);
    let a = parent.begin_assistant(RunId::new(), TurnId::new(), now);
    parent.append_assistant(a, "Day 1: Alfama", "");
    parent.finish_assistant(a, AssistantStatus::Complete, None, now);
    let mut b = parent.branch(a, None, now).unwrap();
    b.push_user("Make it about food", now);
    let st = f.app.state_mut();
    st.chat
        .fold(ChatEvent::Rows(vec![row_for(&parent, "Lisbon trip"), {
            let mut r = row_for(&b, "Lisbon trip (branch)");
            r.parent = b.parent;
            r
        }]));
    st.chat.fold(ChatEvent::Opened(Box::new(b.clone())));
    (parent, b)
}

#[test]
fn a_branched_chat_shows_where_it_came_from_and_asks_for_a_check_once() {
    for size in SIZES {
        let mut f = fixture_with(1);
        let mut h = Harness::new(false);
        let (parent, b) = open_a_branch(&mut f);
        run(&mut f.app, &mut h, size, 4);
        let cmds = sent_chat_commands(&f.sent);
        let checks = cmds
            .iter()
            .filter(|c| matches!(c, ChatCommand::CheckBranch(id) if *id == b.id))
            .count();
        assert_eq!(checks, 1, "{size:?}: {cmds:?}");
        let texts = texts_of(&mut f.app, &mut h, size);
        for want in [
            "Branched from",
            "Lisbon trip",
            "Day 1: Alfama",
            "branch point",
        ] {
            assert!(
                texts.contains(want),
                "{size:?}: {want:?} missing in:\n{texts}"
            );
        }
        let probe = f.app.state_mut().chat_ui.probe.clone();
        let pane = probe.pane.unwrap();
        let (id, link) = probe.strip_links[0];
        assert_eq!(id, parent.id);
        assert!(inside(link, pane), "{size:?}");
        click(&mut f.app, &mut h, size, link.center());
        let cmds = sent_chat_commands(&f.sent);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, ChatCommand::Open(i) if *i == parent.id)),
            "{cmds:?}"
        );
    }
}

#[test]
fn the_strip_says_when_the_original_changed_or_is_gone() {
    use roost_core::chat::BranchState;
    let size = (1180.0, 820.0);
    for (state, want) in [
        (BranchState::Unchanged, "unchanged"),
        (
            BranchState::Changed,
            "The original changed after this branch was made",
        ),
        (BranchState::Deleted, "The original chat was deleted"),
    ] {
        let mut f = fixture_with(1);
        let mut h = Harness::new(false);
        let (_, b) = open_a_branch(&mut f);
        f.app.state_mut().chat.fold(ChatEvent::BranchChecked {
            session: b.id,
            state,
        });
        run(&mut f.app, &mut h, size, 3);
        let texts = texts_of(&mut f.app, &mut h, size);
        assert!(texts.contains(want), "{state:?}: {texts}");
        let view = f.app.state_mut().chat_ui.probe.view_original.is_some();
        assert_eq!(view, state == BranchState::Changed, "{state:?}");
    }
}

#[test]
fn a_chat_that_is_not_a_branch_has_no_strip() {
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, (1180.0, 820.0), 3);
    let texts = texts_of(&mut f.app, &mut h, (1180.0, 820.0));
    assert!(!texts.contains("Branched from"));
}

#[test]
fn branches_are_listed_under_their_parent_in_the_sidebar() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    let (parent, b) = open_a_branch(&mut f);
    run(&mut f.app, &mut h, size, 3);
    let rows = f.app.state_mut().chat_ui.probe.rows.clone();
    let rect_of = |id| {
        rows.iter()
            .find(|(i, _)| *i == id)
            .map(|(_, r)| *r)
            .unwrap()
    };
    let (p, c) = (rect_of(parent.id), rect_of(b.id));
    assert!(c.left() > p.left() + 8.0, "indented: {p:?} {c:?}");
    assert!(c.top() > p.top(), "below its parent");
}

#[test]
fn the_branch_icon_offers_two_ways_and_the_second_picks_a_model() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    let (id, reply) = {
        let s = f.app.state_mut().chat.open.as_ref().unwrap();
        (s.id, s.messages[1].id)
    };
    run(&mut f.app, &mut h, size, 3);
    let icon = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .branch_icon
        .expect("Branch icon");
    click(&mut f.app, &mut h, size, icon.center());
    let plain = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .branch_plain
        .expect("plain item");
    let _ = sent_chat_commands(&f.sent);
    click(&mut f.app, &mut h, size, plain.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Branch { session, message, model: None }]
            if *session == id && *message == reply),
        "{cmds:?}"
    );
    // The model picker.
    click(&mut f.app, &mut h, size, icon.center());
    let pick = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .branch_pick
        .expect("pick item");
    click(&mut f.app, &mut h, size, pick.center());
    assert!(f.app.state_mut().chat_ui.branch_dialog.is_some());
    run(&mut f.app, &mut h, size, 2);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("Branch with another model"), "{texts}");
    let create = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .branch_create
        .expect("Create");
    click(&mut f.app, &mut h, size, create.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, ChatCommand::Branch { model: Some(_), .. })),
        "{cmds:?}"
    );
    assert!(f.app.state_mut().chat_ui.branch_dialog.is_none());
}

#[test]
fn a_side_question_is_asked_answered_and_can_be_added() {
    use roost_core::session::{BtwEntry, BtwId};
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    let (id, reply) = {
        let s = f.app.state_mut().chat.open.as_ref().unwrap();
        (s.id, s.messages[1].id)
    };
    run(&mut f.app, &mut h, size, 3);
    let icon = f.app.state_mut().chat_ui.probe.btw_icon.expect("BTW icon");
    click(&mut f.app, &mut h, size, icon.center());
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("Not added to this chat"), "{texts}");

    f.app.state_mut().chat_ui.btw_input = "why?".into();
    run(&mut f.app, &mut h, size, 2);
    let ask = f.app.state_mut().chat_ui.probe.btw_ask.expect("Ask");
    let _ = sent_chat_commands(&f.sent);
    click(&mut f.app, &mut h, size, ask.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::Btw { session, message, question }]
            if *session == id && *message == reply && question == "why?"),
        "{cmds:?}"
    );
    assert!(f.app.state_mut().chat_ui.btw_input.is_empty());

    let bid = BtwId::new();
    f.app.state_mut().chat.fold(ChatEvent::BtwStarted {
        session: id,
        id: bid,
        message: reply,
        question: "why?".into(),
    });
    run(&mut f.app, &mut h, size, 2);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("Thinking"), "{texts}");

    f.app.state_mut().chat.fold(ChatEvent::BtwDone {
        session: id,
        entry: BtwEntry {
            id: bid,
            at: reply,
            question: "why?".into(),
            answer: "Because borrowing is checked at compile time.".into(),
            model: "m".into(),
            asked_at_ms: 1,
            added: false,
        },
    });
    run(&mut f.app, &mut h, size, 2);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("Because borrowing"), "{texts}");
    let add = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .btw_add
        .expect("Add to chat");
    let _ = sent_chat_commands(&f.sent);
    click(&mut f.app, &mut h, size, add.center());
    let cmds = sent_chat_commands(&f.sent);
    assert!(
        cmds.iter().any(
            |c| matches!(c, ChatCommand::BtwAdd { session, id: i } if *session == id && *i == bid)
        ),
        "{cmds:?}"
    );
}

#[test]
fn a_failed_side_question_shows_why() {
    use roost_core::session::BtwId;
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    open_markdown_reply(&mut f, "A short answer.");
    let (id, reply) = {
        let s = f.app.state_mut().chat.open.as_ref().unwrap();
        (s.id, s.messages[1].id)
    };
    run(&mut f.app, &mut h, size, 3);
    let icon = f.app.state_mut().chat_ui.probe.btw_icon.unwrap();
    click(&mut f.app, &mut h, size, icon.center());
    let bid = BtwId::new();
    f.app.state_mut().chat.fold(ChatEvent::BtwStarted {
        session: id,
        id: bid,
        message: reply,
        question: "why?".into(),
    });
    f.app.state_mut().chat.fold(ChatEvent::BtwFailed {
        session: id,
        id: bid,
        message: "Ollama is not reachable.".into(),
    });
    run(&mut f.app, &mut h, size, 2);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(texts.contains("Ollama is not reachable."), "{texts}");
    assert!(!texts.contains("Thinking"), "{texts}");
}

/// A speech engine that records what it is asked to do.
#[derive(Default)]
struct SpeechLog {
    spoken: Vec<String>,
    stops: usize,
    speaking: bool,
}

struct FakeSpeaker {
    available: Result<(), String>,
    log: std::sync::Arc<std::sync::Mutex<SpeechLog>>,
}

impl crate::speech::Speaker for FakeSpeaker {
    fn available(&self) -> Result<(), String> {
        self.available.clone()
    }
    fn speak(&mut self, text: &str) {
        let mut l = self.log.lock().unwrap();
        l.spoken.push(text.to_string());
        l.speaking = true;
    }
    fn stop(&mut self) {
        let mut l = self.log.lock().unwrap();
        l.stops += 1;
        l.speaking = false;
    }
    fn is_speaking(&mut self) -> bool {
        self.log.lock().unwrap().speaking
    }
}

fn with_speaker(
    f: &mut Fixture,
    available: Result<(), String>,
) -> std::sync::Arc<std::sync::Mutex<SpeechLog>> {
    let log = std::sync::Arc::new(std::sync::Mutex::new(SpeechLog::default()));
    f.app.state_mut().speaker = Box::new(FakeSpeaker {
        available,
        log: log.clone(),
    });
    log
}

#[test]
fn the_read_icon_speaks_the_plain_text_and_a_second_click_stops() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    let log = with_speaker(&mut f, Ok(()));
    open_markdown_reply(&mut f, "A **short** answer.");
    run(&mut f.app, &mut h, size, 3);
    let icon = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .read_icon
        .expect("Read icon");
    assert!(!f.app.state_mut().chat_ui.probe.read_disabled);
    click(&mut f.app, &mut h, size, icon.center());
    assert_eq!(log.lock().unwrap().spoken, ["A short answer."]);
    assert!(f.app.state_mut().chat_ui.reading.is_some());
    let icon = f.app.state_mut().chat_ui.probe.read_icon.unwrap();
    click(&mut f.app, &mut h, size, icon.center());
    assert_eq!(log.lock().unwrap().spoken.len(), 1, "no second speech");
    assert!(log.lock().unwrap().stops >= 1);
    assert!(f.app.state_mut().chat_ui.reading.is_none());
}

#[test]
fn when_the_speech_ends_by_itself_the_icon_goes_back() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    let log = with_speaker(&mut f, Ok(()));
    open_markdown_reply(&mut f, "Hello.");
    run(&mut f.app, &mut h, size, 3);
    let icon = f.app.state_mut().chat_ui.probe.read_icon.unwrap();
    click(&mut f.app, &mut h, size, icon.center());
    assert!(f.app.state_mut().chat_ui.reading.is_some());
    log.lock().unwrap().speaking = false;
    run(&mut f.app, &mut h, size, 2);
    assert!(f.app.state_mut().chat_ui.reading.is_none());
}

#[test]
fn without_a_speech_engine_the_icon_is_disabled_and_does_nothing() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    let log = with_speaker(&mut f, Err("No engine here.".into()));
    open_markdown_reply(&mut f, "Hello.");
    run(&mut f.app, &mut h, size, 3);
    let icon = f
        .app
        .state_mut()
        .chat_ui
        .probe
        .read_icon
        .expect("still shown");
    assert!(f.app.state_mut().chat_ui.probe.read_disabled);
    click(&mut f.app, &mut h, size, icon.center());
    assert!(log.lock().unwrap().spoken.is_empty());
    assert!(f.app.state_mut().chat_ui.reading.is_none());
}

#[test]
fn new_agent_works_while_an_agent_chat_is_open() {
    let size = (1180.0, 820.0);
    let (mut app, sent, s) = agent_fixture();
    {
        let st = app.state_mut();
        st.installed
            .on_installed_loaded(&Ok(vec![crate::runtime::InstalledEntry {
                model: roost_core::ollama::InstalledModel {
                    name: "qwen:7b".into(),
                    size: 1,
                    params: None,
                    quant: None,
                    family: None,
                    modified: String::new(),
                    digest: String::new(),
                },
                source_repo: None,
            }]));
    }
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 3);
    let button = app.state_mut().chat_ui.probe.new_button.expect("+ New");
    let _ = sent_chat_commands(&sent);
    click(
        &mut app,
        &mut h,
        size,
        egui::pos2(button.left() + 30.0, button.center().y),
    );
    assert!(app.state_mut().chat_ui.new_agent.open);
    let texts = texts_of(&mut app, &mut h, size);
    assert!(
        texts.contains("The agent can only read inside the folder"),
        "the form is shown:\n{texts}"
    );
    let cmds = sent_chat_commands(&sent);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, ChatCommand::Close(id) if *id == s.id)),
        "the chat on screen is let go: {cmds:?}"
    );
}

fn ask_for_approval(app: &mut App, session: roost_core::session::SessionId) {
    app.state_mut().chat.fold(ChatEvent::ApprovalRequested {
        session,
        call_id: "call_9".into(),
        name: "read_file".into(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
        preview: None,
    });
}

#[test]
fn a_pending_approval_shows_what_the_agent_wants_and_three_ways_to_answer() {
    use roost_core::chat::ApprovalDecision;
    for size in SIZES {
        for (pick, want) in [
            (0usize, ApprovalDecision::Allow),
            (1, ApprovalDecision::AllowForSession),
            (2, ApprovalDecision::Deny),
        ] {
            let (mut app, sent, s) = agent_fixture();
            let mut h = Harness::new(false);
            ask_for_approval(&mut app, s.id);
            run(&mut app, &mut h, size, 4);
            let texts = texts_of(&mut app, &mut h, size);
            assert!(
                texts.contains("read_file") && texts.contains("src/main.rs"),
                "{texts}"
            );
            let p = app.state_mut().chat_ui.probe.clone();
            let composer = p.composer.unwrap();
            let pane = p.pane.unwrap();
            assert_eq!(p.approval_buttons.len(), 3, "{size:?}");
            for b in &p.approval_buttons {
                assert!(inside(*b, composer), "{size:?}: {b:?} outside {composer:?}");
                assert!(
                    b.top() >= pane.bottom() - 0.5,
                    "{size:?}: below the transcript"
                );
            }
            let _ = sent_chat_commands(&sent);
            click(&mut app, &mut h, size, p.approval_buttons[pick].center());
            let cmds = sent_chat_commands(&sent);
            assert!(
                matches!(cmds.as_slice(), [ChatCommand::ResolveApproval { session, call_id, decision }]
                    if *session == s.id && call_id == "call_9" && *decision == want),
                "{size:?} pick {pick}: {cmds:?}"
            );
            run(&mut app, &mut h, size, 2);
            assert!(
                app.state_mut().chat_ui.probe.approval_buttons.is_empty(),
                "the card goes away once answered"
            );
        }
    }
}

#[test]
fn the_approval_card_goes_when_the_tool_starts_or_the_run_ends() {
    let size = (1180.0, 820.0);
    for end in [0, 1] {
        let (mut app, _sent, s) = agent_fixture();
        let mut h = Harness::new(false);
        ask_for_approval(&mut app, s.id);
        run(&mut app, &mut h, size, 3);
        assert_eq!(app.state_mut().chat_ui.probe.approval_buttons.len(), 3);
        if end == 0 {
            app.state_mut().chat.fold(ChatEvent::ToolStarted {
                session: s.id,
                call_id: "call_9".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({}),
            });
        } else {
            app.state_mut().chat.fold(ChatEvent::RunEnded {
                session: s.id,
                outcome: roost_core::chat::RunOutcome::Cancelled,
                message: None,
            });
        }
        run(&mut app, &mut h, size, 3);
        assert!(
            app.state_mut().chat_ui.probe.approval_buttons.is_empty(),
            "end {end}"
        );
    }
}

#[test]
fn the_tools_menu_sets_files_read_to_off_ask_or_auto() {
    use roost_core::session::Permission;
    let size = (1180.0, 820.0);
    let (mut app, sent, s) = agent_fixture();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 3);
    let chip = app
        .state_mut()
        .chat_ui
        .probe
        .agent_chip
        .expect("Agent chip");
    click(&mut app, &mut h, size, chip.center());
    let ask = app.state_mut().chat_ui.probe.permission_buttons.clone();
    assert_eq!(ask.len(), 3, "Off, Ask, Auto");
    let _ = sent_chat_commands(&sent);
    click(&mut app, &mut h, size, ask[1].center());
    let cmds = sent_chat_commands(&sent);
    assert!(
        matches!(cmds.as_slice(), [ChatCommand::SetCapability(id, key, Permission::Ask)]
            if *id == s.id && key == "files.read"),
        "{cmds:?}"
    );
}

#[test]
fn the_header_names_the_session_kind_and_shows_where_an_agent_works() {
    for size in SIZES {
        let (mut app, _sent, _s) = agent_fixture();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        let texts = texts_of(&mut app, &mut h, size);
        assert!(
            texts.contains("Config walkthrough") || texts.contains("Untitled"),
            "{texts}"
        );
        assert!(texts.contains("Agent settings"), "{texts}");
        assert!(texts.contains("Idle"), "{texts}");
        let path_line = texts
            .lines()
            .find(|l| l.starts_with('(') && (l.contains("a-project") || l.contains('…')))
            .unwrap_or_else(|| panic!("no folder in parentheses:\n{texts}"));
        assert!(path_line.ends_with(')'), "{path_line}");
        let p = app.state_mut().chat_ui.probe.clone();
        let header = p.header.unwrap();
        let settings = p.agent_chip.unwrap();
        let more = p.delete_button.clone().unwrap().0;
        assert!(inside(settings, header) && inside(more, header), "{size:?}");
        assert!(
            settings.right() <= more.left(),
            "{size:?}: settings left of the menu"
        );
    }
}

#[test]
fn a_chat_has_no_settings_button_and_no_folder() {
    let size = (1180.0, 820.0);
    let mut f = fixture();
    let mut h = Harness::new(false);
    run(&mut f.app, &mut h, size, 3);
    let texts = texts_of(&mut f.app, &mut h, size);
    assert!(!texts.contains("Agent settings"), "{texts}");
    assert!(f.app.state_mut().chat_ui.probe.agent_chip.is_none());
    assert_eq!(
        f.app.state_mut().chat_ui.probe.status_label.as_deref(),
        Some("Idle")
    );
}

#[test]
fn the_status_says_when_the_agent_needs_an_answer() {
    let size = (1180.0, 820.0);
    let (mut app, _sent, s) = agent_fixture();
    let mut h = Harness::new(false);
    ask_for_approval(&mut app, s.id);
    run(&mut app, &mut h, size, 3);
    assert_eq!(
        app.state_mut().chat_ui.probe.status_label.as_deref(),
        Some("Waiting for you")
    );
}

#[test]
fn a_very_long_folder_is_cut_in_the_middle_and_the_header_still_fits() {
    let size = (900.0, 640.0);
    let (mut app, _sent, s) = agent_fixture();
    {
        let long = std::env::temp_dir().join(
            "a/very/long/path/that/keeps/going/and/going/through/many/folders/to/some-project-name",
        );
        let st = app.state_mut();
        let open = st.chat.open.as_mut().unwrap();
        open.workspace = Some(long);
        let _ = s;
    }
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let texts = texts_of(&mut app, &mut h, size);
    let line = texts
        .lines()
        .find(|l| l.starts_with('(') && l.contains('…'))
        .unwrap_or_else(|| panic!("the folder was not cut:\n{texts}"));
    assert!(
        line.ends_with("some-project-name)") || line.ends_with(')'),
        "{line}"
    );
    let p = app.state_mut().chat_ui.probe.clone();
    assert!(inside(p.agent_chip.unwrap(), p.header.unwrap()));
}

#[test]
fn a_very_long_title_never_runs_into_the_status_or_the_buttons() {
    let long = "A really quite remarkably long title that just keeps going and going well past the end of the line ".repeat(3);
    for size in SIZES {
        for agent in [false, true] {
            let (mut app, _sent, _s) = agent_fixture();
            let mut h = Harness::new(false);
            let id = {
                let open = app.state_mut().chat.open.as_mut().unwrap();
                open.title = long.clone();
                if !agent {
                    open.mode = SessionMode::Chat;
                }
                open.id
            };
            // The list row carries the title that is shown.
            let mut row = row_for(app.state_mut().chat.open.as_ref().unwrap(), &long);
            row.id = id;
            row.mode = if agent {
                SessionMode::Agent
            } else {
                SessionMode::Chat
            };
            app.state_mut().view = if agent { View::Work } else { View::Chat };
            app.state_mut().chat.fold(ChatEvent::Rows(vec![row]));
            run(&mut app, &mut h, size, 4);
            let p = app.state_mut().chat_ui.probe.clone();
            let title = p.title_rect.expect("title");
            let status = p.status_rect.expect("status text");
            assert!(
                title.right() <= status.left() - 8.0,
                "{size:?} agent={agent}: title {title:?} runs into the status {status:?}"
            );
            if let Some(path) = p.path_rect {
                assert!(
                    path.right() <= status.left() - 8.0,
                    "{size:?}: folder {path:?} runs into the status {status:?}"
                );
                assert!(
                    title.right() <= path.left() + 0.5,
                    "{size:?}: title over the folder"
                );
            }
            let pencil = p.rename_button.unwrap();
            assert!(
                pencil.right() <= status.left() - 4.0,
                "{size:?}: pencil over the status"
            );
        }
    }
}
