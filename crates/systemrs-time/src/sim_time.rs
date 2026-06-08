//! The [`SimTime`] integer time type.

use core::ops::{Add, AddAssign, Sub};

/// A count of resolution units of simulation time.
///
/// All arithmetic on the deterministic path is integer-only (saturating add), so
/// thread interleaving or partitioning can never reorder a floating-point
/// accumulation and change the committed timeline (`doc/systemrs-design.md` §8a).
///
/// The convenience constructors (`from_ps`, `from_ns`, …) assume the default 1 ps
/// resolution ([`crate::Resolution::PICOSECOND`]); for other resolutions use
/// [`SimTime::from_units`].
///
/// # Examples
///
/// ```
/// use systemrs_time::SimTime;
///
/// assert_eq!(SimTime::from_us(1), SimTime::from_ns(1_000)); // 1 µs = 1000 ns
/// assert_eq!(SimTime::from_ns(5) + SimTime::from_ns(3), SimTime::from_ns(8));
/// assert!(SimTime::from_ns(5) > SimTime::from_ns(3));
/// assert_eq!(SimTime::from_ns(5).units(), 5_000); // counted in picoseconds
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SimTime(u64);

impl SimTime {
    /// The zero time (`SC_ZERO_TIME`).
    pub const ZERO: SimTime = SimTime(0);

    /// The infinity sentinel; bit-for-bit equal to SystemC's `sc_time::max()`.
    pub const INF: SimTime = SimTime(u64::MAX);

    /// Constructs a time from a raw count of resolution units.
    ///
    /// # Arguments
    ///
    /// * `units` - The number of resolution units.
    ///
    /// # Returns
    ///
    /// The constructed [`SimTime`].
    pub const fn from_units(units: u64) -> Self {
        SimTime(units)
    }

    /// Returns the raw count of resolution units.
    pub const fn units(self) -> u64 {
        self.0
    }

    /// Constructs a time of `n` picoseconds (default-resolution convenience).
    pub const fn from_ps(n: u64) -> Self {
        SimTime(n)
    }

    /// Constructs a time of `n` nanoseconds (default-resolution convenience).
    pub const fn from_ns(n: u64) -> Self {
        SimTime(n.saturating_mul(1_000))
    }

    /// Constructs a time of `n` microseconds (default-resolution convenience).
    pub const fn from_us(n: u64) -> Self {
        SimTime(n.saturating_mul(1_000_000))
    }

    /// Returns `true` if this is the [`SimTime::ZERO`] time.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns `true` if this is the [`SimTime::INF`] sentinel.
    pub const fn is_inf(self) -> bool {
        self.0 == u64::MAX
    }

    /// Derives a delay by scaling this time by a floating-point factor, rounding
    /// to the nearest unit.
    ///
    /// This is the *only* sanctioned use of `f64` on a time value (e.g. deriving a
    /// clock's half-period as `period * 0.5`). It is a one-shot conversion applied
    /// *before* the result enters the committed timeline as an integer — never
    /// inside a per-step or per-region accumulation (`doc/systemrs-design.md` §8a).
    ///
    /// # Arguments
    ///
    /// * `factor` - The non-negative scaling factor.
    ///
    /// # Returns
    ///
    /// The scaled time, rounded to the nearest resolution unit and saturated.
    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        let scaled = self.0 as f64 * factor + 0.5;
        if scaled >= u64::MAX as f64 {
            SimTime::INF
        } else if scaled <= 0.0 {
            SimTime::ZERO
        } else {
            SimTime(scaled as u64)
        }
    }
}

impl Add for SimTime {
    type Output = SimTime;

    /// Integer saturating addition; `INF` is absorbing.
    fn add(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.saturating_add(rhs.0))
    }
}

impl AddAssign for SimTime {
    fn add_assign(&mut self, rhs: SimTime) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl Sub for SimTime {
    type Output = SimTime;

    /// Integer saturating subtraction (clamped at zero).
    fn sub(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.saturating_sub(rhs.0))
    }
}

impl core::fmt::Display for SimTime {
    /// Renders the time at the default 1 ps resolution, picking the largest clean
    /// SI unit (matching the `from_ps`/`from_ns`/`from_us` constructors).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_inf() {
            return write!(f, "INF");
        }
        match self.0 {
            0 => write!(f, "0 s"),
            v if v % 1_000_000 == 0 => write!(f, "{} us", v / 1_000_000),
            v if v % 1_000 == 0 => write!(f, "{} ns", v / 1_000),
            v => write!(f, "{v} ps"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SimTime;

    /// Verifies the infinity sentinel is exactly `u64::MAX` (SystemC parity).
    #[test]
    fn inf_is_all_ones() {
        assert_eq!(SimTime::INF.units(), u64::MAX);
        assert!(SimTime::INF.is_inf());
    }

    /// Verifies saturating addition does not wrap past infinity.
    #[test]
    fn add_saturates() {
        assert_eq!(SimTime::INF + SimTime::from_ns(1), SimTime::INF);
        assert_eq!(
            SimTime::from_ns(1) + SimTime::from_ns(1),
            SimTime::from_ns(2)
        );
    }

    /// Verifies the one-shot `scaled` rounding used for clock duty derivation.
    #[test]
    fn scaled_rounds_to_nearest() {
        assert_eq!(SimTime::from_ps(10).scaled(0.5), SimTime::from_ps(5));
        assert_eq!(SimTime::from_ps(3).scaled(0.5), SimTime::from_ps(2));
    }
}
