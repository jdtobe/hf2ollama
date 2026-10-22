//! The results list: rows, loading, error and empty states, plus the small
//! number and date formatters the Browse view shares.

use super::state::SearchStatus;
use crate::app::AppState;
use crate::runtime::BackendRequest;
use crate::theme::{semibold, tokens};
use crate::widgets::{small_button, tag};
use eframe::egui::{self, vec2, Margin, Rect, RichText, Sense, Stroke, UiBuilder};
use std::time::{SystemTime, UNIX_EPOCH};

/// `412300` -> `412k`, `1_900_000` -> `1.9M`, small numbers as they are.
pub fn compact_count(n: u64) -> String {
    let scaled = |div: f64, unit: &str| {
        let v = n as f64 / div;
        if v >= 10.0 || (v - v.trunc()).abs() < 0.05 {
            format!("{}{unit}", v.trunc())
        } else {
            format!("{v:.1}{unit}")
        }
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => scaled(1e3, "k"),
        _ => scaled(1e6, "M"),
    }
}

/// Seconds since the Unix epoch for `2026-10-12T07:10:55.000Z`.
pub fn parse_iso(s: &str) -> Option<i64> {
    let num = |range: std::ops::Range<usize>| -> Option<i64> { s.get(range)?.parse().ok() };
    if s.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19).unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil date (proleptic Gregorian), after Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// `3 weeks ago` style age of an ISO timestamp, or `—` if unparseable.
pub fn relative_age(iso: &str, now_unix: i64) -> String {
    let Some(then) = parse_iso(iso) else {
        return "—".to_string();
    };
    let secs = (now_unix - then).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    let days = secs / 86_400;
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => plural(secs / 3_600, "hour"),
        _ if days < 14 => plural(days, "day"),
        _ if days < 60 => plural(days / 7, "week"),
        _ if days < 365 => plural(days / 30, "month"),
        _ => plural(days / 365, "year"),
    }
}

/// `4.68 GB`, or `812 MB` below one gigabyte.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

/// Shortens long architecture names so the tags row never overflows.
fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

const ROW_H: f32 = 88.0;

/// The results column.
pub fn show(ui: &mut egui::Ui, app: &mut AppState, out: &mut Vec<BackendRequest>) {
    let t = tokens(ui);
    egui::Panel::left("results")
        .default_size(350.0)
        .size_range(300.0..=420.0)
        .frame(
            egui::Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.0, t.line)),
        )
        .show(ui, |ui| {
            let visible = app.browse.visible_len();
            let loaded = app.browse.loaded_len();
            let head = match (&app.browse.search, visible == loaded) {
                (SearchStatus::Loading, _) if loaded == 0 => "Searching…".to_string(),
                (_, true) => format!("{visible} results"),
                (_, false) => format!("{visible} of {loaded} results"),
            };
            egui::Frame::new()
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(head).size(12.0).color(t.fg3));
                });
            ui.painter().hline(
                ui.min_rect().x_range(),
                ui.min_rect().bottom(),
                Stroke::new(1.0, t.line),
            );
            match app.browse.search.clone() {
                SearchStatus::Loading if loaded == 0 => {
                    message_box(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Searching Hugging Face…").color(t.fg3));
                        });
                    });
                }
                SearchStatus::Failed(msg) => {
                    message_box(ui, |ui| {
                        ui.label(RichText::new("The search did not work").font(semibold(14.0)));
                        ui.label(RichText::new(msg).size(13.0).color(t.fg2));
                        if small_button(ui, "Retry") {
                            let q = app.browse.search_now(&app.query);
                            out.push(BackendRequest::Search(q));
                        }
                    });
                }
                _ if visible == 0 => {
                    let text = if loaded > 0 {
                        format!(
                            "{loaded} results are hidden by your filters. Loosen the filters on the left to see them."
                        )
                    } else if app.query.trim().is_empty() {
                        "No models came back from Hugging Face.".to_string()
                    } else {
                        format!("No models found for “{}”. Try different words.", app.query.trim())
                    };
                    if app.browse.search != SearchStatus::Idle {
                        message_box(ui, |ui| {
                            ui.label(RichText::new(text).size(13.0).color(t.fg2));
                        });
                    }
                }
                _ => rows(ui, app),
            }
        });
}

