//! Tests of the About tab: chips, states, layout and links.

use super::about::{chips, MAX_LANGUAGE_CHIPS};
use super::state::DetailTab;
use super::test_util::Harness;
use crate::app::App;
use eframe::egui::{self, vec2, Pos2, Rect};
use hf2ollama_core::config::Settings;
use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::{ModelCard, ModelInfo};
use hf2ollama_core::model::RepoFile;

const ID: &str = "o/repo";
const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

fn info() -> ModelInfo {
    let file = |path: &str, size: u64| RepoFile {
        path: path.into(),
        size,
        sha256: Some(format!("sha-{path}")),
        blob_id: None,
    };
    ModelInfo {
        id: ID.into(),
        files: vec![
            file("m-Q4_K_M.gguf", 4_680_000_000),
            file("m-Q8_0.gguf", 8_000_000_000),
        ],
        arch: Some("llama".into()),
        params: Some("7B".into()),
        context: Some(32768),
        license: Some("apache-2.0".into()),
        base_model: Some("mistralai/Mistral-7B-v0.3".into()),
        downloads: 1_000,
        likes: 10,
        updated: "2026-01-01T00:00:00.000Z".into(),
        gated: false,
        pipeline_tag: Some("text-generation".into()),
        languages: vec!["en".into(), "de".into()],
        tags: ["gguf", "llama", "region:us", "endpoints_compatible"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

fn new_app() -> App {
    let (tx, _) = std::sync::mpsc::channel();
    let (_, rx) = std::sync::mpsc::channel();
    let machine = Machine {
        ram_gb: 64.0,
        vram_gb: Some(24.0),
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
    st.browse.on_model_info(ID, Ok(info()));
    st.browse.select_repo(ID);
    app
}

fn card(md: &str) -> Result<Option<ModelCard>, String> {
    Ok(Some(ModelCard {
        markdown: md.to_string(),
        truncated: false,
    }))
}

fn about_app(result: Option<Result<Option<ModelCard>, String>>) -> App {
    let mut app = new_app();
    let st = app.state_mut();
    st.browse.set_tab(DetailTab::About);
    if let Some(r) = result {
        st.browse.on_model_card(ID, r);
    }
    app
}

fn run(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), frames: usize) {
    for _ in 0..frames {
        h.frame(vec2(w, hh), vec![], |ui| app.show(ui));
    }
}

/// Runs frames with a click at `at`; returns every URL opened.
fn click(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), at: Pos2) -> Vec<String> {
    let size = vec2(w, hh);
    let mut urls = Vec::new();
    urls.extend(h.frame_urls(size, vec![egui::Event::PointerMoved(at)], |ui| app.show(ui)));
    for pressed in [true, false] {
        let ev = egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        urls.extend(h.frame_urls(size, vec![ev], |ui| app.show(ui)));
    }
    for _ in 0..4 {
        urls.extend(h.frame_urls(size, vec![], |ui| app.show(ui)));
    }
    urls
}

fn wide_table() -> String {
    let cols = 12;
    let head: Vec<String> = (0..cols).map(|i| format!("column-heading-{i}")).collect();
    let row: Vec<String> = (0..cols)
        .map(|i| format!("a fairly long table cell value {i}"))
        .collect();
    format!(
        "| {} |\n|{}\n| {} |\n| {} |\n",
        head.join(" | "),
        "---|".repeat(cols),
        row.join(" | "),
        row.join(" | ")
    )
}

fn long_card() -> String {
    let mut md = format!(
        "# Title of the model\n\nIntro with a [safe link](https://example.com/docs) and `code` and **bold**.\n\n{}\n\n{}\n\n```\n{}\n```\n\n> quoted text that is quite long and keeps going for a while so it wraps\n\n- one\n  - nested\n- two\n\n",
        "x".repeat(300),
        wide_table(),
        "let very_long_line = 1;".repeat(30),
    );
    for i in 0..30 {
        md.push_str(&format!(
            "Paragraph number {i} with several ordinary words in it.\n\n"
        ));
    }
    md
}

/// Scrolls the detail pane with the mouse wheel and lets it settle.
fn scroll_by(app: &mut App, h: &mut Harness, size: (f32, f32), dy: f32) {
    let wheel = vec![
        egui::Event::PointerMoved(Pos2::new(700.0, 400.0)),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        },
    ];
    h.frame(vec2(size.0, size.1), wheel, |ui| app.show(ui));
    run(app, h, size, 12);
}

fn pane_viewport(app: &mut App) -> Rect {
    app.state_mut().browse.probe.viewport.expect("viewport")
}

// ---- chips ----

#[test]
fn chips_start_with_pipeline_tag_and_languages_then_tags() {
    let c = chips(&info());
    assert_eq!(c, ["text-generation", "en", "de", "gguf", "llama"]);
}

#[test]
fn chips_drop_noisy_tags_and_cap_plain_tags_at_six() {
    let mut i = info();
    i.tags = [
        "region:us",
        "license:mit",
        "endpoints_compatible",
        "autotrain_compatible",
        "text-generation-inference",
        "a1",
        "a2",
        "a3",
        "a4",
        "a5",
        "a6",
        "a7",
        "a8",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let c = chips(&i);
    assert_eq!(&c[3..], ["a1", "a2", "a3", "a4", "a5", "a6"]);
    for noisy in ["region:us", "license:mit", "endpoints_compatible"] {
        assert!(!c.iter().any(|x| x == noisy), "{noisy}");
    }
}

#[test]
fn chips_cap_many_languages_with_a_count() {
    let mut i = info();
    i.pipeline_tag = None;
    i.tags.clear();
    i.languages = (0..40).map(|n| format!("l{n}")).collect();
    let c = chips(&i);
    assert_eq!(c.len(), MAX_LANGUAGE_CHIPS + 1);
    assert_eq!(c.last().map(String::as_str), Some("+35"));
}

#[test]
fn chips_do_not_repeat_the_pipeline_tag() {
    let mut i = info();
    i.tags = vec!["text-generation".into(), "gguf".into()];
    assert_eq!(chips(&i), ["text-generation", "en", "de", "gguf"]);
}

// ---- switch and states ----

/// Equal to within a pixel (layout settles in sub-pixel steps).
fn same_place(a: Rect, b: Rect) -> bool {
    (a.min - b.min).abs().max_elem() < 1.0 && (a.max - b.max).abs().max_elem() < 1.0
}

#[test]
fn switch_is_pinned_above_the_scroll_area_and_never_moves() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = about_app(Some(card(&long_card())));
            app.state_mut().browse.set_card_expanded(true);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 10);
            let first = app.state_mut().browse.probe.switch.expect("switch");
            let view = pane_viewport(&mut app);
            assert!(
                first.height() < 40.0 && first.width() > first.height() * 2.0,
                "{size:?} dark={dark}: tabs are not one row: {first:?}"
            );
            assert!(
                first.bottom() <= view.top() + 0.5,
                "{size:?} dark={dark}: switch {first:?} overlaps the scroll area {view:?}"
            );
            scroll_by(&mut app, &mut h, size, -500.0);
            let scrolled = app.state_mut().browse.probe.switch.expect("switch");
            assert!(
                same_place(first, scrolled),
                "{size:?} dark={dark}: moved on scroll"
            );
            app.state_mut().browse.set_tab(DetailTab::Files);
            run(&mut app, &mut h, size, 6);
            let files = app.state_mut().browse.probe.switch.expect("switch");
            assert!(
                same_place(first, files),
                "{size:?} dark={dark}: moved on tab change"
            );
        }
    }
}

