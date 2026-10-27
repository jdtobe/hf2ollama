//! Headless layout and interaction tests: real painted rectangles at the two
//! window sizes the layout is designed for, in both themes, and real pointer
//! and key events.

use super::SettingsProbe;
use crate::app::{App, View};
use crate::browse::test_util::Harness;
use crate::runtime::{BackendRequest, CachedRow, HfTokenStatus};
use eframe::egui::{self, vec2, Pos2, Rect};
use hf2ollama_core::config::Settings;
use hf2ollama_core::events::{JobEvent, JobId, Stage};
use hf2ollama_core::fit::Machine;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];
const PAST_DELAY: Duration = Duration::from_millis(700);
const CARD_NAMES: [&str; 5] = [
    "Ollama",
    "Downloads",
    "Cached files",
    "Hugging Face",
    "Hardware and appearance",
];
const CONTROLS: [&str; 15] = [
    "ollama-url",
    "ollama-test",
    "staging",
    "staging-browse",
    "toggle-delete",
    "toggle-partials",
    "select-dismiss",
    "select-parallel",
    "clear-cached",
    "token",
    "token-save",
    "token-clear",
    "hf-endpoint",
    "toggle-override",
    "select-theme",
];

fn settings_app() -> (App, Receiver<BackendRequest>) {
    let (tx, sent) = std::sync::mpsc::channel();
    let (_, rx) = std::sync::mpsc::channel();
    let machine = Machine {
        ram_gb: 32.0,
        vram_gb: Some(16.0),
    };
    let mut app = App::new(
        &Settings::default(),
        std::path::PathBuf::from("/tmp/staging"),
        machine,
        tx,
        rx,
    );
    let st = app.state_mut();
    st.ollama = Some(Some("0.9".into()));
    st.view = View::Settings;
    let v = &mut st.settings_view.state;
    v.on_token_status(&HfTokenStatus::Valid("jdtobe".into()));
    v.on_cached_loaded(&Ok(vec![
        CachedRow {
            label: "job-1".into(),
            bytes: 4_680_000_000,
            partial: false,
        },
        CachedRow {
            label: "job-2".into(),
            bytes: 320_000_000,
            partial: true,
        },
    ]));
    (app, sent)
}

fn run(app: &mut App, harness: &mut Harness, (w, h): (f32, f32), frames: usize) {
    for _ in 0..frames {
        harness.frame(vec2(w, h), vec![], |ui| app.show(ui));
    }
}

fn click(app: &mut App, harness: &mut Harness, (w, h): (f32, f32), at: Pos2) {
    harness.frame(vec2(w, h), vec![egui::Event::PointerMoved(at)], |ui| {
        app.show(ui)
    });
    for pressed in [true, false] {
        harness.frame(
            vec2(w, h),
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }],
            |ui| app.show(ui),
        );
    }
    run(app, harness, (w, h), 3);
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.top() >= outer.top() - 0.5
        && inner.bottom() <= outer.bottom() + 0.5
        && inner.left() >= outer.left() - 0.5
        && inner.right() <= outer.right() + 0.5
}

fn window(size: (f32, f32)) -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1))
}

fn probe(app: &mut App) -> SettingsProbe {
    app.state_mut().settings_view.probe.clone()
}

fn scroll_to(app: &mut App, offset: f32) {
    app.state_mut().settings_view.scroll_to = Some(offset);
}

fn control(p: &SettingsProbe, name: &str) -> Option<Rect> {
    p.controls.iter().find(|(n, _)| n == name).map(|(_, r)| *r)
}

/// Scrolls until the control is fully inside the viewport and returns its rect.
fn reveal(app: &mut App, h: &mut Harness, size: (f32, f32), name: &str) -> Rect {
    let mut offset = 0.0;
    loop {
        scroll_to(app, offset);
        run(app, h, size, 3);
        let p = probe(app);
        let view = p.viewport.expect("viewport painted");
        assert!(
            inside(view, window(size)),
            "{size:?}: the viewport {view:?} leaves the window"
        );
        if let Some(r) = control(&p, name) {
            if inside(r, view) {
                return r;
            }
        }
        assert!(
            offset <= p.max_offset + 1.0,
            "{size:?}: {name} is never fully visible (viewport {view:?}, rect {:?})",
            control(&p, name)
        );
        offset += 40.0;
    }
}

