//! Filter model for the Browse rail. Format and sort re-query Hugging Face;
//! parameter, task and "fits my machine" filters apply to loaded results.

use hf2ollama_core::fit::Machine;
use hf2ollama_core::hf::{FormatFilter, ModelSummary, SortKey};

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
    let _ = s;
    unimplemented!()
}

/// Summaries carry no pipeline tag, so the task is guessed from name tokens.
pub fn task_of(name: &str) -> TaskKind {
    let _ = name;
    unimplemented!()
}

/// Whether `summary` is shown under filters `f` on `machine`.
pub fn passes(summary: &ModelSummary, f: &Filters, machine: &Machine) -> bool {
    let _ = (summary, f, machine);
    unimplemented!()
}