#[test]
fn clicking_the_pinned_switch_changes_tab_without_moving_it() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = new_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 8);
            let sw = app.state_mut().browse.probe.switch.expect("switch");
            let about = Pos2::new(sw.right() - 20.0, sw.center().y);
            click(&mut app, &mut h, size, about);
            assert_eq!(app.state_mut().browse.tab(), DetailTab::About);
            let after = app.state_mut().browse.probe.switch.expect("switch");
            assert!(
                same_place(sw, after),
                "{size:?} dark={dark}: {sw:?} {after:?}"
            );
            let files = Pos2::new(sw.left() + 20.0, sw.center().y);
            click(&mut app, &mut h, size, files);
            assert_eq!(app.state_mut().browse.tab(), DetailTab::Files);
        }
    }
}

#[test]
fn about_shows_loading_empty_error_and_card_states() {
    for dark in [false, true] {
        let size = SIZES[0];
        let mut h = Harness::new(dark);
        let mut app = about_app(None);
        run(&mut app, &mut h, size, 6);
        assert_eq!(app.state_mut().browse.probe.about_state, Some("loading"));
        app.state_mut().browse.on_model_card(ID, Ok(None));
        run(&mut app, &mut h, size, 4);
        assert_eq!(app.state_mut().browse.probe.about_state, Some("empty"));
        app.state_mut()
            .browse
            .on_model_card(ID, Err("network error: down".into()));
        run(&mut app, &mut h, size, 4);
        assert_eq!(app.state_mut().browse.probe.about_state, Some("error"));
        app.state_mut()
            .browse
            .on_model_card(ID, card("# Hi\n\ntext"));
        run(&mut app, &mut h, size, 4);
        assert_eq!(app.state_mut().browse.probe.about_state, Some("card"));
    }
}

