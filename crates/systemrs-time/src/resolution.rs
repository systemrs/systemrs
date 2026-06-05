//! Time resolution: the physical meaning of one [`crate::SimTime`] unit.

/// The physical size of one simulation-time unit, in femtoseconds.
///
/// SystemC freezes a process-wide resolution on first use; SystemRS instead makes
/// it a construction parameter frozen by the elaboration→run typestate
/// (`doc/systemrs-design.md` §6a). The default is 1 picosecond, matching SystemC.
///
/// The [`crate::SimTime`] physical-unit constructors (`from_ns`, …) assume the
/// default 1 ps resolution; finer control uses raw [`crate::SimTime::from_units`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Resolution {
    /// Number of femtoseconds represented by one time unit.
    fs_per_unit: u64,
}

impl Resolution {
    /// One femtosecond per unit (the finest SystemC resolution).
    pub const FEMTOSECOND: Resolution = Resolution { fs_per_unit: 1 };

    /// One picosecond per unit (SystemC's default resolution).
    pub const PICOSECOND: Resolution = Resolution { fs_per_unit: 1_000 };

    /// One nanosecond per unit.
    pub const NANOSECOND: Resolution = Resolution {
        fs_per_unit: 1_000_000,
    };

    /// Constructs a resolution from a femtoseconds-per-unit count.
    ///
    /// # Arguments
    ///
    /// * `fs_per_unit` - Femtoseconds represented by one time unit; must be nonzero.
    ///
    /// # Returns
    ///
    /// The constructed [`Resolution`].
    pub const fn from_fs_per_unit(fs_per_unit: u64) -> Self {
        Self { fs_per_unit }
    }

    /// Returns the number of femtoseconds per unit.
    pub const fn fs_per_unit(self) -> u64 {
        self.fs_per_unit
    }

    /// Returns the SI suffix label for this resolution, when it matches a common
    /// power-of-ten boundary; otherwise `"fs(units)"`.
    pub fn label(self) -> &'static str {
        match self.fs_per_unit {
            1 => "fs",
            1_000 => "ps",
            1_000_000 => "ns",
            1_000_000_000 => "us",
            _ => "units",
        }
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Resolution::PICOSECOND
    }
}