fn message_box(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn rows(ui: &mut egui::Ui, app: &mut AppState) {
    let t = tokens(ui);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let list: Vec<_> = app
        .browse
        .visible()
        .map(|s| app.browse.enriched(s))
        .collect();
    let selected = app.browse.selected_id().map(str::to_string);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, m) in list.iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                if ui.is_rect_visible(rect) {
                    app.browse.want_info(&m.id);
                }
                let on = selected.as_deref() == Some(m.id.as_str());
                if on {
                    ui.painter().rect_filled(rect, 0.0, t.accent_soft);
                    ui.painter().rect_filled(
                        Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                        0.0,
                        t.accent,
                    );
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.surface2);
                }
                ui.painter()
                    .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.line));
                ui.scope_builder(
                    UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 10.0))),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(RichText::new(format!("{}/", m.owner)).color(t.fg3));
                            ui.add(
                                egui::Label::new(RichText::new(&m.name).font(semibold(14.0)))
                                    .truncate(),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if m.is_safetensors && !m.is_gguf {
                                tag(ui, "Safetensors", t.warn, t.warn_soft);
                            } else if m.is_gguf {
                                tag(ui, "GGUF", t.ok, t.ok_soft);
                            } else {
                                tag(ui, "Other", t.fg2, t.surface2);
                            }
                            let pending = app.browse.info_pending(&m.id);
                            for value in [&m.params, &m.arch] {
                                match value {
                                    Some(v) => tag(ui, &shorten(v, 16), t.fg2, t.surface2),
                                    None if pending => tag(ui, "…", t.fg3, t.surface2),
                                    None => {}
                                }
                            }
                        });
                        let mut stats = format!(
                            "↓ {}  ·  {} likes",
                            compact_count(m.downloads),
                            compact_count(m.likes)
                        );
                        if parse_iso(&m.updated).is_some() {
                            stats.push_str(&format!("  ·  {}", relative_age(&m.updated, now)));
                        }
                        ui.label(RichText::new(stats).size(12.0).color(t.fg3));
                    },
                );
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        });
    if let Some(i) = picked {
        app.browse.select_model(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_count_abbreviates() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1_000), "1k");
        assert_eq!(compact_count(412_300), "412k");
        assert_eq!(compact_count(1_900_000), "1.9M");
        assert_eq!(compact_count(2_738_344), "2.7M");
        assert_eq!(compact_count(12_000_000), "12M");
    }

    #[test]
    fn parse_iso_matches_known_epochs() {
        assert_eq!(parse_iso("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_iso("2026-10-12T07:10:55.000Z"), Some(1_791_789_055));
        assert_eq!(parse_iso("nope"), None);
        assert_eq!(parse_iso(""), None);
    }

    #[test]
    fn relative_age_uses_the_largest_unit() {
        let t = parse_iso("2026-10-01T00:00:00Z").unwrap_or_default();
        assert_eq!(relative_age("2026-10-01T00:00:00Z", t + 30), "just now");
        assert_eq!(relative_age("2026-10-01T00:00:00Z", t + 120), "2 min ago");
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 3_600),
            "1 hour ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 5 * 86_400),
            "5 days ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 21 * 86_400),
            "3 weeks ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 150 * 86_400),
            "5 months ago"
        );
        assert_eq!(
            relative_age("2026-10-01T00:00:00Z", t + 800 * 86_400),
            "2 years ago"
        );
        assert_eq!(relative_age("garbage", t), "—");
    }

    #[test]
    fn format_size_switches_units() {
        assert_eq!(format_size(4_680_000_000), "4.68 GB");
        assert_eq!(format_size(812_000_000), "812 MB");
        assert_eq!(format_size(0), "0 MB");
    }
}
