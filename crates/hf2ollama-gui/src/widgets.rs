//! Custom-drawn widgets ported from the approved egui spike.

use hf2ollama_core::fit::Fit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillKind {
    Ok,
    Warn,
    Neutral,
}

pub fn fit_pill_kind(_fit: Fit) -> PillKind {
    todo!()
}

pub fn fit_pill_label(_fit: Fit) -> &'static str {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_pill_kind_maps_every_fit_variant() {
        let cases = [
            (Fit::Fits, "Fits in VRAM", PillKind::Ok),
            (Fit::Tight, "Tight fit", PillKind::Warn),
            (Fit::PartialOffload, "Partial CPU offload", PillKind::Warn),
            (Fit::TooLarge, "Too large", PillKind::Warn),
            (Fit::Unknown, "Fit unknown", PillKind::Neutral),
        ];
        for (fit, label, kind) in cases {
            assert_eq!(fit_pill_kind(fit), kind, "{fit:?}");
            assert_eq!(fit_pill_label(fit), label, "{fit:?}");
        }
    }
}
