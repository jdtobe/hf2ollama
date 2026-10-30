//! The close confirmation in the whole window: commands, requests and layout.

use crate::app::App;
use crate::browse::test_util::Harness;
use crate::quit::QuitState;
use crate::runtime::BackendRequest;
use eframe::egui::{self, vec2, Pos2, Rect, ViewportCommand};
use roost_core::config::Settings;
use roost_core::events::{Command, JobEvent, JobId, Stage};
use roost_core::fit::Machine;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

fn new_app() -> (App, Receiver<BackendRequest>) {
    let (tx, req_rx) = channel();
    let (_, rx) = channel();
    let app = App::new(
        &Settings::default(),
        std::path::PathBuf::from("/tmp/staging"),
        Machine {
            ram_gb: 64.0,
            vram_gb: Some(24.0),
        },
        tx,
        rx,
    );
    (app, req_rx)
}

/// Two unsettled jobs (queued, downloading) and one finished.
fn app_with_jobs() -> (App, Receiver<BackendRequest>) {
    let (mut app, rx) = new_app();
    let now = Instant::now();
    let st = app.state_mut();
    for (id, name) in [(1, "done"), (2, "running"), (3, "queued")] {
        st.jobs.accept(JobId(id), name.into(), 100);
    }
    st.jobs.apply(
        &JobEvent::Stage {
            id: JobId(2),
            stage: Stage::Downloading,
        },
        now,
    );
    st.jobs.apply(&JobEvent::Finished { id: JobId(1) }, now);
    (app, rx)
}

fn run(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), frames: usize) {
    for _ in 0..frames {
        h.frame(vec2(w, hh), vec![], |ui| app.show(ui));
    }
}

fn click(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), at: Pos2) {
    let size = vec2(w, hh);
    h.frame(size, vec![egui::Event::PointerMoved(at)], |ui| app.show(ui));
    for pressed in [true, false] {
        let ev = egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(size, vec![ev], |ui| app.show(ui));
    }
    run(app, h, (w, hh), 2);
}

fn cancels(rx: &Receiver<BackendRequest>) -> Vec<u64> {
    let mut ids: Vec<u64> = rx
        .try_iter()
        .filter_map(|r| match r {
            BackendRequest::Core(Command::Cancel(id)) => Some(id.0),
            _ => None,
        })
        .collect();
    ids.sort_unstable();
    ids
}

fn key(pressed: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: Some(egui::Key::Enter),
        pressed,
        repeat,
        modifiers: Default::default(),
    }
}

#[test]
fn closing_with_no_jobs_closes_immediately() {
    let (mut app, _rx) = new_app();
    let cmds = app.process_close(true, Instant::now());
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(app.state_mut().quit, QuitState::Idle);
}

#[test]
fn closing_with_running_jobs_asks_first() {
    for size in SIZES {
        let (mut app, rx) = app_with_jobs();
        let mut h = Harness::new(false);
        let cmds = app.process_close(true, Instant::now());
        assert!(
            matches!(cmds.as_slice(), [ViewportCommand::CancelClose]),
            "{cmds:?}"
        );
        assert_eq!(app.state_mut().quit, QuitState::Confirming);
        run(&mut app, &mut h, size, 4);
        assert!(
            app.state_mut().quit_probe.dialog.is_some(),
            "the dialog is painted"
        );
        assert!(
            cancels(&rx).is_empty(),
            "nothing is cancelled before the user chooses"
        );
    }
}

#[test]
fn keep_running_dismisses_the_dialog_and_leaves_jobs_alone() {
    let (mut app, rx) = app_with_jobs();
    let mut h = Harness::new(false);
    let size = SIZES[0];
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let at = app
        .state_mut()
        .quit_probe
        .keep
        .expect("Keep running painted")
        .center();
    click(&mut app, &mut h, size, at);
    assert_eq!(app.state_mut().quit, QuitState::Idle);
    assert!(cancels(&rx).is_empty());
    assert_eq!(app.state_mut().jobs.unsettled_ids().len(), 2);
    run(&mut app, &mut h, size, 2);
    assert!(
        app.state_mut().quit_probe.dialog.is_none(),
        "the dialog is gone"
    );
}

#[test]
fn quit_and_cancel_sends_cancel_for_every_unsettled_job_and_closes_when_they_settle() {
    let (mut app, rx) = app_with_jobs();
    let mut h = Harness::new(false);
    let size = SIZES[0];
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let at = app
        .state_mut()
        .quit_probe
        .quit
        .expect("Quit and cancel painted")
        .center();
    // Past the guard that ignores a click right after the dialog opened.
    std::thread::sleep(Duration::from_millis(700));
    click(&mut app, &mut h, size, at);
    assert_eq!(cancels(&rx), vec![2, 3]);
    assert!(matches!(app.state_mut().quit, QuitState::Cancelling { .. }));
    // Still waiting while the jobs run.
    assert!(app.process_close(false, Instant::now()).is_empty());
    for id in [2, 3] {
        app.state_mut()
            .jobs
            .apply(&JobEvent::Cancelled { id: JobId(id) }, Instant::now());
    }
    let cmds = app.process_close(false, Instant::now());
    assert!(
        matches!(cmds.as_slice(), [ViewportCommand::Close]),
        "{cmds:?}"
    );
    assert!(
        app.process_close(false, Instant::now()).is_empty(),
        "closes once"
    );
}

