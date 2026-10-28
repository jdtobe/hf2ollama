//! Application shell: top bar, downloads dock, status bar and view bodies.

use crate::browse::state::BrowseState;
use crate::dock::jobs::JobsState;
use crate::dock::{self, DockAction, DockUi};
use crate::installed::state::{InstalledState, ListStatus};
use crate::runtime::{BackendRequest, UiEvent};
use crate::settings::{self, SettingsView};
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{ellipsize_middle, segmented_nav_rect};
use eframe::egui::{self, vec2, Color32, Margin, RichText, Sense, Stroke};
use roost_core::config::{AutoDismiss, Settings};
use roost_core::events::{Command, JobEvent};
use roost_core::fit::Machine;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chat,
    Browse,
    Installed,
    Settings,
}

impl View {
    pub(crate) const ALL: [View; 4] = [View::Chat, View::Browse, View::Installed, View::Settings];

    fn label(self) -> &'static str {
        match self {
            View::Chat => "Chat",
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
    pub installed: InstalledState,
    #[cfg(test)]
    pub installed_ui: crate::installed::InstalledUi,
    pub ollama_host: String,
    pub staging_dir: PathBuf,
    /// Where staging goes when the setting is empty.
    pub default_staging: PathBuf,
    pub delete_after_import: bool,
    /// The live settings; the Settings view edits them in place.
    pub settings: Settings,
    pub settings_view: SettingsView,
    /// Detected or overridden once at startup; NVML probing is not free.
    pub machine: Machine,
    pub browse: BrowseState,
    pub jobs: JobsState,
    pub chat: crate::chat::ChatState,
    pub chat_ui: crate::chat::view::ChatUi,
    pub dock: DockUi,
    /// Where the window-close confirmation stands.
    pub quit: crate::quit::QuitState,
    #[cfg(test)]
    pub quit_probe: crate::quit::QuitProbe,
    /// Current auto-dismiss setting; applies to rows already settled.
    pub auto_dismiss: AutoDismiss,
    /// Where the status pill was last painted (for layout tests).
    pub pill_rect: Option<egui::Rect>,
    /// Where the search field was last painted; `None` when hidden for lack of room.
    pub search_rect: Option<egui::Rect>,
    /// Where the clear (✕) button of the search field was last painted; `None` when the field is empty.
    pub search_clear_rect: Option<egui::Rect>,
    /// Extent of the status bar contents as last painted (for layout tests).
    pub status_rect: Option<egui::Rect>,
    /// One-line startup message (e.g. an ignored saved setting).
    pub startup_notice: Option<String>,
    /// Where each top bar item was last painted (for layout tests).
    #[cfg(test)]
    pub top_bar: Vec<(&'static str, egui::Rect)>,
}

impl AppState {
    /// Why the Download button is disabled, if it is: the browse reasons, then a
    /// job already importing under the same name.
    pub fn import_block_reason(&self) -> Option<String> {
        let reachable = self.ollama.as_ref().map(|o| o.is_some());
        if let Some(why) = self.browse.import_block_reason(reachable) {
            return Some(why);
        }
        let name = &self.browse.name;
        if self.jobs.has_unsettled_job_named(name) || self.browse.is_import_pending(name) {
            return Some(format!("A download for {name} is already running"));
        }
        None
    }

    pub fn new() -> AppState {
        AppState {
            view: View::Browse,
            query: String::new(),
            ollama: None,
            installed: InstalledState::new(),
            #[cfg(test)]
            installed_ui: Default::default(),
            ollama_host: String::new(),
            staging_dir: PathBuf::new(),
            default_staging: PathBuf::new(),
            delete_after_import: true,
            settings: Settings::default(),
            settings_view: SettingsView::new(&Settings::default(), PathBuf::new(), None),
            machine: Machine {
                ram_gb: 0.0,
                vram_gb: None,
            },
            browse: BrowseState::new(Machine {
                ram_gb: 0.0,
                vram_gb: None,
            }),
            jobs: JobsState::new(),
            chat: crate::chat::ChatState::default(),
            chat_ui: crate::chat::view::ChatUi::default(),
            dock: DockUi::default(),
            quit: crate::quit::QuitState::Idle,
            #[cfg(test)]
            quit_probe: Default::default(),
            auto_dismiss: AutoDismiss::Never,
            pill_rect: None,
            search_rect: None,
            search_clear_rect: None,
            status_rect: None,
            startup_notice: None,
            #[cfg(test)]
            top_bar: Vec::new(),
        }
    }

    pub fn select(&mut self, view: View) {
        self.view = view;
        if view == View::Settings {
            self.settings_view.state.on_opened();
        }
    }

    /// Applies what the user changed in the Settings view to the live app
    /// state and returns the requests to send (one `ApplySettings` that
    /// saves the file, only when something changed).
    pub fn apply_change(
        &mut self,
        change: settings::state::SettingsChange,
        ctx: &egui::Context,
    ) -> Vec<BackendRequest> {
        if !change.any() {
            return Vec::new();
        }
        let settings = &self.settings;
        if change.theme {
            crate::theme::apply(ctx, settings.theme_family, settings.theme_mode);
        }
        if change.auto_dismiss {
            self.auto_dismiss = settings.auto_dismiss;
        }
        if change.delete_after_import {
            self.delete_after_import = settings.delete_after_import;
        }
        if change.staging_dir {
            self.staging_dir = settings
                .staging_dir
                .clone()
                .unwrap_or_else(|| self.default_staging.clone());
        }
        if change.ollama_url {
            self.ollama_host = host_of(&settings.ollama_url);
            // Until the poll answers for the new address.
            self.ollama = None;
        }
        if change.hardware {
            // Unknown hardware stays unknown rather than guessed.
            let detected = self
                .settings_view
                .state
                .detected()
                .cloned()
                .unwrap_or(Machine {
                    ram_gb: 0.0,
                    vram_gb: None,
                });
            let machine = crate::settings::state::effective_machine(settings, &detected);
            self.machine = machine.clone();
            self.browse.set_machine(machine);
        }
        self.settings_view.state.note_change(change);
        vec![BackendRequest::ApplySettings(self.settings.clone())]
    }

    /// Folds a backend event into the state; may ask for a follow-up request.
    pub fn on_event(&mut self, ev: &UiEvent) -> Option<BackendRequest> {
        match ev {
            UiEvent::OllamaStatus(v) => {
                let became_reachable =
                    v.is_some() && self.ollama.as_ref().is_none_or(|o| o.is_none());
                self.ollama = Some(v.clone());
                if v.is_none() {
                    self.installed.on_unreachable();
                } else if became_reachable {
                    self.installed.begin_load();
                }
                became_reachable.then_some(BackendRequest::ListInstalled)
            }
            UiEvent::InstalledLoaded(r) => {
                self.installed.on_installed_loaded(r);
                None
            }
            UiEvent::InstalledDetails(name, r) => {
                self.installed.on_details(name, r);
                None
            }
            UiEvent::DeleteDone(name, r) => {
                // A deleted model changes what Ollama reports: read it again.
                // Also after an error: the model may already be gone
                // (Ollama 404) and the row would stay stale until Refresh.
                self.installed.on_delete_done(name, r, Instant::now());
                Some(BackendRequest::ListInstalled)
            }
            UiEvent::SearchDone(q, r) => {
                self.browse.on_search_done(q, r.clone());
                None
            }
            UiEvent::ModelInfoLoaded(id, r) => {
                self.browse.on_model_info(id, r.clone());
                None
            }
            UiEvent::ModelCardLoaded(id, r) => {
                self.browse.on_model_card(id, r.clone());
                None
            }
            UiEvent::JobAccepted {
                id,
                model_name,
                total_bytes,
            } => {
                self.jobs.accept(*id, model_name.clone(), *total_bytes);
                self.browse.clear_import_pending(model_name);
                None
            }
            UiEvent::Job(job) => {
                self.jobs.apply(job, Instant::now());
                // A finished import changes what Ollama has installed.
                matches!(job, JobEvent::Finished { .. }).then_some(BackendRequest::ListInstalled)
            }
            UiEvent::ConnectionTest(r) => {
                self.settings_view.state.on_connection_test(r);
                None
            }
            UiEvent::SettingsSaved(r) => {
                self.settings_view.state.on_saved(r, Instant::now());
                None
            }
            UiEvent::HfTokenStatus(status) => {
                self.settings_view.state.on_token_status(status);
                None
            }
            UiEvent::LegacyTokenImported(result) => {
                self.settings_view.state.on_legacy_token_imported(result);
                None
            }
            UiEvent::CachedLoaded(r) => {
                self.settings_view.state.on_cached_loaded(r);
                None
            }
            UiEvent::Chat(ev) => {
                self.chat.fold(ev.clone());
                None
            }
            UiEvent::ChatUnavailable(why) => {
                self.chat.recorded_unavailable(why.clone());
                if self.view == View::Chat {
                    self.select(View::Browse);
                }
                None
            }
            UiEvent::CachedCleared(r) => {
                self.settings_view
                    .state
                    .on_cached_cleared(r, Instant::now());
                None
            }
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

/// Height of the top bar's single row; the tallest item (the nav) fits.
const TOP_ROW_H: f32 = 36.0;
/// Height of the segmented nav: 28 px segments, 2 px padding, 1 px border.
const NAV_H: f32 = 34.0;

/// Space kept between the search field and the status pill.
const SEARCH_PILL_GAP: f32 = 16.0;
/// Narrowest useful search field; below this the field is hidden.
const SEARCH_MIN_W: f32 = 120.0;
/// Side of the clear (✕) button inside the search field.
const CLEAR_SIZE: f32 = 18.0;
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
        state.settings = settings.clone();
        if settings.staging_dir.is_none() {
            state.default_staging = state.staging_dir.clone();
        }
        let detected = settings
            .hardware_override
            .is_none()
            .then(|| machine.clone());
        state.settings_view = SettingsView::new(settings, state.default_staging.clone(), detected);
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

    /// What the Settings view needs besides the settings: where staging goes
    /// by default and what hardware was detected (before any override).
    pub fn set_environment(&mut self, default_staging: PathBuf, detected: Machine) {
        let state = &mut self.state;
        state.staging_dir = state
            .settings
            .staging_dir
            .clone()
            .unwrap_or_else(|| default_staging.clone());
        state.default_staging = default_staging.clone();
        state.settings_view = SettingsView::new(&state.settings, default_staging, Some(detected));
    }

    /// Something startup ignored or fixed; shown in the status bar.
    pub fn set_startup_notice(&mut self, notice: Option<String>) {
        self.state.startup_notice = notice;
    }

    /// Dev-hooks runs: the token store is in memory, say so in Settings.
    #[cfg(feature = "dev-hooks")]
    pub fn set_dev_memory_store(&mut self, on: bool) {
        self.state.settings_view.state.set_dev_memory_store(on);
    }

    /// Screenshot helper: starts on the given view.
    pub fn preset_view(&mut self, view: View) {
        self.state.view = view;
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
    /// Lets chat runs write their partial replies before the process ends.
    fn on_exit(&mut self) {
        let (ack, done) = std::sync::mpsc::channel();
        if self.tx.send(BackendRequest::ShutdownChat(ack)).is_ok() {
            let _ = done.recv_timeout(std::time::Duration::from_millis(3500));
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events();
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        for cmd in self.process_close(close_requested, Instant::now()) {
            ctx.send_viewport_cmd(cmd);
        }
        if matches!(self.state.quit, crate::quit::QuitState::Cancelling { .. }) {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
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
    /// Decides what the window should do about a close request and about a
    /// quit that is waiting for its jobs to cancel.
    pub(crate) fn process_close(
        &mut self,
        close_requested: bool,
        now: Instant,
    ) -> Vec<egui::ViewportCommand> {
        crate::quit::process_close(&mut self.state, close_requested, now)
    }

    /// Folds every waiting backend event into the state and sends the
    /// requests that follow from them.
    pub(crate) fn pump_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            if let Some(req) = self.state.on_event(&ev) {
                let _ = self.tx.send(req);
            }
        }
        self.flush_settings_requests();
    }

    /// Sends what the Settings view queued (test connection, token, cached files).
    fn flush_settings_requests(&mut self) {
        for req in self.state.settings_view.state.take_requests() {
            let _ = self.tx.send(req);
        }
    }

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
                if self.state.dock.open_log == Some(id) {
                    self.state.dock.open_log = None;
                }
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
                // A fixed-height row: items are centred on one line instead
                // of on the height of whatever was added before them.
                let row = egui::Layout::left_to_right(egui::Align::Center);
                let size = vec2(ui.available_width(), TOP_ROW_H);
                ui.allocate_ui_with_layout(size, row, |ui| {
                    #[cfg(test)]
                    self.state.top_bar.clear();
                    let _logo = crate::identity::paint_tail(ui, 22.0);
                    let _word =
                        ui.label(RichText::new(crate::identity::PRODUCT_NAME).font(semibold(16.0)));
                    #[cfg(test)]
                    {
                        self.state.top_bar.push(("logo", _logo));
                        self.state.top_bar.push(("wordmark", _word.rect));
                    }
                    ui.add_space(8.0);

                    let labels: Vec<&str> = View::ALL.iter().map(|v| v.label()).collect();
                    let badges: Vec<Option<String>> = View::ALL
                        .iter()
                        .map(|v| match v {
                            View::Installed => (self.state.installed.status == ListStatus::Loaded)
                                .then(|| self.state.installed.rows().len().to_string()),
                            View::Chat => (self.state.chat.active_runs() > 0)
                                .then(|| self.state.chat.active_runs().to_string()),
                            _ => None,
                        })
                        .collect();
                    let current = View::ALL
                        .iter()
                        .position(|v| *v == self.state.view)
                        .unwrap_or(0);
                    // A frame ignores the row's centring and sits at its top, so place it
                    // explicitly in the middle.
                    let left = ui.cursor().min.x;
                    let nav_area = egui::Rect::from_min_size(
                        egui::pos2(left, ui.max_rect().min.y + (TOP_ROW_H - NAV_H) / 2.0),
                        vec2(ui.available_width(), NAV_H),
                    );
                    let mut nav_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(nav_area)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    let (picked, _nav_rect) =
                        segmented_nav_rect(&mut nav_ui, &labels, &badges, current);
                    ui.advance_cursor_after_rect(_nav_rect);
                    #[cfg(test)]
                    self.state.top_bar.push(("nav", _nav_rect));
                    if let Some(i) = picked {
                        self.state.select(View::ALL[i]);
                    }

                    if self.state.view == View::Browse {
                        ui.add_space(8.0);
                        // Whatever is left after the pill and a gap; hidden
                        // rather than overlapping when there is no room.
                        let room = ui.available_width() - pill_w - SEARCH_PILL_GAP - 8.0;
                        let field_w = room.min(360.0);
                        if field_w >= SEARCH_MIN_W {
                            let has_text = !self.state.query.is_empty();
                            let r = ui.add_sized(
                                vec2(field_w, 32.0),
                                egui::TextEdit::singleline(&mut self.state.query)
                                    .hint_text("Search Hugging Face models")
                                    .margin(Margin {
                                        left: 10,
                                        right: if has_text { 10 + CLEAR_SIZE as i8 } else { 10 },
                                        top: 7,
                                        bottom: 7,
                                    }),
                            );
                            self.state.search_rect = Some(r.rect);
                            self.state.search_clear_rect = None;
                            if has_text {
                                let rect = egui::Rect::from_center_size(
                                    egui::pos2(
                                        r.rect.right() - 6.0 - CLEAR_SIZE / 2.0,
                                        r.rect.center().y,
                                    ),
                                    vec2(CLEAR_SIZE, CLEAR_SIZE),
                                );
                                self.state.search_clear_rect = Some(rect);
                                let hit = ui
                                    .interact(rect, r.id.with("clear"), Sense::click())
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("Clear search");
                                let fg = if hit.hovered() { t.fg } else { t.fg3 };
                                if hit.hovered() {
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        CLEAR_SIZE / 2.0,
                                        t.surface2,
                                    );
                                }
                                let c = rect.center();
                                let arm = 3.5;
                                let stroke = Stroke::new(1.5, fg);
                                ui.painter().line_segment(
                                    [c + vec2(-arm, -arm), c + vec2(arm, arm)],
                                    stroke,
                                );
                                ui.painter().line_segment(
                                    [c + vec2(-arm, arm), c + vec2(arm, -arm)],
                                    stroke,
                                );
                                if hit.clicked() {
                                    self.state.query.clear();
                                    r.request_focus();
                                }
                            }
                        } else {
                            self.state.search_rect = None;
                            self.state.search_clear_rect = None;
                        }
                    } else {
                        self.state.search_rect = None;
                        self.state.search_clear_rect = None;
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
                let row = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let gap = ui.spacing().item_spacing.x;
                    let path = self.state.staging_dir.display().to_string();
                    let measure = |text: &str, font: egui::FontId| {
                        ui.painter()
                            .layout_no_wrap(text.to_string(), font, t.fg3)
                            .size()
                            .x
                    };
                    let small = egui::FontId::proportional(12.0);
                    let notice = self.state.startup_notice.clone();
                    let note = notice
                        .as_deref()
                        .unwrap_or("(files deleted after each import)");
                    let label_w = measure("Staging folder", small.clone());
                    let path_w = measure(&path, mono(11.5));
                    let note_w = measure(note, small);
                    let room = ui.available_width() - label_w - gap;
                    // The note is the first thing to give way, then the path
                    // loses its middle (the end is the part people look for).
                    let show_note = (self.state.delete_after_import || notice.is_some())
                        && path_w + gap + note_w <= room;
                    let path_room = if show_note { path_w } else { room };
                    let shown = ellipsize_middle(ui, &path, mono(11.5), path_room);
                    ui.label(RichText::new("Staging folder").size(12.0).color(t.fg3));
                    let r = ui.label(RichText::new(&shown).font(mono(11.5)).color(t.fg2));
                    if shown != path {
                        r.on_hover_text(path);
                    }
                    if show_note {
                        let color = if notice.is_some() { t.warn } else { t.fg3 };
                        ui.label(RichText::new(note).size(12.0).color(color));
                    }
                });
                self.state.status_rect = Some(row.response.rect);
            });

        if self.state.view != View::Settings {
            settings::forget_token_field(ui.ctx(), &mut self.state.settings_view.state);
        }

        let actions = dock::show(ui, &self.state.jobs, &mut self.state.dock);
        for action in actions {
            self.apply_dock_action(action);
        }

        if self.state.view == View::Chat {
            let mut requests = Vec::new();
            crate::chat::view::show(ui, &mut self.state, &mut requests);
            for r in requests {
                let _ = self.tx.send(r);
            }
        } else if self.state.view == View::Browse {
            let mut requests = Vec::new();
            crate::browse::show(ui, &mut self.state, &mut requests);
            for r in requests {
                let _ = self.tx.send(r);
            }
        } else if self.state.view == View::Installed {
            let mut requests = Vec::new();
            crate::installed::show(ui, &mut self.state, &mut requests);
            for r in requests {
                let _ = self.tx.send(r);
            }
        } else {
            let mut requests = Vec::new();
            crate::settings::show(ui, &mut self.state, &mut requests);
            for r in requests {
                let _ = self.tx.send(r);
            }
        }
        // Whatever the nav click or a button queued in the Settings state.
        self.flush_settings_requests();
        crate::quit::show(ui, &mut self.state, &self.tx, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::HfTokenStatus;

    #[test]
    fn application_display_identity_is_roost() {
        assert_eq!(crate::identity::PRODUCT_NAME, "Roost");
    }

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

    #[test]
    fn search_field_has_a_clear_button_only_when_it_holds_text() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
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
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1180.0, 820.0));
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.show(ui))
                .drop_without_applying_deltas();
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert!(app.state.search_clear_rect.is_none(), "shown while empty");
        app.state.query = "llama".into();
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let clear = app.state.search_clear_rect.expect("clear button");
        let field = app.state.search_rect.expect("field");
        assert!(field.contains_rect(clear), "{clear:?} outside {field:?}");
        let at = clear.center();
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
            );
        }
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        assert!(app.state.query.is_empty(), "query is {:?}", app.state.query);
        assert!(app.state.search_clear_rect.is_none());
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
        use roost_core::events::{JobId, Stage};
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
        use roost_core::events::JobId;
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
        use roost_core::events::{JobId, Stage};
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
        app.state.dock.open_log = Some(id);
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
        assert!(app.state.dock.open_log.is_none());
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

