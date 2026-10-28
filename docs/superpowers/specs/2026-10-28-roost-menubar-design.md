# Roost menubar

**Status:** Approved in chat 2026-10-28 (native menus, the standard assortment). Written down so the choices are on record.

## Goal

A real menu bar: native on macOS and Windows, in-window on Linux, all rendered from one description of the menus.

## Structure (`roost-gui::menu`)

- `model`: a pure description. `MenuModel` is a list of menus; items are actions (with label, optional shortcut, enabled, checked), separators, submenus and a few platform-predefined items. `build(&AppState, Platform)` creates it from the app state. Dynamic parts: the theme radio items, the loaded models, and whether New Chat is possible.
- `actions`: `MenuAction` and `handle(&mut AppState, ctx, action) -> Vec<BackendRequest>`. The single place where a menu click changes anything.
- `bar` (egui): renders the model in the window and handles shortcuts through egui. Used on Linux and in tests.
- `native` (macOS and Windows only): builds a `muda` menu from the model, rebuilds it when the model changes, and forwards clicks to the app as `MenuAction`s.

## Menus

- **Roost** (macOS app menu): About Roost, Settings… (⌘,), Services, Hide Roost, Hide Others, Show All, Quit Roost (⌘Q). On Windows and Linux: Settings… and Exit are in File.
- **File:** New Chat (⌘N), New Incognito Chat (⇧⌘N).
- **Edit:** Cut, Copy, Paste, Select All. They go through egui (`RequestCut/Copy/Paste`, and a synthesized ⌘A) so text fields keep working. No Undo item, so egui's own ⌘Z is not intercepted.
- **View:** Chat (⌘1), Browse (⌘2), Installed (⌘3), Settings; **Theme** submenu with the four families and System/Light/Dark as radio items.
- **Models:** one "Unload <name>" per loaded model, Unload All, or a disabled "Nothing loaded".
- **Window** (macOS): Minimize (⌘M), Zoom.
- **Help:** Roost on GitHub, Report an Issue.

## Rules

- Quit sends a normal window close request, so the existing quit confirmation still applies.
- Theme changes use the existing settings path (saved and applied at once).
- New Chat and New Incognito Chat are disabled when no model is installed.
- Shortcuts: on macOS the menu system dispatches them. On Windows and Linux egui dispatches them (so a shortcut can never fire twice). Edit shortcuts are never registered natively.
- Labels never include secrets or chat content. Loaded model names are the only dynamic text.

## Testing and known limits

- Tested headlessly: the model (contents, shortcuts unique, checked and enabled states), every action's effect, the in-window bar's layout in all themes at the minimum size, and shortcut dispatch.
- Not testable here: the native menus themselves. macOS needs a manual look; Windows is compile-checked by CI. Linux has no native menu on purpose (`muda` needs GTK, which the app does not use).
- New dependency: `muda` (MIT or Apache-2.0), only for macOS and Windows targets.

## Findings while building (kept so they are not rediscovered)

- Attaching the macOS menu inside eframe's window creator aborts the process (`panic in a function that cannot unwind` from winit's launch callback). It is attached on the first frame instead.
- `muda::MenuEvent::set_event_handler` can be set once per process and silently does nothing otherwise; menu events were never delivered. A thread reading `MenuEvent::receiver()` works.
- `App::new` keeps Browse as the view in tests and the window bar is off for the test platform, so existing layout tests measure content only; the Linux window instead grows by the bar's height.
- Checked by hand on macOS in an isolated portable folder: the menu bar reads Apple, app, File, Edit, View, Models, Window, Help; a Help link action and Quit were exercised through System Events and both worked end to end.
