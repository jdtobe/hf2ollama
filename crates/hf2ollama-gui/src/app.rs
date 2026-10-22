//! Application shell: top bar, downloads dock, status bar and view bodies.

use crate::browse::state::BrowseState;
use crate::dock::jobs::JobsState;
use crate::dock::{self, DockAction, DockUi};
use crate::runtime::{BackendRequest, UiEvent};
use crate::theme::{mono, semibold, tokens};
use crate::widgets::segmented_nav;
use eframe::egui::{self, vec2, Color32, Margin, RichText, Sense, Stroke};
use hf2ollama_core::config::{AutoDismiss, Settings};
use hf2ollama_core::events::Command;
use hf2ollama_core::fit::Machine;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Browse,
    Installed,
    Settings,
}

impl View {
    const ALL: [View; 3] = [View::Browse, View::Installed, View::Settings];

    fn label(self) -> &'static str {
        match self {
            View::Browse => "Browse",
            View::Installed => "Installed",
            View::Settings => "Settings",
        }
    }
}

pub struct AppState {
    pub view: View,
    pub query: String,
    /// `None` until the first poll answers; then the version if reachable.
    pub ollama: Option<Option<String>>,
    pub installed_count: Option<usize>,
    pub ollama_host: String,
    pub staging_dir: PathBuf,
    pub delete_after_import: bool,
    /// Detected or overridden once at startup; NVML probing is not free.
    pub machine: Machine,
    pub browse: BrowseState,
    pub jobs: JobsState,
    pub dock: DockUi,
    /// Current auto-dismiss setting; applies to rows already settled.
    pub auto_dismiss: AutoDismiss,
    /// Where the status pill was last painted (for layout tests).
    pub pill_rect: Option<egui::Rect>,
    /// Where the search field was last painted; `None` when hidden for lack of room.
    pub search_rect: Option<egui::Rect>,
}

impl AppState {
    pub fn new() -> AppState {
        AppState {
            view: View::Browse,
            query: String::new(),
            ollama: None,
            installed_count: None,
            ollama_host: String::new(),
            staging_dir: PathBuf::new(),
            delete_after_import: true,
            machine: Machine {
                ram_gb: 0.0,
                vram_gb: None,
            },
            browse: BrowseState::new(Machine {
                ram_gb: 0.0,
                vram_gb: None,
            }),
            jobs: JobsState::new(),
            dock: DockUi::default(),
            auto_dismiss: AutoDismiss::Never,
            pill_rect: None,
            search_rect: None,
        }
    }

    pub fn select(&mut self, view: View) {
        self.view = view;
    }

    /// Folds a backend event into the state; may ask for a follow-up request.
    pub fn on_event(&mut self, ev: &UiEvent) -> Option<BackendRequest> {
        match ev {
            UiEvent::OllamaStatus(v) => {
                let became_reachable =
                    v.is_some() && self.ollama.as_ref().is_none_or(|o| o.is_none());
                self.ollama = Some(v.clone());
                became_reachable.then_some(BackendRequest::ListInstalled)
            }
            UiEvent::InstalledLoaded(Ok(models)) => {
                self.installed_count = Some(models.len());
                None
            }
            UiEvent::SearchDone(q, r) => {
                self.browse.on_search_done(q, r.clone());
                None
            }
            UiEvent::ModelInfoLoaded(id, r) => {
                self.browse.on_model_info(id, r.clone());
                None
            }
            // Installed and delete events belong to the views that are
            // built next.
            _ => None,
        }
    }
}

/// `http://localhost:11434/` becomes `localhost:11434`.
pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// Exact size of the status pill for a label of `text_width` (dot, gap, text, padding).
pub fn pill_size(text_width: f32) -> egui::Vec2 {
    vec2(2.0 * PILL_PAD_X + PILL_DOT + PILL_GAP + text_width, 24.0)
}

/// Space kept between the search field and the status pill.
const SEARCH_PILL_GAP: f32 = 16.0;
/// Narrowest useful search field; below this the field is hidden.
const SEARCH_MIN_W: f32 = 120.0;
const PILL_PAD_X: f32 = 10.0;
const PILL_DOT: f32 = 8.0;
const PILL_GAP: f32 = 6.0;

pub struct App {
    state: AppState,
    tx: Sender<BackendRequest>,
    rx: Receiver<UiEvent>,
}

