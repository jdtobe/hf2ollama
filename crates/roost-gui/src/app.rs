//! Application shell: top bar, downloads dock, status bar and view bodies.

use crate::browse::state::BrowseState;
use crate::dock::jobs::JobsState;
use crate::dock::{self, DockAction, DockUi};
use crate::installed::state::InstalledState;
use crate::runtime::{BackendRequest, UiEvent};
use crate::settings::{self, SettingsView};
use crate::theme::{mono, semibold, tokens};
use crate::widgets::{ellipsize_middle, segmented_nav_rect};
use eframe::egui::{self, vec2, Color32, Margin, RichText, Sense, Stroke};
use roost_core::config::{AutoDismiss, Settings};
use roost_core::events::{Command, JobEvent};
use roost_core::fit::Machine;
use roost_core::session::SessionMode;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    Chat,
    Work,
    /// Installed models and Hugging Face in one view.
    Models,
    Settings,
}

impl View {
    #[cfg(test)]
    pub(crate) const ALL: [View; 4] = [View::Chat, View::Work, View::Models, View::Settings];
}

pub struct AppState {
    /// Reads replies aloud (a fake in tests).
    pub speaker: Box<dyn crate::speech::Speaker>,
    /// Tells the user something needs them while the window is behind.
    pub notifier: Box<dyn crate::notify::Notifier>,
    pub attention: crate::notify::Attention,
    /// Alerts seen since the last frame, waiting for the window's focus.
    pub alerts: Vec<(roost_core::session::SessionId, crate::notify::Kind)>,
    /// The window title last sent, so it is sent only when it changes.
    pub window_title: String,
    pub view: View,
    pub query: String,
    /// `None` until the first poll answers; then the version if reachable.
    pub ollama: Option<Option<String>>,
    pub installed: InstalledState,
    /// The installed model shown in the detail pane, if that is what is selected.
    pub models_installed: Option<String>,
    pub ollama_host: String,
    pub staging_dir: PathBuf,
    /// Where staging goes when the setting is empty.
    pub default_staging: PathBuf,
    pub delete_after_import: bool,
    /// The live settings; the Settings view edits them in place.
    pub settings: Settings,
    /// The sidebar's width as last drawn, whether a drag moved it, and
    /// whether the new width still has to be saved.
    pub sidebar_seen: f32,
    pub sidebar_moved: bool,
    pub sidebar_dirty: bool,
    pub settings_view: SettingsView,
    /// Detected or overridden once at startup; NVML probing is not free.
    pub machine: Machine,
    pub browse: BrowseState,
    pub jobs: JobsState,
    /// Models Ollama has loaded right now.
    pub running: Vec<roost_core::ollama::RunningModel>,
    /// The About window is open.
    pub show_about: bool,
    /// Which of Browse and Installed the Models tab opens to.
    /// Which platform's menus to build (the real one, except in tests).
    pub platform: crate::menu::Platform,
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
    /// Where the Models search field was last painted; `None` when not showing.
    pub search_rect: Option<egui::Rect>,
    /// Where its clear (✕) button was last painted; `None` when the field is empty.
    pub search_clear_rect: Option<egui::Rect>,
    /// Extent of the status bar contents as last painted (for layout tests).
    pub status_rect: Option<egui::Rect>,
    /// One-line startup message (e.g. an ignored saved setting).
    pub startup_notice: Option<String>,
    /// Where each top bar item was last painted (for layout tests).
    #[cfg(test)]
    pub top_bar: Vec<(&'static str, egui::Rect)>,
    /// The family dots of the theme popover.
    #[cfg(test)]
    pub theme_dots: Vec<egui::Rect>,
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
            view: View::Models,
            query: String::new(),
            ollama: None,
            installed: InstalledState::new(),
            models_installed: None,
            ollama_host: String::new(),
            staging_dir: PathBuf::new(),
            default_staging: PathBuf::new(),
            delete_after_import: true,
            settings: Settings::default(),
            sidebar_seen: 0.0,
            sidebar_moved: false,
            sidebar_dirty: false,
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
            running: Vec::new(),
            speaker: Box::new(crate::speech::SystemSpeaker::detect()),
            // Tests must never run the operating system's notification tool.
            notifier: if cfg!(test) {
                Box::new(crate::notify::NullNotifier)
            } else {
                Box::new(crate::notify::SystemNotifier::detect())
            },
            attention: crate::notify::Attention::default(),
            alerts: Vec::new(),
            window_title: crate::identity::PRODUCT_NAME.to_string(),
            show_about: false,
            // Tests lay out the content alone; the bar is tested on its own.
            platform: if cfg!(test) {
                crate::menu::Platform::Mac
            } else {
                crate::menu::Platform::current()
            },
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
            #[cfg(test)]
            theme_dots: Vec::new(),
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
            UiEvent::RunningLoaded(models) => {
                self.running = models.clone();
                None
            }
            UiEvent::UnloadDone(name, result) => {
                match result {
                    Ok(()) => self.running.retain(|m| m.name != *name),
                    Err(e) => self.chat.notice = Some(format!("Could not unload {name}: {e}")),
                }
                None
            }
            UiEvent::Chat(ev) => {
                if let Some(alert) = crate::notify::kind_of(ev) {
                    self.alerts.push(alert);
                }
                self.chat.fold(ev.clone());
                None
            }
            UiEvent::ChatUnavailable(why) => {
                self.chat.recorded_unavailable(why.clone());
                if self.view == View::Chat {
                    self.select(View::Models);
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

/// Below this window width the status pill shows only the Ollama version.
const PILL_FULL_MIN_WINDOW: f32 = 1100.0;
const PILL_PAD_X: f32 = 10.0;
const PILL_DOT: f32 = 8.0;
const PILL_GAP: f32 = 6.0;

pub struct App {
    state: AppState,
    /// The native menu bar (macOS and Windows).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native: Option<crate::menu::native::NativeMenu>,
    /// The native menu has been attempted (it is installed on the first frame).
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_tried: bool,
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
        App {
            state,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_tried: false,
            tx,
            rx,
        }
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

/// A bar's frame: flat, with no outline. The panel itself draws one line
/// along the edge that faces the content, so stacked bars and the sidebar
/// beside them never double up.
pub(crate) fn panel_frame(fill: Color32, margin: Margin) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(margin)
}

/// What Ollama holds in memory, opened from the status pill.
/// The theme switcher behind a right-click on the app name: one dot per
/// family in its own accent, and System / Light / Dark. Returns the dots'
/// rectangles.
fn theme_popover(
    ui: &mut egui::Ui,
    t: &crate::theme_tokens::Tokens,
    settings: &Settings,
    actions: &mut Vec<crate::menu::MenuAction>,
) -> Vec<egui::Rect> {
    use crate::menu::MenuAction;
    use roost_core::config::{ThemeFamily, ThemeMode};
    let dark = ui.visuals().dark_mode;
    ui.set_min_width(210.0);
    ui.label(RichText::new("Theme").font(semibold(12.0)).color(t.fg3));
    let mut dots = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (family, name) in [
            (ThemeFamily::Meadow, "Meadow"),
            (ThemeFamily::Ember, "Ember"),
            (ThemeFamily::Plum, "Plum"),
            (ThemeFamily::Tide, "Tide"),
        ] {
            let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(name);
            let accent = crate::theme_tokens::palette(family, dark).accent;
            ui.painter().circle_filled(rect.center(), 10.0, accent);
            if settings.theme_family == family {
                ui.painter()
                    .circle_stroke(rect.center(), 13.0, Stroke::new(2.0, t.fg));
            } else if resp.hovered() {
                ui.painter()
                    .circle_stroke(rect.center(), 13.0, Stroke::new(1.0, t.line));
            }
            if resp.clicked() {
                actions.push(MenuAction::SetFamily(family));
            }
            dots.push(rect);
        }
    });
    ui.add_space(6.0);
    let modes = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];
    let chosen = modes
        .iter()
        .position(|m| *m == settings.theme_mode)
        .unwrap_or(0);
    let mut rects = Vec::new();
    if let Some(i) = crate::widgets::segmented(
        ui,
        t,
        &[
            ("System", "Follow the system"),
            ("Light", "Always light"),
            ("Dark", "Always dark"),
        ],
        chosen,
        &mut rects,
    ) {
        actions.push(MenuAction::SetMode(modes[i]));
    }
    dots
}

fn memory_popover(
    pill: &egui::Response,
    state: &mut AppState,
    requests: &mut Vec<BackendRequest>,
    t: &crate::theme::Tokens,
) {
    egui::Popup::menu(pill)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(260.0);
            ui.label(RichText::new("In memory").font(semibold(11.0)).color(t.fg3));
            if state.running.is_empty() {
                ui.label(RichText::new("Nothing loaded").size(12.5).color(t.fg2));
                return;
            }
            let names: Vec<(String, u64)> = state
                .running
                .iter()
                .map(|m| (m.name.clone(), m.size))
                .collect();
            for (name, size) in names {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(RichText::new(&name).size(12.5).color(t.fg)).truncate(),
                    );
                    ui.label(
                        RichText::new(crate::browse::results::format_size(size))
                            .size(11.5)
                            .color(t.fg3),
                    );
                    if crate::widgets::small_button(ui, "Unload") {
                        requests.push(BackendRequest::UnloadModel(name.clone()));
                        state.running.retain(|m| m.name != name);
                    }
                });
            }
        });
}

