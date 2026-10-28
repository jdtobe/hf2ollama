//! Layout and behaviour of the Models tab (list, chips, detail pane).

use crate::app::{App, View};
use crate::browse::test_util::Harness;
use crate::runtime::{BackendRequest, InstalledEntry};
use eframe::egui::{self, vec2, Pos2, Rect};
use roost_core::chat::ChatCommand;
use roost_core::config::Settings;
use roost_core::fit::Machine;
use roost_core::hf::ModelSummary;
use roost_core::ollama::InstalledModel;
use std::sync::mpsc::Receiver;

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

fn installed(name: &str, repo: Option<&str>) -> InstalledEntry {
    InstalledEntry {
        model: InstalledModel {
            name: name.into(),
            size: 4_680_000_000,
            params: Some("7B".into()),
            quant: Some("Q4_K_M".into()),
            family: Some("llama".into()),
            modified: "2026-10-20T10:00:00Z".into(),
            digest: "sha256:abcdef".into(),
        },
        source_repo: repo.map(str::to_string),
    }
}

fn hit(name: &str) -> ModelSummary {
    ModelSummary {
        id: format!("o/{name}"),
        owner: "o".into(),
        name: name.into(),
        downloads: 1_000,
        likes: 10,
        updated: "2026-10-01T00:00:00.000Z".into(),
        params: Some("7B".into()),
        arch: Some("llama".into()),
        is_gguf: true,
        is_safetensors: false,
    }
}

/// An app on the Models tab with three installed models and two results.
fn models_app() -> (App, Receiver<BackendRequest>) {
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
    let st = app.state_mut();
    st.view = View::Models;
    st.ollama = Some(Some("0.9".into()));
    st.installed.on_installed_loaded(&Ok(vec![
        installed(
            "qwen2.5-coder:32b-instruct-q4_K_M-with-a-very-long-name",
            Some("bartowski/Qwen2.5-Coder-32B-Instruct-GGUF"),
        ),
        installed("llama3.2:3b", None),
        installed("mistral-small3.2:24b", Some("unsloth/Mistral-Small-3.2")),
    ]));
    let q = st.browse.search_now("");
    st.browse
        .on_search_done(&q, Ok(vec![hit("alpha-GGUF"), hit("beta-GGUF")]));
    (app, sent)
}

fn run(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), frames: usize) {
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

fn sent(rx: &Receiver<BackendRequest>) -> Vec<BackendRequest> {
    rx.try_iter().collect()
}

fn texts(app: &mut App, h: &mut Harness, size: (f32, f32)) -> String {
    h.frame_texts(vec2(size.0, size.1), vec![], |ui| app.show(ui))
        .join("\n")
}

/// Selects the first installed row and returns its detail pane.
fn select_first(app: &mut App, h: &mut Harness, size: (f32, f32)) {
    let row = app.state_mut().browse.probe.installed_rows[0].1;
    click(app, h, size, row.center());
}

#[test]
fn the_list_holds_the_chips_and_both_sections_inside_its_column() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _sent) = models_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 4);
            let p = app.state_mut().browse.probe.clone();
            let list = p.list.expect("list");
            assert!(
                (300.0..=361.0).contains(&list.width()),
                "{size:?}: list {} px",
                list.width()
            );
            let chip = p.filters_chip.expect("Filters chip");
            assert!(inside(chip, list), "{size:?}: chip {chip:?} list {list:?}");
            assert_eq!(p.installed_rows.len(), 3, "{size:?}");
            for (name, r) in &p.installed_rows {
                assert!(inside(*r, list), "{size:?}: {name} {r:?} {list:?}");
            }
            let t = texts(&mut app, &mut h, size);
            assert!(t.to_uppercase().contains("INSTALLED"), "{t}");
            assert!(t.to_uppercase().contains("HUGGING FACE"), "{t}");
            assert!(t.contains("alpha-GGUF"), "{t}");
        }
    }
}

#[test]
fn an_installed_model_opens_its_detail_with_actions_in_a_roomy_pane() {
    for size in SIZES {
        let (mut app, rx) = models_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        let _ = sent(&rx);
        select_first(&mut app, &mut h, size);
        let reqs = sent(&rx);
        assert!(
            reqs.iter()
                .any(|r| matches!(r, BackendRequest::ShowInstalled(_))),
            "{size:?}: details requested"
        );
        let p = app.state_mut().browse.probe.clone();
        let detail = p.detail.expect("detail pane");
        assert!(
            detail.width() >= 520.0,
            "{size:?}: detail {} px",
            detail.width()
        );
        for (what, r) in [
            ("Chat", p.chat_button),
            ("Work", p.work_button),
            ("Delete", p.delete_button),
        ] {
            let r = r.unwrap_or_else(|| panic!("{what} button"));
            assert!(inside(r, detail), "{size:?}: {what} {r:?} {detail:?}");
        }
        let t = texts(&mut app, &mut h, size);
        assert!(t.contains("Details") && t.contains("ollama run"), "{t}");
    }
}

