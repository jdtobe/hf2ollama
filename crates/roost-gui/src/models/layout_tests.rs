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
            Some("o/alpha-GGUF"),
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

/// Selects the first installed row.
fn select_first(app: &mut App, h: &mut Harness, size: (f32, f32)) {
    let row = app.state_mut().browse.probe.installed_rows[0].1;
    click(app, h, size, row.center());
}

/// Selects the installed row whose name starts with `prefix`.
fn select_named(app: &mut App, h: &mut Harness, size: (f32, f32), prefix: &str) {
    let row = app
        .state_mut()
        .browse
        .probe
        .installed_rows
        .iter()
        .find(|(n, _)| n.starts_with(prefix))
        .map(|(_, r)| *r)
        .expect("row");
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

fn card_info(id: &str) -> roost_core::hf::ModelInfo {
    roost_core::hf::ModelInfo {
        id: id.into(),
        files: Vec::new(),
        arch: Some("llama".into()),
        params: Some("7B".into()),
        context: Some(32768),
        license: Some("apache-2.0".into()),
        base_model: None,
        downloads: 1_000,
        likes: 10,
        updated: "2026-01-01T00:00:00.000Z".into(),
        gated: false,
        pipeline_tag: None,
        languages: Vec::new(),
        tags: Vec::new(),
    }
}

#[test]
fn results_for_an_installed_models_repository_say_so() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let t = texts(&mut app, &mut h, size);
    let badges = t.lines().filter(|l| *l == "installed").count();
    assert_eq!(
        badges, 1,
        "only alpha-GGUF is installed:
{t}"
    );
}

#[test]
fn an_installed_model_from_a_repository_shows_that_repositorys_about_below_the_actions() {
    let size = (1180.0, 820.0);
    let (mut app, rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let _ = sent(&rx);
    select_named(&mut app, &mut h, size, "qwen2.5-coder");
    run(&mut app, &mut h, size, 2);
    let reqs = sent(&rx);
    assert!(
        reqs.iter()
            .any(|r| matches!(r, BackendRequest::ModelCard(id) if id == "o/alpha-GGUF")),
        "{reqs:?}"
    );
    let t = texts(&mut app, &mut h, size);
    assert!(
        t.contains("Loading description") || t.contains("About"),
        "{t}"
    );
    {
        let st = app.state_mut();
        st.browse
            .on_model_info("o/alpha-GGUF", Ok(card_info("o/alpha-GGUF")));
        st.browse.on_model_card(
            "o/alpha-GGUF",
            Ok(Some(roost_core::hf::ModelCard {
                markdown: "A friendly description of alpha.".into(),
                truncated: false,
            })),
        );
    }
    run(&mut app, &mut h, size, 3);
    let t = texts(&mut app, &mut h, size);
    assert!(t.contains("A friendly description of alpha."), "{t}");
    // The About card comes after the action buttons.
    let chat = app.state_mut().browse.probe.chat_button.unwrap();
    let about = app.state_mut().browse.probe.about_top.expect("About card");
    assert!(about > chat.bottom(), "about {about} chat {chat:?}");
}

#[test]
fn a_library_model_says_it_has_no_hugging_face_description() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let llama = app
        .state_mut()
        .browse
        .probe
        .installed_rows
        .iter()
        .find(|(n, _)| n == "llama3.2:3b")
        .unwrap()
        .1;
    click(&mut app, &mut h, size, llama.center());
    let t = texts(&mut app, &mut h, size);
    assert!(t.contains("no Hugging Face description"), "{t}");
}

#[test]
fn installed_rows_have_room_and_show_their_facts_as_pills() {
    for size in SIZES {
        let (mut app, _rx) = models_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        let rows = app.state_mut().browse.probe.installed_rows.clone();
        for (name, r) in &rows {
            assert!(
                r.height() >= 76.0,
                "{size:?}: {name} only {} px",
                r.height()
            );
        }
        let t = texts(&mut app, &mut h, size);
        let lines: Vec<&str> = t.lines().collect();
        for pill in ["7B", "Q4_K_M", "4.68 GB"] {
            assert!(lines.contains(&pill), "{pill} pill missing in:\n{t}");
        }
        assert!(t.contains("Pulled from the Ollama library"), "{t}");
    }
}

#[test]
fn the_filter_bar_is_a_full_width_sort_and_a_filter_button_that_never_moves() {
    for size in SIZES {
        let (mut app, _rx) = models_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        let list = app.state_mut().browse.probe.list.unwrap();
        let before = app.state_mut().browse.probe.filters_chip.unwrap();
        assert!(inside(before, list), "{size:?}");
        assert!(
            before.right() >= list.right() - 20.0,
            "{size:?}: the button sits at the right end: {before:?} in {list:?}"
        );
        // Turning a filter on changes the button's look, not its place.
        app.state_mut().browse.filters.only_fit = true;
        run(&mut app, &mut h, size, 3);
        let after = app.state_mut().browse.probe.filters_chip.unwrap();
        assert_eq!(before, after, "{size:?}: the button moved");
        let t = texts(&mut app, &mut h, size);
        assert!(
            !t.contains("Fits my machine"),
            "no separate chip any more:\n{t}"
        );
    }
}

#[test]
fn the_filter_popup_has_a_reset_and_the_fit_option() {
    let size = (1180.0, 820.0);
    let (mut app, _rx) = models_app();
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 4);
    let chip = app.state_mut().browse.probe.filters_chip.unwrap();
    click(&mut app, &mut h, size, chip.center());
    let t = texts(&mut app, &mut h, size);
    for want in ["Reset", "Only models that fit", "Parameters"] {
        assert!(
            t.contains(want) || t.to_uppercase().contains(&want.to_uppercase()),
            "{want:?} missing in:\n{t}"
        );
    }
}

#[test]
fn the_detail_page_is_three_stacked_bands() {
    for size in SIZES {
        let (mut app, _rx) = models_app();
        let mut h = Harness::new(false);
        run(&mut app, &mut h, size, 4);
        select_first(&mut app, &mut h, size);
        run(&mut app, &mut h, size, 2);
        let p = app.state_mut().browse.probe.clone();
        let detail = p.detail.unwrap();
        assert_eq!(p.bands.len(), 3, "{size:?}");
        for w in p.bands.windows(2) {
            assert!(
                (w[0].bottom() - w[1].top()).abs() < 1.0,
                "{size:?}: gap in {:?}",
                p.bands
            );
        }
        for b in &p.bands {
            assert!(
                (b.width() - detail.width()).abs() < 20.0,
                "{size:?}: band {b:?}"
            );
        }
        // The buttons end well inside the top band.
        let delete = p.delete_button.unwrap();
        assert!(
            p.bands[0].bottom() - delete.bottom() >= 12.0,
            "{size:?}: {} px of padding under the buttons",
            p.bands[0].bottom() - delete.bottom()
        );
    }
}

#[test]
fn the_usage_card_names_its_segments_with_a_legend() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _rx) = models_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 4);
            let t = texts(&mut app, &mut h, size);
            for want in [
                "Disk used by models",
                "Imported with Roost",
                "Pulled from Ollama",
                "14.04 GB",
            ] {
                assert!(
                    t.contains(want),
                    "{size:?} dark={dark}: {want:?} missing in:\n{t}"
                );
            }
            let list = app.state_mut().browse.probe.list.unwrap();
            let first = app.state_mut().browse.probe.installed_rows[0].1;
            assert!(
                first.top() > list.top() + 60.0,
                "the card sits above the rows"
            );
        }
    }
}