#[test]
fn retry_button_asks_for_the_card_again() {
    for dark in [false, true] {
        let size = SIZES[0];
        let mut h = Harness::new(dark);
        let mut app = about_app(Some(Err("network error: down".into())));
        run(&mut app, &mut h, size, 6);
        let retry = app.state_mut().browse.probe.about_retry.expect("retry");
        let view = pane_viewport(&mut app);
        assert!(
            view.contains_rect(retry),
            "retry {retry:?} outside {view:?}"
        );
        click(&mut app, &mut h, size, retry.center());
        let st = app.state_mut();
        assert!(
            !matches!(st.browse.card_for(ID), Some(Err(_))),
            "the error was not cleared"
        );
        assert!(st.browse.card_pending(ID) || st.browse.card_for(ID).is_some());
    }
}

// ---- layout ----

#[test]
fn about_content_stays_inside_the_scroll_viewport() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = about_app(Some(card(&long_card())));
            app.state_mut().browse.set_card_expanded(true);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 10);
            let view = pane_viewport(&mut app);
            let p = &app.state_mut().browse.probe;
            assert!(
                !p.about_rects.is_empty(),
                "{size:?} dark={dark}: nothing drawn"
            );
            for r in &p.about_rects {
                assert!(
                    r.left() >= view.left() - 0.5 && r.right() <= view.right() + 0.5,
                    "{size:?} dark={dark}: {r:?} sticks out of {view:?}"
                );
            }
        }
    }
}

#[test]
fn about_wide_table_scrolls_inside_its_own_viewport() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = about_app(Some(card(&long_card())));
            app.state_mut().browse.set_card_expanded(true);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 10);
            let view = pane_viewport(&mut app);
            let tables = app.state_mut().browse.probe.about_tables.clone();
            assert_eq!(tables.len(), 1, "{size:?} dark={dark}");
            let (vp, content_w) = tables[0];
            assert!(
                vp.right() <= view.right() + 0.5,
                "{size:?} dark={dark}: table viewport {vp:?} wider than {view:?}"
            );
            assert!(
                content_w > vp.width(),
                "{size:?} dark={dark}: the table should be wider ({content_w}) than {vp:?}"
            );
        }
    }
}

