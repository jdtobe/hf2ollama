mod app;
mod browse;
mod confirm;
mod dock;
mod installed;
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

/// Smallest window the layout is designed and tested for (the detail pane
/// budget in `browse::detail` assumes at least this much room).
const MIN_WINDOW: [f32; 2] = [900.0, 640.0];

/// `HF2O_SIZE=WxH` overrides the initial window size, e.g. `900x640`. Only
/// meant for taking screenshots at several widths; it cannot go below
/// `MIN_WINDOW`.
fn size_override() -> Option<[f32; 2]> {
    let raw = std::env::var("HF2O_SIZE").ok()?;
    let (w, h) = raw.split_once('x')?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    Some([w.max(MIN_WINDOW[0]), h.max(MIN_WINDOW[1])])
}

/// `HF2O_QUERY=text` fills the search box and `HF2O_SELECT=owner/name`
/// selects a repository at startup. Only meant for screenshots.
fn preset() -> (Option<String>, Option<String>) {
    (
        std::env::var("HF2O_QUERY").ok(),
        std::env::var("HF2O_SELECT").ok(),
    )
}

/// `HF2O_VIEW=installed|settings|browse` starts on that view. Only meant for
/// screenshots; inert unless set.
fn view_override() -> Option<app::View> {
    match std::env::var("HF2O_VIEW").as_deref() {
        Ok("installed") => Some(app::View::Installed),
        Ok("settings") => Some(app::View::Settings),
        Ok("browse") => Some(app::View::Browse),
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
            .with_inner_size(size_override().unwrap_or([1180.0, 820.0]))
            .with_min_inner_size(MIN_WINDOW)
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
            let mut app = app::App::new(&settings, staging_dir, machine, tx, rx);
            let (query, select) = preset();
            app.preset(query, select);
            if let Some(view) = view_override() {
                app.preset_view(view);
            }
            Ok(Box::new(app))
        }),
    )
}
