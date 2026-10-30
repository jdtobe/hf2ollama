//! The in-window menu bar (Linux, and the tests), and shortcut dispatch for
//! platforms whose menus do not dispatch shortcuts themselves.

use super::model::{ActionItem, Item, Menu, MenuAction, MenuModel, Platform};
use crate::theme::tokens;
use eframe::egui::{self, vec2, KeyboardShortcut, Margin, Modifiers, RichText};

/// Room the in-window bar needs (measured: 27 px), so the window can grow by it.
pub const BAR_H: f32 = 28.0;

pub struct BarOutput {
    pub actions: Vec<MenuAction>,
    /// Where each menu title was painted (for layout tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub titles: Vec<(String, egui::Rect)>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub rect: egui::Rect,
}

/// Draws the bar as a top panel and returns the clicked actions.
pub fn show(ui: &mut egui::Ui, model: &MenuModel, platform: Platform) -> BarOutput {
    let t = tokens(ui);
    let mut actions = Vec::new();
    let mut titles = Vec::new();
    let panel = egui::Panel::top("menubar")
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .inner_margin(Margin::symmetric(8, 2)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for menu in &model.menus {
                    let r = ui.menu_button(RichText::new(&menu.title).size(13.0), |ui| {
                        draw_items(ui, &menu.items, platform, &mut actions);
                    });
                    titles.push((menu.title.clone(), r.response.rect));
                }
            });
        });
    BarOutput {
        actions,
        titles,
        rect: panel.response.rect,
    }
}

fn draw_items(ui: &mut egui::Ui, items: &[Item], platform: Platform, out: &mut Vec<MenuAction>) {
    for item in items {
        match item {
            Item::Separator => {
                ui.separator();
            }
            Item::Submenu(Menu { title, items }) => {
                ui.menu_button(title, |ui| draw_items(ui, items, platform, out));
            }
            // macOS-only items never appear in this bar.
            Item::Predefined(_) => {}
            Item::Action(a) => {
                if draw_action(ui, a, platform) {
                    out.push(a.action.clone());
                    ui.close();
                }
            }
        }
    }
}

fn draw_action(ui: &mut egui::Ui, a: &ActionItem, platform: Platform) -> bool {
    let label = match a.checked {
        Some(true) => format!("✓ {}", a.label),
        Some(false) => format!("   {}", a.label),
        None => a.label.clone(),
    };
    let mut button = egui::Button::new(label).min_size(vec2(180.0, 0.0));
    if let Some(s) = a.shortcut {
        button = button.shortcut_text(s.display(platform));
    }
    ui.add_enabled(a.enabled, button).clicked()
}

/// Shortcut presses for the enabled actions in `model`. The Edit shortcuts are
/// left to the text fields, which already handle them. Call once per frame.
pub fn shortcuts(ctx: &egui::Context, model: &MenuModel) -> Vec<MenuAction> {
    let mut candidates: Vec<(&ActionItem, KeyboardShortcut)> = model
        .action_items()
        .into_iter()
        .filter(|a| a.enabled)
        .filter(|a| {
            !matches!(
                a.action,
                MenuAction::Cut | MenuAction::Copy | MenuAction::Paste | MenuAction::SelectAll
            )
        })
        .filter_map(|a| {
            let s = a.shortcut?;
            let mut mods = Modifiers::COMMAND;
            if s.shift {
                mods |= Modifiers::SHIFT;
            }
            Some((a, KeyboardShortcut::new(mods, s.key)))
        })
        .collect();
    // Most specific first, so Ctrl+Shift+N is not taken for Ctrl+N.
    candidates.sort_by_key(|(a, _)| std::cmp::Reverse(a.shortcut.map(|s| s.shift)));
    let mut fired = Vec::new();
    ctx.input_mut(|i| {
        for (a, shortcut) in &candidates {
            if i.consume_shortcut(shortcut) {
                fired.push(a.action.clone());
            }
        }
    });
    fired
}

/// The smallest window: the content keeps its designed minimum size, so a
/// window with an in-window bar is taller by the bar.
pub fn min_window(platform: Platform, content: [f32; 2]) -> [f32; 2] {
    if uses_window_bar(platform) {
        [content[0], content[1] + BAR_H]
    } else {
        content
    }
}