#[test]
fn dialog_buttons_are_inside_the_window_and_keep_running_is_the_default() {
    for dark in [false, true] {
        for size in SIZES {
            let (mut app, _rx) = app_with_jobs();
            let mut h = Harness::new(dark);
            app.process_close(true, Instant::now());
            run(&mut app, &mut h, size, 4);
            let p = app.state_mut().quit_probe.clone();
            let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
            let (dialog, keep, quit) = (
                p.dialog.expect("dialog"),
                p.keep.expect("keep"),
                p.quit.expect("quit"),
            );
            for (name, r) in [("dialog", dialog), ("keep", keep), ("quit", quit)] {
                assert!(
                    window.contains_rect(r),
                    "dark={dark} {size:?}: {name} {r:?} outside {window:?}"
                );
            }
            assert!(dialog.contains_rect(keep) && dialog.contains_rect(quit));
            assert!(!keep.intersects(quit), "buttons overlap: {keep:?} {quit:?}");
            assert!(
                p.keep_has_focus,
                "dark={dark} {size:?}: Keep running starts focused"
            );
        }
    }
}

#[test]
fn held_enter_key_never_confirms_quit_and_cancel() {
    let size = SIZES[1];
    let (mut app, rx) = app_with_jobs();
    let mut h = Harness::new(false);
    let (w, hh) = size;
    // Enter goes down before the dialog exists and stays down (a held key).
    run(&mut app, &mut h, size, 2);
    h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let id = app
        .state_mut()
        .quit_probe
        .quit_id
        .expect("quit button painted");
    h.frame(vec2(w, hh), vec![], |ui| {
        app.show(ui);
        ui.memory_mut(|m| m.request_focus(id));
    });
    run(&mut app, &mut h, size, 2);
    std::thread::sleep(Duration::from_millis(700));
    for _ in 0..3 {
        h.frame(vec2(w, hh), vec![key(true, true)], |ui| app.show(ui));
    }
    assert!(cancels(&rx).is_empty(), "a held key confirmed the quit");
    assert_eq!(app.state_mut().quit, QuitState::Confirming);
    // A fresh press is deliberate.
    h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
    h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
    h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
    assert_eq!(cancels(&rx), vec![2, 3]);
}

// ---- running chats and agents

fn app_with_a_run() -> (App, Receiver<BackendRequest>) {
    use roost_core::chat::ChatEvent;
    use roost_core::session::{SessionId, TurnId};
    let (mut app, rx) = new_app();
    app.state_mut().chat.fold(ChatEvent::TurnStarted {
        session: SessionId::new(),
        turn: TurnId::new(),
    });
    (app, rx)
}

#[test]
fn closing_with_a_run_in_progress_asks_first_and_says_what_is_running() {
    for size in SIZES {
        let (mut app, rx) = app_with_a_run();
        let mut h = Harness::new(false);
        let cmds = app.process_close(true, Instant::now());
        assert!(
            matches!(cmds.as_slice(), [ViewportCommand::CancelClose]),
            "{size:?}: {cmds:?}"
        );
        assert_eq!(app.state_mut().quit, QuitState::Confirming);
        run(&mut app, &mut h, size, 4);
        assert!(app.state_mut().quit_probe.dialog.is_some(), "{size:?}");
        let texts = h
            .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
            .join("\n");
        assert!(
            texts.contains("1 chat or agent run is still running"),
            "{size:?}: {texts}"
        );
        assert!(!texts.contains("Running downloads"), "{size:?}: {texts}");
        assert!(cancels(&rx).is_empty());
    }
}

#[test]
fn keeping_a_run_going_dismisses_the_dialog() {
    let (mut app, _rx) = app_with_a_run();
    let mut h = Harness::new(false);
    let size = SIZES[0];
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let at = app
        .state_mut()
        .quit_probe
        .keep
        .expect("Keep running")
        .center();
    click(&mut app, &mut h, size, at);
    assert_eq!(app.state_mut().quit, QuitState::Idle);
    assert_eq!(
        app.state_mut().chat.active_runs(),
        1,
        "the run was left alone"
    );
}

#[test]
fn quitting_with_a_run_in_progress_closes_without_waiting_for_downloads() {
    let (mut app, _rx) = app_with_a_run();
    let mut h = Harness::new(false);
    let size = SIZES[0];
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let at = app
        .state_mut()
        .quit_probe
        .quit
        .expect("Quit and cancel")
        .center();
    click(&mut app, &mut h, size, at);
    // No download to settle: the window closes on the next frame. The chat
    // runs are ended by the shutdown that happens when the app exits.
    let cmds = app.process_close(false, Instant::now());
    assert!(
        matches!(cmds.as_slice(), [ViewportCommand::Close]),
        "{cmds:?}"
    );
}

#[test]
fn downloads_and_runs_are_named_together() {
    let (mut app, _rx) = app_with_jobs();
    app.state_mut()
        .chat
        .fold(roost_core::chat::ChatEvent::TurnStarted {
            session: roost_core::session::SessionId::new(),
            turn: roost_core::session::TurnId::new(),
        });
    let mut h = Harness::new(false);
    let size = SIZES[0];
    app.process_close(true, Instant::now());
    run(&mut app, &mut h, size, 4);
    let texts = h
        .frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n");
    assert!(
        texts.contains("2 downloads and 1 chat or agent run are still running"),
        "{texts}"
    );
}
