//! The [`Sim`] builder/driver: elaboration-time construction plus the crunch loop.
//!
//! `Sim` owns the shared kernel state and orchestrates the borrow discipline that
//! makes the design's "running fiber reaches the kernel via a thread-local, never
//! holds `&mut Inner` across a suspension" rule sound (`doc/systemrs-design.md`
//! §6a): the driver borrows `Inner` only to pop the next runnable and to install
//! the next wait, releasing it around each body's execution.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use systemrs_runtime::{Fiber, FiberState};
use systemrs_time::{Resolution, SimTime};

use crate::channel::UpdatableChannel;
use crate::ctx::{self, Ctx};
use crate::ids::{ChanId, EventId, ProcId};
use crate::inner::Inner;
use crate::phase::Phase;
use crate::process::{ProcKind, Process, ProcessBody, WaitState, WakeReason};

/// The top-level simulation: an elaboration-time builder that becomes a runner.
///
/// Construction (events, channels, processes, services) happens before
/// [`Sim::run_until`]; the static hierarchy is immutable once simulation starts.
/// This is the runtime-checked analogue of the design's `Building → Running`
/// typestate (`doc/systemrs-design.md` §6a).
pub struct Sim {
    /// Shared kernel state, also handed to every [`Ctx`].
    inner: Rc<RefCell<Inner>>,
}

impl Sim {
    /// Creates a new simulation with the default (1 ps) resolution.
    pub fn new() -> Self {
        Self::with_resolution(Resolution::default())
    }

    /// Creates a new simulation with an explicit time resolution.
    ///
    /// # Arguments
    ///
    /// * `resolution` - The frozen time resolution.
    ///
    /// # Returns
    ///
    /// A fresh [`Sim`] in the elaboration phase.
    pub fn with_resolution(resolution: Resolution) -> Self {
        Sim {
            inner: Rc::new(RefCell::new(Inner::new(resolution))),
        }
    }

    /// Returns the current simulation time.
    pub fn now(&self) -> SimTime {
        self.inner.borrow().now
    }

    /// Returns the number of completed delta cycles.
    pub fn delta_count(&self) -> u64 {
        self.inner.borrow().delta_count
    }

    /// Returns an elaboration-time [`Ctx`] (e.g. to seed channel state).
    pub fn ctx(&self) -> Ctx {
        Ctx::from_inner(Rc::clone(&self.inner))
    }

    // ---- Elaboration-time construction ---------------------------------------

    /// Allocates a fresh event and returns its id.
    pub fn alloc_event(&self) -> EventId {
        self.inner.borrow_mut().alloc_event()
    }

    /// Registers an updatable channel and returns its id.
    ///
    /// # Arguments
    ///
    /// * `chan` - The shared channel handle.
    ///
    /// # Returns
    ///
    /// The channel's [`ChanId`].
    pub fn register_channel(&self, chan: Rc<dyn UpdatableChannel>) -> ChanId {
        self.inner.borrow_mut().chans.insert(chan)
    }

    /// Registers a type-keyed service reachable from any process via
    /// [`Ctx::service`].
    ///
    /// # Arguments
    ///
    /// * `svc` - The shared service handle.
    pub fn register_service<T: Any>(&self, svc: Rc<T>) {
        self.inner
            .borrow_mut()
            .services
            .insert(std::any::TypeId::of::<T>(), svc);
    }

    /// Registers an `SC_METHOD` process.
    ///
    /// # Arguments
    ///
    /// * `name` - A hierarchical name for diagnostics.
    /// * `static_sens` - Events the method is statically sensitive to.
    /// * `initialize` - If `true`, the method runs once at start of simulation.
    /// * `body` - The run-to-completion callback.
    ///
    /// # Returns
    ///
    /// The new process's [`ProcId`].
    pub fn add_method<F>(
        &self,
        name: &str,
        static_sens: &[EventId],
        initialize: bool,
        body: F,
    ) -> ProcId
    where
        F: FnMut(&Ctx) + 'static,
    {
        self.add_process(
            name,
            ProcKind::Method,
            ProcessBody::Method(Some(Box::new(body))),
            static_sens,
            initialize,
        )
    }