fn override_on(app: &mut App) {
    let s = app.state_mut();
    let mut settings = s.settings.clone();
    s.settings_view.state.set_override(&mut settings, true);
    s.settings = settings;
}

fn add_running_jobs(app: &mut App, n: u64) {
    let now = Instant::now();
    for i in 1..=n {
        let id = JobId(i);
        let st = app.state_mut();
        st.jobs
            .accept(id, format!("bartowski/model-{i}:q4_k_m"), 4_680_000_000);
        st.jobs.apply(
            &JobEvent::Stage {
                id,
                stage: Stage::Downloading,
            },
            now,
        );
        st.jobs.apply(
            &JobEvent::Progress {
                id,
                done: 1_900_000_000,
                total: 4_680_000_000,
            },
            now,
        );
        st.dock.open_log = Some(id);
    }
}

#[test]
fn settings_cards_are_inside_the_window_and_do_not_overlap() {
    for size in SIZES {
        for dark in [false, true] {
            let tag = format!("{size:?} dark={dark}");
            let (mut app, _) = settings_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let p = probe(&mut app);
            let panel = p.panel.expect("view painted");
            let view = p.viewport.expect("viewport painted");
            assert!(inside(panel, window(size)), "{tag}: panel {panel:?}");
            assert!(inside(view, panel), "{tag}: viewport {view:?} vs {panel:?}");
            let names: Vec<&str> = p.cards.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, CARD_NAMES, "{tag}");
            for (n, r) in &p.cards {
                assert!(
                    r.left() >= view.left() - 0.5 && r.right() <= view.right() + 0.5,
                    "{tag}: card {n} {r:?} leaves {view:?}"
                );
                assert!(r.width() >= 500.0, "{tag}: card {n} collapsed: {r:?}");
            }
            for pair in p.cards.windows(2) {
                assert!(
                    pair[0].1.bottom() <= pair[1].1.top() + 0.5,
                    "{tag}: {} overlaps {}",
                    pair[0].0,
                    pair[1].0
                );
            }
            let first = p.cards[0].1;
            let last = p.cards[4].1;
            assert!(
                last.bottom() - first.top() <= p.content_height + 1.0,
                "{tag}: cards taller than the content"
            );
            // The end of the page is reachable.
            scroll_to(&mut app, p.max_offset);
            run(&mut app, &mut h, size, 4);
            let p = probe(&mut app);
            let view = p.viewport.expect("viewport");
            let last = p.cards[4].1;
            assert!(
                last.bottom() <= view.bottom() + 0.5 && last.top() >= view.top() - 0.5,
                "{tag}: last card {last:?} not reachable in {view:?} (max offset {})",
                p.max_offset
            );
        }
    }
}

#[test]
fn every_control_is_fully_visible_after_scrolling_to_it() {
    for size in SIZES {
        for dark in [false, true] {
            for with_override in [false, true] {
                let tag = format!("{size:?} dark={dark} override={with_override}");
                let (mut app, _) = settings_app();
                if with_override {
                    override_on(&mut app);
                }
                let mut h = Harness::new(dark);
                run(&mut app, &mut h, size, 6);
                let mut names: Vec<&str> = CONTROLS.to_vec();
                if with_override {
                    names.extend(["ram", "vram"]);
                }
                let p = probe(&mut app);
                let max = p.max_offset;
                let step = (p.viewport.expect("viewport").height() / 3.0).max(20.0);
                let mut seen = std::collections::HashSet::new();
                let mut offset = 0.0;
                loop {
                    scroll_to(&mut app, offset);
                    run(&mut app, &mut h, size, 3);
                    let p = probe(&mut app);
                    let view = p.viewport.expect("viewport");
                    assert!(
                        inside(view, window(size)),
                        "{tag}: the viewport {view:?} leaves the window"
                    );
                    for (n, r) in &p.controls {
                        assert!(
                            r.left() >= view.left() - 0.5 && r.right() <= view.right() + 0.5,
                            "{tag}: {n} {r:?} is clipped sideways by {view:?}"
                        );
                        if inside(*r, view) {
                            seen.insert(n.clone());
                        }
                    }
                    if offset >= max {
                        break;
                    }
                    offset = (offset + step).min(max);
                }
                for n in names {
                    assert!(seen.contains(n), "{tag}: {n} never fully visible");
                }
            }
        }
    }
}

