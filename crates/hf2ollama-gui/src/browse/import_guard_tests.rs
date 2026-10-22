//! The Download button must not start a second job under a name that is
//! already being imported, and a double click starts exactly one job.

use super::test_util::Harness;
use crate::app::App;
use crate::runtime::BackendRequest;
use eframe::egui::{self, vec2, Pos2};
use hf2ollama_core::config::Settings;
use hf2ollama_core::events::{Command, JobEvent, JobId, Stage};
use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::ModelInfo;
use hf2ollama_core::model::RepoFile;
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

const ID: &str = "o/repo";
const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];

fn info() -> ModelInfo {
    ModelInfo {
        id: ID.into(),
        files: vec![RepoFile {
            path: "m-Q4_K_M.gguf".into(),
            size: 4_680_000_000,
            sha256: Some("sha".into()),
            blob_id: None,
        }],
        arch: Some("llama".into()),
        params: Some("7B".into()),
        context: Some(32768),
        license: Some("apache-2.0".into()),
        base_model: None,
        downloads: 1,
        likes: 1,
        updated: "2026-01-01T00:00:00.000Z".into(),
        gated: false,
        pipeline_tag: Some("text-generation".into()),
        languages: vec![],
        tags: vec!["gguf".into(), "llama".into()],
    }
}

fn new_app() -> (App, Receiver<BackendRequest>) {
    let (tx, req_rx) = channel();
    let (_, rx) = channel();
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
    st.browse.on_model_info(ID, Ok(info()));
    st.browse.select_repo(ID);
    (app, req_rx)
}

fn run(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), frames: usize) {
    for _ in 0..frames {
        h.frame(vec2(w, hh), vec![], |ui| app.show(ui));
    }
}

fn button_center(app: &mut App) -> Pos2 {
    app.state_mut()
        .browse
        .probe
        .button
        .expect("the Download button was painted")
        .center()
}

fn press_release(app: &mut App, h: &mut Harness, (w, hh): (f32, f32), at: Pos2) {
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
}

fn starts(rx: &Receiver<BackendRequest>) -> Vec<String> {
    rx.try_iter()
        .filter_map(|r| match r {
            BackendRequest::Core(Command::StartImport(req)) => Some(req.model_name),
            _ => None,
        })
        .collect()
}

#[test]
fn import_button_is_blocked_while_an_unsettled_job_has_the_same_name() {
    let (mut app, _rx) = new_app();
    let st = app.state_mut();
    assert_eq!(st.import_block_reason(), None);
    let name = st.browse.name.clone();
    assert!(!name.is_empty());
    // Queued, running and paused all count.
    st.jobs.accept(JobId(1), name.clone(), 10);
    let why = st.import_block_reason().expect("blocked");
    assert_eq!(why, format!("A download for {name} is already running"));
    st.jobs.apply(
        &JobEvent::Stage {
            id: JobId(1),
            stage: Stage::Downloading,
        },
        Instant::now(),
    );
    assert!(st.import_block_reason().is_some());
    st.jobs
        .apply(&JobEvent::Paused { id: JobId(1) }, Instant::now());
    assert!(st.import_block_reason().is_some());
    assert!(st.jobs.has_unsettled_job_named(&name));
    assert!(!st.jobs.has_unsettled_job_named("other"));
    // `tiny` and `tiny:latest` are the same model.
    st.jobs.accept(JobId(2), "tiny".into(), 10);
    assert!(st.jobs.has_unsettled_job_named("tiny:latest"));
    assert!(st.jobs.has_unsettled_job_named("tiny"));
}

#[test]
fn settled_jobs_do_not_block_the_name() {
    for terminal in [
        JobEvent::Finished { id: JobId(1) },
        JobEvent::Failed {
            id: JobId(1),
            error: "x".into(),
        },
        JobEvent::Cancelled { id: JobId(1) },
    ] {
        let (mut app, _rx) = new_app();
        let st = app.state_mut();
        let name = st.browse.name.clone();
        st.jobs.accept(JobId(1), name.clone(), 10);
        assert!(st.import_block_reason().is_some());
        st.jobs.apply(&terminal, Instant::now());
        assert_eq!(st.import_block_reason(), None, "{terminal:?}");
        assert!(!st.jobs.has_unsettled_job_named(&name));
    }
}

#[test]
fn double_click_on_import_sends_exactly_one_start_import() {
    for dark in [false, true] {
        for size in SIZES {
            let (mut app, rx) = new_app();
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 4);
            let at = button_center(&mut app);
            press_release(&mut app, &mut h, size, at);
            // A second click in the very next frames, before JobAccepted arrives.
            press_release(&mut app, &mut h, size, at);
            run(&mut app, &mut h, size, 3);
            assert_eq!(starts(&rx).len(), 1, "dark={dark} size={size:?}");
        }
    }
}
