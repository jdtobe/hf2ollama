//! Filter model for the Browse rail. Format and sort re-query Hugging Face;
//! parameter, task and "fits my machine" filters apply to loaded results.

use crate::app::AppState;
use crate::theme::tokens;
use crate::widgets::caption;
use eframe::egui::{self, RichText};
use roost_core::fit::Machine;
use roost_core::hf::{FormatFilter, ModelSummary, SortKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamRange {
    Any,
    Under3,
    From3To14,
    From14To40,
    Over40,
}

impl ParamRange {
    pub const ALL: [ParamRange; 5] = [
        ParamRange::Any,
        ParamRange::Under3,
        ParamRange::From3To14,
        ParamRange::From14To40,
        ParamRange::Over40,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ParamRange::Any => "Any size",
            ParamRange::Under3 => "Under 3B",
            ParamRange::From3To14 => "3B to 14B",
            ParamRange::From14To40 => "14B to 40B",
            ParamRange::Over40 => "40B+",
        }
    }

    /// Whether a model of `billions` parameters is inside the range.
    pub fn contains(self, billions: f64) -> bool {
        match self {
            ParamRange::Any => true,
            ParamRange::Under3 => billions < 3.0,
            ParamRange::From3To14 => (3.0..14.0).contains(&billions),
            ParamRange::From14To40 => (14.0..40.0).contains(&billions),
            ParamRange::Over40 => billions >= 40.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskKind {
    Text,
    Embeddings,
    Vision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Filters {
    pub gguf: bool,
    pub safetensors: bool,
    pub other: bool,
    pub params: ParamRange,
    pub only_fit: bool,
    pub text: bool,
    pub embeddings: bool,
    pub vision: bool,
    pub sort: SortKey,
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            gguf: true,
            safetensors: true,
            other: false,
            params: ParamRange::Any,
            only_fit: false,
            text: true,
            embeddings: true,
            vision: true,
            sort: SortKey::Downloads,
        }
    }
}

impl Filters {
    /// The formats to send to Hugging Face. "Other" cannot be expressed
    /// server side, so it widens the query to everything.
    pub fn query_formats(&self) -> Vec<FormatFilter> {
        if self.other || (!self.gguf && !self.safetensors) {
            return vec![FormatFilter::Any];
        }
        let mut v = Vec::new();
        if self.gguf {
            v.push(FormatFilter::Gguf);
        }
        if self.safetensors {
            v.push(FormatFilter::Safetensors);
        }
        v
    }
}

/// `7.6B` -> 7.6, `135M` -> 0.135, `1.2T` -> 1200.0. None if unparseable.
pub fn parse_params_b(s: &str) -> Option<f64> {
    let s = s.trim();
    let unit = s.chars().next_back()?;
    let scale = match unit.to_ascii_uppercase() {
        'B' => 1.0,
        'M' => 1e-3,
        'T' => 1e3,
        'K' => 1e-6,
        _ => return None,
    };
    let n: f64 = s.get(..s.len() - unit.len_utf8())?.trim().parse().ok()?;
    (n.is_finite() && n >= 0.0).then_some(n * scale)
}

/// Summaries carry no pipeline tag, so the task is guessed from name tokens.
pub fn task_of(name: &str) -> TaskKind {
    let lower = name.to_lowercase();
    let mut tokens = lower.split(|c: char| !c.is_ascii_alphanumeric());
    if tokens
        .clone()
        .any(|t| t.starts_with("embed") || matches!(t, "bge" | "gte" | "e5" | "minilm" | "mpnet"))
    {
        return TaskKind::Embeddings;
    }
    if tokens.any(|t| {
        matches!(
            t,
            "vl" | "vlm" | "vision" | "llava" | "pixtral" | "paligemma" | "internvl" | "moondream"
        )
    }) {
        return TaskKind::Vision;
    }
    TaskKind::Text
}

/// Whether `summary` is shown under filters `f` on `machine`.
///
/// "Only models that fit" is an estimate: a model with a known parameter
/// count is hidden only when even a ~4-bit build (`billions * 0.6` GB plus
/// 1.5 GB of runtime overhead) cannot fit in the larger of RAM and VRAM.
/// Unknown parameter counts, and machines whose memory was not detected,
/// never hide a model.
pub fn passes(summary: &ModelSummary, f: &Filters, machine: &Machine) -> bool {
    let format_ok = (summary.is_gguf && f.gguf)
        || (summary.is_safetensors && f.safetensors)
        || (!summary.is_gguf && !summary.is_safetensors && f.other);
    if !format_ok {
        return false;
    }
    let task_ok = match task_of(&summary.name) {
        TaskKind::Text => f.text,
        TaskKind::Embeddings => f.embeddings,
        TaskKind::Vision => f.vision,
    };
    if !task_ok {
        return false;
    }
    let Some(billions) = summary.params.as_deref().and_then(parse_params_b) else {
        return true;
    };
    if !f.params.contains(billions) {
        return false;
    }
    if f.only_fit {
        let capacity = machine.ram_gb.max(machine.vram_gb.unwrap_or(0.0));
        let four_bit_gb = billions as f32 * FOUR_BIT_GB_PER_B + FIT_OVERHEAD_GB;
        if capacity > 0.0 && four_bit_gb > capacity {
            return false;
        }
    }
    true
}

/// Approximate size of a ~4-bit build per billion parameters.
const FOUR_BIT_GB_PER_B: f32 = 0.6;
const FIT_OVERHEAD_GB: f32 = 1.5;

/// The "Detected ..." line under "Only models that fit".
pub fn machine_line(m: &Machine) -> String {
    let ram = if m.ram_gb > 0.0 {
        format!("{:.0} GB RAM", m.ram_gb)
    } else {
        "RAM not detected".to_string()
    };
    match m.vram_gb {
        Some(v) => format!("Detected {v:.0} GB VRAM, {ram}"),
        None => format!("VRAM not detected, {ram}. You can set VRAM yourself in Settings."),
    }
}

fn sort_label(s: SortKey) -> &'static str {
    match s {
        SortKey::Downloads => "Most downloads",
        SortKey::Trending => "Trending",
        SortKey::Updated => "Recently updated",
        SortKey::Likes => "Most likes",
    }
}

/// How many filters differ from the defaults, for the badge on the button.
fn active_count(f: &Filters) -> usize {
    let d = Filters::default();
    [
        f.gguf != d.gguf || f.safetensors != d.safetensors || f.other != d.other,
        f.params != d.params,
        f.only_fit != d.only_fit,
        f.text != d.text || f.embeddings != d.embeddings || f.vision != d.vision,
    ]
    .into_iter()
    .filter(|on| *on)
    .count()
}

/// One selectable choice drawn as a small pill.
fn choice(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    ui.selectable_label(on, RichText::new(label).size(12.5))
}

/// All filter controls, laid out for a popover (no combo boxes, so a click
/// never lands in a second popup).
fn controls(ui: &mut egui::Ui, f: &mut Filters, machine: &Machine) {
    let t = tokens(ui);
    ui.set_min_width(250.0);
    caption(ui, "Format");
    ui.checkbox(&mut f.gguf, "GGUF");
    ui.checkbox(&mut f.safetensors, "Safetensors");
    ui.checkbox(&mut f.other, "PyTorch / other");
    ui.add_space(8.0);

    caption(ui, "Parameters");
    ui.horizontal_wrapped(|ui| {
        for p in ParamRange::ALL {
            if choice(ui, f.params == p, p.label()).clicked() {
                f.params = p;
            }
        }
    });
    ui.add_space(8.0);

    caption(ui, "Fits my machine");
    ui.checkbox(&mut f.only_fit, "Only models that fit");
    ui.add(egui::Label::new(RichText::new(machine_line(machine)).size(12.0).color(t.fg3)).wrap());
    ui.add_space(8.0);

    caption(ui, "Task");
    ui.checkbox(&mut f.text, "Text generation");
    ui.checkbox(&mut f.embeddings, "Embeddings");
    ui.checkbox(&mut f.vision, "Vision / multimodal");
}

/// The chips row above the Models list: a Filters popover, a quick
/// "Fits my machine" toggle and the sort order.
pub fn chips(ui: &mut egui::Ui, app: &mut AppState) {
    let t = tokens(ui);
    let before = app.browse.filters.clone();
    let machine = app.machine.clone();
    let f = &mut app.browse.filters;
    let mut chip = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        let n = active_count(f);
        let label = if n == 0 {
            "Filters ▾".to_string()
        } else {
            format!("Filters ({n}) ▾")
        };
        let r = ui.selectable_label(n > 0, RichText::new(label).size(12.5));
        egui::Popup::from_toggle_button_response(&r)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| controls(ui, f, &machine));
        chip = Some(r.rect);
        if ui
            .selectable_label(f.only_fit, RichText::new("Fits my machine").size(12.5))
            .on_hover_text(machine_line(&machine))
            .clicked()
        {
            f.only_fit = !f.only_fit;
        }
        egui::ComboBox::from_id_salt("sort")
            .selected_text(RichText::new(sort_label(f.sort)).size(12.5).color(t.fg2))
            .show_ui(ui, |ui| {
                for s in [
                    SortKey::Downloads,
                    SortKey::Trending,
                    SortKey::Updated,
                    SortKey::Likes,
                ] {
                    ui.selectable_value(&mut f.sort, s, sort_label(s));
                }
            });
    });
    #[cfg(test)]
    {
        app.browse.probe.filters_chip = chip;
    }
    let _ = chip;
    if app.browse.filters != before {
        app.browse.apply_filters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(params: Option<&str>, gguf: bool, st: bool, name: &str) -> ModelSummary {
        ModelSummary {
            id: format!("o/{name}"),
            owner: "o".into(),
            name: name.into(),
            downloads: 1,
            likes: 1,
            updated: String::new(),
            params: params.map(str::to_string),
            arch: None,
            is_gguf: gguf,
            is_safetensors: st,
        }
    }

    fn machine(ram: f32, vram: Option<f32>) -> Machine {
        Machine {
            ram_gb: ram,
            vram_gb: vram,
        }
    }

    #[test]
    fn machine_line_is_honest_about_unknown_vram() {
        assert_eq!(
            machine_line(&machine(32.0, Some(16.0))),
            "Detected 16 GB VRAM, 32 GB RAM"
        );
        let unknown = machine_line(&machine(32.0, None));
        assert!(unknown.contains("VRAM not detected") && unknown.contains("Settings"));
        assert!(machine_line(&machine(0.0, None)).contains("RAM not detected"));
    }

    #[test]
    fn parse_params_handles_units() {
        assert_eq!(parse_params_b("7.6B"), Some(7.6));
        assert_eq!(parse_params_b("135M"), Some(0.135));
        assert_eq!(parse_params_b("1.2T"), Some(1200.0));
        assert_eq!(parse_params_b("70b"), Some(70.0));
        assert_eq!(parse_params_b("lots"), None);
        assert_eq!(parse_params_b(""), None);
    }

    #[test]
    fn param_ranges_have_half_open_bounds() {
        assert!(ParamRange::Under3.contains(2.99) && !ParamRange::Under3.contains(3.0));
        assert!(ParamRange::From3To14.contains(3.0) && !ParamRange::From3To14.contains(14.0));
        assert!(ParamRange::From14To40.contains(14.0) && !ParamRange::From14To40.contains(40.0));
        assert!(ParamRange::Over40.contains(40.0));
    }

    #[test]
    fn filters_only_models_that_fit_hides_too_large() {
        let f = Filters {
            only_fit: true,
            ..Filters::default()
        };
        let m = machine(32.0, Some(16.0));
        // 70B at ~4 bit is 43.5 GB: more than 32 GB RAM and 16 GB VRAM.
        assert!(!passes(&summary(Some("70B"), true, false, "big"), &f, &m));
        assert!(passes(&summary(Some("7.6B"), true, false, "ok"), &f, &m));
        // Unknown parameter counts always pass.
        assert!(passes(&summary(None, true, false, "unk"), &f, &m));
        // Without the filter nothing is hidden.
        let off = Filters::default();
        assert!(passes(&summary(Some("70B"), true, false, "big"), &off, &m));
        // An undetected machine never hides anything.
        let blind = machine(0.0, None);
        assert!(passes(
            &summary(Some("70B"), true, false, "big"),
            &f,
            &blind
        ));
    }

    #[test]
    fn format_filter_hides_unchecked_formats() {
        let m = machine(32.0, Some(16.0));
        let gguf = summary(None, true, false, "g");
        let st = summary(None, false, true, "s");
        let other = summary(None, false, false, "o");
        let f = Filters::default();
        assert!(passes(&gguf, &f, &m) && passes(&st, &f, &m) && !passes(&other, &f, &m));
        let f = Filters {
            gguf: false,
            ..Filters::default()
        };
        assert!(!passes(&gguf, &f, &m) && passes(&st, &f, &m));
        let f = Filters {
            other: true,
            ..Filters::default()
        };
        assert!(passes(&other, &f, &m));
    }

    #[test]
    fn param_range_and_task_apply_client_side() {
        let m = machine(32.0, Some(16.0));
        let f = Filters {
            params: ParamRange::From3To14,
            ..Filters::default()
        };
        assert!(passes(&summary(Some("7B"), true, false, "a"), &f, &m));
        assert!(!passes(&summary(Some("70B"), true, false, "a"), &f, &m));
        assert!(passes(&summary(None, true, false, "a"), &f, &m));
        let f = Filters {
            embeddings: false,
            vision: false,
            ..Filters::default()
        };
        assert!(!passes(
            &summary(None, true, false, "bge-small-embed"),
            &f,
            &m
        ));
        assert!(!passes(&summary(None, true, false, "Qwen2-VL-7B"), &f, &m));
        assert!(passes(&summary(None, true, false, "Qwen2-7B"), &f, &m));
        assert_eq!(task_of("nomic-embed-text"), TaskKind::Embeddings);
        assert_eq!(task_of("llava-1.5-7b"), TaskKind::Vision);
        assert_eq!(task_of("Llama-3.2-1B"), TaskKind::Text);
    }
}
