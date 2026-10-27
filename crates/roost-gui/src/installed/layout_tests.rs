//! Headless layout tests: real painted rectangles at the two window sizes
//! the layout is designed for, in both themes.

use super::state::{visible_columns, Column};
use super::RowProbe;
use crate::app::{App, View};
use crate::browse::test_util::Harness;
use crate::runtime::{BackendRequest, InstalledEntry};
use eframe::egui::{self, vec2, Pos2, Rect};
use roost_core::config::Settings;
use roost_core::events::{JobEvent, JobId, Stage};
use roost_core::fit::Machine;
use roost_core::ollama::InstalledModel;
use std::sync::mpsc::Receiver;
use std::time::Instant;

const SIZES: [(f32, f32); 2] = [(1180.0, 820.0), (900.0, 640.0)];
const LONG_NAME: &str =
    "registry.example.com/community/an-extremely-long-model-name-for-testing:70b-instruct-q4-k-m-extended";

fn entry(name: &str, size: u64, repo: Option<&str>, modified: &str) -> InstalledEntry {
    InstalledEntry {
        model: InstalledModel {
            name: name.into(),
            size,
            params: Some("7B".into()),
            quant: Some("Q4_K_M".into()),
            family: Some("llama".into()),
            modified: modified.into(),
            digest: format!("sha256:{:0<64}", name.len()),
        },
        source_repo: repo.map(str::to_string),
    }
}

fn entries(n: usize, with_long: bool) -> Vec<InstalledEntry> {
    let base = [
        (
            "qwen2.5-coder:7b-instruct-q4-k-m",
            4_680_000_000,
            Some("bartowski/Qwen2.5-Coder-7B-Instruct-GGUF"),
        ),
        ("llama3.2:3b", 2_020_000_000, None),
        ("mistral:7b", 4_110_000_000, None),
        (
            "phi3:mini",
            2_180_000_000,
            Some("someone/Phi-3-mini-4k-instruct-gguf"),
        ),
        ("gemma2:9b", 5_440_000_000, None),
        ("tiny:1b", 637_000_000, Some("o/tiny")),
        ("deepseek-r1:14b", 8_990_000_000, None),
        ("codellama:13b", 7_370_000_000, None),
        ("nomic-embed-text:latest", 274_000_000, None),
        ("neutral-a:8b", 4_900_000_000, Some("x/neutral-a")),
        ("neutral-b:2b", 1_500_000_000, None),
        ("neutral-c:30b", 18_000_000_000, Some("x/neutral-c")),
    ];
    let mut v: Vec<InstalledEntry> = base
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, (name, size, repo))| {
            entry(
                name,
                *size,
                *repo,
                &format!("2026-10-{:02}T09:30:00.123456789-08:00", 20 - i),
            )
        })
        .collect();
    if with_long {
        v.push(entry(
            LONG_NAME,
            3_000_000_000,
            Some("a-very-long-organisation-name/an-even-longer-repository-name-GGUF"),
            "2026-09-01T00:00:00Z",
        ));
    }
    v
}

fn installed_app(rows: Vec<InstalledEntry>) -> (App, Receiver<BackendRequest>) {
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
    st.view = View::Installed;
    st.installed.on_installed_loaded(&Ok(rows));
    (app, sent)
}

fn run(app: &mut App, harness: &mut Harness, (w, h): (f32, f32), frames: usize) {
    for _ in 0..frames {
        harness.frame(vec2(w, h), vec![], |ui| app.show(ui));
    }
}

fn click(app: &mut App, harness: &mut Harness, (w, h): (f32, f32), at: Pos2) {
    let ev = |e: egui::Event| vec![e];
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

fn rows(app: &mut App) -> Vec<RowProbe> {
    app.state_mut().installed_ui.probe.rows.clone()
}

fn cell(row: &RowProbe, c: Column) -> Option<Rect> {
    row.cells.iter().find(|(k, _)| *k == c).map(|(_, r)| *r)
}

#[test]
fn installed_table_is_inside_the_window_with_all_visible_columns() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _) = installed_app(entries(6, true));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let p = app.state_mut().installed_ui.probe.clone();
            let panel = p.panel.expect("view painted");
            let view = p.viewport.expect("table viewport");
            let tag = format!("{size:?} dark={dark}");
            assert!(inside(panel, window(size)), "{tag}: panel {panel:?}");
            assert!(inside(view, panel), "{tag}: viewport {view:?} vs {panel:?}");
            assert_eq!(p.columns, visible_columns(view.width()), "{tag}");
            if size.0 >= 1180.0 {
                assert_eq!(p.columns.len(), 7, "{tag}: all columns at full width");
            } else {
                assert!(!p.columns.contains(&Column::Quant), "{tag}");
                assert!(!p.columns.contains(&Column::Modified), "{tag}");
                assert!(p.columns.contains(&Column::Model), "{tag}");
                assert!(p.columns.contains(&Column::Size), "{tag}");
                assert!(p.columns.contains(&Column::Actions), "{tag}");
            }
            assert_eq!(p.header_cells.len(), p.columns.len(), "{tag}");
            for (c, r) in &p.header_cells {
                assert!(
                    inside(*r, view.expand2(vec2(0.0, 40.0))),
                    "{tag}: header {c:?} {r:?}"
                );
            }
            assert!(!p.rows.is_empty(), "{tag}: no rows painted");
            for row in &p.rows {
                assert_eq!(row.cells.len(), p.columns.len(), "{tag}: {}", row.name);
                for (c, r) in &row.cells {
                    assert!(
                        r.left() >= view.left() - 0.5 && r.right() <= view.right() + 0.5,
                        "{tag}: {} {c:?} {r:?} leaves {view:?}",
                        row.name
                    );
                }
            }
        }
    }
}

