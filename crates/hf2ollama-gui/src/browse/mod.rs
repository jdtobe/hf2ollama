//! The Browse view: search Hugging Face, inspect a repository, import it.

pub mod detail;
pub mod filters;
pub mod options;
pub mod plan_card;
pub mod results;
pub mod state;

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
                    ui.set_max_width(width);
                    last = Some(f(ui));
                });
            });
            out.drop_without_applying_deltas();
        }
        last.expect("at least one frame ran")
    }
}
