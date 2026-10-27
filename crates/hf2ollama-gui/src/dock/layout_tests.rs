//! Headless layout tests: real painted rectangles at the two window sizes
//! the layout is designed for, in both themes.

use super::*;
use crate::app::App;
use crate::browse::detail::MIN_SCROLL_H;
use crate::browse::test_util::Harness;
use eframe::egui::{Pos2, Rect};
use hf2ollama_core::config::Settings;
use hf2ollama_core::events::{JobEvent, Stage};
use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::ModelInfo;
use hf2ollama_core::model::RepoFile;
use std::time::Instant;

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

fn rf(path: &str, size: u64) -> RepoFile {
    RepoFile {
        path: path.into(),
        size,
        sha256: Some(format!("sha-{path}")),
        blob_id: None,
    }
}

/// An app with a selected repository (the tallest Browse content: a gated
/// model with many options) and Ollama reachable.
fn browse_app() -> App {
    browse_app_of(false)
}

/// `safetensors`: the longest import card (conversion chips, Ollama unreachable).
fn browse_app_of(safetensors: bool) -> App {
    browse_app_chan(safetensors).0
}

/// Like `browse_app_of`, also returning what the UI sends to the backend.
fn browse_app_chan(
    safetensors: bool,
) -> (
    App,
    std::sync::mpsc::Receiver<crate::runtime::BackendRequest>,
) {
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
    st.ollama = Some(Some("0.9".into()));
    let id = "o/repo";
    let mut files: Vec<RepoFile> = (1..=25)
        .map(|i| {
            rf(
                &format!("m-IQ{i}_XS.gguf"),
                10_000_000_000 + i as u64 * 100_000_000,
            )
        })
        .collect();
    files.push(rf("m-Q4_K_M.gguf", 4_680_000_000));
    if safetensors {
        st.ollama = Some(None);
        st.browse.on_model_info(
            id,
            Ok(ModelInfo {
                id: id.into(),
                files: vec![
                    rf("config.json", 600),
                    rf("tokenizer.json", 3_000_000),
                    rf("model.safetensors", 14_000_000_000),
                ],
                arch: Some("MistralForCausalLM".into()),
                params: Some("7B".into()),
                context: Some(32768),
                license: Some("apache-2.0".into()),
                base_model: Some("mistralai/Mistral-7B-v0.3".into()),
                downloads: 1_000,
                likes: 10,
                updated: "2026-01-01T00:00:00.000Z".into(),
                gated: false,
                pipeline_tag: None,
                languages: Vec::new(),
                tags: Vec::new(),
            }),
        );
        st.browse.select_repo(id);
        return (app, sent);
    }
    st.browse.on_model_info(
        id,
        Ok(ModelInfo {
            id: id.into(),
            files,
            arch: Some("llama".into()),
            params: Some("7B".into()),
            context: Some(32768),
            license: Some("apache-2.0".into()),
            base_model: Some("mistralai/Mistral-7B-v0.3".into()),
            downloads: 1_000,
            likes: 10,
            updated: "2026-01-01T00:00:00.000Z".into(),
            gated: true,
            pipeline_tag: None,
            languages: Vec::new(),
            tags: Vec::new(),
        }),
    );
    st.browse.select_repo(id);
    (app, sent)
}

#[derive(Clone, Copy)]
enum Kind {
    Running,
    Paused,
    Queued,
    Finished,
    Failed,
    Cancelled,
}

/// Adds a job in the given state; `lines` log lines are attached.
fn add_job(app: &mut App, n: u64, kind: Kind, lines: usize) {
    let st = app.state_mut();
    let id = JobId(n);
    let now = Instant::now();
    st.jobs
        .accept(id, format!("bartowski/model-{n}:q4_k_m"), 4_680_000_000);
    let jobs = &mut st.jobs;
    jobs.apply(
        &JobEvent::Stage {
            id,
            stage: Stage::Queued,
        },
        now,
    );
    for i in 0..lines {
        jobs.apply(
            &JobEvent::Log {
                id,
                line: format!("12:00:{:02}  step {i}: HTTP 206 range resume ok", i % 60),
            },
            now,
        );
    }
    let downloading = |jobs: &mut crate::dock::jobs::JobsState| {
        jobs.apply(
            &JobEvent::Stage {
                id,
                stage: Stage::Downloading,
            },
            now,
        );
        jobs.apply(
            &JobEvent::Progress {
                id,
                done: 1_900_000_000,
                total: 4_680_000_000,
            },
            now,
        );
    };
    match kind {
        Kind::Queued => {}
        Kind::Running => downloading(jobs),
        Kind::Paused => {
            downloading(jobs);
            jobs.apply(&JobEvent::Paused { id }, now);
        }
        Kind::Finished => jobs.apply(&JobEvent::Finished { id }, now),
        Kind::Failed => jobs.apply(
            &JobEvent::Failed {
                id,
                error: "Ollama rejected the model: unsupported architecture".into(),
            },
            now,
        ),
        Kind::Cancelled => jobs.apply(&JobEvent::Cancelled { id }, now),
    }
}

