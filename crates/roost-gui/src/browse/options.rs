//! Bundled option data: quality notes for GGUF quantizations and the
//! quantization table for safetensors imports.

use roost_core::compat::GgufOption;
use roost_core::fit::{fit, Fit, Machine};

/// A quality note and the 0..=1 length of its bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    pub note: &'static str,
    pub frac: f32,
}

/// Fixed lookup by quantization name (case-insensitive).
pub fn gguf_quality(quant: &str) -> Option<Quality> {
    let (note, frac) = match quant.to_ascii_uppercase().as_str() {
        "F16" | "BF16" => ("Full precision", 1.0),
        "Q8_0" => ("Near lossless", 1.0),
        "Q6_K" => ("Excellent", 0.96),
        "Q5_K_M" => ("Very good", 0.92),
        "Q4_K_M" => ("Good, best balance", 0.86),
        "Q4_0" => ("Legacy, use Q4_K_M", 0.78),
        "Q3_K_M" => ("Noticeable loss", 0.66),
        "Q2_K" => ("Heavy loss", 0.45),
        _ => return None,
    };
    Some(Quality { note, frac })
}

/// One selectable row of the options table.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionRow {
    pub label: String,
    pub size_bytes: u64,
    /// True when the size is an estimate (safetensors quantization).
    pub estimated: bool,
    pub note: Option<&'static str>,
    pub quality: f32,
    pub recommended: bool,
    /// Value for `ImportRequest::quantize`.
    pub quantize: Option<&'static str>,
}

pub fn gb(bytes: u64) -> f32 {
    bytes as f32 / 1_000_000_000.0
}

/// Index of the recommended GGUF option: Q4_K_M, else the largest that fits,
/// else the smallest.
pub fn recommended_gguf(options: &[GgufOption], machine: &Machine) -> usize {
    let q4 = options.iter().position(|o| {
        o.quant
            .as_deref()
            .is_some_and(|q| q.eq_ignore_ascii_case("Q4_K_M"))
    });
    if let Some(i) = q4 {
        return i;
    }
    let largest_fit = options
        .iter()
        .enumerate()
        .filter(|(_, o)| fit(gb(o.total_bytes), machine) == Fit::Fits)
        .max_by_key(|(_, o)| o.total_bytes)
        .map(|(i, _)| i);
    largest_fit
        .or_else(|| {
            options
                .iter()
                .enumerate()
                .min_by_key(|(_, o)| o.total_bytes)
                .map(|(i, _)| i)
        })
        .unwrap_or(0)
}

pub fn gguf_rows(options: &[GgufOption], machine: &Machine) -> Vec<OptionRow> {
    let rec = recommended_gguf(options, machine);
    options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let label = o.quant.clone().unwrap_or_else(|| o.name.clone());
            let q = o.quant.as_deref().and_then(gguf_quality);
            OptionRow {
                label,
                size_bytes: o.total_bytes,
                estimated: false,
                note: q.map(|q| q.note),
                quality: q.map_or(0.0, |q| q.frac),
                recommended: i == rec,
                quantize: None,
            }
        })
        .collect()
}

/// `none`, `int8` and `int4` for a repo whose files total `source_bytes`.
pub fn safetensors_rows(source_bytes: u64) -> Vec<OptionRow> {
    // (label, quantize value, note, quality, share of the source size)
    let table: [(&str, Option<&'static str>, &'static str, f32, u64); 3] = [
        ("none (full precision)", None, "Full precision", 1.0, 1),
        ("int8", Some("int8"), "Near lossless", 0.99, 2),
        ("int4", Some("int4"), "Some quality loss", 0.80, 4),
    ];
    table
        .iter()
        .map(|(label, quantize, note, quality, divisor)| OptionRow {
            label: (*label).to_string(),
            size_bytes: source_bytes / divisor,
            estimated: *divisor != 1,
            note: Some(note),
            quality: *quality,
            recommended: *quantize == Some("int8"),
            quantize: *quantize,
        })
        .collect()
}