#[test]
fn help_text_wraps_without_clipping() {
    for size in SIZES {
        for dark in [false, true] {
            let tag = format!("{size:?} dark={dark}");
            let (mut app, _) = settings_app();
            override_on(&mut app);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let p = probe(&mut app);
            assert!(
                p.help.len() >= 10,
                "{tag}: only {} help texts",
                p.help.len()
            );
            for help in &p.help {
                assert!(
                    help.rect.right() <= help.column_right + 0.5,
                    "{tag}: help of '{}' {:?} runs past its column ({})",
                    help.title,
                    help.rect,
                    help.column_right
                );
                assert!(
                    help.rect.height() + 0.5 >= help.galley.size().y,
                    "{tag}: help of '{}' is clipped: {:?} vs {:?}",
                    help.title,
                    help.rect,
                    help.galley.size()
                );
                // Lines break between words, never inside one.
                let rows = &help.galley.rows;
                for row in rows.iter().take(rows.len().saturating_sub(1)) {
                    let last = row.glyphs.last().map(|g| g.chr);
                    assert!(
                        last.is_some_and(char::is_whitespace),
                        "{tag}: '{}' breaks inside a word (line ends with {last:?})",
                        help.title
                    );
                }
            }
        }
    }
}

#[test]
fn settings_view_keeps_usable_height_with_dock_open() {
    for size in SIZES {
        for dark in [false, true] {
            let tag = format!("{size:?} dark={dark}");
            let (mut app, _) = settings_app();
            add_running_jobs(&mut app, 6);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 40);
            let p = probe(&mut app);
            let view = p.viewport.expect("viewport");
            let panel = p.panel.expect("panel");
            assert!(
                view.height() >= 150.0,
                "{tag}: cards viewport only {} px",
                view.height()
            );
            assert!(inside(view, panel), "{tag}");
            assert!(inside(panel, window(size)), "{tag}");
            assert!(
                p.content_height <= view.height() + p.max_offset + 1.0,
                "{tag}: content {} cannot be scrolled into {} + {}",
                p.content_height,
                view.height(),
                p.max_offset
            );
            let r = reveal(&mut app, &mut h, size, "select-theme");
            assert!(inside(r, view), "{tag}");
        }
    }
}

#[test]
fn password_field_is_masked() {
    const TYPED: &str = "hf_zQ9xKmTpL2";
    for size in SIZES {
        for dark in [false, true] {
            let tag = format!("{size:?} dark={dark}");
            let (mut app, _) = settings_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 4);
            let r = reveal(&mut app, &mut h, size, "token");
            click(&mut app, &mut h, size, r.center());
            let (w, hh) = size;
            let events = vec![egui::Event::Text(TYPED.to_string())];
            let mut texts = h.frame_texts(vec2(w, hh), events, |ui| app.show(ui));
            texts.extend(h.frame_texts(vec2(w, hh), vec![], |ui| app.show(ui)));
            assert_eq!(
                app.state_mut().settings_view.state.token_input,
                TYPED,
                "{tag}: the field took the typed text"
            );
            for t in &texts {
                assert!(!t.contains(TYPED), "{tag}: painted the token: {t:?}");
                for part in ["hf_z", "zQ9", "xKm", "TpL", "pL2"] {
                    assert!(!t.contains(part), "{tag}: painted {part} in {t:?}");
                }
            }
            assert!(
                texts
                    .iter()
                    .any(|t| t.chars().filter(|c| *c == '•').count() >= TYPED.len()),
                "{tag}: no masked field painted: {texts:?}"
            );
        }
    }
}

fn key_z(command: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Z,
        physical_key: Some(egui::Key::Z),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command,
            ctrl: command,
            ..Default::default()
        },
    }
}