fn laid_out(mut app: App, dark: bool, (w, h): (f32, f32)) -> App {
    let mut harness = Harness::new(dark);
    for _ in 0..10 {
        harness.frame(vec2(w, h), vec![], |ui| app.show(ui));
    }
    app
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.top() >= outer.top() - 0.5
        && inner.bottom() <= outer.bottom() + 0.5
        && inner.left() >= outer.left() - 0.5
        && inner.right() <= outer.right() + 0.5
}

fn many_jobs_app() -> App {
    many_jobs_on(browse_app())
}

fn many_jobs_on(mut app: App) -> App {
    let kinds = [
        Kind::Running,
        Kind::Paused,
        Kind::Queued,
        Kind::Finished,
        Kind::Failed,
        Kind::Cancelled,
        Kind::Running,
        Kind::Finished,
    ];
    for (i, k) in kinds.into_iter().enumerate() {
        add_job(&mut app, i as u64 + 1, k, 40);
        app.state_mut().dock.open_log = Some(JobId(i as u64 + 1));
    }
    app
}

fn one_log_app() -> App {
    let mut app = browse_app();
    add_job(&mut app, 1, Kind::Running, 40);
    app.state_mut().dock.open_log = Some(JobId(1));
    app
}

#[test]
fn log_drawer_shows_five_scrollable_lines_above_the_dock() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = laid_out(one_log_app(), dark, size);
            let probe = app.state_mut().dock.probe.clone();
            let panel = probe.panel.expect("dock panel");
            let drawer = probe.log_panel.expect("log drawer");
            let view = probe.log_viewport.expect("log viewport");
            let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
            assert!(
                inside(drawer, window),
                "{size:?} dark={dark}: log drawer {drawer:?} outside {window:?}"
            );
            assert!(
                drawer.bottom() <= panel.top() + 0.5,
                "{size:?} dark={dark}: log drawer {drawer:?} overlaps the dock {panel:?}"
            );
            assert!(
                view.height() >= 5.0 * 12.0,
                "{size:?} dark={dark}: log viewport only {} px, not five lines",
                view.height()
            );
            assert!(
                inside(view, drawer),
                "{size:?} dark={dark}: log text {view:?} outside the drawer {drawer:?}"
            );
            assert!(
                drawer.width() >= panel.width() - 1.0,
                "{size:?} dark={dark}: drawer {} px is narrower than the dock {} px",
                drawer.width(),
                panel.width()
            );
        }
    }
}

#[test]
fn opening_a_log_does_not_resize_the_dock() {
    for size in SIZES {
        for dark in [false, true] {
            let mut closed = browse_app();
            add_job(&mut closed, 1, Kind::Running, 40);
            let mut closed = laid_out(closed, dark, size);
            let without = closed.state_mut().dock.probe.panel.expect("dock panel");

            let mut with = laid_out(one_log_app(), dark, size);
            let with = with.state_mut().dock.probe.panel.expect("dock panel");

            assert!(
                (with.height() - without.height()).abs() <= 0.5,
                "{size:?} dark={dark}: dock grew from {} to {} px when the log opened",
                without.height(),
                with.height()
            );
        }
    }
}

#[test]
fn a_log_for_a_job_that_is_gone_closes_itself() {
    let mut app = browse_app();
    add_job(&mut app, 1, Kind::Running, 3);
    app.state_mut().dock.open_log = Some(JobId(99));
    let mut app = laid_out(app, false, SIZES[0]);
    assert_eq!(app.state_mut().dock.open_log, None);
}