/// Width of the settings gear in the top bar.
const GEAR_W: f32 = 28.0;

/// The settings gear: a small hand-painted button, highlighted on Settings.
fn settings_gear(ui: &mut egui::Ui, on: bool) -> (egui::Response, egui::Rect) {
    let t = tokens(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(GEAR_W, GEAR_W), Sense::click());
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Settings");
    let fill = if on {
        t.accent_soft
    } else if resp.hovered() {
        t.surface2
    } else {
        egui::Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 7.0, fill);
    let color = if on { t.accent } else { t.fg2 };
    let c = rect.center();
    p.circle_stroke(c, 5.0, Stroke::new(1.5, color));
    for k in 0..8 {
        let angle = k as f32 * std::f32::consts::FRAC_PI_4;
        let d = vec2(angle.cos(), angle.sin());
        p.line_segment([c + d * 7.0, c + d * 9.5], Stroke::new(2.0, color));
    }
    (resp, rect)
}

impl App {
    /// A segmented nav placed in the middle of the top row (a frame sits at
    /// the top of its row, so it is placed explicitly).
    fn place_nav(
        &self,
        ui: &mut egui::Ui,
        labels: &[&str],
        badges: &[Option<String>],
        current: usize,
    ) -> (Option<usize>, egui::Rect) {
        let left = ui.cursor().min.x;
        let area = egui::Rect::from_min_size(
            egui::pos2(left, ui.max_rect().min.y + (TOP_ROW_H - NAV_H) / 2.0),
            vec2(ui.available_width(), NAV_H),
        );
        let mut nav_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let (picked, rect) = segmented_nav_rect(&mut nav_ui, labels, badges, current);
        ui.advance_cursor_after_rect(rect);
        (picked, rect)
    }
}

