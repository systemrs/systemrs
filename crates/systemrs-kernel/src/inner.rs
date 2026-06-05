//! The kernel's mutable state ([`Inner`]) and the scheduling logic that operates
//! under a single borrow (notification collapse, `trigger` ordering, sensitivity,
//! the timed wheel). The borrow-releasing *driver* (the crunch loop that runs
//! process bodies) lives in [`crate::sim`].

use std::any::{Any, TypeId};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::rc::Rc;

use slotmap::{SecondaryMap, SlotMap};
use systemrs_time::{Resolution, SimTime};

use crate::channel::UpdatableChannel;
use crate::event::{Event, Pending};
use crate::ids::{ChanId, EventId, ProcId};
use crate::phase::Phase;
use crate::process::{ProcKind, Process, WaitReq, WaitState};
use crate::timed::{TimedEntry, TimedTarget};

/// All mutable kernel state, owned behind a single `Rc<RefCell<Inner>>`.
///
/// Mirrors the `Kernel` struct of `doc/systemrs-design.md` §6a: arenas keyed by
/// generational ids, the runnable double-buffers, the update queue, the delta-event
/// vector, the timed heap, and the determinism counters.
pub(crate) struct Inner {
    /// Process arena.
    pub(crate) procs: SlotMap<ProcId, Process>,

    /// Event arena.
    pub(crate) events: SlotMap<EventId, Event>,

    /// Updatable-channel arena (shared `Rc` so the driver can clone-out for update).
    pub(crate) chans: SlotMap<ChanId, Rc<dyn UpdatableChannel>>,

    /// Type-keyed service store (e.g. the TLM socket registry), reachable from any
    /// process body via `Ctx::service`. This is the internal "extension" seam of
    /// the design's ECS-flavoured store (§9) — invisible to model authors.
    pub(crate) services: HashMap<TypeId, Rc<dyn Any>>,

    /// Methods being drained this evaluate-batch.
    pub(crate) method_pop: VecDeque<ProcId>,

    /// Methods made runnable *during* this batch (picked up by the next toggle).
    pub(crate) method_push: VecDeque<ProcId>,

    /// Threads being drained this evaluate-batch.
    pub(crate) thread_pop: VecDeque<ProcId>,

    /// Threads made runnable *during* this batch.
    pub(crate) thread_push: VecDeque<ProcId>,

    /// Channels that requested an update this delta, in request order.
    pub(crate) update_queue: Vec<ChanId>,

    /// Idempotency flags for `update_queue` (`request_update` is idempotent/delta).
    pub(crate) update_pending: SecondaryMap<ChanId, ()>,

    /// Events with a pending delta notification, drained high-index→0 each delta.
    pub(crate) delta_events: Vec<EventId>,

    /// Processes performing a zero-time wait (`wait(SC_ZERO_TIME)`); made runnable
    /// for the next delta during the delta-notify phase.
    pub(crate) delta_wakes: Vec<(ProcId, u64)>,

    /// The timed-event min-heap (deterministic `(when, seq)` order).
    pub(crate) timed: BinaryHeap<TimedEntry>,

    /// Processes to make runnable at start of simulation (non-`dont_initialize`).
    pub(crate) initial_procs: Vec<ProcId>,

    /// Current simulation time.
    pub(crate) now: SimTime,

    /// Number of completed (non-empty) delta cycles.
    pub(crate) delta_count: u64,

    /// The change stamp; bumped at the top of every non-empty update phase and on
    /// every time advance. Underpins `triggered()`/`event()` (§3.1).
    pub(crate) change_stamp: u64,

    /// `delta_count` at the start of the current time (informational; mirrors
    /// `m_initial_delta_count_at_current_time`).
    pub(crate) delta_count_baseline_at_now: u64,

    /// The current scheduler phase.
    pub(crate) phase: Phase,

    /// Monotonic tie-break sequence for the timed heap.
    pub(crate) seq: u64,

    /// The currently running process (the self-notification guard's `cur`).
    pub(crate) running: Option<ProcId>,

    /// Whether `start_of_simulation` has run.
    pub(crate) started: bool,

    /// The frozen time resolution.
    pub(crate) resolution: Resolution,
}

