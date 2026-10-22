//! The import plan card: model name, pipeline step chips, sizes and the
//! primary button.

use eframe::egui;

/// Pipeline steps shown as chips for the chosen import.
pub fn steps(is_gguf: bool, quantize: bool, delete_after: bool) -> Vec<&'static str> {
    let _ = (is_gguf, quantize, delete_after);
    unimplemented!()
}

/// Where one chip goes: its row and x offset from the left edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipSlot {
    pub row: usize,
    pub x: f32,
}

/// Flows chips of the given widths into rows no wider than `avail`. Chips on
/// the same row are separated by `gap + sep_w + gap`.
pub fn layout_chips(widths: &[f32], gap: f32, sep_w: f32, avail: f32) -> Vec<ChipSlot> {
    let _ = (widths, gap, sep_w, avail);
    unimplemented!()
}

/// Draws the chips (wrapping to as many rows as needed) and returns their
/// rectangles.
pub fn step_chips(ui: &mut egui::Ui, labels: &[&str]) -> Vec<egui::Rect> {
    let _ = (ui, labels);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::test_util::headless;
    use crate::theme::mono;

    #[test]
    fn steps_follow_the_import_kind() {
        assert_eq!(
            steps(true, false, true),
            [
                "Download",
                "Verify sha256",
                "Upload blob(s)",
                "Create model",
                "Delete staging"
            ]
        );
        assert_eq!(steps(true, false, false).last(), Some(&"Keep files"));
        assert_eq!(
            steps(false, true, true),
            [
                "Download",
                "Verify sha256",
                "Upload blobs",
                "Quantize",
                "Create model",
                "Delete staging"
            ]
        );
        assert_eq!(
            steps(false, false, false),
            [
                "Download",
                "Verify sha256",
                "Upload blobs",
                "Create model",
                "Keep files"
            ]
        );
    }

    #[test]
    fn layout_chips_wraps_when_the_row_is_full() {
        // 60 + (6+14+6) + 80 = 166 fits in 170; the third chip needs 26 + 70 more.
        let slots = layout_chips(&[60.0, 80.0, 70.0], 6.0, 14.0, 170.0);
        assert_eq!(slots[0], ChipSlot { row: 0, x: 0.0 });
        assert_eq!(slots[1], ChipSlot { row: 0, x: 86.0 });
        assert_eq!(slots[2], ChipSlot { row: 1, x: 0.0 });
        // Everything fits in one row when wide enough.
        let one = layout_chips(&[60.0, 80.0, 70.0], 6.0, 14.0, 400.0);
        assert!(one.iter().all(|s| s.row == 0));
        // A chip wider than the row still gets a row of its own.
        let wide = layout_chips(&[300.0, 20.0], 6.0, 14.0, 100.0);
        assert_eq!(wide[0], ChipSlot { row: 0, x: 0.0 });
        assert_eq!(wide[1].row, 1);
    }

    #[test]
    fn step_chips_wrap_onto_rows_without_widening_any_chip() {
        let labels = steps(false, true, true);
        for dark in [false, true] {
            // 700 px window: the card is far narrower than the window.
            let (rects, widths) = headless(dark, 300.0, |ui| {
                let rects = step_chips(ui, &labels);
                let widths: Vec<f32> = labels
                    .iter()
                    .map(|l| {
                        ui.painter()
                            .layout_no_wrap((*l).to_string(), mono(11.0), egui::Color32::WHITE)
                            .size()
                            .x
                    })
                    .collect();
                (rects, widths)
            });
            assert_eq!(rects.len(), labels.len(), "dark={dark}");
            let mut ys: Vec<i32> = rects.iter().map(|r| r.min.y.round() as i32).collect();
            ys.dedup();
            assert!(ys.len() >= 2, "dark={dark}: chips should wrap, rows {ys:?}");
            for (r, w) in rects.iter().zip(&widths) {
                assert!(
                    (r.width() - (w + CHIP_PAD)).abs() < 0.6,
                    "dark={dark}: chip {} wide vs content {}",
                    r.width(),
                    w
                );
                assert!(r.height() < 26.0, "dark={dark}: chip wrapped its label");
            }
            let left = rects.iter().map(|r| r.min.x).fold(f32::MAX, f32::min);
            let right = rects.iter().map(|r| r.max.x).fold(0.0, f32::max);
            assert!(
                right - left <= 300.5,
                "dark={dark}: overflow {}",
                right - left
            );
        }
    }
}

/// Horizontal padding added to a chip label (7 px each side).
pub const CHIP_PAD: f32 = 14.0;
