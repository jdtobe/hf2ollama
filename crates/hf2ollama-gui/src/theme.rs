//! Design tokens (spec section 7.6), bundled fonts and egui style installation.
#![allow(unused_imports)]

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme};
use hf2ollama_core::config::ThemeChoice;

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

const PLACEHOLDER: Tokens = Tokens {
    bg: Color32::BLACK,
    surface: Color32::BLACK,
    surface2: Color32::BLACK,
    line: Color32::BLACK,
    fg: Color32::BLACK,
    fg2: Color32::BLACK,
    fg3: Color32::BLACK,
    accent: Color32::BLACK,
    accent_fg: Color32::BLACK,
    accent_soft: Color32::BLACK,
    ok: Color32::BLACK,
    ok_soft: Color32::BLACK,
    warn: Color32::BLACK,
    warn_soft: Color32::BLACK,
};
pub const LIGHT: Tokens = PLACEHOLDER;
pub const DARK: Tokens = PLACEHOLDER;

pub fn theme_preference(_choice: ThemeChoice) -> egui::ThemePreference {
    todo!()
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
