//! The menu bar: a pure description (`model`), what clicks do (`actions`)
//! and the renderers (`bar` in the window, `native` on macOS and Windows).

pub mod about;
pub mod actions;
pub mod bar;
pub mod model;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod native;

pub use model::{MenuAction, Platform};
