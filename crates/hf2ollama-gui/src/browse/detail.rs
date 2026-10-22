//! The detail pane for the selected repository.

use eframe::egui;

/// Where one metadata value was drawn.
#[derive(Debug, Clone)]
pub struct MetaCell {
    pub label: String,
    pub value: egui::Rect,
}

/// Draws the metadata strip (label over value, "—" for unknown values) and
/// returns where each value went. Long values never wrap.
pub fn meta_strip(ui: &mut egui::Ui, items: &[(&str, String)]) -> Vec<MetaCell> {
    let _ = (ui, items);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::test_util::headless;
    use crate::theme::mono;

    fn items() -> Vec<(&'static str, String)> {
        vec![
            ("Parameters", "7.25B".to_string()),
            ("Architecture", "MistralForCausalLM".to_string()),
            ("Context", "32,768".to_string()),
            ("License", "apache-2.0".to_string()),
            ("Base model", "—".to_string()),
            ("Prompt template", "—".to_string()),
        ]
    }

    #[test]
    fn metadata_strip_never_wraps_a_long_architecture_name() {
        for dark in [false, true] {
            for width in [900.0, 520.0, 260.0] {
                let (cells, natural) = headless(dark, width, |ui| {
                    let cells = meta_strip(ui, &items());
                    let natural = ui
                        .painter()
                        .layout_no_wrap(
                            "MistralForCausalLM".to_string(),
                            mono(13.0),
                            egui::Color32::WHITE,
                        )
                        .size();
                    (cells, natural)
                });
                assert_eq!(cells.len(), 6, "dark={dark} width={width}");
                let arch = cells
                    .iter()
                    .find(|c| c.label == "Architecture")
                    .expect("architecture cell");
                assert!(
                    (arch.value.width() - natural.x).abs() < 1.0,
                    "dark={dark} width={width}: arch {} vs natural {}",
                    arch.value.width(),
                    natural.x
                );
                assert!(
                    arch.value.height() < natural.y * 1.5,
                    "dark={dark} width={width}: arch wrapped, height {}",
                    arch.value.height()
                );
                // Nothing may spill past the available width.
                let right = cells.iter().map(|c| c.value.max.x).fold(0.0, f32::max);
                let left = cells.iter().map(|c| c.value.min.x).fold(f32::MAX, f32::min);
                assert!(right - left <= width + 0.5, "dark={dark} width={width}");
            }
        }
    }
}
