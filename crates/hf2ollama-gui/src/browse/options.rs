//! Bundled option data: quality notes for GGUF quantizations and the
//! quantization table for safetensors imports.

use hf2ollama_core::compat::GgufOption;
use hf2ollama_core::fit::Machine;

/// A quality note and the 0..=1 length of its bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    pub note: &'static str,
    pub frac: f32,
}

/// Fixed lookup by quantization name (case-insensitive).
pub fn gguf_quality(quant: &str) -> Option<Quality> {
    let _ = quant;
    unimplemented!()
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
    let _ = (options, machine);
    unimplemented!()
}

pub fn gguf_rows(options: &[GgufOption], machine: &Machine) -> Vec<OptionRow> {
    let _ = (options, machine);
    unimplemented!()
}

/// `none`, `int8` and `int4` for a repo whose files total `source_bytes`.
pub fn safetensors_rows(source_bytes: u64) -> Vec<OptionRow> {
    let _ = source_bytes;
    unimplemented!()
}
