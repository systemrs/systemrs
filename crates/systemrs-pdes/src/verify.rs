//! Determinism verification: compare a Tier-0 (golden, single-kernel) trace to a Tier-1
//! (partitioned) trace.

use crate::error::PdesError;

/// Asserts two observable traces are bit-identical, returning the first divergence.
///
/// This is the `--verify-determinism` primitive, shipped from day one
/// (`doc/systemrs-design.md` §8a): build a model both as a single Tier-0
/// [`LocalHost`](crate::LocalHost) and as a Tier-1 [`Orchestrator`](crate::Orchestrator)
/// partition, run each to the same `end` with the same quantum, collect a comparable
/// trace from each, and call this. A divergence is a correctness bug — the partition or
/// quantum must never perturb the observable result.
///
/// # Arguments
///
/// * `tier0` - The golden single-kernel trace.
/// * `tier1` - The partitioned trace to check against it.
///
/// # Errors
///
/// [`PdesError::TraceLengthMismatch`] if the traces differ in length, or
/// [`PdesError::TraceMismatch`] at the first differing record.
pub fn assert_traces_match<R: PartialEq>(tier0: &[R], tier1: &[R]) -> Result<(), PdesError> {
    if tier0.len() != tier1.len() {
        return Err(PdesError::TraceLengthMismatch {
            tier0: tier0.len(),
            tier1: tier1.len(),
        });
    }
    for (index, (a, b)) in tier0.iter().zip(tier1.iter()).enumerate() {
        if a != b {
            return Err(PdesError::TraceMismatch { index });
        }
    }
    Ok(())
}