#[test]
fn leaving_settings_wipes_the_token_field_and_its_undo_history() {
    const TYPED: &str = "hf_zQ9xKmTpL2";
    let size = SIZES[0];
    let (w, hh) = size;
    let (mut app, _sent) = settings_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let field = reveal(&mut app, &mut h, size, "token");
    click(&mut app, &mut h, size, field.center());
    h.frame(vec2(w, hh), vec![egui::Event::Text(TYPED.into())], |ui| {
        app.show(ui)
    });
    run(&mut app, &mut h, size, 3);
    assert_eq!(app.state_mut().settings_view.state.token_input, TYPED);

    app.state_mut().select(View::Installed);
    run(&mut app, &mut h, size, 3);
    assert!(
        app.state_mut().settings_view.state.token_input.is_empty(),
        "the typed token must not outlive the Settings view"
    );

    app.state_mut().select(View::Settings);
    run(&mut app, &mut h, size, 4);
    let field = reveal(&mut app, &mut h, size, "token");
    click(&mut app, &mut h, size, field.center());
    for _ in 0..2 {
        h.frame(vec2(w, hh), vec![key_z(true)], |ui| app.show(ui));
    }
    run(&mut app, &mut h, size, 2);
    assert!(
        app.state_mut().settings_view.state.token_input.is_empty(),
        "Ctrl+Z brought the token back"
    );
}

#[test]
fn saving_the_token_also_wipes_the_undo_history() {
    const TYPED: &str = "hf_zQ9xKmTpL2";
    let size = SIZES[0];
    let (w, hh) = size;
    let (mut app, _sent) = settings_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let field = reveal(&mut app, &mut h, size, "token");
    click(&mut app, &mut h, size, field.center());
    h.frame(vec2(w, hh), vec![egui::Event::Text(TYPED.into())], |ui| {
        app.show(ui)
    });
    run(&mut app, &mut h, size, 3);
    let save = reveal(&mut app, &mut h, size, "token-save");
    click(&mut app, &mut h, size, save.center());
    assert!(app.state_mut().settings_view.state.token_input.is_empty());
    let field = reveal(&mut app, &mut h, size, "token");
    click(&mut app, &mut h, size, field.center());
    for _ in 0..2 {
        h.frame(vec2(w, hh), vec![key_z(true)], |ui| app.show(ui));
    }
    run(&mut app, &mut h, size, 2);
    assert!(
        app.state_mut().settings_view.state.token_input.is_empty(),
        "Ctrl+Z brought the saved token back"
    );
}

#[test]
fn saving_the_token_clears_the_field_and_sends_it_only_to_the_backend() {
    const TYPED: &str = "hf_zQ9xKmTpL2";
    let size = SIZES[0];
    let (mut app, sent) = settings_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let field = reveal(&mut app, &mut h, size, "token");
    click(&mut app, &mut h, size, field.center());
    h.frame(
        vec2(size.0, size.1),
        vec![egui::Event::Text(TYPED.into())],
        |ui| app.show(ui),
    );
    run(&mut app, &mut h, size, 3);
    let save = reveal(&mut app, &mut h, size, "token-save");
    click(&mut app, &mut h, size, save.center());
    assert!(app.state_mut().settings_view.state.token_input.is_empty());
    let reqs: Vec<_> = sent.try_iter().collect();
    assert!(
        matches!(reqs.as_slice(), [BackendRequest::SetHfToken(Some(t))] if t.expose() == TYPED),
        "{reqs:?}"
    );
    assert!(!format!("{reqs:?}").contains(TYPED));
}

fn fast_double_click(app: &mut App, h: &mut Harness, size: (f32, f32), at: Pos2) {
    // Two complete clicks without a pause between them.
    for _ in 0..2 {
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
    }
    run(app, h, size, 3);
}

