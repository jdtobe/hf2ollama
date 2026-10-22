mod app;
mod browse;
mod confirm;
mod dock;
mod installed;
mod runtime;
mod settings;
mod startup;
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

/// Only the exact value `1` enables the in-memory token store.
// Always compiled so it is tested in every build; only dev-hooks builds call it.
#[cfg_attr(not(feature = "dev-hooks"), allow(dead_code))]
fn dev_memory_store_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Dev-hooks builds only: `HF2O_MEMORY_TOKEN_STORE=1` keeps the Hugging Face
/// token in memory instead of the system keychain, starting with
/// `HF2O_MEMORY_TOKEN` when set. For screenshots and trials that must not
/// touch the real keychain. Without the `dev-hooks` feature this does not
/// exist, so a release binary cannot be switched to it.
#[cfg(feature = "dev-hooks")]
fn memory_token_store() -> Option<std::sync::Arc<dyn hf2ollama_core::secrets::TokenStore>> {
    use hf2ollama_core::secrets::{MemoryStore, TokenStore};
    let value = std::env::var("HF2O_MEMORY_TOKEN_STORE").ok();
    if !dev_memory_store_enabled(value.as_deref()) {
        return None;
    }
    eprintln!("hf2ollama: dev-hooks: using an in-memory token store; the token will not be saved");
    let store = MemoryStore::default();
    if let Ok(token) = std::env::var("HF2O_MEMORY_TOKEN") {
        // Best effort: the in-memory store cannot fail.
        let _ = store.set(&token);
    }
    Some(std::sync::Arc::new(store))
}

#[cfg(not(feature = "dev-hooks"))]
fn memory_token_store() -> Option<std::sync::Arc<dyn hf2ollama_core::secrets::TokenStore>> {
    None
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
    // Probing the GPU is not free: once, then reused for the override.
    let detected = hf2ollama_core::hardware::detect();
    let machine = settings
        .hardware_override
        .clone()
        .unwrap_or_else(|| detected.clone());
    let default_staging = paths.default_staging.clone();
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
            let store = memory_token_store();
            #[cfg(feature = "dev-hooks")]
            let dev_store = store.is_some();
            let (tx, rx) = match store {
                Some(store) => runtime::Backend::spawn_with(
                    cc.egui_ctx.clone(),
                    settings.clone(),
                    paths,
                    store,
                ),
                None => runtime::Backend::spawn(cc.egui_ctx.clone(), settings.clone(), paths),
            };
            let mut app = app::App::new(&settings, staging_dir, machine, tx, rx);
            #[cfg(feature = "dev-hooks")]
            app.set_dev_memory_store(dev_store);
            app.set_environment(default_staging, detected);
            let (query, select) = preset();
            app.preset(query, select);
            if let Some(view) = view_override() {
                app.preset_view(view);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_one_enables_the_memory_store() {
        assert!(!dev_memory_store_enabled(None));
        for v in ["", "0", "true", "yes", " 1", "1 ", "11"] {
            assert!(!dev_memory_store_enabled(Some(v)), "{v:?}");
        }
        assert!(dev_memory_store_enabled(Some("1")));
    }

    #[cfg(not(feature = "dev-hooks"))]
    #[test]
    fn memory_token_store_is_absent_without_the_dev_hooks_feature() {
        std::env::set_var("HF2O_MEMORY_TOKEN_STORE", "1");
        std::env::set_var("HF2O_MEMORY_TOKEN", "hf_not_used");
        assert!(memory_token_store().is_none());
    }
}
