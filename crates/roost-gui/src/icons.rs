//! Small line icons, painted as shapes so they look the same on every
//! platform and in every theme (no icon font).

use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Copy,
    Regenerate,
    Details,
    Edit,
    Branch,
    Btw,
    Read,
    Stop,
    Filter,
    Trash,
    Prev,
    Next,
}

/// Paints `icon` centred in `rect`, drawn on a 16 x 16 grid.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let origin = rect.center() - vec2(8.0, 8.0);
    let p = |x: f32, y: f32| origin + vec2(x, y);
    let stroke = Stroke::new(1.5, color);
    match icon {
        Icon::Copy => {
            painter.rect_stroke(
                Rect::from_min_max(p(5.5, 5.5), p(13.5, 13.5)),
                1.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::line(
                vec![
                    p(10.5, 5.5),
                    p(10.5, 3.5),
                    p(9.0, 2.0),
                    p(4.0, 2.0),
                    p(2.5, 3.5),
                    p(2.5, 8.5),
                    p(4.0, 10.0),
                    p(5.5, 10.0),
                ],
                stroke,
            ));
        }
        Icon::Regenerate => {
            let c = p(8.0, 8.0);
            let arc: Vec<Pos2> = (0..=26)
                .map(|i| {
                    let a = (i as f32 * 12.0).to_radians();
                    c + vec2(a.cos(), a.sin()) * 5.5
                })
                .collect();
            painter.add(egui::Shape::line(arc, stroke));
            painter.add(egui::Shape::line(
                vec![p(13.5, 2.5), p(13.5, 5.5), p(10.5, 5.5)],
                stroke,
            ));
        }
        Icon::Edit => {
            painter.add(egui::Shape::line(
                vec![
                    p(10.5, 2.5),
                    p(13.5, 5.5),
                    p(6.0, 13.0),
                    p(2.5, 13.5),
                    p(3.0, 10.0),
                    p(10.5, 2.5),
                ],
                stroke,
            ));
        }
        Icon::Branch => {
            painter.circle_stroke(p(4.5, 3.5), 1.5, stroke);
            painter.circle_stroke(p(4.5, 12.5), 1.5, stroke);
            painter.circle_stroke(p(11.5, 5.5), 1.5, stroke);
            painter.line_segment([p(4.5, 5.0), p(4.5, 11.0)], stroke);
            painter.add(egui::Shape::line(
                vec![
                    p(11.5, 7.0),
                    p(11.3, 8.6),
                    p(10.2, 9.6),
                    p(8.0, 10.2),
                    p(4.5, 11.0),
                ],
                stroke,
            ));
        }
        Icon::Btw => {
            painter.add(egui::Shape::line(
                vec![
                    p(2.5, 3.5),
                    p(13.5, 3.5),
                    p(13.5, 10.5),
                    p(7.5, 10.5),
                    p(4.5, 13.0),
                    p(4.5, 10.5),
                    p(2.5, 10.5),
                    p(2.5, 3.5),
                ],
                stroke,
            ));
            painter.line_segment([p(6.0, 7.0), p(10.0, 7.0)], stroke);
        }
        Icon::Read => {
            painter.add(egui::Shape::line(
                vec![
                    p(2.5, 6.5),
                    p(5.0, 6.5),
                    p(8.5, 3.5),
                    p(8.5, 12.5),
                    p(5.0, 9.5),
                    p(2.5, 9.5),
                    p(2.5, 6.5),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(10.8, 5.6), p(11.8, 8.0), p(10.8, 10.4)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(12.6, 3.8), p(14.0, 8.0), p(12.6, 12.2)],
                stroke,
            ));
        }
        Icon::Filter => {
            painter.add(egui::Shape::line(
                vec![
                    p(2.5, 3.5),
                    p(13.5, 3.5),
                    p(9.0, 8.5),
                    p(9.0, 13.0),
                    p(7.0, 12.0),
                    p(7.0, 8.5),
                    p(2.5, 3.5),
                ],
                stroke,
            ));
        }
        Icon::Trash => {
            painter.line_segment([p(2.5, 4.0), p(13.5, 4.0)], stroke);
            painter.add(egui::Shape::line(
                vec![p(6.0, 4.0), p(6.0, 2.5), p(10.0, 2.5), p(10.0, 4.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    p(3.8, 4.0),
                    p(4.6, 13.0),
                    p(5.2, 13.5),
                    p(10.8, 13.5),
                    p(11.4, 13.0),
                    p(12.2, 4.0),
                ],
                stroke,
            ));
            painter.line_segment([p(6.6, 6.5), p(6.8, 11.0)], stroke);
            painter.line_segment([p(9.4, 6.5), p(9.2, 11.0)], stroke);
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(p(4.5, 4.5), p(11.5, 11.5)), 1.5, color);
        }
        Icon::Prev => {
            painter.add(egui::Shape::line(
                vec![p(10.0, 3.5), p(5.5, 8.0), p(10.0, 12.5)],
                stroke,
            ));
        }
        Icon::Next => {
            painter.add(egui::Shape::line(
                vec![p(6.0, 3.5), p(10.5, 8.0), p(6.0, 12.5)],
                stroke,
            ));
        }
        Icon::Details => {
            painter.circle_stroke(p(8.0, 8.0), 5.8, stroke);
            painter.line_segment([p(8.0, 7.3), p(8.0, 10.8)], stroke);
            painter.circle_filled(p(8.0, 5.3), 0.9, color);
        }
    }
}

/// Where the `n`th 28 px icon slot of a row starts, `gap` apart.
pub fn slot(row: Rect, n: usize) -> Rect {
    Rect::from_min_size(
        pos2(row.left() + 2.0 + n as f32 * 30.0, row.top()),
        vec2(28.0, row.height()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_do_not_overlap_and_stay_in_the_row() {
        let row = Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 28.0));
        let a = slot(row, 0);
        let b = slot(row, 1);
        assert!(a.right() < b.left());
        assert!(a.min.x >= row.min.x && a.height() == row.height());
    }
}