#[test]
fn clear_cached_button_needs_a_deliberate_second_click() {
    for size in SIZES {
        for dark in [false, true] {
            let tag = format!("{size:?} dark={dark}");
            let (mut app, sent) = settings_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 4);
            let r = reveal(&mut app, &mut h, size, "clear-cached");
            fast_double_click(&mut app, &mut h, size, r.center());
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(
                !reqs
                    .iter()
                    .any(|q| matches!(q, BackendRequest::ClearCached)),
                "{tag}: a fast double click cleared: {reqs:?}"
            );
            assert!(
                app.state_mut()
                    .settings_view
                    .state
                    .clear
                    .is_armed(crate::settings::state::CLEAR_KEY, Instant::now()),
                "{tag}: the first click arms"
            );
            // The button keeps its place and size while armed.
            let r2 = reveal(&mut app, &mut h, size, "clear-cached");
            assert_eq!(r.size(), r2.size(), "{tag}: the button changed size");
            std::thread::sleep(PAST_DELAY);
            click(&mut app, &mut h, size, r2.center());
            let reqs: Vec<_> = sent.try_iter().collect();
            let clears = reqs
                .iter()
                .filter(|q| matches!(q, BackendRequest::ClearCached))
                .count();
            assert_eq!(clears, 1, "{tag}: {reqs:?}");
        }
    }
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
fn held_enter_key_never_clears_cached_files() {
    for size in SIZES {
        let (mut app, sent) = settings_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        reveal(&mut app, &mut h, size, "clear-cached");
        // Focus the button and press Enter: that arms it.
        let (w, hh) = size;
        let id = probe(&mut app).clear_id.expect("clear button painted");
        h.frame(vec2(w, hh), vec![], |ui| {
            app.show(ui);
            ui.memory_mut(|m| m.request_focus(id));
        });
        h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
        let armed = app
            .state_mut()
            .settings_view
            .state
            .clear
            .is_armed(crate::settings::state::CLEAR_KEY, Instant::now());
        assert!(armed, "{size:?}: Enter on the focused button arms");
        std::thread::sleep(PAST_DELAY);
        for _ in 0..3 {
            h.frame(vec2(w, hh), vec![key(true, true)], |ui| app.show(ui));
        }
        let reqs: Vec<_> = sent.try_iter().collect();
        assert!(
            !reqs
                .iter()
                .any(|q| matches!(q, BackendRequest::ClearCached)),
            "{size:?}: a held key cleared: {reqs:?}"
        );
        // Release, then a fresh press is a deliberate confirmation.
        h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
        h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
        h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
        let reqs: Vec<_> = sent.try_iter().collect();
        assert_eq!(
            reqs.iter()
                .filter(|q| matches!(q, BackendRequest::ClearCached))
                .count(),
            1,
            "{size:?}: fresh Enter press: {reqs:?}"
        );
    }
}

fn tab() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: Some(egui::Key::Tab),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }
}

#[test]
fn clicking_a_toggle_applies_and_sends_one_apply_settings() {
    for size in SIZES {
        let (mut app, sent) = settings_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        let r = reveal(&mut app, &mut h, size, "toggle-delete");
        click(&mut app, &mut h, size, r.center());
        assert!(!app.state_mut().settings.delete_after_import);
        assert!(
            !app.state_mut().delete_after_import,
            "live for the status bar"
        );
        let reqs: Vec<_> = sent.try_iter().collect();
        assert!(
            matches!(reqs.as_slice(), [BackendRequest::ApplySettings(s)] if !s.delete_after_import),
            "{reqs:?}"
        );
    }
}

#[test]
fn a_bad_address_shows_an_error_and_sends_nothing() {
    let size = SIZES[0];
    let (mut app, sent) = settings_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let r = reveal(&mut app, &mut h, size, "ollama-url");
    click(&mut app, &mut h, size, r.center());
    // Replace the whole text.
    h.frame(
        vec2(size.0, size.1),
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: Some(egui::Key::A),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text("ftp://nope".into()),
        ],
        |ui| app.show(ui),
    );
    run(&mut app, &mut h, size, 2);
    h.frame(vec2(size.0, size.1), vec![tab()], |ui| app.show(ui));
    run(&mut app, &mut h, size, 3);
    let st = app.state_mut();
    assert!(st.settings_view.state.ollama_error.is_some());
    assert_eq!(st.settings.ollama_url, "http://localhost:11434");
    let reqs: Vec<_> = sent.try_iter().collect();
    assert!(reqs.is_empty(), "{reqs:?}");
}

#[test]
fn test_button_asks_the_backend_to_test_the_typed_address() {
    let size = SIZES[0];
    let (mut app, sent) = settings_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let r = reveal(&mut app, &mut h, size, "ollama-test");
    click(&mut app, &mut h, size, r.center());
    let reqs: Vec<_> = sent.try_iter().collect();
    assert!(
        matches!(reqs.as_slice(), [BackendRequest::TestConnection(u)] if u == "http://localhost:11434"),
        "{reqs:?}"
    );
}
