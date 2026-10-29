//! Design tokens (spec section 7.6), bundled fonts and egui style installation.
use crate::theme_tokens::palette;
pub use crate::theme_tokens::Tokens;
use eframe::egui::{self, vec2, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme};
use roost_core::config::{ThemeFamily, ThemeMode};
use std::sync::Arc;

/// Tokens for the theme the given `Ui` is currently drawn in.
pub fn tokens(ui: &egui::Ui) -> Tokens {
    palette(family(ui.ctx()), ui.visuals().dark_mode)
}

fn family_id() -> egui::Id {
    egui::Id::unique("roost-theme-family")
}

/// The family last passed to `apply` (Meadow before any).
pub fn family(ctx: &egui::Context) -> ThemeFamily {
    ctx.data(|d| d.get_temp(family_id()))
        .unwrap_or(ThemeFamily::Meadow)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn theme_preference(mode: ThemeMode) -> egui::ThemePreference {
    match mode {
        ThemeMode::System => egui::ThemePreference::System,
        ThemeMode::Light => egui::ThemePreference::Light,
        ThemeMode::Dark => egui::ThemePreference::Dark,
    }
}

/// Selects the family and mode; takes effect on the next frame.
pub fn apply(ctx: &egui::Context, family: ThemeFamily, mode: ThemeMode) {
    ctx.data_mut(|d| d.insert_temp(family_id(), family));
    install_styles(ctx, family);
    ctx.set_theme(theme_preference(mode));
}

/// Installs the bundled fonts and the default (Meadow) theme styles.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    install_styles(ctx, family(ctx));
}

fn install_styles(ctx: &egui::Context, family: ThemeFamily) {
    ctx.style_mut_of(Theme::Light, |s| style_for(s, &palette(family, false)));
    ctx.style_mut_of(Theme::Dark, |s| style_for(s, &palette(family, true)));
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data: [(&str, &[u8]); 4] = [
        ("hanken", include_bytes!("../assets/Hanken-400.ttf")),
        ("hanken-semi", include_bytes!("../assets/Hanken-600.ttf")),
        ("jb", include_bytes!("../assets/JetBrainsMono-400.ttf")),
        (
            "jb-medium",
            include_bytes!("../assets/JetBrainsMono-500.ttf"),
        ),
    ];
    for (name, bytes) in data {
        fonts
            .font_data
            .insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "hanken".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jb".into());
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["hanken-semi".into(), "hanken".into()],
    );
    ctx.set_fonts(fonts);
}

fn style_for(style: &mut egui::Style, t: &Tokens) {
    style.text_styles = [
        (TextStyle::Heading, semibold(22.0)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (TextStyle::Monospace, mono(13.0)),
    ]
    .into();
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(14.0, 7.0);
    style.interaction.selectable_labels = false;
    let v = &mut style.visuals;
    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.surface;
    v.faint_bg_color = t.surface2;
    v.text_edit_bg_color = Some(t.surface);
    v.hyperlink_color = t.accent2;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    let r = CornerRadius::same(6);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, t.surface),
        (&mut v.widgets.inactive, t.surface2),
        (&mut v.widgets.hovered, t.surface2),
        (&mut v.widgets.active, t.surface2),
    ] {
        w.corner_radius = r;
        w.weak_bg_fill = fill;
        w.bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, t.line);
        w.fg_stroke = Stroke::new(1.0, t.fg);
    }
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.fg3);
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_selects_family_for_tokens_and_styles() {
        let ctx = egui::Context::default();
        install(&ctx);
        assert_eq!(family(&ctx), ThemeFamily::Meadow);
        apply(&ctx, ThemeFamily::Ember, ThemeMode::Dark);
        assert_eq!(family(&ctx), ThemeFamily::Ember);
        let dark = palette(ThemeFamily::Ember, true);
        let light = palette(ThemeFamily::Ember, false);
        assert_eq!(ctx.style_of(Theme::Dark).visuals.panel_fill, dark.surface);
        assert_eq!(ctx.style_of(Theme::Light).visuals.panel_fill, light.surface);
        assert_eq!(
            ctx.style_of(Theme::Dark).visuals.selection.stroke.color,
            dark.accent
        );
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Dark
        );
    }

    #[test]
    fn theme_mode_maps_to_theme_preference() {
        assert_eq!(
            theme_preference(ThemeMode::System),
            egui::ThemePreference::System
        );
        assert_eq!(
            theme_preference(ThemeMode::Light),
            egui::ThemePreference::Light
        );
        assert_eq!(
            theme_preference(ThemeMode::Dark),
            egui::ThemePreference::Dark
        );
    }
}
