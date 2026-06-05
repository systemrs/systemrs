//! Processes: `SC_METHOD` (run-to-completion) and `SC_THREAD` (stackful coroutine).
//!
//! See `doc/systemrs-design.md` §3.2, §6a. A method is a plain `FnMut(&Ctx)`; a
//! thread is a [`systemrs_runtime::Fiber`] resumed serially. Both carry their
//! static sensitivity, their current dynamic wait state, and a `wait_gen` used to
//! lazily cancel superseded timeouts.

use crate::ctx::Ctx;
use crate::ids::EventId;
use systemrs_runtime::Fiber;
use systemrs_time::SimTime;

/// A boxed `SC_METHOD` body.
pub(crate) type MethodBody = Box<dyn FnMut(&Ctx)>;

/// The body of a process, mirroring `ProcessBody` in the design (§9).
pub(crate) enum ProcessBody {
    /// `SC_METHOD`: a stackless, run-to-completion callback. Re-armed via static
    /// sensitivity or `next_trigger`. Taken out of the arena while running.
    Method(Option<MethodBody>),

    /// `SC_THREAD`: a stackful coroutine. The continuation lives here; the kernel
    /// resumes it serially. Taken out of the arena while running.
    Thread(Option<Fiber>),
}

/// The request a process makes when it wants to (re-)arm its sensitivity.
///
/// Produced by `Ctx::wait_*` (threads, then suspend) and `Ctx::next_trigger_*`
/// (methods, then return); consumed by the scheduler which installs the matching
/// [`WaitState`].
#[derive(Debug, Clone)]
pub(crate) enum WaitReq {
    /// Wait for a relative amount of time (`wait(t)` / `next_trigger(t)`).
    Time(SimTime),

    /// Wait for a single event.
    Event(EventId),

    /// Wait for a single event, or a relative timeout, whichever is first.
    EventTimeout(EventId, SimTime),

    /// Wait for the first of several events (OR list).
    Or(Vec<EventId>),

    /// Wait for the first of several events, or a timeout.
    OrTimeout(Vec<EventId>, SimTime),

    /// Wait for all of several events (AND list).
    And(Vec<EventId>),
}

/// The installed dynamic-wait state of a process.
///
/// Mirrors the sensitivity state machine of `doc/systemrs-design.md` §6a.
#[derive(Debug, Clone)]
pub(crate) enum WaitState {
    /// Not waiting dynamically; wakes on its static sensitivity (methods) or has
    /// not yet started.
    Static,

    /// Waiting purely on a timeout (woken by a scheduled process-wake entry).
    Timed,

    /// Waiting on a single event.
    Event(EventId),

    /// Waiting on the first of an OR list.
    Or(Vec<EventId>),

    /// Waiting on all of an AND list; `remaining` holds the members that have yet
    /// to fire. Tracking the *set* (not a bare counter) keeps the decrement
    /// idempotent, so a duplicate/stale subscription to the same event cannot
    /// double-count and prematurely complete the AND.
    And { remaining: Vec<EventId> },
}

/// Why a process was most recently made runnable (delivered to a resumed thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    /// Woken normally by an event or by elapsed time.
    Normal,

    /// Cooperative cancellation requested (kill); the body should unwind.
    ///
    /// Reserved for the deferred kill/reset path (`doc/systemrs-design.md` §6a);
    /// the MVP marks dead processes and reaps them rather than throwing.
    Killed,
}

/// Whether a process is a method or a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcKind {
    /// An `SC_METHOD`.
    Method,

    /// An `SC_THREAD`.
    Thread,
}

/// A kernel process: its body, sensitivity, and run-queue bookkeeping.
pub(crate) struct Process {
    /// A human-readable hierarchical name (for diagnostics).
    pub(crate) name: String,

    /// Method or thread.
    pub(crate) kind: ProcKind,

    /// The executable body (taken out while running).
    pub(crate) body: ProcessBody,

    /// Events this process is statically sensitive to (persistent).
    pub(crate) static_sens: Vec<EventId>,

    /// Current dynamic-wait state.
    pub(crate) wait: WaitState,

    /// Bumped on every (re-)arm so a stale timed process-wake entry is recognised
    /// and skipped (lazy timeout cancellation).
    pub(crate) wait_gen: u64,

    /// The pending re-arm request set by the body before it returns/suspends.
    pub(crate) pending_wait: Option<WaitReq>,

    /// Why the process was last woken (delivered to threads on resume).
    pub(crate) wake: WakeReason,

    /// `true` while the process sits in a run queue (prevents double-queueing).
    pub(crate) queued: bool,

    /// `true` once the process has terminated and must not run again.
    pub(crate) dead: bool,
}

impl Process {
    /// Returns `true` if this process is currently in a dynamic wait (so its
    /// static sensitivity must be ignored until it next runs without re-arming
    /// dynamically). Mirrors SystemC's "next_trigger replaces static for one wake".
    pub(crate) fn in_dynamic_wait(&self) -> bool {
        !matches!(self.wait, WaitState::Static)
    }
}