#[test]
fn dock_height_is_bounded_with_many_jobs_and_open_logs() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = laid_out(many_jobs_app(), dark, size);
            let panel = app.state_mut().dock.probe.panel.expect("dock panel");
            let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
            assert!(
                inside(panel, window),
                "{size:?} dark={dark}: panel {panel:?} outside the window"
            );
            assert!(
                panel.height() <= 0.35 * size.1 + 0.5,
                "{size:?} dark={dark}: dock {} px is over 35% of {}",
                panel.height(),
                size.1
            );
            assert!(
                panel.height() <= dock_cap(size.1) + 0.5,
                "{size:?} dark={dark}: dock {} px over its cap {}",
                panel.height(),
                dock_cap(size.1)
            );
            let view = app.state_mut().dock.probe.viewport.expect("rows viewport");
            assert!(
                view.height() >= 2.0 * ROW_H,
                "{size:?} dark={dark}: rows viewport only {} px",
                view.height()
            );
            assert!(
                inside(view, panel),
                "{size:?} dark={dark}: viewport {view:?} outside {panel:?}"
            );
        }
    }
}

fn assert_buttons_visible(kinds: &[Kind], label: &str) {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = browse_app();
            for (i, k) in kinds.iter().enumerate() {
                add_job(&mut app, i as u64 + 1, *k, 0);
            }
            let mut app = laid_out(app, dark, size);
            let probe = app.state_mut().dock.probe.clone();
            let view = probe.viewport.expect("rows viewport");
            let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
            assert_eq!(probe.rows.len(), kinds.len(), "{label} {size:?}: rows");
            assert!(!probe.buttons.is_empty(), "{label}: no buttons painted");
            let clear = probe.clear_button.expect("Clear finished is shown");
            let panel = probe.panel.expect("dock panel");
            assert!(
                inside(clear, panel) && clear.height() >= 20.0,
                "{label} {size:?} dark={dark}: Clear finished {clear:?} outside {panel:?}"
            );
            for b in &probe.buttons {
                assert!(
                    inside(*b, view) && inside(*b, window) && inside(*b, panel),
                    "{label} {size:?} dark={dark}: button {b:?} clipped by {view:?}"
                );
                assert!(b.width() >= 40.0, "{label}: button {b:?} too narrow");
            }
            for (i, a) in probe.buttons.iter().enumerate() {
                for b in &probe.buttons[i + 1..] {
                    assert!(
                        !a.expand(-0.5).intersects(*b),
                        "{label} {size:?} dark={dark}: buttons overlap {a:?} {b:?}"
                    );
                }
            }
            // Every button belongs to a row and stays inside its width.
            for b in &probe.buttons {
                assert!(
                    probe
                        .rows
                        .iter()
                        .any(|r| b.left() >= r.left() && b.right() <= r.right() + 0.5),
                    "{label} {size:?} dark={dark}: button {b:?} outside every row"
                );
            }
        }
    }
}

#[test]
fn dock_buttons_are_fully_visible_for_running_and_settled_rows() {
    assert_buttons_visible(&[Kind::Running, Kind::Finished], "running+finished");
    assert_buttons_visible(&[Kind::Paused, Kind::Cancelled], "paused+cancelled");
    assert_buttons_visible(&[Kind::Queued, Kind::Failed], "queued+failed");
}

#[test]
fn idle_dock_is_a_single_compact_line() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = laid_out(browse_app(), dark, size);
            let p = &app.state_mut().dock.probe;
            let panel = p.panel.expect("dock panel");
            assert!(
                panel.height() <= 44.0,
                "{size:?} dark={dark}: idle dock {} px tall",
                panel.height()
            );
            assert!(p.buttons.is_empty() && p.viewport.is_none() && p.rows.is_empty());
        }
    }
}

#[test]
fn browse_scroll_area_keeps_min_height_with_dock_open() {
    for size in SIZES {
        for dark in [false, true] {
            for safetensors in [false, true] {
                let mut app = laid_out(many_jobs_on(browse_app_of(safetensors)), dark, size);
                let view = app.state_mut().browse.probe.viewport.expect("viewport");
                assert!(
                    view.height() >= MIN_SCROLL_H - 0.5,
                    "{size:?} dark={dark}: detail scroll area only {} px with the dock open",
                    view.height()
                );
                let card = app.state_mut().browse.probe.card_panel.expect("card panel");
                let window = Rect::from_min_size(Pos2::ZERO, vec2(size.0, size.1));
                assert!(inside(card, window), "{size:?} dark={dark}: card {card:?}");
            }
        }
    }
}

