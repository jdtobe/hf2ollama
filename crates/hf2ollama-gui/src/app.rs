//! Application shell: top bar, downloads dock, status bar and view bodies.

use crate::browse::state::BrowseState;
use crate::runtime::{BackendRequest, UiEvent};
use crate::theme::{mono, semibold, tokens};
use crate::widgets::segmented_nav;
use eframe::egui::{self, vec2, Color32, Margin, RichText, Sense, Stroke};
use hf2ollama_core::config::Settings;
use hf2ollama_core::fit::Machine;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

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
    /// Where the status pill was last painted (for layout tests).
    pub pill_rect: Option<egui::Rect>,
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
            pill_rect: None,
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
            // Job, installed and delete events belong to the views that are
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
        state.browse = BrowseState::new(machine.clone());
        state.machine = machine;
        App { state, tx, rx }
    }
}

/// Red for the "not reachable" dot; the spec palette has no error token.
fn bad(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(0xf2, 0x76, 0x6b)
    } else {
        Color32::from_rgb(0xb3, 0x26, 0x1e)
    }
}

fn panel_frame(fill: Color32, line: Color32, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(margin)
        .stroke(Stroke::new(1.0, line))
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            if let Some(req) = self.state.on_event(&ev) {
                let _ = self.tx.send(req);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl App {
    /// The whole window; separate from `eframe::App::ui` so tests can run it headless.
    fn show(&mut self, ui: &mut egui::Ui) {
        let t = tokens(ui);
        let dark = ui.visuals().dark_mode;

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
                        ui.add_sized(
                            vec2(360.0, 32.0),
                            egui::TextEdit::singleline(&mut self.state.query)
                                .hint_text("Search Hugging Face models")
                                .margin(Margin::symmetric(10, 7)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (dot, label) = match &self.state.ollama {
                            Some(Some(v)) => {
                                (t.ok, format!("Ollama {v} · {}", self.state.ollama_host))
                            }
                            Some(None) => (bad(dark), "Ollama not reachable".to_string()),
                            None => (t.fg3, "Checking Ollama…".to_string()),
                        };
                        // Hand-painted at an exact size so no layout can stretch it.
                        let galley = ui.painter().layout_no_wrap(
                            label,
                            egui::FontId::proportional(12.0),
                            t.fg2,
                        );
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

        egui::Panel::bottom("dock")
            .frame(panel_frame(t.surface2, t.line, Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
                    ui.label(
                        RichText::new("Nothing running. Pick a model above to start.")
                            .size(13.0)
                            .color(t.fg3),
                    );
                });
            });

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
}
