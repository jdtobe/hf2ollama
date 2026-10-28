//! Incognito text must not outlive its session or reach disk or logs.

use super::layout_tests::{fixture_with, run};
use crate::browse::test_util::Harness;
use eframe::egui::{self, text_edit::TextEditState, vec2};
use roost_core::chat::ChatEvent;
use std::path::Path;

const SECRET: &str = "the launch codes are 0000";

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn eframe_persistence_stays_off_so_no_text_state_reaches_disk() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .expect("Cargo.lock");
    assert!(
        !lock.contains("name = \"ron\""),
        "eframe persistence appears to be enabled; it would write text-edit undo history \
         (including incognito drafts) to disk"
    );
}

#[test]
fn chat_code_never_prints_or_logs() {
    let mut files = Vec::new();
    for dir in ["/src/chat", "/../roost-core/src/chat"] {
        rust_files(
            Path::new(&format!("{}{dir}", env!("CARGO_MANIFEST_DIR"))),
            &mut files,
        );
    }
    assert!(!files.is_empty());
    let banned = [
        "println!",
        "eprintln!",
        "dbg!",
        "log::",
        "tracing::",
        "print!(",
    ];
    for f in files {
        if f.ends_with("leak_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        // Tests may print while debugging; only production code is checked.
        let production = text.split("#[cfg(test)]").next().unwrap();
        for b in banned {
            assert!(!production.contains(b), "{} uses {b}", f.display());
        }
    }
}

#[test]
fn a_destroyed_incognito_session_leaves_no_draft_or_text_editor_state() {
    let size = (1180.0, 820.0);
    let mut f = fixture_with(1);
    let mut h = Harness::new(false);
    // The fixture's open session is saved; open the incognito one instead.
    let inc = f
        .app
        .state_mut()
        .chat
        .rows()
        .iter()
        .find(|r| r.lifetime == roost_core::session::SessionLifetime::Incognito)
        .cloned()
        .expect("incognito row");
    let session = roost_core::session::Session::new_chat(
        &inc.model,
        roost_core::session::SessionLifetime::Incognito,
        1,
    );
    let mut session = session;
    session.id = inc.id;
    f.app
        .state_mut()
        .chat
        .fold(ChatEvent::Opened(Box::new(session)));
    *f.app.state_mut().chat.draft_mut(inc.id) = SECRET.into();
    f.app.state_mut().chat_ui.focus_composer = true;
    run(&mut f.app, &mut h, size, 3);
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::Text("more secret typing".into())],
        |ui| f.app.show(ui),
    );
    let editor = super::view::composer_id(inc.id);
    assert!(
        TextEditState::load(h.ctx(), editor).is_some(),
        "the composer should have editor state while it is open"
    );

    // The session is destroyed: its row disappears.
    let remaining: Vec<_> = f
        .app
        .state_mut()
        .chat
        .rows()
        .iter()
        .filter(|r| r.id != inc.id)
        .cloned()
        .collect();
    f.app.state_mut().chat.fold(ChatEvent::Rows(remaining));
    run(&mut f.app, &mut h, size, 3);

    assert_eq!(f.app.state_mut().chat.draft(inc.id), "");
    assert!(
        f.app.state_mut().chat.open.is_none()
            || f.app.state_mut().chat.open.as_ref().unwrap().id != inc.id
    );
    assert!(
        TextEditState::load(h.ctx(), editor).is_none(),
        "text editor state (with undo history) must be dropped with the session"
    );
}
