//! A pure description of the menus. Platform renderers (the native menu
//! bar, the in-window bar) draw it; `actions::handle` acts on clicks.

use crate::app::{AppState, View};
use eframe::egui::Key;
use roost_core::config::{ThemeFamily, ThemeMode};

pub const REPO_URL: &str = "https://github.com/jdtobe/roost";
pub const ISSUES_URL: &str = "https://github.com/jdtobe/roost/issues";
const MODEL_NAME_CHARS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// A keyboard shortcut. `primary` is Command on macOS and Ctrl elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub primary: bool,
    pub shift: bool,
    pub key: Key,
}

impl Shortcut {
    const fn primary(key: Key) -> Shortcut {
        Shortcut {
            primary: true,
            shift: false,
            key,
        }
    }

    const fn primary_shift(key: Key) -> Shortcut {
        Shortcut {
            primary: true,
            shift: true,
            key,
        }
    }

    fn key_text(self) -> String {
        match self.key {
            Key::Comma => ",".to_string(),
            Key::Num1 => "1".to_string(),
            Key::Num2 => "2".to_string(),
            Key::Num3 => "3".to_string(),
            other => format!("{other:?}"),
        }
    }

    /// "⇧⌘N" on macOS, "Ctrl+Shift+N" elsewhere.
    pub fn display(self, platform: Platform) -> String {
        match platform {
            Platform::Mac => format!(
                "{}{}{}",
                if self.shift { "⇧" } else { "" },
                if self.primary { "⌘" } else { "" },
                self.key_text()
            ),
            _ => {
                let mut parts = Vec::new();
                if self.primary {
                    parts.push("Ctrl".to_string());
                }
                if self.shift {
                    parts.push("Shift".to_string());
                }
                parts.push(self.key_text());
                parts.join("+")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    About,
    Settings,
    Quit,
    NewChat,
    NewIncognito,
    ShowView(View),
    Cut,
    Copy,
    Paste,
    SelectAll,
    SetFamily(ThemeFamily),
    SetMode(ThemeMode),
    Unload(String),
    UnloadAll,
    Minimize,
    Zoom,
    OpenUrl(&'static str),
}

/// Items only macOS provides (the system fills in label and behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predefined {
    Services,
    Hide,
    HideOthers,
    ShowAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionItem {
    pub action: MenuAction,
    pub label: String,
    pub shortcut: Option<Shortcut>,
    pub enabled: bool,
    /// `Some` for radio-style items.
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Action(ActionItem),
    Separator,
    Submenu(Menu),
    Predefined(Predefined),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuModel {
    pub menus: Vec<Menu>,
}

fn action(label: &str, action: MenuAction, shortcut: Option<Shortcut>) -> Item {
    Item::Action(ActionItem {
        action,
        label: label.to_string(),
        shortcut,
        enabled: true,
        checked: None,
    })
}

fn gated(label: &str, a: MenuAction, shortcut: Option<Shortcut>, enabled: bool) -> Item {
    Item::Action(ActionItem {
        action: a,
        label: label.to_string(),
        shortcut,
        enabled,
        checked: None,
    })
}

fn radio(label: &str, a: MenuAction, checked: bool) -> Item {
    Item::Action(ActionItem {
        action: a,
        label: label.to_string(),
        shortcut: None,
        enabled: true,
        checked: Some(checked),
    })
}

fn short_name(name: &str) -> String {
    if name.chars().count() <= MODEL_NAME_CHARS {
        return name.to_string();
    }
    let cut: String = name.chars().take(MODEL_NAME_CHARS - 1).collect();
    format!("{cut}…")
}

/// The menus for the current state of the app.
pub fn build(app: &AppState, platform: Platform) -> MenuModel {
    let installed: Vec<String> = app
        .installed
        .rows()
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    let can_chat = app.chat.default_model(&installed).is_some();
    let mac = platform == Platform::Mac;
    let settings_shortcut = Some(Shortcut::primary(Key::Comma));
    let quit_shortcut = match platform {
        Platform::Windows => None,
        _ => Some(Shortcut::primary(Key::Q)),
    };

    let mut menus = Vec::new();
    if mac {
        menus.push(Menu {
            title: "Roost".into(),
            items: vec![
                action("About Roost", MenuAction::About, None),
                Item::Separator,
                action("Settings…", MenuAction::Settings, settings_shortcut),
                Item::Separator,
                Item::Predefined(Predefined::Services),
                Item::Separator,
                Item::Predefined(Predefined::Hide),
                Item::Predefined(Predefined::HideOthers),
                Item::Predefined(Predefined::ShowAll),
                Item::Separator,
                action("Quit Roost", MenuAction::Quit, quit_shortcut),
            ],
        });
    }

    let mut file = vec![
        gated(
            "New Chat",
            MenuAction::NewChat,
            Some(Shortcut::primary(Key::N)),
            can_chat,
        ),
        gated(
            "New Incognito Chat",
            MenuAction::NewIncognito,
            Some(Shortcut::primary_shift(Key::N)),
            can_chat,
        ),
    ];
    if !mac {
        file.push(Item::Separator);
        file.push(action("Settings…", MenuAction::Settings, settings_shortcut));
        file.push(Item::Separator);
        file.push(action("Exit", MenuAction::Quit, quit_shortcut));
    }
    menus.push(Menu {
        title: "File".into(),
        items: file,
    });

    // Windows leaves the edit shortcuts to the text fields themselves.
    let edit_key = |k: Key| (platform != Platform::Windows).then(|| Shortcut::primary(k));
    menus.push(Menu {
        title: "Edit".into(),
        items: vec![
            action("Cut", MenuAction::Cut, edit_key(Key::X)),
            action("Copy", MenuAction::Copy, edit_key(Key::C)),
            action("Paste", MenuAction::Paste, edit_key(Key::V)),
            Item::Separator,
            action("Select All", MenuAction::SelectAll, edit_key(Key::A)),
        ],
    });

    let families = [
        ("Meadow", ThemeFamily::Meadow),
        ("Ember", ThemeFamily::Ember),
        ("Plum", ThemeFamily::Plum),
        ("Tide", ThemeFamily::Tide),
    ];
    let modes = [
        ("System", ThemeMode::System),
        ("Light", ThemeMode::Light),
        ("Dark", ThemeMode::Dark),
    ];
    let mut theme = Vec::new();
    for (label, f) in families {
        theme.push(radio(
            label,
            MenuAction::SetFamily(f),
            app.settings.theme_family == f,
        ));
    }
    theme.push(Item::Separator);
    for (label, m) in modes {
        theme.push(radio(
            label,
            MenuAction::SetMode(m),
            app.settings.theme_mode == m,
        ));
    }
    menus.push(Menu {
        title: "View".into(),
        items: vec![
            action(
                "Chat",
                MenuAction::ShowView(View::Chat),
                Some(Shortcut::primary(Key::Num1)),
            ),
            action(
                "Browse",
                MenuAction::ShowView(View::Browse),
                Some(Shortcut::primary(Key::Num2)),
            ),
            action(
                "Installed",
                MenuAction::ShowView(View::Installed),
                Some(Shortcut::primary(Key::Num3)),
            ),
            action("Settings", MenuAction::ShowView(View::Settings), None),
            Item::Separator,
            Item::Submenu(Menu {
                title: "Theme".into(),
                items: theme,
            }),
        ],
    });

    let mut models = Vec::new();
    if app.running.is_empty() {
        models.push(gated("Nothing loaded", MenuAction::UnloadAll, None, false));
    } else {
        for m in &app.running {
            models.push(action(
                &format!("Unload {}", short_name(&m.name)),
                MenuAction::Unload(m.name.clone()),
                None,
            ));
        }
        models.push(Item::Separator);
        models.push(action("Unload All", MenuAction::UnloadAll, None));
    }
    menus.push(Menu {
        title: "Models".into(),
        items: models,
    });

    if mac {
        menus.push(Menu {
            title: "Window".into(),
            items: vec![
                action(
                    "Minimize",
                    MenuAction::Minimize,
                    Some(Shortcut::primary(Key::M)),
                ),
                action("Zoom", MenuAction::Zoom, None),
            ],
        });
    }

    let mut help = Vec::new();
    if !mac {
        help.push(action("About Roost", MenuAction::About, None));
        help.push(Item::Separator);
    }
    help.push(action(
        "Roost on GitHub",
        MenuAction::OpenUrl(REPO_URL),
        None,
    ));
    help.push(action(
        "Report an Issue",
        MenuAction::OpenUrl(ISSUES_URL),
        None,
    ));
    menus.push(Menu {
        title: "Help".into(),
        items: help,
    });
    MenuModel { menus }
}

impl MenuModel {
    /// Every action item, nested ones included.
    pub fn action_items(&self) -> Vec<&ActionItem> {
        fn walk<'a>(items: &'a [Item], out: &mut Vec<&'a ActionItem>) {
            for i in items {
                match i {
                    Item::Action(a) => out.push(a),
                    Item::Submenu(m) => walk(&m.items, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            walk(&m.items, &mut out);
        }
        out
    }

    #[cfg(test)]
    pub fn menu(&self, title: &str) -> Option<&Menu> {
        self.menus.iter().find(|m| m.title == title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roost_core::ollama::RunningModel;
    use std::collections::HashSet;

    fn titles(m: &MenuModel) -> Vec<&str> {
        m.menus.iter().map(|m| m.title.as_str()).collect()
    }

    fn labels(m: &Menu) -> Vec<String> {
        m.items
            .iter()
            .filter_map(|i| match i {
                Item::Action(a) => Some(a.label.clone()),
                _ => None,
            })
            .collect()
    }

    fn app_with_a_model() -> AppState {
        let mut app = AppState::new();
        app.chat.fold(roost_core::chat::ChatEvent::Rows(vec![]));
        app.installed
            .on_installed_loaded(&Ok(vec![crate::runtime::InstalledEntry {
                model: roost_core::ollama::InstalledModel {
                    name: "llama3:8b".into(),
                    size: 1,
                    params: None,
                    quant: None,
                    family: None,
                    modified: String::new(),
                    digest: String::new(),
                },
                source_repo: None,
            }]));
        app
    }

    #[test]
    fn mac_has_an_app_menu_and_a_window_menu_the_others_do_not() {
        let app = AppState::new();
        assert_eq!(
            titles(&build(&app, Platform::Mac)),
            ["Roost", "File", "Edit", "View", "Models", "Window", "Help"]
        );
        for p in [Platform::Windows, Platform::Linux] {
            assert_eq!(
                titles(&build(&app, p)),
                ["File", "Edit", "View", "Models", "Help"]
            );
        }
    }

    #[test]
    fn about_and_settings_exist_on_every_platform() {
        let app = AppState::new();
        for p in [Platform::Mac, Platform::Windows, Platform::Linux] {
            let m = build(&app, p);
            let actions: Vec<_> = m.action_items().iter().map(|a| a.action.clone()).collect();
            assert!(actions.contains(&MenuAction::About), "{p:?}");
            assert!(actions.contains(&MenuAction::Settings), "{p:?}");
            assert!(actions.contains(&MenuAction::Quit), "{p:?}");
        }
    }

    #[test]
    fn shortcuts_are_unique_on_each_platform() {
        let app = AppState::new();
        for p in [Platform::Mac, Platform::Windows, Platform::Linux] {
            let m = build(&app, p);
            let mut seen = HashSet::new();
            for a in m.action_items() {
                if let Some(s) = a.shortcut {
                    assert!(seen.insert(s), "{p:?}: duplicate {s:?} ({})", a.label);
                }
            }
        }
    }

    #[test]
    fn shortcuts_display_the_platform_way() {
        let s = Shortcut::primary_shift(Key::N);
        assert_eq!(s.display(Platform::Mac), "⇧⌘N");
        assert_eq!(s.display(Platform::Linux), "Ctrl+Shift+N");
        assert_eq!(Shortcut::primary(Key::Comma).display(Platform::Mac), "⌘,");
        assert_eq!(
            Shortcut::primary(Key::Num1).display(Platform::Windows),
            "Ctrl+1"
        );
    }

    #[test]
    fn windows_leaves_edit_shortcuts_to_the_text_fields() {
        let app = AppState::new();
        let edit = |p| {
            build(&app, p)
                .menu("Edit")
                .unwrap()
                .items
                .iter()
                .filter_map(|i| match i {
                    Item::Action(a) => Some(a.shortcut),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert!(edit(Platform::Windows).iter().all(Option::is_none));
        assert!(edit(Platform::Mac).iter().all(Option::is_some));
    }

    #[test]
    fn new_chat_needs_an_installed_model() {
        let none = build(&AppState::new(), Platform::Linux);
        let file = none.menu("File").unwrap();
        for item in &file.items {
            if let Item::Action(a) = item {
                if matches!(a.action, MenuAction::NewChat | MenuAction::NewIncognito) {
                    assert!(!a.enabled, "{}", a.label);
                }
            }
        }
        let some = build(&app_with_a_model(), Platform::Linux);
        let on: Vec<_> = some
            .action_items()
            .into_iter()
            .filter(|a| matches!(a.action, MenuAction::NewChat | MenuAction::NewIncognito))
            .map(|a| a.enabled)
            .collect();
        assert_eq!(on, [true, true]);
    }

    #[test]
    fn theme_items_are_radio_checked_from_the_settings() {
        let mut app = AppState::new();
        app.settings.theme_family = ThemeFamily::Plum;
        app.settings.theme_mode = ThemeMode::Dark;
        let m = build(&app, Platform::Linux);
        let checked: Vec<_> = m
            .action_items()
            .into_iter()
            .filter(|a| a.checked == Some(true))
            .map(|a| a.action.clone())
            .collect();
        assert_eq!(
            checked,
            [
                MenuAction::SetFamily(ThemeFamily::Plum),
                MenuAction::SetMode(ThemeMode::Dark)
            ]
        );
        let theme_items = m
            .action_items()
            .into_iter()
            .filter(|a| a.checked.is_some())
            .count();
        assert_eq!(theme_items, 7);
    }

    #[test]
    fn the_models_menu_lists_loaded_models_or_a_disabled_placeholder() {
        let mut app = AppState::new();
        let empty = build(&app, Platform::Linux);
        let m = empty.menu("Models").unwrap();
        assert_eq!(labels(m), ["Nothing loaded"]);
        let Item::Action(a) = &m.items[0] else {
            panic!()
        };
        assert!(!a.enabled);

        app.running = vec![
            RunningModel {
                name: "llama3:8b".into(),
                size: 1,
                size_vram: 1,
                expires_at: None,
            },
            RunningModel {
                name: "x".repeat(80),
                size: 1,
                size_vram: 1,
                expires_at: None,
            },
        ];
        let loaded = build(&app, Platform::Linux);
        let m = loaded.menu("Models").unwrap();
        let l = labels(m);
        assert_eq!(l[0], "Unload llama3:8b");
        assert!(l[1].chars().count() <= "Unload ".len() + 40, "{}", l[1]);
        assert_eq!(l.last().unwrap(), "Unload All");
        let Item::Action(first) = &m.items[0] else {
            panic!()
        };
        assert_eq!(first.action, MenuAction::Unload("llama3:8b".into()));
    }

    #[test]
    fn help_links_point_at_the_project() {
        let m = build(&AppState::new(), Platform::Linux);
        let urls: Vec<_> = m
            .action_items()
            .iter()
            .filter_map(|a| match a.action {
                MenuAction::OpenUrl(u) => Some(u),
                _ => None,
            })
            .collect();
        assert_eq!(urls, [REPO_URL, ISSUES_URL]);
        assert!(urls.iter().all(|u| u.starts_with("https://github.com/")));
    }
}