    /// Registers an `SC_THREAD` process (a stackful coroutine).
    ///
    /// # Arguments
    ///
    /// * `name` - A hierarchical name for diagnostics.
    /// * `static_sens` - Events the thread is statically sensitive to.
    /// * `initialize` - If `true`, the thread starts at start of simulation.
    /// * `body` - The thread body; may call `Ctx::wait` from any depth. Must be
    ///   `Send` (corosensei's requirement).
    ///
    /// # Returns
    ///
    /// The new process's [`ProcId`].
    pub fn add_thread<F>(
        &self,
        name: &str,
        static_sens: &[EventId],
        initialize: bool,
        body: F,
    ) -> ProcId
    where
        F: FnOnce(&Ctx) + Send + 'static,
    {
        let fiber = Fiber::new(move || {
            let ctx = Ctx::current();
            body(&ctx);
        });
        self.add_process(
            name,
            ProcKind::Thread,
            ProcessBody::Thread(Some(fiber)),
            static_sens,
            initialize,
        )
    }

    /// Shared process registration: inserts the process, wires static sensitivity,
    /// and records it as an initial process if requested.
    fn add_process(
        &self,
        name: &str,
        kind: ProcKind,
        body: ProcessBody,
        static_sens: &[EventId],
        initialize: bool,
    ) -> ProcId {
        let mut g = self.inner.borrow_mut();
        let pid = g.procs.insert(Process {
            name: name.to_owned(),
            kind,
            body,
            static_sens: static_sens.to_vec(),
            wait: WaitState::Static,
            wait_gen: 0,
            pending_wait: None,
            wake: WakeReason::Normal,
            queued: false,
            dead: false,
        });
        for &ev in static_sens {
            match kind {
                ProcKind::Method => g.events[ev].static_methods.push(pid),
                ProcKind::Thread => g.events[ev].static_threads.push(pid),
            }
        }
        if initialize {
            g.initial_procs.push(pid);
        }
        pid
    }

    // ---- The driver -----------------------------------------------------------

    /// Runs the simulation until `end`, advancing through delta cycles and timed
    /// events, with run-to-time starvation semantics.
    ///
    /// # Arguments
    ///
    /// * `end` - The time to stop at.
    pub fn run_until(&self, end: SimTime) {
        let _guard = ctx::install_current(&self.inner);
        self.ensure_started();

        loop {
            self.crunch();

            let next = self.inner.borrow_mut().next_timed_when();
            match next {
                None => break,
                Some(when) if when > end => {
                    let mut g = self.inner.borrow_mut();
                    if g.now < end {
                        g.now = end;
                    }
                    break;
                }
                Some(when) => {
                    self.do_timestep(when);
                    self.inner.borrow_mut().fire_timed_at(when);
                }
            }
        }
    }

    /// Marks the simulation started and queues initial processes.
    fn ensure_started(&self) {
        let mut g = self.inner.borrow_mut();
        if g.started {
            return;
        }
        g.started = true;
        let initial = std::mem::take(&mut g.initial_procs);
        for pid in initial {
            g.procs[pid].queued = true;
            match g.procs[pid].kind {
                ProcKind::Method => g.method_push.push_back(pid),
                ProcKind::Thread => g.thread_push.push_back(pid),
            }
        }
    }

    /// Runs delta cycles at the current time until the runnable set is empty
    /// (`crunch`, `doc/systemrs-design.md` §6a).
    fn crunch(&self) {
        loop {
            // ---- EVALUATE: methods to completion, then one thread, repeat ----
            self.inner.borrow_mut().phase = Phase::Evaluate;
            let mut ran = false;
            loop {
                self.toggle_methods();
                while let Some(pid) = self.pop_method() {
                    self.run_method(pid);
                    ran = true;
                }
                self.toggle_threads();
                if let Some(pid) = self.pop_thread() {
                    self.resume_thread(pid);
                    ran = true;
                    continue;
                }
                if self.inner.borrow().runnable_empty() {
                    break;
                }
            }

            // EMPTY-DELTA GUARD: an empty evaluate advances neither counter.
            if !ran {
                break;
            }

            // ---- UPDATE ----
            let to_update = {
                let mut g = self.inner.borrow_mut();
                g.phase = Phase::Update;
                g.change_stamp += 1; // bump before perform_update, non-empty only
                let q = std::mem::take(&mut g.update_queue);
                g.update_pending.clear();
                q
            };
            let ctx = self.ctx();
            for cid in to_update {
                let chan = self.inner.borrow().chans.get(cid).cloned();
                if let Some(chan) = chan {
                    chan.update(&ctx);
                }
            }

            // ---- DELTA NOTIFY (high-index → 0) ----
            {
                let mut g = self.inner.borrow_mut();
                g.phase = Phase::Notify;
                let evs = std::mem::take(&mut g.delta_events);
                for ev in evs.into_iter().rev() {
                    let fire = matches!(
                        g.events.get(ev).map(|e| e.pending),
                        Some(crate::event::Pending::Delta { .. })
                    );
                    if fire {
                        g.events[ev].pending = crate::event::Pending::None;
                        g.trigger(ev);
                    }
                }
                // Resume zero-time (`wait(SC_ZERO_TIME)`) waiters for the next delta.
                let wakes = std::mem::take(&mut g.delta_wakes);
                for (pid, generation) in wakes {
                    let live = g
                        .procs
                        .get(pid)
                        .is_some_and(|p| !p.dead && p.wait_gen == generation);
                    if live {
                        g.make_runnable(pid);
                    }
                }
                g.delta_count += 1; // non-empty only
            }

            if self.inner.borrow().runnable_empty() {
                break;
            }
        }
    }

