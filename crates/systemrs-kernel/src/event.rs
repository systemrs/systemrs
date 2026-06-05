//! Events: the fundamental synchronization object.
//!
//! Reproduces SystemC's `sc_event` (`doc/systemrs-design.md` §3.3, §6a): four
//! subscriber lists walked in a fixed, *verified* order on `trigger()`, a single
//! pending-notification state machine with the immediate > delta > timed collapse
//! rules, and a `trigger_stamp` underpinning `triggered()`.

use crate::ids::ProcId;
use systemrs_time::SimTime;

/// The single pending-notification state of an event.
///
/// At most one notification is pending; the earliest wins (`doc/systemrs-design.md`
/// §3.3). Immediate notifications do not live here — they fire synchronously.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pending {
    /// No notification is pending.
    None,

    /// A delta notification is pending; `idx` is its slot in the delta-event list
    /// (so it can be cancelled in O(1) by swap-remove).
    Delta { idx: usize },

    /// A timed notification is pending at absolute time `when`, tagged by the
    /// insertion sequence `seq` so a tombstoned heap entry can be recognised.
    Timed { seq: u64, when: SimTime },
}

/// A kernel event with its four subscriber lists and pending state.
///
/// The subscriber lists are split static/dynamic × method/thread exactly as in
/// `sc_event.cpp:378-458`, so `trigger()` can reproduce the observable ordering.
pub(crate) struct Event {
    /// Methods statically sensitive to this event (persistent subscription).
    pub(crate) static_methods: Vec<ProcId>,

    /// Threads statically sensitive to this event (persistent subscription).
    pub(crate) static_threads: Vec<ProcId>,

    /// Methods dynamically waiting on this event (consumed on fire).
    pub(crate) dynamic_methods: Vec<ProcId>,

    /// Threads dynamically waiting on this event (consumed on fire).
    pub(crate) dynamic_threads: Vec<ProcId>,

    /// The single pending notification, if any.
    pub(crate) pending: Pending,

    /// The `change_stamp` at which this event most recently fired; `triggered()`
    /// is `trigger_stamp == change_stamp`.
    pub(crate) trigger_stamp: u64,
}

impl Event {
    /// Creates an event with no subscribers and no pending notification.
    pub(crate) fn new() -> Self {
        Event {
            static_methods: Vec::new(),
            static_threads: Vec::new(),
            dynamic_methods: Vec::new(),
            dynamic_threads: Vec::new(),
            pending: Pending::None,
            // Seeded to u64::MAX (matching SystemC's `m_trigger_stamp(~UINT64_ZERO)`,
            // sc_event.cpp:280) so a never-fired event never aliases the initial
            // `change_stamp == 0` and `triggered()` correctly returns false.
            trigger_stamp: u64::MAX,
        }
    }
}
