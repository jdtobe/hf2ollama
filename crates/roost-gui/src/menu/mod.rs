//! The menu bar: a pure description (`model`), what clicks do (`actions`)
//! and the renderers (`bar` in the window, `native` on macOS and Windows).

pub mod actions;
pub mod model;

pub use model::{MenuAction, MenuModel, Platform};
