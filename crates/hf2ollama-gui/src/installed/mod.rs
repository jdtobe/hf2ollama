//! The Installed models view.

pub mod state;

/// Height of a table row.
pub const ROW_H: f32 = 40.0;

#[cfg(test)]
mod layout_tests;

use crate::app::AppState;
use crate::runtime::BackendRequest;
use eframe::egui::{self, Margin, RichText};
use state::{Column, InstalledState};
use std::time::Instant;

/// Painted rectangles recorded for the headless layout tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct InstalledProbe {
    /// The whole view body.
    pub panel: Option<egui::Rect>,
    /// Visible area of the scrolling table rows.
    pub viewport: Option<egui::Rect>,
    /// Columns the table painted.
    pub columns: Vec<Column>,
    pub header_cells: Vec<(Column, egui::Rect)>,
    pub rows: Vec<RowProbe>,
    pub toast: Option<egui::Rect>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct RowProbe {
    pub name: String,
    pub rect: egui::Rect,
    /// Painted content (text or widget) of each column.
    pub cells: Vec<(Column, egui::Rect)>,
    pub details_btn: egui::Rect,
    pub delete_btn: egui::Rect,
}

/// View state that is not part of the pure model (only the test probe today).
#[derive(Debug, Default)]
pub struct InstalledUi {
    #[cfg(test)]
    pub probe: InstalledProbe,
}

/// The view body: a first, unconstrained layout.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let now = Instant::now();
    let InstalledState { .. } = app.installed;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(Margin::symmetric(24, 20)))
        .show(ui, |ui| {
            ui.heading("Installed models");
            let names: Vec<String> = app
                .installed
                .visible()
                .iter()
                .map(|r| r.name().to_string())
                .collect();
            #[cfg(test)]
            {
                app.installed_ui.probe = InstalledProbe::default();
                app.installed_ui.probe.panel = Some(ui.max_rect());
                app.installed_ui.probe.viewport = Some(ui.available_rect_before_wrap());
                app.installed_ui.probe.columns = state::visible_columns(ui.available_width());
            }
            for name in names {
                ui.horizontal(|ui| {
                    let mut cells = Vec::new();
                    let label = ui.label(RichText::new(&name));
                    cells.push((Column::Model, label.rect));
                    let size = ui.label("0 MB");
                    cells.push((Column::Size, size.rect));
                    let d = ui.button("Details");
                    let armed = app.installed.is_armed(&name, now);
                    let x = ui.button(if armed { "Confirm delete" } else { "Delete" });
                    if d.clicked() {
                        if let Some(n) = app.installed.toggle_expanded(&name) {
                            out.push(BackendRequest::ShowInstalled(n));
                        }
                    }
                    if x.clicked() {
                        if let Some(a) = app.installed.click_delete(&name, now) {
                            out.push(BackendRequest::DeleteInstalled(a.name));
                        }
                    }
                    #[cfg(test)]
                    app.installed_ui.probe.rows.push(RowProbe {
                        name: name.clone(),
                        rect: label.rect.union(x.rect),
                        cells,
                        details_btn: d.rect,
                        delete_btn: x.rect,
                    });
                });
            }
        });
}
