//! The native menu bar (macOS and Windows), built from the menu model with
//! `muda`. Structure changes rebuild the menus; changes to enabled or checked
//! states are applied in place so an open menu is not disturbed.

use super::model::{ActionItem, Item, Menu, MenuAction, MenuModel, Platform, Predefined, Shortcut};
use eframe::egui::{self, Key};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

enum Handle {
    Plain(MenuItem),
    Check(CheckMenuItem),
}

pub struct NativeMenu {
    menu: muda::Menu,
    platform: Platform,
    /// The structure last built: labels and shortcuts, not states.
    shape: Vec<String>,
    states: Vec<(bool, Option<bool>)>,
    handles: Vec<Handle>,
    actions: HashMap<MenuId, MenuAction>,
    events: Receiver<MenuId>,
}

fn code_for(key: Key) -> Option<Code> {
    Some(match key {
        Key::A => Code::KeyA,
        Key::C => Code::KeyC,
        Key::M => Code::KeyM,
        Key::N => Code::KeyN,
        Key::Q => Code::KeyQ,
        Key::V => Code::KeyV,
        Key::X => Code::KeyX,
        Key::Comma => Code::Comma,
        Key::Num1 => Code::Digit1,
        Key::Num2 => Code::Digit2,
        Key::Num3 => Code::Digit3,
        _ => return None,
    })
}

/// macOS dispatches shortcuts in the menu. Windows does not register any: the
/// window handles them, so a shortcut can never fire twice.
fn accelerator(platform: Platform, s: Option<Shortcut>) -> Option<Accelerator> {
    let s = s?;
    if platform != Platform::Mac {
        return None;
    }
    let mut mods = Modifiers::META;
    if s.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(mods, code_for(s.key)?))
}

/// "&" would start a mnemonic.
fn escape(label: &str) -> String {
    label.replace('&', "&&")
}

fn label(platform: Platform, a: &ActionItem) -> String {
    match (platform, a.shortcut) {
        // Win32 shows text after a tab as the right-hand accelerator column.
        (Platform::Windows, Some(s)) => {
            format!("{}\t{}", escape(&a.label), s.display(platform))
        }
        _ => escape(&a.label),
    }
}

fn describe(items: &[Item], platform: Platform, out: &mut Vec<String>) {
    for i in items {
        match i {
            Item::Separator => out.push("-".into()),
            Item::Predefined(p) => out.push(format!("{p:?}")),
            Item::Action(a) => out.push(format!(
                "{}|{:?}|{}",
                label(platform, a),
                a.shortcut,
                a.checked.is_some()
            )),
            Item::Submenu(m) => {
                out.push(format!("[{}", m.title));
                describe(&m.items, platform, out);
                out.push("]".into());
            }
        }
    }
}

fn shape_of(model: &MenuModel, platform: Platform) -> Vec<String> {
    let mut out = Vec::new();
    for m in &model.menus {
        out.push(format!("[{}", m.title));
        describe(&m.items, platform, &mut out);
        out.push("]".into());
    }
    out
}

struct Builder {
    platform: Platform,
    next_id: u64,
    handles: Vec<Handle>,
    actions: HashMap<MenuId, MenuAction>,
}

impl Builder {
    fn fill(&mut self, parent: &Submenu, items: &[Item]) -> muda::Result<()> {
        for item in items {
            match item {
                Item::Separator => parent.append(&PredefinedMenuItem::separator())?,
                Item::Predefined(p) => {
                    let native = match p {
                        Predefined::Services => PredefinedMenuItem::services(None),
                        Predefined::Hide => PredefinedMenuItem::hide(None),
                        Predefined::HideOthers => PredefinedMenuItem::hide_others(None),
                        Predefined::ShowAll => PredefinedMenuItem::show_all(None),
                    };
                    parent.append(&native)?;
                }
                Item::Submenu(Menu { title, items }) => {
                    let sub = Submenu::new(escape(title), true);
                    self.fill(&sub, items)?;
                    parent.append(&sub)?;
                }
                Item::Action(a) => {
                    self.next_id += 1;
                    let id = MenuId::new(format!("roost-menu-{}", self.next_id));
                    let accel = accelerator(self.platform, a.shortcut);
                    let text = label(self.platform, a);
                    match a.checked {
                        Some(checked) => {
                            let item =
                                CheckMenuItem::with_id(id.clone(), text, a.enabled, checked, accel);
                            parent.append(&item)?;
                            self.handles.push(Handle::Check(item));
                        }
                        None => {
                            let item = MenuItem::with_id(id.clone(), text, a.enabled, accel);
                            parent.append(&item)?;
                            self.handles.push(Handle::Plain(item));
                        }
                    }
                    self.actions.insert(id, a.action.clone());
                }
            }
        }
        Ok(())
    }
}

