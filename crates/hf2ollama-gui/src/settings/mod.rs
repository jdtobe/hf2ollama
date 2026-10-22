//! The Settings view: validated fields that apply and persist immediately.

#![allow(dead_code)] // temporary: the view that uses the state lands in the next commits

pub mod state;
pub mod validate;

use eframe::egui;
use hf2ollama_core::config::Settings;
use hf2ollama_core::fit::Machine;
use state::{SettingsChange, SettingsState};
use std::path::PathBuf;

/// The Settings view: its pure state plus what it painted (for tests).
pub struct SettingsView {
    pub state: SettingsState,
}

impl SettingsView {
    pub fn new(
        settings: &Settings,
        default_staging: PathBuf,
        detected: Option<Machine>,
    ) -> SettingsView {
        SettingsView {
            state: SettingsState::new(settings, default_staging, detected),
        }
    }

    /// Draws the cards; returns which settings the user changed this frame.
    pub fn show(&mut self, ui: &mut egui::Ui, settings: &mut Settings) -> SettingsChange {
        let _ = (ui, settings);
        SettingsChange::default()
    }
}
