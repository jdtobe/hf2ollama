// The Browse, Installed and Settings views (next tasks) consume the rest of
// the widget and backend API; until then parts of it are unused.
#![allow(dead_code)]

mod app;
mod browse;
mod runtime;
mod theme;
mod widgets;

use eframe::egui;
use hf2ollama_core::config::{Settings, ThemeChoice};
use hf2ollama_core::paths::{OsDirs, Paths};

/// `HF2O_THEME=light|dark` overrides the saved theme. Only meant for taking
/// screenshots of both themes; it is not a user-facing setting.
fn theme_override() -> Option<ThemeChoice> {
    match std::env::var("HF2O_THEME").as_deref() {
        Ok("light") => Some(ThemeChoice::Light),
        Ok("dark") => Some(ThemeChoice::Dark),
        _ => None,
    }
}

fn main() -> eframe::Result<()> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let paths = Paths::resolve(&exe_dir, OsDirs::detect());
    let settings = Settings::load(&paths.config_file).unwrap_or_else(|e| {
        eprintln!("could not load settings ({e}); using defaults");
        Settings::default()
    });
    let machine = hf2ollama_core::hardware::effective(&settings);
    let staging_dir = settings
        .staging_dir
        .clone()
        .unwrap_or_else(|| paths.default_staging.clone());

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 820.0])
            .with_title("hf2ollama"),
        ..Default::default()
    };
    eframe::run_native(
        "hf2ollama",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            theme::apply_choice(&cc.egui_ctx, theme_override().unwrap_or(settings.theme));
            let (tx, rx) = runtime::Backend::spawn(cc.egui_ctx.clone(), settings.clone(), paths);
            Ok(Box::new(app::App::new(
                &settings,
                staging_dir,
                machine,
                tx,
                rx,
            )))
        }),
    )
}