#[test]
fn filter_rail_stays_above_the_dock() {
    for size in SIZES {
        for dark in [false, true] {
            let mut app = laid_out(many_jobs_app(), dark, size);
            let rail = app.state_mut().browse.probe.rail.expect("filter rail");
            let dock = app.state_mut().dock.probe.panel.expect("dock panel");
            assert!(
                rail.bottom() <= dock.top() + 0.5,
                "{size:?} dark={dark}: filter rail {rail:?} reaches into the dock {dock:?}"
            );
        }
    }
}

/// Clicks `at` (press and release in separate frames) and returns the
/// backend requests the app sent as a result.
fn click_at(
    app: &mut App,
    sent: &std::sync::mpsc::Receiver<crate::runtime::BackendRequest>,
    harness: &mut Harness,
    size: (f32, f32),
    at: Pos2,
) -> Vec<crate::runtime::BackendRequest> {
    let (w, h) = size;
    let ev = |e| vec![e];
    harness.frame(vec2(w, h), ev(egui::Event::PointerMoved(at)), |ui| {
        app.show(ui)
    });
    for pressed in [true, false] {
        harness.frame(
            vec2(w, h),
            ev(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }),
            |ui| app.show(ui),
        );
    }
    // The dock grows upward and settles over a few frames.
    for _ in 0..40 {
        harness.frame(vec2(w, h), vec![], |ui| app.show(ui));
    }
    sent.try_iter().collect()
}

#[test]
fn dock_buttons_respond_while_a_log_is_open() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = browse_app_chan(false);
            add_job(&mut app, 1, Kind::Running, 0);
            add_job(&mut app, 2, Kind::Paused, 3);
            add_job(&mut app, 3, Kind::Queued, 0);
            app.state_mut().dock.open_log = Some(JobId(2));
            let mut harness = Harness::new(dark);
            for _ in 0..10 {
                harness.frame(vec2(size.0, size.1), vec![], |ui| app.show(ui));
            }
            // Row buttons are recorded top to bottom: the queued job (Cancel,
            // Log) comes first, so its Cancel is the first button.
            let probe = app.state_mut().dock.probe.clone();
            let cancel_queued = probe.buttons[1].center();
            let sent = click_at(&mut app, &sent, &mut harness, size, cancel_queued);
            assert!(
                sent.iter().any(|r| matches!(
                    r,
                    crate::runtime::BackendRequest::Core(hf2ollama_core::events::Command::Cancel(
                        JobId(3)
                    ))
                )),
                "{size:?} dark={dark}: clicking Cancel on the queued row sent {sent:?}"
            );
        }
    }
}

#[test]
fn dock_buttons_respond_after_a_log_was_opened_by_click() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = browse_app_chan(false);
            add_job(&mut app, 1, Kind::Running, 0);
            add_job(&mut app, 2, Kind::Paused, 3);
            add_job(&mut app, 3, Kind::Queued, 0);
            let mut harness = Harness::new(dark);
            for _ in 0..10 {
                harness.frame(vec2(size.0, size.1), vec![], |ui| app.show(ui));
            }
            // Recorded rightmost first: queued (Log, Cancel), paused (Log,
            // Cancel, Resume), running (Log, Cancel, Pause). Log of job 2 is #2.
            let probe = app.state_mut().dock.probe.clone();
            let log_of_2 = probe.buttons[2].center();
            click_at(&mut app, &sent, &mut harness, size, log_of_2);
            assert!(
                app.state_mut().dock.open_log == Some(JobId(2)),
                "{size:?} dark={dark}: the Log click did not open the log"
            );
            let probe = app.state_mut().dock.probe.clone();
            let cancel_queued = probe.buttons[1].center();
            let sent = click_at(&mut app, &sent, &mut harness, size, cancel_queued);
            assert!(
                sent.iter().any(|r| matches!(
                    r,
                    crate::runtime::BackendRequest::Core(hf2ollama_core::events::Command::Cancel(
                        JobId(3)
                    ))
                )),
                "{size:?} dark={dark}: Cancel after opening a log sent {sent:?}"
            );
        }
    }
}