impl Inner {
    /// Creates empty kernel state at time zero with the given resolution.
    pub(crate) fn new(resolution: Resolution) -> Self {
        Inner {
            procs: SlotMap::with_key(),
            events: SlotMap::with_key(),
            chans: SlotMap::with_key(),
            services: HashMap::new(),
            method_pop: VecDeque::new(),
            method_push: VecDeque::new(),
            thread_pop: VecDeque::new(),
            thread_push: VecDeque::new(),
            update_queue: Vec::new(),
            update_pending: SecondaryMap::new(),
            delta_events: Vec::new(),
            delta_wakes: Vec::new(),
            timed: BinaryHeap::new(),
            initial_procs: Vec::new(),
            now: SimTime::ZERO,
            delta_count: 0,
            change_stamp: 0,
            delta_count_baseline_at_now: 0,
            phase: Phase::Build,
            seq: 0,
            running: None,
            started: false,
            resolution,
        }
    }

    /// Allocates a fresh, unsubscribed event and returns its id.
    pub(crate) fn alloc_event(&mut self) -> EventId {
        self.events.insert(Event::new())
    }

    /// Returns the next monotonic sequence number (timed-heap tie-break).
    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    // ---- Notification ---------------------------------------------------------

    /// Fires an immediate notification (legal only in the evaluate phase).
    ///
    /// # Panics
    ///
    /// Panics if called outside the evaluate phase (`SC_ID_IMMEDIATE_NOTIFICATION`).
    pub(crate) fn notify_immediate(&mut self, ev: EventId) {
        assert_eq!(
            self.phase,
            Phase::Evaluate,
            "immediate notification is legal only in the evaluate phase"
        );
        self.cancel(ev);
        self.trigger(ev);
    }

    /// Arms a delta notification (collapse: delta beats a pending timed, idempotent
    /// with a pending delta).
    pub(crate) fn notify_delta(&mut self, ev: EventId) {
        match self.events[ev].pending {
            Pending::Delta { .. } => {}
            Pending::Timed { .. } => {
                self.cancel(ev);
                self.arm_delta(ev);
            }
            Pending::None => self.arm_delta(ev),
        }
    }

    /// Arms a timed notification (collapse: delta always wins; otherwise the
    /// soonest survives).
    pub(crate) fn notify_timed(&mut self, ev: EventId, after: SimTime) {
        if after.is_zero() {
            self.notify_delta(ev);
            return;
        }
        let when = self.now + after;
        match self.events[ev].pending {
            Pending::Delta { .. } => {}
            Pending::Timed { when: w, .. } if w <= when => {}
            _ => {
                self.cancel(ev);
                self.arm_timed(ev, when);
            }
        }
    }

    /// Cancels any pending notification on an event.
    pub(crate) fn cancel(&mut self, ev: EventId) {
        match self.events[ev].pending {
            Pending::Delta { idx } => self.cancel_delta(idx),
            // Timed: lazy tombstone — the heap entry is skipped on pop because the
            // event's pending no longer carries its seq.
            Pending::Timed { .. } | Pending::None => {}
        }
        self.events[ev].pending = Pending::None;
    }

    /// Arms a delta notification, recording the delta-vector slot for O(1) cancel.
    fn arm_delta(&mut self, ev: EventId) {
        let idx = self.delta_events.len();
        self.delta_events.push(ev);
        self.events[ev].pending = Pending::Delta { idx };
    }

    /// Arms a timed notification on the heap with a fresh sequence tag.
    fn arm_timed(&mut self, ev: EventId, when: SimTime) {
        let seq = self.next_seq();
        self.events[ev].pending = Pending::Timed { seq, when };
        self.timed.push(TimedEntry {
            when,
            seq,
            target: TimedTarget::Event { ev },
        });
    }

    /// Removes a delta-vector entry by swap-remove, fixing the moved event's index.
    fn cancel_delta(&mut self, idx: usize) {
        let last = self.delta_events.len() - 1;
        self.delta_events.swap(idx, last);
        self.delta_events.pop();
        if idx < self.delta_events.len() {
            let moved = self.delta_events[idx];
            if let Pending::Delta { idx: slot } = &mut self.events[moved].pending {
                *slot = idx;
            }
        }
    }

    // ---- Trigger (the verified subscriber ordering) ---------------------------

