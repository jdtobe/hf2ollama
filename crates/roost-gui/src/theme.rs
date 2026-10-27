//! Design tokens (spec section 7.6), bundled fonts and egui style installation.
use eframe::egui::{
    self, vec2, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme,
};
use roost_core::config::ThemeChoice;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tokens {
    pub bg: Color32,
    pub surface: Color32,
    pub surface2: Color32,
    pub line: Color32,
    pub fg: Color32,
    pub fg2: Color32,
    pub fg3: Color32,
    pub accent: Color32,
    pub accent_fg: Color32,
    pub accent_soft: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const LIGHT: Tokens = Tokens {
    bg: rgb(0xee, 0xf0, 0xf2),
    surface: rgb(0xff, 0xff, 0xff),
    surface2: rgb(0xf5, 0xf6, 0xf8),
    line: rgb(0xdd, 0xe0, 0xe5),
    fg: rgb(0x16, 0x19, 0x1d),
    fg2: rgb(0x53, 0x5a, 0x64),
    fg3: rgb(0x8a, 0x91, 0x9b),
    accent: rgb(0x0b, 0x6e, 0x66),
    accent_fg: rgb(0xff, 0xff, 0xff),
    accent_soft: rgb(0xdf, 0xf1, 0xee),
    ok: rgb(0x1b, 0x7f, 0x3b),
    ok_soft: rgb(0xe0, 0xf3, 0xe5),
    warn: rgb(0x9a, 0x62, 0x00),
    warn_soft: rgb(0xfb, 0xef, 0xd3),
};

pub const DARK: Tokens = Tokens {
    bg: rgb(0x10, 0x13, 0x16),
    surface: rgb(0x17, 0x1b, 0x1f),
    surface2: rgb(0x1d, 0x22, 0x27),
    line: rgb(0x2a, 0x30, 0x36),
    fg: rgb(0xe7, 0xea, 0xee),
    fg2: rgb(0xa3, 0xab, 0xb5),
    fg3: rgb(0x6f, 0x77, 0x82),
    accent: rgb(0x3c, 0xc2, 0xb3),
    accent_fg: rgb(0x06, 0x20, 0x1d),
    accent_soft: rgb(0x14, 0x31, 0x2e),
    ok: rgb(0x5c, 0xc9, 0x7b),
    ok_soft: rgb(0x15, 0x30, 0x1c),
    warn: rgb(0xe5, 0xb0, 0x4a),
    warn_soft: rgb(0x33, 0x27, 0x0e),
};

/// Tokens for the theme the given `Ui` is currently drawn in.
pub fn tokens(ui: &egui::Ui) -> Tokens {
    if ui.visuals().dark_mode {
        DARK
    } else {
        LIGHT
    }
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn theme_preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    }
}

pub fn apply_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(theme_preference(choice));
}

/// Installs the bundled fonts, text styles and both theme styles.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.style_mut_of(Theme::Light, |s| style_for(s, &LIGHT));
    ctx.style_mut_of(Theme::Dark, |s| style_for(s, &DARK));
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
    v.hyperlink_color = t.accent;
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

    const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }

    #[test]
    fn tokens_light_and_dark_match_spec_table() {
        assert_eq!(LIGHT.bg, rgb(0xee, 0xf0, 0xf2));
        assert_eq!(LIGHT.surface, rgb(0xff, 0xff, 0xff));
        assert_eq!(LIGHT.surface2, rgb(0xf5, 0xf6, 0xf8));
        assert_eq!(LIGHT.line, rgb(0xdd, 0xe0, 0xe5));
        assert_eq!(LIGHT.fg, rgb(0x16, 0x19, 0x1d));
        assert_eq!(LIGHT.fg2, rgb(0x53, 0x5a, 0x64));
        assert_eq!(LIGHT.fg3, rgb(0x8a, 0x91, 0x9b));
        assert_eq!(LIGHT.accent, rgb(0x0b, 0x6e, 0x66));
        assert_eq!(LIGHT.accent_fg, rgb(0xff, 0xff, 0xff));
        assert_eq!(LIGHT.accent_soft, rgb(0xdf, 0xf1, 0xee));
        assert_eq!(LIGHT.ok, rgb(0x1b, 0x7f, 0x3b));
        assert_eq!(LIGHT.ok_soft, rgb(0xe0, 0xf3, 0xe5));
        assert_eq!(LIGHT.warn, rgb(0x9a, 0x62, 0x00));
        assert_eq!(LIGHT.warn_soft, rgb(0xfb, 0xef, 0xd3));

        assert_eq!(DARK.bg, rgb(0x10, 0x13, 0x16));
        assert_eq!(DARK.surface, rgb(0x17, 0x1b, 0x1f));
        assert_eq!(DARK.surface2, rgb(0x1d, 0x22, 0x27));
        assert_eq!(DARK.line, rgb(0x2a, 0x30, 0x36));
        assert_eq!(DARK.fg, rgb(0xe7, 0xea, 0xee));
        assert_eq!(DARK.fg2, rgb(0xa3, 0xab, 0xb5));
        assert_eq!(DARK.fg3, rgb(0x6f, 0x77, 0x82));
        assert_eq!(DARK.accent, rgb(0x3c, 0xc2, 0xb3));
        assert_eq!(DARK.accent_fg, rgb(0x06, 0x20, 0x1d));
        assert_eq!(DARK.accent_soft, rgb(0x14, 0x31, 0x2e));
        assert_eq!(DARK.ok, rgb(0x5c, 0xc9, 0x7b));
        assert_eq!(DARK.ok_soft, rgb(0x15, 0x30, 0x1c));
        assert_eq!(DARK.warn, rgb(0xe5, 0xb0, 0x4a));
        assert_eq!(DARK.warn_soft, rgb(0x33, 0x27, 0x0e));
    }

    #[test]
    fn theme_choice_maps_to_theme_preference() {
        assert_eq!(
            theme_preference(ThemeChoice::System),
            egui::ThemePreference::System
        );
        assert_eq!(
            theme_preference(ThemeChoice::Light),
            egui::ThemePreference::Light
        );
        assert_eq!(
            theme_preference(ThemeChoice::Dark),
            egui::ThemePreference::Dark
        );
    }
}
