//! The Browse view: search Hugging Face, inspect a repository, import it.

use crate::app::AppState;
use crate::runtime::BackendRequest;
use eframe::egui;
use std::time::Instant;

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub card: Option<egui::Rect>,
    pub selected_row: Option<egui::Rect>,
    pub viewport: Option<egui::Rect>,
    pub scroll_offset: f32,
    /// Rectangles of pinned notices (gated warning, errors, unsupported reason).
    pub notices: Vec<egui::Rect>,
    /// Content rectangle of the detail pane (inside its margins).
    pub pane: Option<egui::Rect>,
    /// Visible rectangle of the notices region.
    pub notices_view: Option<egui::Rect>,
    /// Rectangle of the pinned import card panel.
    pub card_panel: Option<egui::Rect>,
    pub name_field: Option<egui::Rect>,
    pub button: Option<egui::Rect>,
    /// The section caption above the options table.
    pub table_label: Option<egui::Rect>,
    /// The options table header as painted (in place, or stuck at the top).
    pub table_header: Option<egui::Rect>,
    /// The left filter rail panel.
    pub rail: Option<egui::Rect>,
    /// The metadata strip and the `Files | About` switch below it.
    pub meta_strip: Option<egui::Rect>,
    pub switch: Option<egui::Rect>,
    /// Which About state was drawn: loading, empty, error or card.
    pub about_state: Option<&'static str>,
    /// Every content widget of the About tab (text, code blocks, tables).
    pub about_rects: Vec<egui::Rect>,
    /// Painted links with their URLs.
    pub about_links: Vec<(egui::Rect, String)>,
    /// Table scroll viewport and the full width of the table inside it.
    pub about_tables: Vec<(egui::Rect, f32)>,
    pub about_retry: Option<egui::Rect>,
    pub about_toggle: Option<egui::Rect>,
    /// The "Description truncated" notice.
    pub about_notice: Option<egui::Rect>,
}

pub mod about;
#[cfg(test)]
mod about_tests;
pub mod detail;
pub mod filters;
pub mod markdown;
pub mod options;
pub mod plan_card;
pub mod results;
pub mod state;

/// Draws the whole Browse view (rail, results, detail) and collects the
/// backend requests it wants sent.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let now = Instant::now();
    if let Some(q) = app.browse.tick(&app.query, now) {
        out.push(BackendRequest::Search(q));
    }
    if let Some(wait) = app.browse.debounce_remaining(now) {
        ui.ctx().request_repaint_after(wait);
    }
    filters::show(ui, app);
    results::show(ui, app, out);
    detail::show(ui, app, out);
    if let Some(q) = app.browse.next_related_request() {
        out.push(BackendRequest::Search(q));
    }
    for id in app.browse.next_info_requests() {
        out.push(BackendRequest::ModelInfo(id));
    }
    for id in app.browse.next_card_requests() {
        out.push(BackendRequest::ModelCard(id));
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use eframe::egui;

    /// Runs `f` for four frames in a panel `width` wide (screen 1180x820)
    /// and returns the result of the last frame.
    pub fn headless<R>(dark: bool, width: f32, mut f: impl FnMut(&mut egui::Ui) -> R) -> R {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx.set_theme(if dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1180.0, 820.0));
        let mut last = None;
        for _ in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let area = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, 780.0));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        last = Some(f(ui));
                    });
                });
            });
            out.drop_without_applying_deltas();
        }
        last.expect("at least one frame ran")
    }

    /// Frame-by-frame harness for tests that inject input events.
    pub struct Harness {
        ctx: egui::Context,
        time: f64,
    }

    impl Harness {
        pub fn new(dark: bool) -> Harness {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            ctx.set_theme(if dark {
                egui::ThemePreference::Dark
            } else {
                egui::ThemePreference::Light
            });
            Harness { ctx, time: 0.0 }
        }

        /// Like `frame`, returning the URLs the UI asked the OS to open.
        pub fn frame_urls(
            &mut self,
            size: egui::Vec2,
            events: Vec<egui::Event>,
            mut f: impl FnMut(&mut egui::Ui),
        ) -> Vec<String> {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(input, |ui| f(ui));
            let urls = out
                .platform_output
                .commands
                .iter()
                .filter_map(|c| match c {
                    egui::OutputCommand::OpenUrl(o) => Some(o.url.clone()),
                    _ => None,
                })
                .collect();
            out.drop_without_applying_deltas();
            urls
        }

        /// Like `frame`, returning every text the frame painted (what a user
        /// could read off the screen).
        pub fn frame_texts(
            &mut self,
            size: egui::Vec2,
            events: Vec<egui::Event>,
            mut f: impl FnMut(&mut egui::Ui),
        ) -> Vec<String> {
            fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let out = self.ctx.run_ui(input, |ui| f(ui));
            let mut texts = Vec::new();
            for clipped in &out.shapes {
                collect(&clipped.shape, &mut texts);
            }
            out.drop_without_applying_deltas();
            texts
        }

        /// Runs one frame on a `size` screen with the given input events.
        pub fn frame(
            &mut self,
            size: egui::Vec2,
            events: Vec<egui::Event>,
            mut f: impl FnMut(&mut egui::Ui),
        ) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            self.ctx
                .run_ui(input, |ui| f(ui))
                .drop_without_applying_deltas();
        }
    }
}