#[test]
fn chat_starts_a_chat_with_the_model_and_work_opens_the_agent_form() {
    let size = (1180.0, 820.0);
    let (mut app, rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    select_first(&mut app, &mut h, size);
    let name = app.state_mut().models_installed.clone().unwrap();
    let chat = app.state_mut().browse.probe.chat_button.unwrap();
    let _ = sent(&rx);
    click(&mut app, &mut h, size, chat.center());
    let reqs = sent(&rx);
    assert!(
        reqs.iter().any(|r| matches!(r,
            BackendRequest::Chat(ChatCommand::Create { model, .. }) if *model == name)),
        "{reqs:?}"
    );
    assert_eq!(app.state_mut().view, View::Chat);

    // Back to Models for Work.
    app.state_mut().select(View::Models);
    run(&mut app, &mut h, size, 3);
    let work = app
        .state_mut()
        .browse
        .probe
        .work_button
        .expect("Work button");
    click(&mut app, &mut h, size, work.center());
    assert_eq!(app.state_mut().view, View::Work);
    let st = app.state_mut();
    assert!(st.chat_ui.new_agent.open);
    assert_eq!(st.chat_ui.new_agent.model.as_deref(), Some(name.as_str()));
}

#[test]
fn delete_takes_two_clicks_with_a_pause_between() {
    let size = (1180.0, 820.0);
    let (mut app, rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    select_first(&mut app, &mut h, size);
    let name = app.state_mut().models_installed.clone().unwrap();
    let del = app.state_mut().browse.probe.delete_button.unwrap();
    let _ = sent(&rx);
    click(&mut app, &mut h, size, del.center());
    assert!(
        !sent(&rx)
            .iter()
            .any(|r| matches!(r, BackendRequest::DeleteInstalled(_))),
        "the first click only arms"
    );
    let t = texts(&mut app, &mut h, size);
    assert!(t.contains("Confirm delete"), "{t}");
    std::thread::sleep(crate::confirm::ARM_MIN_DELAY + std::time::Duration::from_millis(100));
    let del = app.state_mut().browse.probe.delete_button.unwrap();
    click(&mut app, &mut h, size, del.center());
    let reqs = sent(&rx);
    assert!(
        reqs.iter()
            .any(|r| matches!(r, BackendRequest::DeleteInstalled(n) if *n == name)),
        "{reqs:?}"
    );
}

#[test]
fn the_search_box_narrows_the_installed_list_too() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    app.state_mut().query = "llama".into();
    run(&mut app, &mut h, size, 4);
    let rows = app.state_mut().browse.probe.installed_rows.clone();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "llama3.2:3b");
}

#[test]
fn picking_a_result_replaces_the_installed_detail() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    select_first(&mut app, &mut h, size);
    assert!(app.state_mut().models_installed.is_some());
    // The first result row sits below the installed rows in the same column.
    let list = app.state_mut().browse.probe.list.unwrap();
    let last = app
        .state_mut()
        .browse
        .probe
        .installed_rows
        .last()
        .unwrap()
        .1;
    let at = Pos2::new(list.center().x, last.bottom() + 28.0 + 40.0);
    click(&mut app, &mut h, size, at);
    assert!(app.state_mut().models_installed.is_none());
    assert!(app.state_mut().browse.probe.detail.is_none());
}

#[test]
fn the_filters_chip_opens_the_filter_controls() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let chip = app.state_mut().browse.probe.filters_chip.unwrap();
    click(&mut app, &mut h, size, chip.center());
    let t = texts(&mut app, &mut h, size);
    for want in ["FORMAT", "PARAMETERS", "TASK"] {
        assert!(t.to_uppercase().contains(want), "{want:?} missing in:\n{t}");
    }
}

#[test]
fn deleting_the_selected_model_clears_the_selection() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    select_first(&mut app, &mut h, size);
    let name = app.state_mut().models_installed.clone().unwrap();
    app.state_mut()
        .installed
        .on_delete_done(&name, &Ok(()), std::time::Instant::now());
    run(&mut app, &mut h, size, 3);
    assert!(app.state_mut().models_installed.is_none());
}