impl App {
    pub fn new(
        settings: &Settings,
        staging_dir: PathBuf,
        machine: Machine,
        tx: Sender<BackendRequest>,
        rx: Receiver<UiEvent>,
    ) -> App {
        let mut state = AppState::new();
        state.ollama_host = host_of(&settings.ollama_url);
        state.staging_dir = staging_dir;
        state.delete_after_import = settings.delete_after_import;
        state.auto_dismiss = settings.auto_dismiss;
        state.browse = BrowseState::new(machine.clone());
        state.machine = machine;
        App { state, tx, rx }
    }
}

impl App {
    #[cfg(test)]
    pub(crate) fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// Screenshot helper: pre-fills the search box and selects a repository.
    pub fn preset(&mut self, query: Option<String>, select: Option<String>) {
        if let Some(q) = query {
            self.state.query = q;
        }
        if let Some(id) = select {
            self.state.browse.select_repo(&id);
        }
    }
}

/// Red for the "not reachable" dot; the spec palette has no error token.
pub(crate) fn bad(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0xf2, 0x76, 0x6b)
    } else {
        Color32::from_rgb(0xb3, 0x26, 0x1e)
    }
}

pub(crate) fn panel_frame(fill: Color32, line: Color32, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(margin)
        .stroke(Stroke::new(1.0, line))
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            if let Some(req) = self.state.on_event(&ev) {
                let _ = self.tx.send(req);
            }
        }
        // Wake up again only when a settled row is waiting for its deadline.
        let now = Instant::now();
        if let Some(due) = self.state.jobs.tick(now, self.state.auto_dismiss) {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl App {
    fn apply_dock_action(&mut self, action: DockAction) {
        let command = |c: Command| BackendRequest::Core(c);
        match action {
            DockAction::Pause(id) => {
                let _ = self.tx.send(command(Command::Pause(id)));
            }
            DockAction::Resume(id) => {
                let _ = self.tx.send(command(Command::Resume(id)));
            }
            DockAction::Cancel(id) => {
                let _ = self.tx.send(command(Command::Cancel(id)));
            }
            DockAction::Dismiss(id) => {
                self.state.jobs.dismiss(id);
                self.state.dock.open_logs.remove(&id);
            }
            DockAction::ClearFinished => self.state.jobs.clear_finished(),
        }
    }

    /// The whole window; separate from `eframe::App::ui` so tests can run it headless.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        let t = tokens(ui);
        let dark = ui.visuals().dark_mode;

        let (dot, label) = match &self.state.ollama {
            Some(Some(v)) => (t.ok, format!("Ollama {v} · {}", self.state.ollama_host)),
            Some(None) => (bad(dark), "Ollama not reachable".to_string()),
            None => (t.fg3, "Checking Ollama…".to_string()),
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, egui::FontId::proportional(12.0), t.fg2);
        let pill_w = pill_size(galley.size().x).x;

        egui::Panel::top("top")
            .frame(panel_frame(t.surface, t.line, Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                let row = egui::Layout::left_to_right(egui::Align::Center);
                ui.with_layout(row, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    ui.painter().rect_filled(r, 6.0, t.accent);
                    ui.painter()
                        .circle_stroke(r.center(), 5.0, Stroke::new(2.0, t.accent_fg));
                    ui.label(RichText::new("hf2ollama").font(semibold(16.0)));
                    ui.add_space(8.0);

                    let labels: Vec<&str> = View::ALL.iter().map(|v| v.label()).collect();
                    let badges: Vec<Option<String>> = View::ALL
                        .iter()
                        .map(|v| match v {
                            View::Installed => self.state.installed_count.map(|n| n.to_string()),
                            _ => None,
                        })
                        .collect();
                    let current = View::ALL
                        .iter()
                        .position(|v| *v == self.state.view)
                        .unwrap_or(0);
                    if let Some(i) = segmented_nav(ui, &labels, &badges, current) {
                        self.state.select(View::ALL[i]);
                    }

                    if self.state.view == View::Browse {
                        ui.add_space(8.0);
                        // Whatever is left after the pill and a gap; hidden
                        // rather than overlapping when there is no room.
                        let room = ui.available_width() - pill_w - SEARCH_PILL_GAP - 8.0;
                        let field_w = room.min(360.0);
                        if field_w >= SEARCH_MIN_W {
                            let r = ui.add_sized(
                                vec2(field_w, 32.0),
                                egui::TextEdit::singleline(&mut self.state.query)
                                    .hint_text("Search Hugging Face models")
                                    .margin(Margin::symmetric(10, 7)),
                            );
                            self.state.search_rect = Some(r.rect);
                        } else {
                            self.state.search_rect = None;
                        }
                    } else {
                        self.state.search_rect = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Hand-painted at an exact size so no layout can stretch it.
                        let (rect, _) =
                            ui.allocate_exact_size(pill_size(galley.size().x), Sense::hover());
                        self.state.pill_rect = Some(rect);
                        let p = ui.painter();
                        p.rect_filled(rect, 12.0, t.surface2);
                        p.rect_stroke(
                            rect,
                            12.0,
                            Stroke::new(1.0, t.line),
                            egui::StrokeKind::Inside,
                        );
                        let cy = rect.center().y;
                        let dot_x = rect.min.x + PILL_PAD_X + PILL_DOT / 2.0;
                        p.circle_filled(egui::pos2(dot_x, cy), PILL_DOT / 2.0, dot);
                        let text_x = rect.min.x + PILL_PAD_X + PILL_DOT + PILL_GAP;
                        p.galley(
                            egui::pos2(text_x, cy - galley.size().y / 2.0),
                            galley,
                            t.fg2,
                        );
                    });
                });
            });

        // Free space of the staging folder is not exposed by the core, so the
        // status bar leaves it out. The Ollama store path is omitted until the
        // server reports it.
        egui::Panel::bottom("status")
            .frame(panel_frame(t.surface, t.line, Margin::symmetric(16, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new("Staging folder").size(12.0).color(t.fg3));
                    ui.label(
                        RichText::new(self.state.staging_dir.display().to_string())
                            .font(mono(11.5))
                            .color(t.fg2),
                    );
                    if self.state.delete_after_import {
                        ui.label(
                            RichText::new("(files deleted after each import)")
                                .size(12.0)
                                .color(t.fg3),
                        );
                    }
                });
            });

        let actions = dock::show(ui, &self.state.jobs, &mut self.state.dock);
        for action in actions {
            self.apply_dock_action(action);
        }

        if self.state.view == View::Browse {
            let mut requests = Vec::new();
            crate::browse::show(ui, &mut self.state, &mut requests);
            for r in requests {
                let _ = self.tx.send(r);
            }
        } else {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(t.surface)
                        .inner_margin(Margin::symmetric(24, 20)),
                )
                .show(ui, |ui| {
                    let text = match self.state.view {
                        View::Browse => "",
                        View::Installed => "Installed models coming next",
                        View::Settings => "Settings coming next",
                    };
                    ui.heading(self.state.view.label());
                    ui.label(RichText::new(text).color(t.fg3));
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hf2ollama_core::events::JobEvent;

    #[test]
    fn nav_selection_changes_view_state() {
        let mut s = AppState::new();
        assert_eq!(s.view, View::Browse);
        s.select(View::Installed);
        assert_eq!(s.view, View::Installed);
        s.select(View::Settings);
        assert_eq!(s.view, View::Settings);
        s.select(View::Browse);
        assert_eq!(s.view, View::Browse);
    }

    #[test]
    fn host_of_strips_scheme_and_path() {
        assert_eq!(host_of("http://localhost:11434"), "localhost:11434");
        assert_eq!(host_of("https://box.lan:8443/"), "box.lan:8443");
        assert_eq!(host_of("box:11434/api"), "box:11434");
    }

    fn headless_pill(dark: bool) -> (egui::Rect, f32, f32) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let mut app = App::new(
            &Settings::default(),
            PathBuf::from("/tmp/staging"),
            Machine {
                ram_gb: 16.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        app.state.ollama = Some(Some("0.9.2".to_string()));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1180.0, 820.0));
        for _ in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.show(ui))
                .drop_without_applying_deltas();
        }
        let label = "Ollama 0.9.2 · localhost:11434";
        let w = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                label.to_string(),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            )
            .size()
            .x
        });
        (
            app.state.pill_rect.expect("pill painted"),
            w,
            screen.right(),
        )
    }

    #[test]
    fn status_pill_sizing_is_deterministic() {
        for dark in [false, true] {
            let (rect, text_w, screen_right) = headless_pill(dark);
            let want = pill_size(text_w);
            assert!(
                (rect.width() - want.x).abs() < 1.5,
                "dark={dark} width {} vs {}",
                rect.width(),
                want.x
            );
            assert!((rect.height() - want.y).abs() < 1.5, "dark={dark} height");
            assert!(
                (rect.right() - (screen_right - 16.0)).abs() < 2.0,
                "dark={dark} right edge {}",
                rect.right()
            );
        }
    }

    fn run_app(dark: bool, width: f32) -> (Option<egui::Rect>, egui::Rect) {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let mut app = App::new(
            &Settings::default(),
            PathBuf::from("/tmp/staging"),
            Machine {
                ram_gb: 16.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        app.state.ollama = Some(None);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 820.0));
        for _ in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.show(ui))
                .drop_without_applying_deltas();
        }
        (
            app.state.search_rect,
            app.state.pill_rect.expect("pill painted"),
        )
    }

    #[test]
    fn search_field_never_overlaps_the_status_pill() {
        for dark in [false, true] {
            for width in [700.0, 900.0, 1180.0] {
                let (search, pill) = run_app(dark, width);
                if width >= 900.0 {
                    assert!(search.is_some(), "dark={dark} width={width}: field hidden");
                }
                if let Some(s) = search {
                    assert!(
                        s.right() + 8.0 <= pill.left(),
                        "dark={dark} width={width}: search ends {} pill starts {}",
                        s.right(),
                        pill.left()
                    );
                    assert!(s.width() >= 120.0, "dark={dark} width={width}");
                }
            }
        }
    }

    // ---- dock wiring ----

    fn app_with_channels() -> (
        App,
        std::sync::mpsc::Receiver<BackendRequest>,
        std::sync::mpsc::Sender<UiEvent>,
    ) {
        let (tx, req_rx) = std::sync::mpsc::channel();
        let (ev_tx, rx) = std::sync::mpsc::channel();
        let app = App::new(
            &Settings::default(),
            PathBuf::from("/tmp/staging"),
            Machine {
                ram_gb: 16.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        (app, req_rx, ev_tx)
    }

    #[test]
    fn job_events_fold_into_the_dock_state() {
        use crate::dock::jobs::RowStatus;
        use hf2ollama_core::events::{JobId, Stage};
        let mut s = AppState::new();
        let id = JobId(4);
        let accepted = UiEvent::JobAccepted {
            id,
            model_name: "qwen:q4".into(),
            total_bytes: 900,
        };
        assert!(s.on_event(&accepted).is_none());
        let stage = UiEvent::Job(JobEvent::Stage {
            id,
            stage: Stage::Downloading,
        });
        assert!(s.on_event(&stage).is_none());
        let row = &s.jobs.rows()[0];
        assert_eq!((row.name.as_str(), row.total), ("qwen:q4", 900));
        assert_eq!(row.status, RowStatus::Running);
    }

    #[test]
    fn finished_job_asks_the_backend_for_the_installed_list() {
        use hf2ollama_core::events::JobId;
        let mut s = AppState::new();
        let fin = UiEvent::Job(JobEvent::Finished { id: JobId(1) });
        assert!(matches!(
            s.on_event(&fin),
            Some(BackendRequest::ListInstalled)
        ));
        let failed = UiEvent::Job(JobEvent::Failed {
            id: JobId(2),
            error: "x".into(),
        });
        assert!(s.on_event(&failed).is_none());
    }

    #[test]
    fn dock_actions_send_core_commands_and_dismiss_locally() {
        use hf2ollama_core::events::{JobId, Stage};
        let (mut app, reqs, _) = app_with_channels();
        let id = JobId(9);
        let now = Instant::now();
        app.state.jobs.accept(id, "m:q4".into(), 10);
        app.state.jobs.apply(
            &JobEvent::Stage {
                id,
                stage: Stage::Downloading,
            },
            now,
        );
        app.state.dock.open_logs.insert(id);
        app.apply_dock_action(DockAction::Pause(id));
        app.apply_dock_action(DockAction::Resume(id));
        app.apply_dock_action(DockAction::Cancel(id));
        let sent: Vec<_> = reqs.try_iter().collect();
        assert!(matches!(
            sent.as_slice(),
            [
                BackendRequest::Core(Command::Pause(a)),
                BackendRequest::Core(Command::Resume(b)),
                BackendRequest::Core(Command::Cancel(c)),
            ] if *a == id && *b == id && *c == id
        ));
        assert_eq!(
            app.state.jobs.rows().len(),
            1,
            "commands do not remove rows"
        );
        app.apply_dock_action(DockAction::Dismiss(id));
        assert!(app.state.jobs.rows().is_empty());
        assert!(app.state.dock.open_logs.is_empty());
        assert!(reqs.try_recv().is_err(), "dismiss is local");
    }

    #[test]
    fn auto_dismiss_setting_comes_from_settings() {
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let settings = Settings {
            auto_dismiss: AutoDismiss::FiveMinutes,
            ..Settings::default()
        };
        let app = App::new(
            &settings,
            PathBuf::from("/tmp/staging"),
            Machine {
                ram_gb: 16.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        assert_eq!(app.state.auto_dismiss, AutoDismiss::FiveMinutes);
    }
}
