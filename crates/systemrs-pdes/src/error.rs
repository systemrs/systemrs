//! The crate's error type.

use thiserror::Error;

/// An error from constructing or verifying a Tier-1 PDES topology.
#[derive(Debug, Error)]
pub enum PdesError {
    /// A cross-region link was given a latency below the global quantum. The latency is
    /// the conservative-PDES lookahead and must be `>= quantum`, so a message sent in a
    /// quantum can never need delivery within that same quantum. Rejected at
    /// construction (`connect`), not at run time.
    #[error(
        "boundary link latency ({latency} units) is below the quantum ({quantum} units): \
         the lookahead must be >= the quantum"
    )]
    LatencyBelowQuantum {
        /// The offending link latency, in time-resolution units.
        latency: u64,
        /// The global quantum, in time-resolution units.
        quantum: u64,
    },

    /// A `verify_determinism`-style comparison found the Tier-0 and Tier-1 traces
    /// diverge at `index`. A divergence means the partition or quantum perturbed the
    /// observable result — a correctness bug, never expected on a valid partition.
    #[error("Tier-0 and Tier-1 traces diverge at record {index}")]
    TraceMismatch {
        /// The index of the first differing trace record.
        index: usize,
    },

    /// The two traces being compared have different lengths (`tier0` vs `tier1`).
    #[error("Tier-0 trace has {tier0} records but Tier-1 has {tier1}")]
    TraceLengthMismatch {
        /// The Tier-0 (golden, single-kernel) trace length.
        tier0: usize,
        /// The Tier-1 (partitioned) trace length.
        tier1: usize,
    },
}
