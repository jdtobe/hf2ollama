//! Product identity shared by the window chrome and bundled icon.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

mod generated {
    include!("identity_generated.rs");
}

pub const PRODUCT_NAME: &str = "Roost";

fn color([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Constructs the canonical SVG's actual paint shapes in `rect`.
pub fn tail_shapes(rect: Rect) -> Vec<egui::Shape> {
    let scale = rect.width() / generated::CANVAS;
    let point = |x, y| Pos2::new(rect.left() + x * scale, rect.top() + y * scale);
    let mut shapes = vec![egui::Shape::rect_filled(
        rect,
        generated::CORNER_RADIUS * scale,
        color(generated::BACKGROUND),
    )];
    for ([x1, y1, x2, y2, x3, y3, x4, y4], stroke_color) in generated::PATHS {
        shapes.push(
            egui::epaint::CubicBezierShape::from_points_stroke(
                [point(x1, y1), point(x2, y2), point(x3, y3), point(x4, y4)],
                false,
                Color32::TRANSPARENT,
                Stroke::new(generated::STROKE_WIDTH * scale, color(stroke_color)),
            )
            .into(),
        );
    }
    let (hub, radius, hub_color) = generated::HUB;
    shapes.push(egui::Shape::circle_filled(
        point(hub[0], hub[1]),
        radius * scale,
        color(hub_color),
    ));
    shapes
}

/// Paints the canonical Roost tail SVG in an exact square allocation.
pub fn paint_tail(ui: &mut egui::Ui, size: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().extend(tail_shapes(rect));
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{self, vec2};

    #[test]
    fn tail_painter_stays_in_its_allocated_square_at_supported_sizes() {
        for size in [16.0, 22.0, 64.0] {
            let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), vec2(size, size));
            for shape in tail_shapes(rect) {
                assert!(
                    rect.expand(0.01)
                        .contains_rect(shape.visual_bounding_rect()),
                    "size={size}: {:?} outside {rect:?}",
                    shape.visual_bounding_rect()
                );
            }
        }
    }
}