impl NativeMenu {
    /// Attaches a new menu bar to the application (macOS) or the window
    /// (Windows, which needs its HWND). `None` if the platform refuses.
    pub fn install(
        ctx: &egui::Context,
        platform: Platform,
        hwnd: Option<isize>,
    ) -> Option<NativeMenu> {
        let menu = muda::Menu::new();
        let (tx, events) = channel();
        let repaint = ctx.clone();
        // muda accepts only one event handler per process and silently
        // ignores a second, so read its channel from a forwarding thread
        // instead; the thread also wakes the UI when a menu is chosen.
        let forward = std::thread::Builder::new()
            .name("roost-menu-events".into())
            .spawn(move || {
                while let Ok(e) = MenuEvent::receiver().recv() {
                    let _ = tx.send(e.id().clone());
                    repaint.request_repaint();
                }
            });
        forward.ok()?;
        #[cfg(target_os = "macos")]
        {
            let _ = hwnd;
            menu.init_for_nsapp();
        }
        #[cfg(target_os = "windows")]
        {
            // SAFETY: the handle comes from the live eframe window.
            unsafe { menu.init_for_hwnd(hwnd?) }.ok()?;
        }
        Some(NativeMenu {
            menu,
            platform,
            shape: Vec::new(),
            states: Vec::new(),
            handles: Vec::new(),
            actions: HashMap::new(),
            events,
        })
    }

    /// Brings the native menus in line with the model.
    pub fn sync(&mut self, model: &MenuModel) {
        let shape = shape_of(model, self.platform);
        if shape != self.shape {
            // On failure keep the old menus; the next change tries again.
            if self.rebuild(model).is_ok() {
                self.shape = shape;
            }
        }
        let wanted: Vec<(bool, Option<bool>)> = model
            .action_items()
            .iter()
            .map(|a| (a.enabled, a.checked))
            .collect();
        if wanted == self.states {
            return;
        }
        for (i, (handle, (enabled, checked))) in self.handles.iter().zip(&wanted).enumerate() {
            if self.states.get(i) == Some(&(*enabled, *checked)) {
                continue;
            }
            match handle {
                Handle::Plain(h) => h.set_enabled(*enabled),
                Handle::Check(h) => {
                    h.set_enabled(*enabled);
                    if let Some(c) = checked {
                        h.set_checked(*c);
                    }
                }
            }
        }
        self.states = wanted;
    }

    fn rebuild(&mut self, model: &MenuModel) -> muda::Result<()> {
        while self.menu.remove_at(0).is_some() {}
        let mut b = Builder {
            platform: self.platform,
            next_id: 0,
            handles: Vec::new(),
            actions: HashMap::new(),
        };
        for m in &model.menus {
            let sub = Submenu::new(escape(&m.title), true);
            b.fill(&sub, &m.items)?;
            self.menu.append(&sub)?;
            #[cfg(target_os = "macos")]
            match m.title.as_str() {
                "Window" => sub.set_as_windows_menu_for_nsapp(),
                "Help" => sub.set_as_help_menu_for_nsapp(),
                _ => {}
            }
        }
        self.handles = b.handles;
        self.actions = b.actions;
        self.states.clear();
        Ok(())
    }

    /// The actions chosen in the native menus since the last call.
    pub fn poll(&self) -> Vec<MenuAction> {
        self.events
            .try_iter()
            .filter_map(|id| self.actions.get(&id).cloned())
            .collect()
    }
}