#[test]
fn delete_and_details_buttons_are_fully_visible_in_every_row() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _) = installed_app(entries(6, false));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let view = app
                .state_mut()
                .installed_ui
                .probe
                .viewport
                .expect("viewport");
            let rs = rows(&mut app);
            assert_eq!(rs.len(), 6, "{size:?} dark={dark}: every row painted");
            for r in rs {
                let tag = format!("{size:?} dark={dark} {}", r.name);
                assert!(
                    inside(r.details_btn, view),
                    "{tag}: details {:?}",
                    r.details_btn
                );
                assert!(
                    inside(r.delete_btn, view),
                    "{tag}: delete {:?}",
                    r.delete_btn
                );
                assert!(
                    inside(r.details_btn, r.rect),
                    "{tag}: details {:?} vs row {:?}",
                    r.details_btn,
                    r.rect
                );
                assert!(inside(r.delete_btn, r.rect), "{tag}: delete vs row");
                assert!(
                    r.details_btn.right() < r.delete_btn.left(),
                    "{tag}: buttons overlap"
                );
                assert!(
                    r.delete_btn.width() >= 60.0 && r.details_btn.width() >= 40.0,
                    "{tag}"
                );
            }
        }
    }
}

fn same_geometry(tag: &str, before: &[RowProbe], after: &[RowProbe]) {
    assert_eq!(before.len(), after.len(), "{tag}");
    for (b, a) in before.iter().zip(after) {
        assert_eq!(b.name, a.name, "{tag}");
        assert!(
            (b.rect.top() - a.rect.top()).abs() < 0.5
                && (b.rect.height() - a.rect.height()).abs() < 0.5,
            "{tag}: row {} moved {:?} -> {:?}",
            b.name,
            b.rect,
            a.rect
        );
        assert!(
            (b.delete_btn.width() - a.delete_btn.width()).abs() < 0.5
                && (b.delete_btn.left() - a.delete_btn.left()).abs() < 0.5
                && (b.details_btn.left() - a.details_btn.left()).abs() < 0.5,
            "{tag}: buttons of {} resized {:?}/{:?} -> {:?}/{:?}",
            b.name,
            b.details_btn,
            b.delete_btn,
            a.details_btn,
            a.delete_btn
        );
    }
}

const PAST_DELAY: std::time::Duration = std::time::Duration::from_millis(700);

#[test]
fn confirm_button_state_keeps_the_row_height_stable() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = installed_app(entries(6, false));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let before = rows(&mut app);
            let target = before[2].clone();
            click(&mut app, &mut h, size, target.delete_btn.center());
            let tag = format!("{size:?} dark={dark}");
            assert!(
                app.state_mut()
                    .installed
                    .is_armed(&target.name, Instant::now()),
                "{tag}: first click arms"
            );
            assert!(sent.try_recv().is_err(), "{tag}: arming sends nothing");
            // Inside the delay (dimmed) and after it (red, enabled).
            same_geometry(&tag, &before, &rows(&mut app));
            std::thread::sleep(PAST_DELAY);
            run(&mut app, &mut h, size, 3);
            let ready = rows(&mut app);
            same_geometry(&tag, &before, &ready);
            // A deliberate second click confirms with exactly that name.
            click(&mut app, &mut h, size, ready[2].delete_btn.center());
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(
                matches!(reqs.as_slice(), [BackendRequest::DeleteInstalled(n)] if *n == target.name),
                "{tag}: {reqs:?}"
            );
        }
    }
}