    /// Fires an event: walks the four subscriber lists in the verified order and
    /// iteration direction (`sc_event.cpp:378-458`), making subscribers runnable.
    ///
    /// Order: static methods → dynamic methods → static threads → dynamic threads.
    /// Static lists iterate high-index→0; dynamic lists iterate 0→high with
    /// consumed entries swapped in from the tail (`doc/systemrs-design.md` §6a).
    pub(crate) fn trigger(&mut self, ev: EventId) {
        self.events[ev].trigger_stamp = self.change_stamp;

        // (1) static methods, high-index → 0
        let sm = std::mem::take(&mut self.events[ev].static_methods);
        let mut i = sm.len();
        while i > 0 {
            i -= 1;
            self.make_runnable_static(sm[i]);
        }
        self.events[ev].static_methods = sm;

        // (2) dynamic methods, 0 → high with tail swap-in
        self.fire_dynamic(ev, ProcKind::Method);

        // (3) static threads, high-index → 0
        let st = std::mem::take(&mut self.events[ev].static_threads);
        let mut i = st.len();
        while i > 0 {
            i -= 1;
            self.make_runnable_static(st[i]);
        }
        self.events[ev].static_threads = st;

        // (4) dynamic threads, 0 → high with tail swap-in
        self.fire_dynamic(ev, ProcKind::Thread);
    }

    /// Walks one dynamic subscriber list (0→high, tail swap-in), consuming entries.
    fn fire_dynamic(&mut self, ev: EventId, kind: ProcKind) {
        let mut list = match kind {
            ProcKind::Method => std::mem::take(&mut self.events[ev].dynamic_methods),
            ProcKind::Thread => std::mem::take(&mut self.events[ev].dynamic_threads),
        };
        let mut i = 0;
        while i < list.len() {
            let pid = list[i];
            if self.trigger_dynamic(pid, ev) {
                let last = list.len() - 1;
                list.swap(i, last);
                list.pop();
                // re-examine the swapped-in element at index i
            } else {
                i += 1;
            }
        }
        match kind {
            ProcKind::Method => self.events[ev].dynamic_methods = list,
            ProcKind::Thread => self.events[ev].dynamic_threads = list,
        }
    }

    /// Resolves a dynamic subscriber against the firing event.
    ///
    /// # Returns
    ///
    /// `true` if the entry is consumed (woken, satisfied, or stale) and must be
    /// removed from the list; `false` to keep the subscription.
    fn trigger_dynamic(&mut self, pid: ProcId, ev: EventId) -> bool {
        let Some(p) = self.procs.get_mut(pid) else {
            return true; // stale id → remove
        };
        if p.dead {
            return true;
        }
        let wake = match &mut p.wait {
            WaitState::Event(e) => *e == ev,
            WaitState::Or(list) => list.contains(&ev),
            WaitState::And { remaining } => {
                // Idempotent: only decrement if this event is still an outstanding
                // member, so a stale/duplicate subscription cannot double-count.
                if let Some(pos) = remaining.iter().position(|&e| e == ev) {
                    remaining.remove(pos);
                    remaining.is_empty()
                } else {
                    false
                }
            }
            // Not dynamically waiting on this event → stale, remove.
            WaitState::Static | WaitState::Timed => false,
        };
        if wake {
            self.wake_process(pid);
        }
        true
    }

    /// Wakes a process: invalidates any pending timeout, clears its dynamic wait,
    /// and makes it runnable.
    fn wake_process(&mut self, pid: ProcId) {
        if let Some(p) = self.procs.get_mut(pid) {
            p.wait_gen += 1;
            p.wait = WaitState::Static;
        }
        self.make_runnable(pid);
    }

    /// Makes a statically-sensitive subscriber runnable, but only if it is not
    /// currently in a dynamic wait (next_trigger/wait replaces static for one wake).
    fn make_runnable_static(&mut self, pid: ProcId) {
        match self.procs.get(pid) {
            Some(p) if !p.dead && !p.in_dynamic_wait() => {}
            _ => return,
        }
        self.make_runnable(pid);
    }

    /// Makes a process runnable, applying the immediate self-notification guard and
    /// the "cannot queue twice" rule.
    pub(crate) fn make_runnable(&mut self, pid: ProcId) {
        // Per-process self-notification guard: the currently-running process is
        // never re-made-runnable by its own (immediate) notification.
        if self.running == Some(pid) {
            return;
        }
        let Some(p) = self.procs.get_mut(pid) else {
            return;
        };
        if p.dead || p.queued {
            return;
        }
        p.queued = true;
        match p.kind {
            ProcKind::Method => self.method_push.push_back(pid),
            ProcKind::Thread => self.thread_push.push_back(pid),
        }
    }

    // ---- Sensitivity ----------------------------------------------------------

    /// Subscribes a process to an event's dynamic list, by kind.
    fn subscribe_dynamic(&mut self, ev: EventId, pid: ProcId, kind: ProcKind) {
        match kind {
            ProcKind::Method => self.events[ev].dynamic_methods.push(pid),
            ProcKind::Thread => self.events[ev].dynamic_threads.push(pid),
        }
    }

