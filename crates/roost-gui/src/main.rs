mod app;
mod browse;
mod chat;
mod confirm;
mod dock;
mod icons;
mod identity;
mod installed;
mod menu;
mod quit;
#[cfg(test)]
mod quit_tests;
mod runtime;
mod settings;
mod startup;
mod theme;
mod theme_tokens;
mod widgets;

use eframe::egui;
use roost_core::config::{ThemeFamily, ThemeMode};
use roost_core::paths::OsDirs;
use std::io::Cursor;

/// `ROOST_THEME` overrides the saved theme: `light` or `dark` change only the
/// mode, `<family>-<mode>` (for example `ember-dark`) change both. Only meant
/// for taking screenshots; it is not a user-facing setting.
fn theme_override() -> Option<(Option<ThemeFamily>, ThemeMode)> {
    theme_override_with(|name| std::env::var(name).ok())
}

fn theme_override_with(
    lookup: impl FnOnce(&str) -> Option<String>,
) -> Option<(Option<ThemeFamily>, ThemeMode)> {
    let value = lookup("ROOST_THEME")?;
    let (family, mode) = match value.split_once('-') {
        Some((family, mode)) => (Some(family), mode),
        None => (None, value.as_str()),
    };
    let family = match family {
        None => None,
        Some("meadow") => Some(ThemeFamily::Meadow),
        Some("ember") => Some(ThemeFamily::Ember),
        Some("plum") => Some(ThemeFamily::Plum),
        Some("tide") => Some(ThemeFamily::Tide),
        Some(_) => return None,
    };
    match mode {
        "light" => Some((family, ThemeMode::Light)),
        "dark" => Some((family, ThemeMode::Dark)),
        _ => None,
    }
}

/// Smallest window the layout is designed and tested for (the detail pane
/// budget in `browse::detail` assumes at least this much room).
const MIN_WINDOW: [f32; 2] = [900.0, 640.0];

/// The 256 px PNG bundled with the app, for window managers that show a
/// per-window icon. The macOS bundle icon comes from `icon-1024.png`.
fn window_icon() -> Option<egui::IconData> {
    let mut decoder = png::Decoder::new(Cursor::new(include_bytes!("../assets/icon-256.png")));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(egui::IconData {
        rgba: buf,
        width: info.width,
        height: info.height,
    })
}

/// `ROOST_SIZE=WxH` overrides the initial window size, e.g. `900x640`. Only
/// meant for taking screenshots at several widths; it cannot go below
/// `MIN_WINDOW`.
fn size_override() -> Option<[f32; 2]> {
    size_override_with(|name| std::env::var(name).ok())
}

fn size_override_with(lookup: impl FnOnce(&str) -> Option<String>) -> Option<[f32; 2]> {
    let raw = lookup("ROOST_SIZE")?;
    let (w, h) = raw.split_once('x')?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    Some([w.max(MIN_WINDOW[0]), h.max(MIN_WINDOW[1])])
}

/// `ROOST_QUERY=text` fills the search box and `ROOST_SELECT=owner/name`
/// selects a repository at startup. Only meant for screenshots.
fn preset() -> (Option<String>, Option<String>) {
    preset_with(|name| std::env::var(name).ok())
}

fn preset_with(lookup: impl Fn(&str) -> Option<String>) -> (Option<String>, Option<String>) {
    (lookup("ROOST_QUERY"), lookup("ROOST_SELECT"))
}

/// `ROOST_VIEW=chat|installed|settings|browse` starts on that view. Only meant for
/// screenshots; inert unless set.
fn view_override() -> Option<app::View> {
    view_override_with(|name| std::env::var(name).ok())
}

fn view_override_with(lookup: impl FnOnce(&str) -> Option<String>) -> Option<app::View> {
    match lookup("ROOST_VIEW").as_deref() {
        Some("chat") => Some(app::View::Chat),
        Some("installed") => Some(app::View::Installed),
        Some("settings") => Some(app::View::Settings),
        Some("browse") => Some(app::View::Browse),
        _ => None,
    }
}

/// Only the exact value `1` enables the in-memory token store.
// Always compiled so it is tested in every build; only dev-hooks builds call it.
#[cfg_attr(not(feature = "dev-hooks"), allow(dead_code))]
fn dev_memory_store_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Dev-hooks builds only: `ROOST_MEMORY_TOKEN_STORE=1` keeps the Hugging Face
/// token in memory instead of the system keychain, starting with
/// `ROOST_MEMORY_TOKEN` when set. For screenshots and trials that must not
/// touch the real keychain. Without the `dev-hooks` feature this does not
/// exist, so a release binary cannot be switched to it.
#[cfg(feature = "dev-hooks")]
fn memory_token_store() -> Option<std::sync::Arc<dyn roost_core::secrets::TokenStore>> {
    memory_token_store_with(|name| std::env::var(name).ok())
}