#[test]
fn fast_double_click_via_real_pointer_events_does_not_delete() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = installed_app(entries(6, false));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let target = rows(&mut app)[1].clone();
            let at = target.delete_btn.center();
            click(&mut app, &mut h, size, at);
            std::thread::sleep(std::time::Duration::from_millis(100));
            click(&mut app, &mut h, size, at);
            let tag = format!("{size:?} dark={dark}");
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(reqs.is_empty(), "{tag}: a fast double click sent {reqs:?}");
            assert!(
                app.state_mut()
                    .installed
                    .is_armed(&target.name, Instant::now()),
                "{tag}: the row stays armed"
            );
        }
    }
}

#[test]
fn deliberate_second_click_after_delay_sends_delete() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = installed_app(entries(6, false));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let target = rows(&mut app)[1].clone();
            let at = target.delete_btn.center();
            click(&mut app, &mut h, size, at);
            std::thread::sleep(PAST_DELAY);
            click(&mut app, &mut h, size, at);
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(
                matches!(reqs.as_slice(), [BackendRequest::DeleteInstalled(n)] if *n == target.name),
                "{size:?} dark={dark}: {reqs:?}"
            );
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
fn held_enter_key_never_confirms() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, sent) = installed_app(entries(6, false));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let target = rows(&mut app)[1].clone();
            let id = target.delete_id;
            let (w, hh) = size;
            let tag = format!("{size:?} dark={dark}");
            // Focus the Delete button and press Enter: that arms the row.
            h.frame(vec2(w, hh), vec![], |ui| {
                app.show(ui);
                ui.memory_mut(|m| m.request_focus(id));
            });
            h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
            assert!(
                app.state_mut()
                    .installed
                    .is_armed(&target.name, Instant::now()),
                "{tag}: Enter on the focused button arms"
            );
            // Keep holding it past the delay: only repeats arrive.
            std::thread::sleep(PAST_DELAY);
            for _ in 0..3 {
                h.frame(vec2(w, hh), vec![key(true, true)], |ui| app.show(ui));
            }
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(reqs.is_empty(), "{tag}: a held key sent {reqs:?}");
            // Release, then a fresh press is a deliberate confirmation.
            h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
            h.frame(vec2(w, hh), vec![key(true, false)], |ui| app.show(ui));
            h.frame(vec2(w, hh), vec![key(false, false)], |ui| app.show(ui));
            let reqs: Vec<_> = sent.try_iter().collect();
            assert!(
                matches!(reqs.as_slice(), [BackendRequest::DeleteInstalled(n)] if *n == target.name),
                "{tag}: fresh Enter press: {reqs:?}"
            );
        }
    }
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
fn installed_table_keeps_usable_height_with_dock_open() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _) = installed_app(entries(12, false));
            add_running_jobs(&mut app, 6);
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 40);
            let view = app
                .state_mut()
                .installed_ui
                .probe
                .viewport
                .expect("viewport");
            let tag = format!("{size:?} dark={dark}");
            assert!(
                view.height() >= 3.0 * super::ROW_H,
                "{tag}: table viewport only {} px",
                view.height()
            );
            let panel = app.state_mut().installed_ui.probe.panel.expect("panel");
            assert!(inside(view, panel), "{tag}");
            assert!(inside(panel, window(size)), "{tag}");
        }
    }
}

#[test]
fn long_model_names_do_not_overlap_the_size_column() {
    for size in SIZES {
        for dark in [false, true] {
            let (mut app, _) = installed_app(entries(3, true));
            let mut h = Harness::new(dark);
            run(&mut app, &mut h, size, 6);
            let tag = format!("{size:?} dark={dark}");
            let long = rows(&mut app)
                .into_iter()
                .find(|r| r.name == LONG_NAME)
                .unwrap_or_else(|| panic!("{tag}: long row not painted"));
            let name = cell(&long, Column::Model).expect("model cell");
            let size_cell = cell(&long, Column::Size).expect("size cell");
            assert!(
                name.right() + 4.0 <= size_cell.left(),
                "{tag}: name {name:?} runs into size {size_cell:?}"
            );
            let mut prev_right = f32::MIN;
            for (c, r) in &long.cells {
                assert!(
                    r.left() >= prev_right - 0.5,
                    "{tag}: {c:?} {r:?} overlaps the previous cell"
                );
                prev_right = r.right();
            }
            let view = app
                .state_mut()
                .installed_ui
                .probe
                .viewport
                .expect("viewport");
            assert!(
                inside(long.rect, view),
                "{tag}: row {:?} leaves {view:?}",
                long.rect
            );
            assert!(inside(long.details_btn, long.rect), "{tag}");
            assert!(name.width() > 100.0, "{tag}: name collapsed: {name:?}");
        }
    }
}