    /// Schedules a timed process-wake (for pure-time waits and timeouts).
    fn schedule_proc_wake(&mut self, pid: ProcId, generation: u64, when: SimTime) {
        let seq = self.next_seq();
        self.timed.push(TimedEntry {
            when,
            seq,
            target: TimedTarget::Wake { pid, generation },
        });
    }

    /// Installs a (re-)arm request as the process's dynamic wait state.
    pub(crate) fn install_wait(&mut self, pid: ProcId, req: WaitReq) {
        let kind = self.procs[pid].kind;
        // Bump wait_gen so any previously-armed timeout for this process is stale.
        self.procs[pid].wait_gen += 1;
        let generation = self.procs[pid].wait_gen;

        match req {
            WaitReq::Time(t) => {
                self.procs[pid].wait = WaitState::Timed;
                if t.is_zero() {
                    // wait(SC_ZERO_TIME): resume one delta later.
                    self.delta_wakes.push((pid, generation));
                } else {
                    let when = self.now + t;
                    self.schedule_proc_wake(pid, generation, when);
                }
            }
            WaitReq::Event(ev) => {
                self.procs[pid].wait = WaitState::Event(ev);
                self.subscribe_dynamic(ev, pid, kind);
            }
            WaitReq::EventTimeout(ev, t) => {
                self.procs[pid].wait = WaitState::Event(ev);
                self.subscribe_dynamic(ev, pid, kind);
                let when = self.now + t;
                self.schedule_proc_wake(pid, generation, when);
            }
            WaitReq::Or(list) => {
                for &ev in &list {
                    self.subscribe_dynamic(ev, pid, kind);
                }
                self.procs[pid].wait = WaitState::Or(list);
            }
            WaitReq::OrTimeout(list, t) => {
                for &ev in &list {
                    self.subscribe_dynamic(ev, pid, kind);
                }
                self.procs[pid].wait = WaitState::Or(list);
                let when = self.now + t;
                self.schedule_proc_wake(pid, generation, when);
            }
            WaitReq::And(list) => {
                for &ev in &list {
                    self.subscribe_dynamic(ev, pid, kind);
                }
                self.procs[pid].wait = WaitState::And { remaining: list };
            }
        }
    }

    // ---- Timed wheel ----------------------------------------------------------

    /// Returns the time of the next *live* timed entry, discarding tombstones from
    /// the top of the heap.
    pub(crate) fn next_timed_when(&mut self) -> Option<SimTime> {
        loop {
            let top = *self.timed.peek()?;
            if self.is_live(&top) {
                return Some(top.when);
            }
            self.timed.pop();
        }
    }

    /// Returns `true` if a timed entry still refers to a pending notification or an
    /// un-superseded process wait.
    fn is_live(&self, entry: &TimedEntry) -> bool {
        match entry.target {
            TimedTarget::Event { ev } => matches!(
                self.events.get(ev).map(|e| e.pending),
                Some(Pending::Timed { seq, .. }) if seq == entry.seq
            ),
            TimedTarget::Wake { pid, generation } => self
                .procs
                .get(pid)
                .is_some_and(|p| !p.dead && p.wait_gen == generation),
        }
    }

    /// Fires all live timed entries at exactly `when` (called after time advanced
    /// to `when`).
    pub(crate) fn fire_timed_at(&mut self, when: SimTime) {
        while let Some(top) = self.timed.peek() {
            if top.when != when {
                break;
            }
            let entry = self.timed.pop().expect("peeked entry must pop");
            match entry.target {
                TimedTarget::Event { ev } => {
                    let live = matches!(
                        self.events.get(ev).map(|e| e.pending),
                        Some(Pending::Timed { seq, .. }) if seq == entry.seq
                    );
                    if live {
                        self.events[ev].pending = Pending::None;
                        self.trigger(ev);
                    }
                }
                TimedTarget::Wake { pid, generation } => {
                    let live = self
                        .procs
                        .get(pid)
                        .is_some_and(|p| !p.dead && p.wait_gen == generation);
                    if live {
                        self.wake_process(pid);
                    }
                }
            }
        }
    }

    /// Returns `true` if no method or thread is currently runnable.
    pub(crate) fn runnable_empty(&self) -> bool {
        self.method_pop.is_empty()
            && self.method_push.is_empty()
            && self.thread_pop.is_empty()
            && self.thread_push.is_empty()
    }
}
