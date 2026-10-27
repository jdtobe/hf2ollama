//! Product identity shared by the window chrome and bundled icon.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

pub const PRODUCT_NAME: &str = "Roost";

const CANVAS: f32 = 96.0;
const BACKGROUND: Color32 = Color32::from_rgb(0x10, 0x20, 0x1d);
const HUB: Color32 = Color32::from_rgb(0xf7, 0xf3, 0xe8);

/// Paints the canonical 96px Roost tail SVG in an exact square allocation.
///
/// Coordinates, radii, stroke widths, control points, and colors are copied
/// directly from `docs/design/roost-tail.svg`.
pub fn paint_tail(ui: &mut egui::Ui, size: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let scale = size / CANVAS;
    let point = |x, y| Pos2::new(rect.left() + x * scale, rect.top() + y * scale);
    let painter = ui.painter();
    painter.rect_filled(rect, 24.0 * scale, BACKGROUND);
    for ([x1, y1, x2, y2, x3, y3, x4, y4], color) in [
        (
            [26., 70., 40., 41., 58., 27., 79., 23.],
            Color32::from_rgb(0x3c, 0xc2, 0xb3),
        ),
        (
            [26., 70., 42., 47., 61., 37., 82., 37.],
            Color32::from_rgb(0x8b, 0xe0, 0xd2),
        ),
        (
            [26., 70., 44., 54., 64., 50., 82., 51.],
            Color32::from_rgb(0xf4, 0xb8, 0x60),
        ),
        (
            [26., 70., 46., 61., 66., 62., 80., 67.],
            Color32::from_rgb(0xf1, 0x7c, 0x67),
        ),
    ] {
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [point(x1, y1), point(x2, y2), point(x3, y3), point(x4, y4)],
            false,
            Color32::TRANSPARENT,
            Stroke::new(7.0 * scale, color),
        ));
    }
    painter.circle_filled(point(26.0, 70.0), 6.0 * scale, HUB);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, vec2};

    #[test]
    fn tail_painter_stays_in_its_allocated_square_at_supported_sizes() {
        let ctx = egui::Context::default();
        for size in [16.0, 22.0, 64.0] {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(100.0, 100.0));
            let mut painted = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| painted = Some(paint_tail(ui, size)),
            );
            let rect = painted.expect("tail painted");
            assert_eq!(rect.size(), vec2(size, size));
            assert!(screen.contains_rect(rect), "size={size}: {rect:?}");
            output.drop_without_applying_deltas();
        }
    }
}
