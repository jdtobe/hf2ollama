#![allow(dead_code)] // stubs; removed in implementation commit

use crate::config::Settings;
use crate::fit::Machine;

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
/// Share of unified memory the GPU can use on Apple Silicon.
const APPLE_GPU_SHARE: f64 = 0.7;

/// What the GPU probe found. Injectable so the conversion is testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuProbe {
    Nvidia { total_vram_bytes: u64 },
    AppleUnified { total_ram_bytes: u64 },
    None,
}

/// Convert a probe result to usable VRAM in GB.
fn vram_from(probe: GpuProbe) -> Option<f32> {
    let _ = probe;
    todo!()
}

/// Probe the real GPU. Never fails: any problem yields `GpuProbe::None`.
fn probe_gpu() -> GpuProbe {
    todo!()
}

/// Detect this machine's RAM and VRAM.
pub fn detect() -> Machine {
    todo!()
}

/// The machine to use for fit estimates: the user's override, else detected.
pub fn effective(settings: &Settings) -> Machine {
    let _ = settings;
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_over_detected() {
        let settings = Settings {
            hardware_override: Some(Machine {
                ram_gb: 8.0,
                vram_gb: Some(4.0),
            }),
            ..Settings::default()
        };
        assert_eq!(
            effective(&settings),
            Machine {
                ram_gb: 8.0,
                vram_gb: Some(4.0)
            }
        );
    }

    #[test]
    fn effective_falls_back_to_detect_when_no_override() {
        let settings = Settings {
            hardware_override: None,
            ..Settings::default()
        };
        assert_eq!(effective(&settings), detect());
    }

    #[test]
    fn detect_reports_positive_ram() {
        assert!(detect().ram_gb > 0.0);
    }

    #[test]
    fn vram_none_when_undetectable() {
        assert_eq!(vram_from(GpuProbe::None), None);
    }

    #[test]
    fn vram_from_nvidia_converts_bytes_to_gb() {
        let bytes = 16 * 1024 * 1024 * 1024;
        assert_eq!(
            vram_from(GpuProbe::Nvidia {
                total_vram_bytes: bytes
            }),
            Some(16.0)
        );
    }

    #[test]
    fn vram_from_apple_is_70_percent() {
        let bytes = 32 * 1024 * 1024 * 1024;
        assert_eq!(
            vram_from(GpuProbe::AppleUnified {
                total_ram_bytes: bytes
            }),
            Some(22.4)
        );
    }
}