    #[test]
    fn startup_notice_is_shown_in_the_status_bar_and_stays_inside_the_window() {
        let long = format!("/Users/someone/{}", "very-long-folder-name/".repeat(5));
        for (dark, path) in [(false, "/tmp/staging"), (true, long.as_str())] {
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
                PathBuf::from(path),
                Machine {
                    ram_gb: 16.0,
                    vram_gb: None,
                },
                tx,
                rx,
            );
            app.set_startup_notice(Some("ignored an invalid saved staging folder".into()));
            assert_eq!(
                app.state.startup_notice.as_deref(),
                Some("ignored an invalid saved staging folder")
            );
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 640.0));
            let mut painted = String::new();
            for _ in 0..4 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let out = ctx.run_ui(input, |ui| app.show(ui));
                painted = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::epaint::Shape::Text(t) => Some(t.galley.job.text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("|");
                out.drop_without_applying_deltas();
            }
            let r = app.state.status_rect.expect("status bar painted");
            assert!(
                r.right() <= 900.0 - 16.0 + 0.5,
                "dark={dark}: status bar content {r:?} leaves the window"
            );
            if !dark {
                assert!(
                    painted.contains("ignored an invalid saved staging folder"),
                    "notice not painted: {painted}"
                );
            }
        }
    }

    #[test]
    fn status_bar_path_is_truncated_inside_the_window() {
        let long = format!("/Users/someone/{}", "very-long-folder-name/".repeat(5));
        assert!(long.chars().count() >= 100);
        for dark in [false, true] {
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
                PathBuf::from(&long),
                Machine {
                    ram_gb: 16.0,
                    vram_gb: None,
                },
                tx,
                rx,
            );
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 640.0));
            for _ in 0..4 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                ctx.run_ui(input, |ui| app.show(ui))
                    .drop_without_applying_deltas();
            }
            let r = app.state.status_rect.expect("status bar painted");
            assert!(
                r.right() <= 900.0 - 16.0 + 0.5 && r.left() >= 16.0 - 0.5,
                "dark={dark}: status bar content {r:?} leaves the window"
            );
            assert!(r.width() > 300.0, "dark={dark}: path vanished: {r:?}");
        }
    }

    // ---- installed wiring ----

    fn entry(name: &str, size: u64, repo: Option<&str>) -> crate::runtime::InstalledEntry {
        crate::runtime::InstalledEntry {
            model: roost_core::ollama::InstalledModel {
                name: name.into(),
                size,
                params: None,
                quant: None,
                family: None,
                modified: "2026-10-20T09:30:00Z".into(),
                digest: "sha256:x".into(),
            },
            source_repo: repo.map(str::to_string),
        }
    }

    #[test]
    fn installed_events_fold_into_state_and_nav_badge() {
        let mut s = AppState::new();
        let loaded = UiEvent::InstalledLoaded(Ok(vec![
            entry("a:1", 10, None),
            entry("b:2", 20, Some("o/b")),
        ]));
        assert!(s.on_event(&loaded).is_none());
        assert_eq!(s.installed.rows().len(), 2);
        assert_eq!(s.installed.status, ListStatus::Loaded);
        let details = UiEvent::InstalledDetails(
            "a:1".into(),
            Ok(roost_core::ollama::ModelDetails {
                family: "llama".into(),
                context: None,
                capabilities: vec![],
                digest: "sha256:x".into(),
                template: None,
            }),
        );
        s.installed.toggle_expanded("a:1");
        assert!(s.on_event(&details).is_none());
        assert!(matches!(
            s.installed.details_of("a:1"),
            Some(crate::installed::state::DetailsState::Loaded(_))
        ));
    }

    #[test]
    fn backend_delete_installed_is_followed_by_list_refresh_request() {
        let mut s = AppState::new();
        s.on_event(&UiEvent::InstalledLoaded(Ok(vec![entry("a:1", 10, None)])));
        let ok = UiEvent::DeleteDone("a:1".into(), Ok(()));
        assert!(matches!(
            s.on_event(&ok),
            Some(BackendRequest::ListInstalled)
        ));
        assert!(s.installed.rows().is_empty());
    }

    #[test]
    fn delete_error_requests_list_refresh() {
        let mut s = AppState::new();
        s.on_event(&UiEvent::InstalledLoaded(Ok(vec![entry("a:1", 10, None)])));
        let failed = UiEvent::DeleteDone("a:1".into(), Err("model not found".into()));
        assert!(matches!(
            s.on_event(&failed),
            Some(BackendRequest::ListInstalled)
        ));
        assert_eq!(
            s.installed.rows().len(),
            1,
            "the row stays until the list says otherwise"
        );
        assert!(s
            .installed
            .toast(Instant::now())
            .is_some_and(|t| t.text.contains("model not found")));
    }

    #[test]
    fn unreachable_ollama_turns_a_pending_list_into_an_error() {
        let mut s = AppState::new();
        assert_eq!(s.installed.status, ListStatus::Loading);
        s.on_event(&UiEvent::OllamaStatus(None));
        assert!(matches!(s.installed.status, ListStatus::Error(_)));
        // Coming back asks for the list again and shows it as loading.
        let req = s.on_event(&UiEvent::OllamaStatus(Some("0.9".into())));
        assert!(matches!(req, Some(BackendRequest::ListInstalled)));
        assert_eq!(s.installed.status, ListStatus::Loading);
        // A list that was already loaded stays on screen while unreachable.
        s.on_event(&UiEvent::InstalledLoaded(Ok(vec![entry("a:1", 10, None)])));
        s.on_event(&UiEvent::OllamaStatus(None));
        assert_eq!(s.installed.status, ListStatus::Loaded);
    }

    // ---- settings wiring ----

    fn gguf_info(id: &str) -> roost_core::hf::ModelInfo {
        use roost_core::model::RepoFile;
        let file = |path: &str, gb: u64| RepoFile {
            path: path.into(),
            size: gb * 1_000_000_000,
            sha256: Some(format!("sha-{path}")),
            blob_id: None,
        };
        roost_core::hf::ModelInfo {
            id: id.into(),
            files: vec![
                file("m-Q8_0.gguf", 20),
                file("m-Q5_K_M.gguf", 12),
                file("m-Q2_K.gguf", 3),
            ],
            arch: Some("qwen2".into()),
            params: None,
            context: None,
            license: None,
            base_model: None,
            downloads: 0,
            likes: 0,
            updated: String::new(),
            gated: false,
            pipeline_tag: None,
            languages: Vec::new(),
            tags: Vec::new(),
        }
    }

    fn settings_state(detected: Option<Machine>) -> AppState {
        let mut s = AppState::new();
        s.settings_view = SettingsView::new(&s.settings, PathBuf::from("/tmp/default"), detected);
        s.default_staging = PathBuf::from("/tmp/default");
        s.staging_dir = PathBuf::from("/tmp/default");
        s
    }

    fn only_apply(reqs: Vec<BackendRequest>) -> Settings {
        match <[BackendRequest; 1]>::try_from(reqs) {
            Ok([BackendRequest::ApplySettings(s)]) => s,
            other => panic!("expected exactly one ApplySettings, got {other:?}"),
        }
    }

    #[test]
    fn auto_dismiss_change_reaches_the_dock_state() {
        use roost_core::events::{JobId, Stage};
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        let t0 = Instant::now();
        let id = JobId(1);
        s.jobs.accept(id, "m:q4".into(), 10);
        s.jobs.apply(
            &JobEvent::Stage {
                id,
                stage: Stage::Downloading,
            },
            t0,
        );
        s.jobs.apply(&JobEvent::Finished { id }, t0);
        let later = t0 + std::time::Duration::from_secs(61);
        s.jobs.tick(later, s.auto_dismiss);
        assert_eq!(s.jobs.rows().len(), 1, "Never keeps settled rows");
        let change = s
            .settings_view
            .state
            .set_auto_dismiss(&mut s.settings, AutoDismiss::OneMinute);
        only_apply(s.apply_change(change, &ctx));
        assert_eq!(s.auto_dismiss, AutoDismiss::OneMinute);
        s.jobs.tick(later, s.auto_dismiss);
        assert!(s.jobs.rows().is_empty(), "the already settled row follows");
    }

    #[test]
    fn hardware_override_recomputes_machine_and_fit() {
        use roost_core::fit::{fit, Fit};
        let ctx = egui::Context::default();
        let detected = Machine {
            ram_gb: 16.0,
            vram_gb: None,
        };
        let mut s = settings_state(Some(detected.clone()));
        s.machine = detected.clone();
        s.browse = BrowseState::new(detected.clone());
        let id = "o/m-GGUF";
        s.browse.on_model_info(id, Ok(gguf_info(id)));
        s.browse.select_repo(id);
        let recommended = |s: &AppState| {
            s.browse
                .plan()
                .and_then(|p| p.rows.iter().find(|r| r.recommended))
                .map(|r| r.label.clone())
        };
        assert_eq!(recommended(&s).as_deref(), Some("Q2_K"), "VRAM unknown");
        assert_eq!(fit(12.0, &s.machine), Fit::Unknown);

        let mut change = s.settings_view.state.set_override(&mut s.settings, true);
        s.settings_view.state.ram_text = "64".into();
        s.settings_view.state.vram_text = "24".into();
        change.merge(s.settings_view.state.commit_hardware(&mut s.settings));
        assert!(change.hardware);
        let applied = only_apply(s.apply_change(change, &ctx));
        assert_eq!(
            applied.hardware_override,
            Some(Machine {
                ram_gb: 64.0,
                vram_gb: Some(24.0)
            })
        );
        let want = Machine {
            ram_gb: 64.0,
            vram_gb: Some(24.0),
        };
        assert_eq!(s.machine, want);
        assert_eq!(s.browse.machine(), &want);
        assert_eq!(fit(12.0, &s.machine), Fit::Fits);
        assert_eq!(recommended(&s).as_deref(), Some("Q5_K_M"));

        // Turning the override off goes back to the detected machine.
        let change = s.settings_view.state.set_override(&mut s.settings, false);
        only_apply(s.apply_change(change, &ctx));
        assert_eq!(s.machine, detected);
        assert_eq!(s.browse.machine(), &detected);
        assert_eq!(recommended(&s).as_deref(), Some("Q2_K"));
    }

    #[test]
    fn delete_after_import_change_is_live_for_the_plan_card_and_status_bar() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        assert!(s.delete_after_import);
        let change = s
            .settings_view
            .state
            .set_delete_after_import(&mut s.settings, false);
        let applied = only_apply(s.apply_change(change, &ctx));
        assert!(
            !s.delete_after_import,
            "the plan card and status bar read this"
        );
        assert!(!applied.delete_after_import);
    }

    #[test]
    fn theme_change_applies_immediately() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        let change = s
            .settings_view
            .state
            .set_theme(&mut s.settings, roost_core::config::ThemeMode::Dark);
        only_apply(s.apply_change(change, &ctx));
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Dark
        );
    }

    #[test]
    fn theme_family_change_applies_immediately_and_is_saved() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut s = settings_state(None);
        let change = s
            .settings_view
            .state
            .set_theme_family(&mut s.settings, roost_core::config::ThemeFamily::Tide);
        let saved = only_apply(s.apply_change(change, &ctx));
        assert_eq!(
            crate::theme::family(&ctx),
            roost_core::config::ThemeFamily::Tide
        );
        assert_eq!(saved.theme_family, roost_core::config::ThemeFamily::Tide);
    }

    #[test]
    fn apply_change_sends_one_apply_settings_and_only_when_something_changed() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        assert!(s.apply_change(Default::default(), &ctx).is_empty());
        let mut change = s
            .settings_view
            .state
            .set_keep_partials(&mut s.settings, false);
        change.merge(s.settings_view.state.set_max_parallel(&mut s.settings, 3));
        let applied = only_apply(s.apply_change(change, &ctx));
        assert!(!applied.keep_partials);
        assert_eq!(applied.max_parallel, 3);
        assert_eq!(applied, s.settings);
        assert_eq!(
            s.settings_view.state.save,
            crate::settings::state::SaveIndicator::Saving
        );
    }

    #[test]
    fn invalid_values_never_reach_the_app_or_the_backend() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        let before = s.settings.clone();
        s.settings_view.state.ollama_text = "ftp://nope".into();
        let change = s.settings_view.state.commit_ollama(&mut s.settings);
        assert!(s.apply_change(change, &ctx).is_empty(), "nothing is sent");
        assert_eq!(s.settings, before);
        assert_eq!(s.ollama_host, "");
        assert_eq!(
            s.settings_view.state.save,
            crate::settings::state::SaveIndicator::Idle
        );
        assert!(s.settings_view.state.ollama_error.is_some());
    }

    #[test]
    fn ollama_address_change_updates_the_host_and_forgets_the_old_status() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        s.ollama = Some(Some("0.9.2".into()));
        s.settings_view.state.ollama_text = "http://box.lan:11434/".into();
        let change = s.settings_view.state.commit_ollama(&mut s.settings);
        let applied = only_apply(s.apply_change(change, &ctx));
        assert_eq!(applied.ollama_url, "http://box.lan:11434");
        assert_eq!(s.ollama_host, "box.lan:11434");
        assert_eq!(
            s.ollama, None,
            "the pill says Checking until the poll answers"
        );
    }

    #[test]
    fn staging_change_updates_the_status_bar_path_and_back_to_default() {
        let ctx = egui::Context::default();
        let mut s = settings_state(None);
        let dir = tempfile::tempdir().unwrap();
        s.settings_view.state.staging_text = dir.path().display().to_string();
        let change = s.settings_view.state.commit_staging(&mut s.settings);
        only_apply(s.apply_change(change, &ctx));
        assert_eq!(s.staging_dir, dir.path());
        s.settings_view.state.staging_text = String::new();
        let change = s.settings_view.state.commit_staging(&mut s.settings);
        only_apply(s.apply_change(change, &ctx));
        assert_eq!(s.staging_dir, PathBuf::from("/tmp/default"));
    }

    #[test]
    fn settings_events_fold_into_the_view_state() {
        use crate::runtime::CachedRow;
        use crate::settings::state::{CachedStatus, ConnectionState, SaveIndicator};
        let mut s = settings_state(None);
        assert!(s
            .on_event(&UiEvent::ConnectionTest(Ok("0.40.8".into())))
            .is_none());
        assert_eq!(
            s.settings_view.state.connection,
            ConnectionState::Done(Ok("0.40.8".into()))
        );
        s.on_event(&UiEvent::SettingsSaved(Err("disk full".into())));
        assert_eq!(
            s.settings_view.state.save,
            SaveIndicator::Failed("disk full".into())
        );
        s.on_event(&UiEvent::SettingsSaved(Ok(())));
        assert!(matches!(
            s.settings_view.state.save,
            SaveIndicator::Saved { .. }
        ));
        s.on_event(&UiEvent::HfTokenStatus(HfTokenStatus::Valid("jd".into())));
        assert_eq!(
            s.settings_view.state.token_status,
            HfTokenStatus::Valid("jd".into())
        );
        let rows = vec![CachedRow {
            label: "job-1".into(),
            bytes: 10,
            partial: false,
        }];
        s.on_event(&UiEvent::CachedLoaded(Ok(rows.clone())));
        assert_eq!(s.settings_view.state.cached, CachedStatus::Loaded(rows));
        s.on_event(&UiEvent::CachedCleared(Ok(2_000_000_000)));
        assert_eq!(
            s.settings_view.state.note.as_ref().map(|n| n.0.as_str()),
            Some("Freed 2.00 GB")
        );
    }

    #[test]
    fn settings_requests_are_forwarded_to_the_backend() {
        let (mut app, reqs, ev) = app_with_channels();
        app.state
            .on_event(&UiEvent::HfTokenStatus(HfTokenStatus::None));
        app.state.settings_view.state.token_input = "x".into();
        app.state.settings_view.state.save_token();
        ev.send(UiEvent::CachedCleared(Ok(1))).unwrap();
        app.pump_events();
        let sent: Vec<_> = reqs.try_iter().collect();
        assert!(
            sent.iter()
                .any(|r| matches!(r, BackendRequest::SetHfToken(Some(_)))),
            "{sent:?}"
        );
        assert!(
            sent.iter().any(|r| matches!(r, BackendRequest::ListCached)),
            "a clear is followed by a list refresh: {sent:?}"
        );
    }

    #[test]
    fn opening_settings_refreshes_cached_files_and_checks_a_saved_token_once() {
        let mut s = settings_state(None);
        s.on_event(&UiEvent::HfTokenStatus(HfTokenStatus::Saved));
        s.select(View::Settings);
        let reqs = s.settings_view.state.take_requests();
        assert!(
            reqs.iter().any(|r| matches!(r, BackendRequest::ListCached)),
            "{reqs:?}"
        );
        assert!(
            reqs.iter()
                .any(|r| matches!(r, BackendRequest::CheckHfToken)),
            "{reqs:?}"
        );
        // Coming back while the check runs does not repeat it.
        s.select(View::Browse);
        s.select(View::Settings);
        let reqs = s.settings_view.state.take_requests();
        assert!(
            !reqs
                .iter()
                .any(|r| matches!(r, BackendRequest::CheckHfToken)),
            "{reqs:?}"
        );
        // No token, nothing to check.
        let mut s = settings_state(None);
        s.on_event(&UiEvent::HfTokenStatus(HfTokenStatus::None));
        s.select(View::Settings);
        let reqs = s.settings_view.state.take_requests();
        assert!(
            !reqs
                .iter()
                .any(|r| matches!(r, BackendRequest::CheckHfToken)),
            "{reqs:?}"
        );
    }

    #[test]
    fn app_new_seeds_the_live_settings_and_the_view_from_the_saved_ones() {
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let settings = Settings {
            ollama_url: "http://box:11434".into(),
            delete_after_import: false,
            ..Settings::default()
        };
        let mut app = App::new(
            &settings,
            PathBuf::from("/tmp/staging"),
            Machine {
                ram_gb: 16.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        assert_eq!(app.state.settings, settings);
        assert_eq!(
            app.state.settings_view.state.ollama_text,
            "http://box:11434"
        );
        app.set_environment(
            PathBuf::from("/tmp/default"),
            Machine {
                ram_gb: 32.0,
                vram_gb: Some(16.0),
            },
        );
        assert_eq!(app.state.default_staging, PathBuf::from("/tmp/default"));
        assert_eq!(
            app.state.settings_view.state.detected().map(|m| m.ram_gb),
            Some(32.0)
        );
        assert_eq!(app.state.settings_view.state.staging_text, "/tmp/default");
    }

    #[test]
    fn top_bar_items_share_one_vertical_center() {
        use roost_core::config::ThemeFamily::*;
        for family in [Meadow, Ember, Plum, Tide] {
            for dark in [false, true] {
                let mut h = crate::browse::test_util::Harness::themed(family, dark);
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
                app.state.view = View::Browse;
                app.state.query = "qwen".into();
                for _ in 0..4 {
                    h.frame(vec2(900.0, 640.0), vec![], |ui| app.show(ui));
                }
                let mut items = app.state.top_bar.clone();
                items.push(("search", app.state.search_rect.expect("search shown")));
                items.push(("pill", app.state.pill_rect.expect("pill shown")));
                let centers: Vec<(&str, f32)> =
                    items.iter().map(|(n, r)| (*n, r.center().y)).collect();
                let lo = centers.iter().map(|c| c.1).fold(f32::MAX, f32::min);
                let hi = centers.iter().map(|c| c.1).fold(f32::MIN, f32::max);
                assert!(
                    hi - lo <= 1.0,
                    "{family:?} dark={dark}: top bar centers differ by {}: {centers:?} rects {items:?}",
                    hi - lo
                );
            }
        }
    }
}
