use serde::{Deserialize, Serialize};

/// Memory a machine has available for running a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub ram_gb: f32,
    pub vram_gb: Option<f32>,
}

/// How well a model of a given size is expected to run on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Fits,
    Tight,
    PartialOffload,
    TooLarge,
    Unknown,
}

/// Estimate how a model of `size_gb` will fit on machine `m`.
pub fn fit(_size_gb: f32, _m: &Machine) -> Fit {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        Machine {
            ram_gb: 32.0,
            vram_gb: Some(16.0),
        }
    }

    #[test]
    fn fits_when_well_under_vram() {
        assert_eq!(fit(4.68, &machine()), Fit::Fits);
    }

    #[test]
    fn tight_at_exactly_vram() {
        assert_eq!(fit(14.5, &machine()), Fit::Tight);
    }

    #[test]
    fn partial_offload_when_over_vram_under_ram() {
        assert_eq!(fit(20.0, &machine()), Fit::PartialOffload);
    }

    #[test]
    fn too_large_over_ram() {
        assert_eq!(fit(40.0, &machine()), Fit::TooLarge);
    }

    #[test]
    fn unknown_when_vram_missing() {
        let m = Machine {
            ram_gb: 32.0,
            vram_gb: None,
        };
        assert_eq!(fit(4.68, &m), Fit::Unknown);
        assert_eq!(fit(40.0, &m), Fit::Unknown);
    }

    #[test]
    fn boundary_fits_at_80_percent() {
        assert_eq!(fit(11.3, &machine()), Fit::Fits);
    }
}
