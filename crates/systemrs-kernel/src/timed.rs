//! The timed-event wheel.
//!
//! A min-heap keyed by `(when, seq)` so equal-time ordering is **deterministic** —
//! FIFO by insertion sequence, pinning what SystemC leaves heap-defined
//! (`doc/systemrs-design.md` §6a). Timed cancellation is lazy: a popped entry whose
//! tag no longer matches is a tombstone and is skipped.

use crate::ids::{EventId, ProcId};
use core::cmp::Ordering;
use systemrs_time::SimTime;

/// What a timed entry does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimedTarget {
    /// Fire a timed event notification.
    Event {
        /// The event to trigger; valid only if its pending tag still matches `seq`.
        ev: EventId,
    },

    /// Wake a process from a pure-time wait or a timeout.
    Wake {
        /// The process to wake.
        pid: ProcId,

        /// The process's `wait_gen` at arming time; a mismatch means the timeout
        /// was superseded (the process already woke another way) and is skipped.
        generation: u64,
    },
}

/// One entry in the timed-event min-heap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimedEntry {
    /// Absolute fire time.
    pub(crate) when: SimTime,

    /// Monotonic insertion sequence; the deterministic tie-break for equal `when`.
    pub(crate) seq: u64,

    /// What to do when this entry fires.
    pub(crate) target: TimedTarget,
}

impl PartialEq for TimedEntry {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when && self.seq == other.seq
    }
}

impl Eq for TimedEntry {}

impl Ord for TimedEntry {
    /// Orders as a *min*-heap on `(when, seq)`: the reversed comparison makes
    /// [`std::collections::BinaryHeap`] (a max-heap) pop the soonest, lowest-seq
    /// entry first.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .when
            .cmp(&self.when)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for TimedEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