#[cfg(feature = "dev-hooks")]
fn memory_token_store_with(
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<std::sync::Arc<dyn roost_core::secrets::TokenStore>> {
    use roost_core::secrets::{MemoryStore, TokenStore};
    let value = lookup("ROOST_MEMORY_TOKEN_STORE");
    if !dev_memory_store_enabled(value.as_deref()) {
        return None;
    }
    eprintln!("roost: dev-hooks: using an in-memory token store; the token will not be saved");
    let store = MemoryStore::default();
    if let Some(token) = lookup("ROOST_MEMORY_TOKEN") {
        // Best effort: the in-memory store cannot fail.
        let _ = store.set(&token);
    }
    Some(std::sync::Arc::new(store))
}

#[cfg(not(feature = "dev-hooks"))]
fn memory_token_store() -> Option<std::sync::Arc<dyn roost_core::secrets::TokenStore>> {
    None
}

fn main() -> eframe::Result<()> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let started = startup::startup(&exe_dir, OsDirs::detect(), OsDirs::detect_legacy());
    let startup::Startup {
        paths,
        migration,
        settings,
        staging_dir,
        notice,
    } = started;
    debug_assert!(
        migration.migrated.is_empty() && migration.warnings.is_empty() || notice.is_some(),
        "migration results must be visible to the UI as a startup notice"
    );
    if let Some(n) = &notice {
        startup::append_log(&paths.log_dir, n);
    }
    // Before any job exists, off the UI thread; it only removes job folders
    // that carry our marker.
    startup::spawn_sweep(staging_dir.clone(), paths.log_dir.clone());
    // Probing the GPU is not free: once, then reused for the override.
    let detected = roost_core::hardware::detect();
    let machine = settings
        .hardware_override
        .clone()
        .unwrap_or_else(|| detected.clone());
    let default_staging = paths.default_staging.clone();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(size_override().unwrap_or([1180.0, 820.0]))
        .with_min_inner_size(menu::bar::min_window(menu::Platform::current(), MIN_WINDOW))
        .with_title(identity::PRODUCT_NAME);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        identity::PRODUCT_NAME,
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let (family, mode) = theme_override().unwrap_or((None, settings.theme_mode));
            theme::apply(&cc.egui_ctx, family.unwrap_or(settings.theme_family), mode);
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
            app.set_startup_notice(notice);
            let (query, select) = preset();
            app.preset(query, select);
            // Chat is where people start; ROOST_VIEW still wins for screenshots.
            app.preset_view(view_override().unwrap_or(app::View::Chat));
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_loads_the_bundled_png() {
        let icon = window_icon().expect("bundled icon decodes");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(
            icon.rgba.as_chunks::<4>().0.iter().any(|p| p[3] == 255),
            "the icon must have opaque pixels"
        );
        assert_eq!(
            icon.rgba[3], 0,
            "the rounded SVG tile has transparent corners"
        );
        assert!(
            icon.rgba
                .as_chunks::<4>()
                .0
                .contains(&[0x10, 0x20, 0x1d, 255]),
            "the canonical dark SVG background is present"
        );
        assert!(
            icon.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[..3] == [0x3c, 0xc2, 0xb3]),
            "the canonical teal stroke is present"
        );
    }

    #[test]
    fn theme_override_accepts_every_family_and_mode() {
        let theme = |v: &'static str| {
            theme_override_with(move |name| (name == "ROOST_THEME").then(|| v.to_string()))
        };
        for (name, family) in [
            ("meadow", ThemeFamily::Meadow),
            ("ember", ThemeFamily::Ember),
            ("plum", ThemeFamily::Plum),
            ("tide", ThemeFamily::Tide),
        ] {
            let light: &'static str = Box::leak(format!("{name}-light").into_boxed_str());
            let dark: &'static str = Box::leak(format!("{name}-dark").into_boxed_str());
            assert_eq!(theme(light), Some((Some(family), ThemeMode::Light)));
            assert_eq!(theme(dark), Some((Some(family), ThemeMode::Dark)));
        }
        assert_eq!(theme("light"), Some((None, ThemeMode::Light)));
        assert_eq!(theme("rose-dark"), None);
        assert_eq!(theme("ember-system"), None);
        assert_eq!(theme(""), None);
    }

    #[test]
    fn only_exact_one_enables_the_memory_store() {
        assert!(!dev_memory_store_enabled(None));
        for v in ["", "0", "true", "yes", " 1", "1 ", "11"] {
            assert!(!dev_memory_store_enabled(Some(v)), "{v:?}");
        }
        assert!(dev_memory_store_enabled(Some("1")));
    }

    #[test]
    fn environment_overrides_use_roost_names_through_an_injected_lookup() {
        let lookup = |name: &str| match name {
            "ROOST_THEME" => Some("dark".to_string()),
            "ROOST_SIZE" => Some("800x500".to_string()),
            "ROOST_QUERY" => Some("qwen".to_string()),
            "ROOST_SELECT" => Some("org/model".to_string()),
            "ROOST_VIEW" => Some("settings".to_string()),
            _ => None,
        };
        assert_eq!(theme_override_with(lookup), Some((None, ThemeMode::Dark)));
        assert_eq!(size_override_with(lookup), Some(MIN_WINDOW));
        assert_eq!(
            preset_with(lookup),
            (Some("qwen".into()), Some("org/model".into()))
        );
        assert_eq!(view_override_with(lookup), Some(app::View::Settings));
    }

    #[test]
    fn legacy_environment_names_are_not_aliases() {
        let legacy = |name: &str| match name {
            "HF2O_THEME" => Some("dark".to_string()),
            "HF2O_SIZE" => Some("1200x800".to_string()),
            "HF2O_QUERY" => Some("qwen".to_string()),
            "HF2O_SELECT" => Some("org/model".to_string()),
            "HF2O_VIEW" => Some("settings".to_string()),
            _ => None,
        };
        assert_eq!(theme_override_with(legacy), None);
        assert_eq!(size_override_with(legacy), None);
        assert_eq!(preset_with(legacy), (None, None));
        assert_eq!(view_override_with(legacy), None);
    }
}