#[test]
fn about_keeps_the_pinned_card_and_the_scroll_minimum() {
    for size in SIZES {
        for dark in [false, true] {
            let mut h = Harness::new(dark);
            let mut files = new_app();
            run(&mut files, &mut h, size, 10);
            let files_card = files.state_mut().browse.probe.card_panel.expect("card");
            let mut h = Harness::new(dark);
            let mut app = about_app(Some(card(&long_card())));
            run(&mut app, &mut h, size, 10);
            let p = &app.state_mut().browse.probe;
            assert_eq!(p.card_panel, Some(files_card), "{size:?} dark={dark}");
            let view = p.viewport.expect("viewport");
            assert!(
                view.height() >= super::detail::MIN_SCROLL_H - 0.5,
                "{size:?} dark={dark}: scroll area {}",
                view.height()
            );
        }
    }
}

#[test]
fn recommended_row_scroll_request_applies_only_to_the_files_tab() {
    let size = SIZES[0];
    let mut h = Harness::new(false);
    let mut app = new_app();
    app.state_mut().browse.set_tab(DetailTab::About);
    run(&mut app, &mut h, size, 8);
    assert!(
        app.state_mut().browse.scroll_request_pending(),
        "the scroll request was used up on the About tab"
    );
}

// ---- links ----

const LINKS: &str = "See [the docs](https://example.com/docs) and [bad](javascript:alert(1)) and [rel](other.md).\n\nMore text.";

#[test]
fn link_rects_are_inside_the_viewport() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = about_app(Some(card(LINKS)));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 8);
            let view = pane_viewport(&mut app);
            let links: Vec<_> = app
                .state_mut()
                .browse
                .probe
                .about_links
                .iter()
                .filter(|(_, u)| !u.starts_with("https://huggingface.co"))
                .cloned()
                .collect();
            assert_eq!(links.len(), 1, "only the https link is a link: {links:?}");
            let r = links[0].0;
            assert!(
                r.left() >= view.left() - 0.5 && r.right() <= view.right() + 0.5,
                "{r:?} outside {view:?}"
            );
        }
    }
}

#[test]
fn a_safe_link_opens_only_when_clicked() {
    let size = SIZES[0];
    let mut app = about_app(Some(card(LINKS)));
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 8);
    let (rect, url) = app.state_mut().browse.probe.about_links[0].clone();
    assert_eq!(url, "https://example.com/docs");
    let mut urls = Vec::new();
    for _ in 0..3 {
        urls.extend(h.frame_urls(vec2(size.0, size.1), vec![], |ui| app.show(ui)));
    }
    assert!(urls.is_empty(), "opened without a click: {urls:?}");
    let urls = click(&mut app, &mut h, size, rect.center());
    assert_eq!(urls, vec!["https://example.com/docs".to_string()]);
}

// ---- preview ----

#[test]
fn show_more_and_show_less_toggle_the_card() {
    let size = SIZES[0];
    let mut md = String::from("# T\n\n");
    for i in 0..40 {
        md.push_str(&format!("Paragraph {i}.\n\n"));
    }
    let mut app = about_app(Some(card(&md)));
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 8);
    let collapsed = app.state_mut().browse.probe.about_rects.len();
    scroll_by(&mut app, &mut h, size, -250.0);
    let toggle = app
        .state_mut()
        .browse
        .probe
        .about_toggle
        .expect("show more");
    let view = pane_viewport(&mut app);
    assert!(
        view.contains_rect(toggle),
        "toggle {toggle:?} outside {view:?}"
    );
    click(&mut app, &mut h, size, toggle.center());
    assert!(app.state_mut().browse.card_expanded());
    let expanded = app.state_mut().browse.probe.about_rects.len();
    assert!(expanded > collapsed, "{expanded} <= {collapsed}");
    // Scroll to the end and press Show less.
    scroll_by(&mut app, &mut h, size, -5000.0);
    let toggle = app
        .state_mut()
        .browse
        .probe
        .about_toggle
        .expect("show less");
    click(&mut app, &mut h, size, toggle.center());
    assert!(!app.state_mut().browse.card_expanded());
}

