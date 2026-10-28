//! The small About window.

use crate::menu::model::REPO_URL;
use eframe::egui;

pub fn show(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    let mut keep = true;
    egui::Window::new("About Roost")
        .collapsible(false)
        .resizable(false)
        .open(&mut keep)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.heading(crate::identity::PRODUCT_NAME);
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(6.0);
            ui.label(
                "Find models on Hugging Face, import them into Ollama, and chat with them locally.",
            );
            ui.add_space(6.0);
            ui.label("Licensed under MIT or Apache-2.0.");
            ui.hyperlink_to("Roost on GitHub", REPO_URL);
        });
    *open = keep;
}