/// Whether this platform shows the in-window bar.
pub fn uses_window_bar(platform: Platform) -> bool {
    platform == Platform::Linux
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::browse::test_util::Harness;
    use crate::menu::model::build;
    use eframe::egui::{Event, Key};
    use roost_core::config::ThemeFamily;

    fn key(k: Key, mods: Modifiers) -> Vec<Event> {
        vec![Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: mods,
        }]
    }

    fn fired(model: &MenuModel, events: Vec<Event>) -> Vec<MenuAction> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        let full = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| out = shortcuts(ui.ctx(), model),
        );
        full.drop_without_applying_deltas();
        out
    }

    fn model_with_chat_enabled() -> MenuModel {
        let mut app = AppState::new();
        app.installed
            .on_installed_loaded(&Ok(vec![crate::runtime::InstalledEntry {
                model: roost_core::ollama::InstalledModel {
                    name: "m:1".into(),
                    size: 1,
                    params: None,
                    quant: None,
                    family: None,
                    modified: String::new(),
                    digest: String::new(),
                },
                source_repo: None,
            }]));
        build(&app, Platform::Linux)
    }

    #[test]
    fn ctrl_n_and_ctrl_shift_n_are_told_apart() {
        let m = model_with_chat_enabled();
        assert_eq!(
            fired(&m, key(Key::N, Modifiers::COMMAND)),
            [MenuAction::NewChat]
        );
        assert_eq!(
            fired(&m, key(Key::N, Modifiers::COMMAND | Modifiers::SHIFT)),
            [MenuAction::NewIncognito]
        );
    }

    #[test]
    fn other_shortcuts_fire_their_actions() {
        let m = model_with_chat_enabled();
        assert_eq!(
            fired(&m, key(Key::Num2, Modifiers::COMMAND)),
            [MenuAction::ShowView(crate::app::View::Work)]
        );
        assert_eq!(
            fired(&m, key(Key::Num3, Modifiers::COMMAND)),
            [MenuAction::ShowView(crate::app::View::Models)]
        );
        assert_eq!(
            fired(&m, key(Key::Comma, Modifiers::COMMAND)),
            [MenuAction::Settings]
        );
        assert_eq!(
            fired(&m, key(Key::Q, Modifiers::COMMAND)),
            [MenuAction::Quit]
        );
    }

    #[test]
    fn disabled_items_and_edit_shortcuts_do_not_fire() {
        let none = build(&AppState::new(), Platform::Linux);
        assert!(fired(&none, key(Key::N, Modifiers::COMMAND)).is_empty());
        let m = model_with_chat_enabled();
        for k in [Key::X, Key::C, Key::V, Key::A] {
            assert!(fired(&m, key(k, Modifiers::COMMAND)).is_empty(), "{k:?}");
        }
        assert!(fired(&m, key(Key::N, Modifiers::NONE)).is_empty());
    }

    #[test]
    fn the_minimum_window_grows_by_the_bar_on_linux_only() {
        assert_eq!(
            min_window(Platform::Linux, [900.0, 640.0]),
            [900.0, 640.0 + BAR_H]
        );
        assert_eq!(min_window(Platform::Mac, [900.0, 640.0]), [900.0, 640.0]);
        assert_eq!(
            min_window(Platform::Windows, [900.0, 640.0]),
            [900.0, 640.0]
        );
    }

    #[test]
    fn only_linux_uses_the_window_bar() {
        assert!(uses_window_bar(Platform::Linux));
        assert!(!uses_window_bar(Platform::Mac));
        assert!(!uses_window_bar(Platform::Windows));
    }

    #[test]
    fn the_bar_is_one_row_inside_the_window_in_every_theme() {
        let model = build(&AppState::new(), Platform::Linux);
        for family in [
            ThemeFamily::Meadow,
            ThemeFamily::Ember,
            ThemeFamily::Plum,
            ThemeFamily::Tide,
        ] {
            for dark in [false, true] {
                for size in [(900.0, 640.0), (1180.0, 820.0)] {
                    let mut h = Harness::themed(family, dark);
                    let mut out = None;
                    for _ in 0..3 {
                        h.frame(vec2(size.0, size.1), vec![], |ui| {
                            out = Some(show(ui, &model, Platform::Linux));
                        });
                    }
                    let out = out.unwrap();
                    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(size.0, size.1));
                    let tag = format!("{family:?} dark={dark} {size:?}");
                    assert!(window.contains_rect(out.rect), "{tag}: {:?}", out.rect);
                    assert!(out.rect.height() <= BAR_H, "{tag}: {:?}", out.rect);
                    assert_eq!(out.titles.len(), model.menus.len(), "{tag}");
                    let mut right = f32::MIN;
                    for (name, r) in &out.titles {
                        assert!(out.rect.contains_rect(*r), "{tag}: {name} {r:?}");
                        assert!(r.left() >= right - 0.5, "{tag}: {name} overlaps");
                        right = r.right();
                    }
                    let cy: Vec<f32> = out.titles.iter().map(|(_, r)| r.center().y).collect();
                    let spread = cy.iter().cloned().fold(f32::MIN, f32::max)
                        - cy.iter().cloned().fold(f32::MAX, f32::min);
                    assert!(spread <= 1.0, "{tag}: titles not on one line ({cy:?})");
                }
            }
        }
    }

    #[test]
    fn opening_a_menu_lists_its_items() {
        let model = build(&AppState::new(), Platform::Linux);
        let mut h = Harness::new(false);
        let size = vec2(900.0, 640.0);
        let mut titles = Vec::new();
        for _ in 0..2 {
            h.frame(size, vec![], |ui| {
                titles = show(ui, &model, Platform::Linux).titles;
            });
        }
        let file = titles.iter().find(|(n, _)| n == "File").unwrap().1;
        let at = file.center();
        h.frame(size, vec![Event::PointerMoved(at)], |ui| {
            show(ui, &model, Platform::Linux);
        });
        for pressed in [true, false] {
            h.frame(
                size,
                vec![Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }],
                |ui| {
                    show(ui, &model, Platform::Linux);
                },
            );
        }
        let texts = h.frame_texts(size, vec![], |ui| {
            show(ui, &model, Platform::Linux);
        });
        let all = texts.join("\n");
        assert!(all.contains("New Chat"), "{all}");
        assert!(all.contains("Ctrl+Shift+N"), "{all}");
        assert!(all.contains("Exit"), "{all}");
    }
}