impl eframe::App for App {
    /// Lets chat runs write their partial replies before the process ends.
    fn on_exit(&mut self) {
        let (ack, done) = std::sync::mpsc::channel();
        if self.tx.send(BackendRequest::ShutdownChat(ack)).is_ok() {
            let _ = done.recv_timeout(std::time::Duration::from_millis(3500));
        }
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if !self.native_tried {
            self.native_tried = true;
            self.install_native_menu(ctx, frame);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = frame;
        self.pump_events();
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let frame_alerts = self.state.deliver_alerts(focused);
        if frame_alerts.attention {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
        if let Some(title) = frame_alerts.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        self.pump_menu(ctx);
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

    /// Runs the menu actions the platform's shortcuts produced this frame.
    /// macOS dispatches shortcuts in its native menu, so nothing is read there.
    pub(crate) fn pump_menu(&mut self, ctx: &egui::Context) {
        let platform = self.state.platform;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(native) = &mut self.native {
            let model = crate::menu::model::build(&self.state, platform);
            native.sync(&model);
            let actions = native.poll();
            self.run_menu_actions(ctx, actions);
        }
        if platform == crate::menu::Platform::Mac {
            return;
        }
        let model = crate::menu::model::build(&self.state, platform);
        let actions = crate::menu::bar::shortcuts(ctx, &model);
        self.run_menu_actions(ctx, actions);
    }

    /// Creates the native menu bar on the first frame. It must not happen in
    /// the window creator: macOS calls back into the event loop while the
    /// menu is attached, which aborts the process from inside that callback.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
    fn install_native_menu(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "windows")]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match frame.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
                _ => None,
            }
        };
        #[cfg(not(target_os = "windows"))]
        let hwnd = None;
        self.native = crate::menu::native::NativeMenu::install(ctx, self.state.platform, hwnd);
    }

    pub(crate) fn run_menu_actions(
        &mut self,
        ctx: &egui::Context,
        actions: Vec<crate::menu::MenuAction>,
    ) {
        for action in actions {
            for request in crate::menu::actions::handle(&mut self.state, ctx, action) {
                let _ = self.tx.send(request);
            }
        }
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

        // Narrow windows drop the host from the pill (it moves to the hover
        // text) so the search field keeps its room.
        let narrow = ui.ctx().content_rect().width() < PILL_FULL_MIN_WINDOW;
        let (dot, label) = match &self.state.ollama {
            Some(Some(v)) if narrow => (t.ok, format!("Ollama {v}")),
            Some(Some(v)) => (t.ok, format!("Ollama {v} · {}", self.state.ollama_host)),
            Some(None) => (bad(dark), "Ollama not reachable".to_string()),
            None => (t.fg3, "Checking Ollama…".to_string()),
        };
        let galley = ui
            .painter()
            .layout_no_wrap(label, egui::FontId::proportional(12.0), t.fg2);

        let platform = self.state.platform;
        if crate::menu::bar::uses_window_bar(platform) {
            let model = crate::menu::model::build(&self.state, platform);
            let bar = crate::menu::bar::show(ui, &model, platform);
            self.run_menu_actions(ui.ctx(), bar.actions);
        }

        let mut pill_requests: Vec<BackendRequest> = Vec::new();
        let mut theme_actions: Vec<crate::menu::MenuAction> = Vec::new();
        egui::Panel::top("top")
            .frame(panel_frame(t.surface, Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                // A strip of the theme colour along the top edge, drawn in the
                // bar's own top padding so the bar keeps its height.
                let top = ui.max_rect().top() - 10.0;
                let screen = ui.ctx().content_rect();
                ui.ctx().layer_painter(ui.layer_id()).rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(screen.left(), top),
                        egui::pos2(screen.right(), top + 3.0),
                    ),
                    0.0,
                    t.accent,
                );
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
                    // A quiet way to change the theme: right-click the name.
                    let name = ui
                        .interact(
                            _logo.union(_word.rect),
                            ui.scope_id().with("app-name"),
                            Sense::click(),
                        )
                        .on_hover_text("Right-click for themes");
                    egui::Popup::context_menu(&name).show(|ui| {
                        let _dots = theme_popover(ui, &t, &self.state.settings, &mut theme_actions);
                        #[cfg(test)]
                        {
                            self.state.theme_dots = _dots;
                        }
                    });
                    ui.add_space(8.0);

                    // Chat, Work and Models; Browse and Installed live inside Models.
                    let labels = ["Chat", "Work", "Models"];
                    let chat_runs = self.state.chat.active_runs_for(SessionMode::Chat);
                    let work_runs = self.state.chat.active_runs_for(SessionMode::Agent);
                    let badges: Vec<Option<String>> = vec![
                        (chat_runs > 0).then(|| chat_runs.to_string()),
                        (work_runs > 0).then(|| work_runs.to_string()),
                        None,
                    ];
                    let current = match self.state.view {
                        View::Chat => 0,
                        View::Work => 1,
                        View::Models => 2,
                        // Settings is the gear, not a tab.
                        View::Settings => usize::MAX,
                    };
                    let (picked, _nav_rect) = self.place_nav(ui, &labels, &badges, current);
                    #[cfg(test)]
                    self.state.top_bar.push(("nav", _nav_rect));
                    if let Some(i) = picked {
                        let view = [View::Chat, View::Work, View::Models][i];
                        self.state.select(view);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let on = self.state.view == View::Settings;
                        let (gear, _gear_rect) = settings_gear(ui, on);
                        #[cfg(test)]
                        self.state.top_bar.push(("gear", _gear_rect));
                        if gear.clicked() {
                            self.state.select(View::Settings);
                        }
                        // Hand-painted at an exact size so no layout can stretch it.
                        let (rect, pill_resp) =
                            ui.allocate_exact_size(pill_size(galley.size().x), Sense::click());
                        self.state.pill_rect = Some(rect);
                        let pill_resp = pill_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(match &self.state.ollama {
                                Some(Some(v)) => format!(
                                    "Ollama {v} · {}\nClick to see models in memory",
                                    self.state.ollama_host
                                ),
                                _ => "Models in memory".to_string(),
                            });
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
                        memory_popover(&pill_resp, &mut self.state, &mut pill_requests, &t);
                    });
                });
            });
        for r in pill_requests {
            let _ = self.tx.send(r);
        }
        if std::mem::take(&mut self.state.sidebar_dirty) {
            // The sidebar was dragged to a new width: remember it.
            let _ = self
                .tx
                .send(BackendRequest::ApplySettings(self.state.settings.clone()));
        }
        if !theme_actions.is_empty() {
            let ctx = ui.ctx().clone();
            self.run_menu_actions(&ctx, theme_actions);
        }

        // Free space of the staging folder is not exposed by the core, so the
        // status bar leaves it out. The Ollama store path is omitted until the
        // server reports it.
        egui::Panel::bottom("status")
            .frame(panel_frame(t.surface, Margin::symmetric(16, 6)))
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

        if self.state.view == View::Chat || self.state.view == View::Work {
            let mut requests = Vec::new();
            if self.state.view == View::Work {
                crate::chat::view::show_work(ui, &mut self.state, &mut requests);
            } else {
                crate::chat::view::show(ui, &mut self.state, &mut requests);
            }
            for r in requests {
                let _ = self.tx.send(r);
            }
        } else if self.state.view == View::Models {
            let mut requests = Vec::new();
            crate::models::show(ui, &mut self.state, &mut requests);
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
        crate::menu::about::show(ui.ctx(), &mut self.state.show_about);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed::state::ListStatus;
    use crate::runtime::HfTokenStatus;

    #[test]
    fn application_display_identity_is_roost() {
        assert_eq!(crate::identity::PRODUCT_NAME, "Roost");
    }

    #[test]
    fn nav_selection_changes_view_state() {
        let mut s = AppState::new();
        assert_eq!(s.view, View::Models);
        s.select(View::Work);
        assert_eq!(s.view, View::Work);
        s.select(View::Settings);
        assert_eq!(s.view, View::Settings);
        s.select(View::Models);
        assert_eq!(s.view, View::Models);
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
                (rect.right() - (screen_right - 16.0 - GEAR_W - 8.0)).abs() < 2.0,
                "dark={dark} right edge {}",
                rect.right()
            );
        }
    }

    #[allow(clippy::type_complexity)]
    fn run_app(
        dark: bool,
        width: f32,
    ) -> (Option<egui::Rect>, Option<egui::Rect>, Option<egui::Rect>) {
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
        // The longest realistic pill: a connected server with a full URL.
        app.state.ollama = Some(Some("0.12.10".into()));
        app.state.ollama_host = "http://192.168.100.200:11434".into();
        app.state.select(View::Models);
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
            app.state.browse.probe.list,
            app.state.browse.probe.filters_chip,
        )
    }

    #[test]
    fn the_models_search_is_the_first_thing_in_the_sidebar_above_the_filters() {
        for dark in [false, true] {
            for width in [700.0, 900.0, 1180.0] {
                let (search, list, filters) = run_app(dark, width);
                let (search, list) = (search.expect("search"), list.expect("sidebar"));
                let tag = format!("dark={dark} width={width}: {search:?} in {list:?}");
                assert!(list.contains_rect(search), "{tag}");
                assert!(search.top() - list.top() < 20.0, "not at the top: {tag}");
                if let Some(chip) = filters {
                    assert!(search.bottom() <= chip.top(), "below the filters: {tag}");
                }
            }
        }
    }

    /// Every shape drawn last frame that is a line segment inside `rect`.
    fn strokes_in(
        app: &mut App,
        h: &mut crate::browse::test_util::Harness,
        rect: egui::Rect,
    ) -> usize {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1180.0, 820.0),
            )),
            ..Default::default()
        };
        let out = h.ctx().run_ui(input, |ui| app.show(ui));
        let n = out
            .shapes
            .iter()
            .filter(|s| {
                matches!(&s.shape, egui::epaint::Shape::LineSegment { points, .. }
                    if rect.contains(points[0]) && rect.contains(points[1])
                        && s.clip_rect.contains_rect(rect))
            })
            .count();
        out.drop_without_applying_deltas();
        n
    }

    #[test]
    fn the_search_box_is_in_the_same_place_on_every_tab_with_a_visible_clear_button() {
        let (mut app, mut h) = nav_app(View::Models);
        app.state.query = "llama".into();
        app.state.chat_ui.rail_query = "llama".into();
        let mut seen = Vec::new();
        for view in [View::Models, View::Chat, View::Work] {
            app.state.select(view);
            for _ in 0..4 {
                h.frame(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
            }
            let field = app.state.search_rect.expect("search field");
            let clear = app.state.search_clear_rect.expect("clear button");
            assert!(
                field.contains_rect(clear),
                "{view:?}: {clear:?} not in {field:?}"
            );
            assert_eq!(
                strokes_in(&mut app, &mut h, clear),
                2,
                "{view:?}: no ✕ drawn"
            );
            seen.push((view, field));
        }
        let (_, first) = seen[0];
        for (view, f) in &seen {
            // Chat and Work make room for the new-chat buttons beside the
            // field, so it may be narrower, but it starts in the same place.
            assert!(
                (f.min - first.min).length() < 0.5
                    && (f.height() - first.height()).abs() < 0.5
                    && f.width() <= first.width() + 0.5,
                "{view:?} search {f:?} differs from Models {first:?}"
            );
        }
    }

    #[test]
    fn the_header_has_no_search_field() {
        let mut app = {
            let (tx, _) = std::sync::mpsc::channel();
            let (_, rx) = std::sync::mpsc::channel();
            App::new(
                &Settings::default(),
                PathBuf::from("/tmp/staging"),
                Machine {
                    ram_gb: 16.0,
                    vram_gb: None,
                },
                tx,
                rx,
            )
        };
        app.state.select(View::Models);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1180.0, 820.0));
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| app.show(ui))
                .drop_without_applying_deltas();
        }
        let header_bottom = app.state.pill_rect.expect("pill").bottom();
        let search = app.state.search_rect.expect("search");
        assert!(
            search.top() > header_bottom,
            "{search:?} vs header {header_bottom}"
        );
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
        app.state.select(View::Models);
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
        s.installed.want_details("a:1");
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
        s.select(View::Models);
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
                app.state.view = View::Models;
                app.state.query = "qwen".into();
                for _ in 0..4 {
                    h.frame(vec2(900.0, 640.0), vec![], |ui| app.show(ui));
                }
                let mut items = app.state.top_bar.clone();
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

    fn shortcut_event(key: egui::Key, mods: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            ..Default::default()
        }
    }

    fn menu_app(platform: crate::menu::Platform) -> App {
        let (tx, _) = std::sync::mpsc::channel();
        let (_, rx) = std::sync::mpsc::channel();
        let mut app = App::new(
            &Settings::default(),
            PathBuf::new(),
            Machine {
                ram_gb: 8.0,
                vram_gb: None,
            },
            tx,
            rx,
        );
        app.state.platform = platform;
        app.state.view = View::Chat;
        app
    }

    #[test]
    fn shortcuts_switch_views_on_windows_and_linux() {
        for platform in [crate::menu::Platform::Linux, crate::menu::Platform::Windows] {
            let mut app = menu_app(platform);
            let ctx = egui::Context::default();
            let input = shortcut_event(egui::Key::Num3, egui::Modifiers::COMMAND);
            ctx.run_ui(input, |ui| app.pump_menu(ui.ctx()))
                .drop_without_applying_deltas();
            assert_eq!(app.state.view, View::Models, "{platform:?}");
        }
    }

    #[test]
    fn macos_leaves_shortcuts_to_its_native_menu() {
        let mut app = menu_app(crate::menu::Platform::Mac);
        let ctx = egui::Context::default();
        let input = shortcut_event(egui::Key::Num3, egui::Modifiers::COMMAND);
        ctx.run_ui(input, |ui| app.pump_menu(ui.ctx()))
            .drop_without_applying_deltas();
        assert_eq!(app.state.view, View::Chat, "egui must not also act on it");
    }

    #[test]
    fn the_window_bar_shows_only_on_linux_and_pushes_nothing_off_screen() {
        for (platform, bar) in [
            (crate::menu::Platform::Linux, true),
            (crate::menu::Platform::Mac, false),
            (crate::menu::Platform::Windows, false),
        ] {
            let mut app = menu_app(platform);
            let mut h = crate::browse::test_util::Harness::new(false);
            for _ in 0..4 {
                h.frame(vec2(900.0, 640.0), vec![], |ui| app.show(ui));
            }
            let texts = h.frame_texts(vec2(900.0, 640.0), vec![], |ui| app.show(ui));
            assert_eq!(texts.iter().any(|t| t == "Help"), bar, "{platform:?}");
        }
    }

    fn nav_app(view: View) -> (App, crate::browse::test_util::Harness) {
        let mut app = menu_app(crate::menu::Platform::Mac);
        app.state.view = view;
        app.state.ollama = Some(Some("0.9".into()));
        let mut h = crate::browse::test_util::Harness::new(false);
        for _ in 0..4 {
            h.frame(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
        }
        (app, h)
    }

    /// The shapes painted in one more frame, with groups flattened.
    fn painted_now(
        app: &mut App,
        h: &mut crate::browse::test_util::Harness,
    ) -> Vec<egui::epaint::Shape> {
        fn flat(shape: &egui::epaint::Shape, into: &mut Vec<egui::epaint::Shape>) {
            match shape {
                egui::epaint::Shape::Vec(items) => items.iter().for_each(|s| flat(s, into)),
                other => into.push(other.clone()),
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(1180.0, 820.0),
            )),
            ..Default::default()
        };
        let out = h.ctx().run_ui(input, |ui| app.show(ui));
        let mut shapes = Vec::new();
        for clipped in &out.shapes {
            flat(&clipped.shape, &mut shapes);
        }
        out.drop_without_applying_deltas();
        shapes
    }

    fn near(a: egui::Color32, b: egui::Color32) -> bool {
        (a.r() as i32 - b.r() as i32).abs() < 4
            && (a.g() as i32 - b.g() as i32).abs() < 4
            && (a.b() as i32 - b.b() as i32).abs() < 4
    }

    #[test]
    fn the_top_bar_has_a_strip_of_the_theme_colour_along_its_top_edge() {
        let (mut app, mut h) = nav_app(View::Chat);
        let accent =
            crate::theme_tokens::palette(roost_core::config::ThemeFamily::Meadow, false).accent;
        let logo = app
            .state
            .top_bar
            .iter()
            .find(|(n, _)| *n == "logo")
            .map(|(_, r)| *r)
            .unwrap();
        let strip = painted_now(&mut app, &mut h)
            .into_iter()
            .find_map(|s| match s {
                egui::epaint::Shape::Rect(r)
                    if near(r.fill, accent)
                        && r.rect.width() >= 1179.0
                        && (r.rect.height() - 3.0).abs() < 0.5 =>
                {
                    Some(r.rect)
                }
                _ => None,
            });
        let strip = strip.expect("a strip of the theme colour across the window");
        assert!(strip.top() <= 1.0, "at the top edge: {strip:?}");
        assert!(
            strip.bottom() <= logo.top(),
            "above the logo: {strip:?} {logo:?}"
        );
    }

    #[test]
    fn the_active_tab_has_one_underline_in_the_theme_colour_and_it_follows_the_tab() {
        let accent =
            crate::theme_tokens::palette(roost_core::config::ThemeFamily::Meadow, false).accent;
        let underline_x = |app: &mut App, h: &mut crate::browse::test_util::Harness| {
            let lines: Vec<f32> = painted_now(app, h)
                .into_iter()
                .filter_map(|s| match s {
                    egui::epaint::Shape::LineSegment { points, stroke }
                        if stroke.width == 2.0
                            && near(stroke.color, accent)
                            && (points[0].y - points[1].y).abs() < 0.1
                            && points[0].y < 60.0 =>
                    {
                        Some((points[0].x + points[1].x) / 2.0)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(lines.len(), 1, "one underline: {lines:?}");
            lines[0]
        };
        let (mut app, mut h) = nav_app(View::Chat);
        let nav = app
            .state
            .top_bar
            .iter()
            .find(|(n, _)| *n == "nav")
            .map(|(_, r)| *r)
            .unwrap();
        let chat_x = underline_x(&mut app, &mut h);
        assert!(
            nav.contains(egui::pos2(chat_x, nav.center().y)),
            "under a tab"
        );
        app.state.select(View::Models);
        for _ in 0..3 {
            h.frame(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
        }
        let models_x = underline_x(&mut app, &mut h);
        assert!(
            models_x > chat_x + 60.0,
            "moved right: {chat_x} -> {models_x}"
        );
        // The gear is not a tab: no underline under the tabs on Settings.
        app.state.select(View::Settings);
        for _ in 0..3 {
            h.frame(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
        }
        let none = painted_now(&mut app, &mut h).into_iter().any(|s| {
            matches!(&s, egui::epaint::Shape::LineSegment { points, stroke }
                if stroke.width == 2.0
                    && near(stroke.color, accent)
                    && nav.contains(points[0]))
        });
        assert!(!none, "Settings is not a tab");
    }

    fn click_at(app: &mut App, h: &mut crate::browse::test_util::Harness, at: egui::Pos2) {
        let size = vec2(1180.0, 820.0);
        h.frame(size, vec![egui::Event::PointerMoved(at)], |ui| app.show(ui));
        for pressed in [true, false] {
            h.frame(
                size,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                |ui| app.show(ui),
            );
        }
        for _ in 0..2 {
            h.frame(size, vec![], |ui| app.show(ui));
        }
    }

    fn top_item(app: &App, name: &str) -> Option<egui::Rect> {
        app.state
            .top_bar
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r)
    }

    #[test]
    fn right_clicking_the_app_name_offers_the_themes() {
        use roost_core::config::{ThemeFamily, ThemeMode};
        let (mut app, mut h) = nav_app(View::Models);
        let word = top_item(&app, "wordmark").expect("app name");
        let size = vec2(1180.0, 820.0);
        let at = word.center();
        h.frame(size, vec![egui::Event::PointerMoved(at)], |ui| app.show(ui));
        for pressed in [true, false] {
            h.frame(
                size,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                }],
                |ui| app.show(ui),
            );
        }
        for _ in 0..2 {
            h.frame(size, vec![], |ui| app.show(ui));
        }
        let dots = app.state.theme_dots.clone();
        assert_eq!(dots.len(), 4, "one dot per family");
        let texts = h.frame_texts(size, vec![], |ui| app.show(ui)).join("\n");
        for want in ["System", "Light", "Dark"] {
            assert!(texts.contains(want), "{want}: {texts}");
        }
        // Pick Tide: the dots are in the order Meadow, Ember, Plum, Tide.
        click_at(&mut app, &mut h, dots[3].center());
        assert_eq!(app.state.settings.theme_family, ThemeFamily::Tide);
        let _ = ThemeMode::System;
    }

    /// The sidebar's width on the tab `view`, after a few frames.
    fn sidebar_width_on(
        app: &mut App,
        h: &mut crate::browse::test_util::Harness,
        view: View,
    ) -> f32 {
        app.state.select(view);
        for _ in 0..4 {
            h.frame(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
        }
        match view {
            View::Models => app.state.browse.probe.list.expect("models list").width(),
            _ => app.state.chat_ui.probe.rail.expect("rail").width() + 24.0,
        }
    }

    #[test]
    fn every_tab_has_the_same_sidebar_and_a_drag_on_one_changes_all() {
        let (mut app, mut h) = nav_app(View::Models);
        let size = vec2(1180.0, 820.0);
        let w = sidebar_width_on(&mut app, &mut h, View::Models);
        assert!((w - 340.0).abs() < 5.0, "default width {w}");
        for v in [View::Chat, View::Work] {
            let other = sidebar_width_on(&mut app, &mut h, v);
            assert!((other - w).abs() < 5.0, "{v:?}: {other} vs {w}");
        }
        // Drag the edge on Models 60 points wider.
        app.state.select(View::Models);
        for _ in 0..3 {
            h.frame(size, vec![], |ui| app.show(ui));
        }
        let edge = app.state.browse.probe.list.unwrap().right();
        let y = 400.0;
        h.frame(
            size,
            vec![egui::Event::PointerMoved(egui::pos2(edge, y))],
            |ui| app.show(ui),
        );
        let button = |pressed| egui::Event::PointerButton {
            pos: egui::pos2(edge, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        h.frame(size, vec![button(true)], |ui| app.show(ui));
        for step in 1..=6 {
            h.frame(
                size,
                vec![egui::Event::PointerMoved(egui::pos2(
                    edge + 10.0 * step as f32,
                    y,
                ))],
                |ui| app.show(ui),
            );
        }
        h.frame(
            size,
            vec![egui::Event::PointerButton {
                pos: egui::pos2(edge + 60.0, y),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            |ui| app.show(ui),
        );
        for _ in 0..3 {
            h.frame(size, vec![], |ui| app.show(ui));
        }
        let wider = app.state.browse.probe.list.unwrap().width();
        assert!(wider > w + 40.0, "dragged from {w} to {wider}");
        assert!(
            (app.state.settings.sidebar_width - wider).abs() < 5.0,
            "remembered {} vs drawn {wider}",
            app.state.settings.sidebar_width
        );
        for v in [View::Chat, View::Work] {
            let other = sidebar_width_on(&mut app, &mut h, v);
            assert!((other - wider).abs() < 5.0, "{v:?}: {other} vs {wider}");
        }
    }

    #[test]
    fn the_sidebar_never_leaves_the_page_less_room_than_it_needs() {
        let (mut app, mut h) = nav_app(View::Models);
        app.state.settings.sidebar_width = 2000.0;
        let size = vec2(900.0, 640.0);
        for _ in 0..4 {
            h.frame(size, vec![], |ui| app.show(ui));
        }
        let list = app.state.browse.probe.list.unwrap();
        assert!(
            list.width() <= crate::sidebar::max_width(900.0) + 1.0,
            "{}",
            list.width()
        );
        assert!(900.0 - list.width() >= crate::sidebar::MIN_CONTENT - 1.0);
    }

    #[test]
    fn models_is_one_tab_with_no_sub_nav() {
        for v in View::ALL {
            let (app, _h) = nav_app(v);
            assert!(top_item(&app, "subnav").is_none(), "{v:?}");
        }
    }

    #[test]
    fn clicking_the_models_tab_opens_it() {
        let (mut app, mut h) = nav_app(View::Chat);
        let nav = top_item(&app, "nav").unwrap();
        // The third of three tabs is Models.
        click_at(
            &mut app,
            &mut h,
            egui::pos2(nav.right() - 30.0, nav.center().y),
        );
        assert_eq!(app.state.view, View::Models);
        click_at(
            &mut app,
            &mut h,
            egui::pos2(nav.left() + 20.0, nav.center().y),
        );
        assert_eq!(app.state.view, View::Chat);
    }

    #[test]
    fn the_gear_opens_settings_and_sits_at_the_far_right_of_the_top_bar() {
        let (mut app, mut h) = nav_app(View::Chat);
        let gear = top_item(&app, "gear").expect("gear painted");
        let pill = app.state.pill_rect.unwrap();
        assert!(
            gear.left() >= pill.right(),
            "gear {gear:?} right of pill {pill:?}"
        );
        assert!(gear.right() <= 1180.0 - 15.0);
        click_at(&mut app, &mut h, gear.center());
        assert_eq!(app.state.view, View::Settings);
    }

    #[test]
    fn settings_is_not_a_tab() {
        let (mut app, mut h) = nav_app(View::Settings);
        let texts = h.frame_texts(vec2(1180.0, 820.0), vec![], |ui| app.show(ui));
        for tab in ["Chat", "Work", "Models"] {
            assert!(texts.iter().any(|t| t == tab), "{tab}");
        }
        let nav = top_item(&app, "nav").unwrap();
        assert!(nav.width() < 260.0, "three short tabs, not five: {nav:?}");
    }
}
