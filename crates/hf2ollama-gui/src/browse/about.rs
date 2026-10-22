//! The "About this model" tab: chips from the repository metadata and the
//! rendered model card.

use crate::app::AppState;
use eframe::egui;
use hf2ollama_core::hf::ModelInfo;

/// Tags that say nothing useful to a person choosing a model.
pub const NOISY_TAGS: &[&str] = &[
    "region:",
    "endpoints_compatible",
    "license:",
    "autotrain_compatible",
    "text-generation-inference",
];

/// Most plain tags shown as chips.
pub const MAX_TAG_CHIPS: usize = 6;

/// Most languages shown before "+N".
pub const MAX_LANGUAGE_CHIPS: usize = 5;

/// Chip labels: pipeline tag, languages, then up to `MAX_TAG_CHIPS` tags.
pub fn chips(_info: &ModelInfo) -> Vec<String> {
    Vec::new()
}

/// Draws the About tab content for the selected repository.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, _info: &ModelInfo, id: &str) {
    let view = match app.browse.card_for(id) {
        Some(Ok(Some(v))) => Some(v.clone()),
        _ => None,
    };
    if let Some(v) = view {
        let text = format!("{:?}", v.full);
        let r = ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
        #[cfg(test)]
        app.browse.probe.about_rects.push(r.rect);
        #[cfg(not(test))]
        let _ = r;
    }
}