    /// Swaps the method push buffer into the pop buffer if the pop buffer is empty.
    fn toggle_methods(&self) {
        let g = &mut *self.inner.borrow_mut();
        if g.method_pop.is_empty() {
            std::mem::swap(&mut g.method_pop, &mut g.method_push);
        }
    }

    /// Swaps the thread push buffer into the pop buffer if the pop buffer is empty.
    fn toggle_threads(&self) {
        let g = &mut *self.inner.borrow_mut();
        if g.thread_pop.is_empty() {
            std::mem::swap(&mut g.thread_pop, &mut g.thread_push);
        }
    }

    /// Pops the next runnable method, if any.
    fn pop_method(&self) -> Option<ProcId> {
        self.inner.borrow_mut().method_pop.pop_front()
    }

    /// Pops the next runnable thread, if any.
    fn pop_thread(&self) -> Option<ProcId> {
        self.inner.borrow_mut().thread_pop.pop_front()
    }

    /// Runs one method body to completion, then installs its next sensitivity.
    fn run_method(&self, pid: ProcId) {
        let mut body = {
            let mut g = self.inner.borrow_mut();
            g.running = Some(pid);
            g.procs[pid].queued = false;
            g.procs[pid].pending_wait = None;
            match &mut g.procs[pid].body {
                ProcessBody::Method(slot) => slot.take().expect("method re-entered while running"),
                ProcessBody::Thread(_) => unreachable!("method pid is not a thread"),
            }
        };

        let ctx = self.ctx();
        body(&ctx);

        let mut g = self.inner.borrow_mut();
        g.running = None;
        if !g.procs[pid].dead
            && let ProcessBody::Method(slot) = &mut g.procs[pid].body
        {
            *slot = Some(body);
        }
        let req = g.procs[pid].pending_wait.take();
        match req {
            Some(r) => g.install_wait(pid, r),
            None => g.procs[pid].wait = WaitState::Static, // re-arm static sensitivity
        }
    }

    /// Resumes one thread until its next `wait()` or its return, then installs its
    /// next sensitivity (or terminates it).
    fn resume_thread(&self, pid: ProcId) {
        let mut fiber = {
            let mut g = self.inner.borrow_mut();
            g.running = Some(pid);
            g.procs[pid].queued = false;
            g.procs[pid].pending_wait = None;
            match &mut g.procs[pid].body {
                ProcessBody::Thread(slot) => slot.take().expect("thread re-entered while running"),
                ProcessBody::Method(_) => unreachable!("thread pid is not a method"),
            }
        };

        let state = fiber.resume();

        let mut g = self.inner.borrow_mut();
        g.running = None;
        match state {
            FiberState::Suspended => {
                if let ProcessBody::Thread(slot) = &mut g.procs[pid].body {
                    *slot = Some(fiber);
                }
                let req = g.procs[pid]
                    .pending_wait
                    .take()
                    .expect("a suspended thread must have requested a wait");
                g.install_wait(pid, req);
            }
            FiberState::Done => {
                g.procs[pid].dead = true;
                // `fiber` is dropped here (already finished, no force-unwind needed).
            }
        }
    }

    /// Advances time to `when`, bumping the change stamp on every time advance.
    fn do_timestep(&self, when: SimTime) {
        let mut g = self.inner.borrow_mut();
        debug_assert!(g.now < when, "time must advance monotonically");
        // Tracing stage callbacks (PreTimestep) would fire here once -trace lands.
        g.now = when;
        g.change_stamp += 1; // bump on every time advance (sc_simcontext.cpp:986)
        g.delta_count_baseline_at_now = g.delta_count;
    }
}

impl Default for Sim {
    fn default() -> Self {
        Sim::new()
    }
}
