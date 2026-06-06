//! [`AnalysisTriple`] — a timestamped analysis value (`tlm_analysis_triple`).
//!
//! Pairs a broadcast value with the simulation time (and delta) it was produced at,
//! for timestamped telemetry over an `AnalysisPort<AnalysisTriple<T>>`
//! (`doc/systemrs-design.md` §3.7).

use systemrs_kernel::Ctx;
use systemrs_time::SimTime;

/// A value tagged with the time and delta it was produced at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisTriple<T> {
    /// The simulation time of production.
    pub time: SimTime,

    /// The delta count at production.
    pub delta: u64,

    /// The produced value.
    pub value: T,
}

impl<T> AnalysisTriple<T> {
    /// Creates a triple stamped with the current simulation time and delta.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    /// * `value` - The value to stamp.
    ///
    /// # Returns
    ///
    /// The timestamped [`AnalysisTriple`].
    pub fn now(ctx: &Ctx, value: T) -> Self {
        AnalysisTriple {
            time: ctx.now(),
            delta: ctx.delta_count(),
            value,
        }
    }
}
