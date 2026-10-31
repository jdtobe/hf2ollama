//! Semantic colour tokens for every theme family (spec: "Theme-family tokens").
use eframe::egui::Color32;
use roost_core::config::ThemeFamily;

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
    /// A second hue for links and information, so the accent is not used
    /// for everything. `accent2_soft` is its background.
    pub accent2: Color32,
    pub accent2_soft: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub warn_soft: Color32,
    pub bad: Color32,
    pub bad_soft: Color32,
}

impl Tokens {
    /// The sidebar's list: a third of the way from the surface to the theme's light tint.
    pub fn side_list(&self) -> Color32 {
        self.surface.lerp_to_gamma(self.accent_soft, 0.35)
    }

    /// The band behind the sidebar's search field and buttons, and behind the
    /// header of every page beside it: the full tint.
    pub fn side_band(&self) -> Color32 {
        self.accent_soft
    }

    /// The sidebar's divider on its right edge: a darker tint.
    pub fn side_edge(&self) -> Color32 {
        self.accent_soft.lerp_to_gamma(self.accent, 0.3)
    }
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The family-specific colours, in table order: bg, surface, surface2, line,
/// fg, fg2, fg3, accent, accent_fg, accent_soft, accent2, accent2_soft.
struct Base([u32; 12]);

const fn tokens_from(b: Base, dark: bool) -> Tokens {
    let c = b.0;
    let (ok, ok_soft, warn, warn_soft, bad, bad_soft) = if dark {
        (0x5CC97B, 0x15301C, 0xE5B04A, 0x33270E, 0xF2766B, 0x3A1D1B)
    } else {
        (0x1B7F3B, 0xE0F3E5, 0x9A6200, 0xFBEFD3, 0xB3261E, 0xFBE3E1)
    };
    Tokens {
        bg: hex(c[0]),
        surface: hex(c[1]),
        surface2: hex(c[2]),
        line: hex(c[3]),
        fg: hex(c[4]),
        fg2: hex(c[5]),
        fg3: hex(c[6]),
        accent: hex(c[7]),
        accent_fg: hex(c[8]),
        accent_soft: hex(c[9]),
        accent2: hex(c[10]),
        accent2_soft: hex(c[11]),
        ok: hex(ok),
        ok_soft: hex(ok_soft),
        warn: hex(warn),
        warn_soft: hex(warn_soft),
        bad: hex(bad),
        bad_soft: hex(bad_soft),
    }
}

pub const MEADOW_LIGHT: Tokens = tokens_from(
    Base([
        0xEEF0F2, 0xFFFFFF, 0xF5F6F8, 0xDDE0E5, 0x16191D, 0x535A64, 0x6F7782, 0x0B6E66, 0xFFFFFF,
        0xDFF1EE, 0x3D5AA8, 0xE3E9F6,
    ]),
    false,
);
pub const MEADOW_DARK: Tokens = tokens_from(
    Base([
        0x101316, 0x171B1F, 0x1D2227, 0x2A3036, 0xE7EAEE, 0xA3ABB5, 0x7E8793, 0x3CC2B3, 0x06201D,
        0x14312E, 0x8DA7F0, 0x1F2842,
    ]),
    true,
);
pub const EMBER_LIGHT: Tokens = tokens_from(
    Base([
        0xF3EFEA, 0xFFFFFF, 0xF8F4F0, 0xE3DAD1, 0x211813, 0x6B5A50, 0x7B675B, 0xC2410C, 0xFFFFFF,
        0xFBE4D8, 0x0F766E, 0xD9F0EC,
    ]),
    false,
);
pub const EMBER_DARK: Tokens = tokens_from(
    Base([
        0x15100D, 0x1F1814, 0x281F1A, 0x352821, 0xF4EDE6, 0xB8A69A, 0x938175, 0xFB923C, 0x2A0F03,
        0x3A2418, 0x5EEAD4, 0x123430,
    ]),
    true,
);
pub const PLUM_LIGHT: Tokens = tokens_from(
    Base([
        0xF2EFF4, 0xFFFFFF, 0xF7F3F9, 0xDED7E4, 0x19131F, 0x63586B, 0x74657E, 0x7E22CE, 0xFFFFFF,
        0xF0E3FA, 0xBE185D, 0xFBE1EC,
    ]),
    false,
);
pub const PLUM_DARK: Tokens = tokens_from(
    Base([
        0x120E18, 0x1B1522, 0x241C2C, 0x30263B, 0xEFE9F5, 0xB3A6C0, 0x9183A0, 0xC084FC, 0x24112F,
        0x2E1D3E, 0xF472B6, 0x3A1A2A,
    ]),
    true,
);
pub const TIDE_LIGHT: Tokens = tokens_from(
    Base([
        0xEDF1F4, 0xFFFFFF, 0xF4F7F9, 0xD8E0E7, 0x121A20, 0x52636F, 0x647783, 0x0369A1, 0xFFFFFF,
        0xDCEEF8, 0x6D4AD1, 0xECE6FA,
    ]),
    false,
);
pub const TIDE_DARK: Tokens = tokens_from(
    Base([
        0x0D1216, 0x141B21, 0x1B252C, 0x26323B, 0xE6EDF3, 0x9AAAB7, 0x8194A1, 0x38BDF8, 0x08202D,
        0x112D3D, 0xA99BF5, 0x272046,
    ]),
    true,
);

#[cfg(test)]
pub const ALL_FAMILIES: [ThemeFamily; 4] = [
    ThemeFamily::Meadow,
    ThemeFamily::Ember,
    ThemeFamily::Plum,
    ThemeFamily::Tide,
];

pub fn palette(family: ThemeFamily, dark: bool) -> Tokens {
    match (family, dark) {
        (ThemeFamily::Meadow, false) => MEADOW_LIGHT,
        (ThemeFamily::Meadow, true) => MEADOW_DARK,
        (ThemeFamily::Ember, false) => EMBER_LIGHT,
        (ThemeFamily::Ember, true) => EMBER_DARK,
        (ThemeFamily::Plum, false) => PLUM_LIGHT,
        (ThemeFamily::Plum, true) => PLUM_DARK,
        (ThemeFamily::Tide, false) => TIDE_LIGHT,
        (ThemeFamily::Tide, true) => TIDE_DARK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(c: u8) -> f64 {
        let v = f64::from(c) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(c: Color32) -> f64 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    pub(crate) fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn contrast_helper_matches_known_ratios() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.01);
    }

    #[test]
    fn the_second_hue_differs_from_the_accent_in_every_palette() {
        for family in ALL_FAMILIES {
            for dark in [false, true] {
                let t = palette(family, dark);
                let tag = format!("{family:?} dark={dark}");
                assert_ne!(t.accent2, t.accent, "{tag}");
                assert_ne!(t.accent2_soft, t.accent_soft, "{tag}");
            }
        }
    }

    #[test]
    fn palettes_match_spec_table() {
        let rgb = Color32::from_rgb;
        let t = palette(ThemeFamily::Ember, false);
        assert_eq!(t.bg, rgb(0xF3, 0xEF, 0xEA));
        assert_eq!(t.fg3, rgb(0x7B, 0x67, 0x5B));
        assert_eq!(t.accent, rgb(0xC2, 0x41, 0x0C));
        assert_eq!(t.accent_soft, rgb(0xFB, 0xE4, 0xD8));
        let t = palette(ThemeFamily::Plum, true);
        assert_eq!(t.surface2, rgb(0x24, 0x1C, 0x2C));
        assert_eq!(t.accent_fg, rgb(0x24, 0x11, 0x2F));
        let t = palette(ThemeFamily::Tide, true);
        assert_eq!(t.line, rgb(0x26, 0x32, 0x3B));
        assert_eq!(t.fg2, rgb(0x9A, 0xAA, 0xB7));
        let t = palette(ThemeFamily::Meadow, false);
        assert_eq!(t.fg3, rgb(0x6F, 0x77, 0x82));
        assert_eq!(t.ok, rgb(0x1B, 0x7F, 0x3B));
        assert_eq!(t.bad_soft, rgb(0xFB, 0xE3, 0xE1));
        let t = palette(ThemeFamily::Meadow, true);
        assert_eq!(t.fg3, rgb(0x7E, 0x87, 0x93));
        assert_eq!(t.warn, rgb(0xE5, 0xB0, 0x4A));
        assert_eq!(t.bad, rgb(0xF2, 0x76, 0x6B));
    }

    #[test]
    fn all_eight_palettes_meet_wcag_aa_text_contrast() {
        for family in ALL_FAMILIES {
            for dark in [false, true] {
                let t = palette(family, dark);
                let tag = format!("{family:?} dark={dark}");
                for (name, fg, bg) in [
                    ("fg/surface", t.fg, t.surface),
                    ("fg2/surface", t.fg2, t.surface),
                    ("fg3/surface", t.fg3, t.surface),
                    ("accent-fg/accent", t.accent_fg, t.accent),
                    // The roles added with the second hue.
                    // Muted text on a band is fg2; fg3 is for plain surfaces.
                    ("fg2/accent-soft", t.fg2, t.accent_soft),
                    ("accent2/surface", t.accent2, t.surface),
                    ("accent2/surface2", t.accent2, t.surface2),
                    ("accent2/accent2-soft", t.accent2, t.accent2_soft),
                    ("fg/accent-soft", t.fg, t.accent_soft),
                ] {
                    let c = contrast(fg, bg);
                    assert!(c >= 4.5, "{tag}: {name} is {c:.2}:1");
                }
                // Accent-coloured tags are short, bold labels: 4:1 is enough.
                let c = contrast(t.accent, t.accent_soft);
                assert!(c >= 4.0, "{tag}: accent/accent-soft is {c:.2}:1");
            }
        }
    }
}