#[test]
fn truncated_cards_say_so_and_link_to_huggingface() {
    let size = SIZES[0];
    let mut app = about_app(Some(Ok(Some(ModelCard {
        markdown: "hello".into(),
        truncated: true,
    }))));
    let mut h = Harness::new(false);
    run(&mut app, &mut h, size, 8);
    let p = &app.state_mut().browse.probe;
    assert!(p.about_notice.is_some(), "no truncation notice");
    assert!(p
        .about_links
        .iter()
        .any(|(_, u)| u == "https://huggingface.co/o/repo"));
}

// ---- hostile cards ----

#[test]
fn hostile_readme_renders_inert_and_inside_the_viewport() {
    let hostile = format!(
        "<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n\n[x](javascript:alert(1))\n\n![t](http://tracker/pixel.png)\n\n{} deep\n\n[file](file:///etc/passwd) [data](data:text/html;base64,AAAA)\n",
        ">".repeat(10_000)
    );
    for size in SIZES {
        for dark in [false, true] {
            let mut app = about_app(Some(card(&hostile)));
            app.state_mut().browse.set_card_expanded(true);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 8);
            let view = pane_viewport(&mut app);
            let p = &app.state_mut().browse.probe;
            assert_eq!(p.about_state, Some("card"));
            assert!(
                p.about_links
                    .iter()
                    .all(|(_, u)| u == "https://huggingface.co/o/repo"),
                "{:?}",
                p.about_links
            );
            for r in &p.about_rects {
                assert!(r.right() <= view.right() + 0.5, "{r:?} outside {view:?}");
            }
        }
    }
}

#[test]
fn a_quarter_megabyte_line_renders_inside_the_viewport() {
    let size = SIZES[1];
    let mut app = about_app(Some(card(&"word ".repeat(51_000))));
    app.state_mut().browse.set_card_expanded(true);
    let mut h = Harness::new(true);
    run(&mut app, &mut h, size, 4);
    let view = pane_viewport(&mut app);
    let p = &app.state_mut().browse.probe;
    assert!(!p.about_rects.is_empty());
    for r in &p.about_rects {
        assert!(r.right() <= view.right() + 0.5, "{r:?} outside {view:?}");
    }
}

#[test]
fn expanded_hostile_documents_render_quickly_with_few_widgets() {
    use super::markdown::MAX_DOC_ITEMS;
    let docs = [
        format!("|a|b|\n|-|-|\n{}", "|||\n".repeat(4_000)),
        "*a* ".repeat(12_000),
        (0..3_000)
            .map(|i| format!("*p* {i}\n\n"))
            .collect::<String>(),
    ];
    for md in docs {
        let mut app = about_app(Some(card(&md)));
        app.state_mut().browse.set_card_expanded(true);
        let mut h = Harness::new(false);
        let start = std::time::Instant::now();
        run(&mut app, &mut h, SIZES[0], 3);
        let took = start.elapsed();
        assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
        let n = app.state_mut().browse.probe.about_rects.len();
        assert!(n <= MAX_DOC_ITEMS, "{n} widgets");
        assert!(app.state_mut().browse.probe.about_notice.is_some());
    }
}

#[test]
fn an_expanded_empty_list_item_bomb_renders_quickly_with_few_widgets() {
    use super::markdown::MAX_DOC_ITEMS;
    let mut app = about_app(Some(card(&"-\n".repeat(6_000))));
    app.state_mut().browse.set_card_expanded(true);
    let mut h = Harness::new(false);
    let start = std::time::Instant::now();
    run(&mut app, &mut h, SIZES[0], 3);
    let took = start.elapsed();
    assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
    let p = &app.state_mut().browse.probe;
    assert!(
        p.about_rects.len() <= MAX_DOC_ITEMS,
        "{} widgets",
        p.about_rects.len()
    );
    assert!(p.about_notice.is_some());
}
