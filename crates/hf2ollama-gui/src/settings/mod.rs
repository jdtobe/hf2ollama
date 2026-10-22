//! The Settings view: validated fields that apply and persist immediately.

#![allow(dead_code)] // temporary: the view that uses the state lands in the next commits

pub mod state;
pub mod validate;

use crate::app::AppState;
use crate::runtime::BackendRequest;
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

/// Draws the Settings view in the central panel, applies what changed and
/// queues the request that saves it.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = crate::theme::tokens(ui);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .inner_margin(egui::Margin::symmetric(24, 20)),
        )
        .show(ui, |ui| {
            let change = app.settings_view.show(ui, &mut app.settings);
            out.extend(app.apply_change(change, ui.ctx()));
        });
}
