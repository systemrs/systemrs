//! The [`Ctx`] handle: a process body's window into the kernel.
//!
//! A running process reaches the kernel through a thread-local `Ctx` set for the
//! duration of one `run_until` call (exactly like `sc_get_curr_simcontext()`,
//! `doc/systemrs-design.md` §6a). The handle never holds `&mut Inner` across a
//! suspension; it borrows re-entrantly only for the duration of each call, which
//! returns before the next `wait()`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use systemrs_time::SimTime;

use crate::ids::{ChanId, EventId, ProcId};
use crate::inner::Inner;
use crate::process::{ProcKind, Process, ProcessBody, WaitReq, WaitState, WakeReason};

thread_local! {
    /// The kernel state of the simulation running on this thread, if any.
    static CURRENT_SIM: RefCell<Option<Rc<RefCell<Inner>>>> = const { RefCell::new(None) };
}

/// Installs `inner` as the current simulation for the calling thread and returns a
/// guard that clears it on drop.
pub(crate) fn install_current(inner: &Rc<RefCell<Inner>>) -> CurrentGuard {
    CURRENT_SIM.with(|c| *c.borrow_mut() = Some(Rc::clone(inner)));
    CurrentGuard
}

/// RAII guard clearing the thread-local current simulation.
pub(crate) struct CurrentGuard;

impl Drop for CurrentGuard {
    fn drop(&mut self) {
        CURRENT_SIM.with(|c| *c.borrow_mut() = None);
    }
}

/// A handle a process body uses to interact with the kernel: query time, wait,
/// notify events, request channel updates, and reach services.
#[derive(Clone)]
pub struct Ctx {
    /// Shared kernel state. Borrowed briefly per call, never across a `wait()`.
    inner: Rc<RefCell<Inner>>,
}

impl Ctx {
    /// Constructs a `Ctx` from shared kernel state.
    pub(crate) fn from_inner(inner: Rc<RefCell<Inner>>) -> Self {
        Ctx { inner }
    }

    /// Returns the `Ctx` for the simulation currently running on this thread.
    ///
    /// # Panics
    ///
    /// Panics if no simulation is running (no `run_until` is on the stack).
    pub fn current() -> Self {
        CURRENT_SIM.with(|c| {
            let g = c.borrow();
            let inner = g
                .as_ref()
                .expect("Ctx::current() called outside a running simulation");
            Ctx {
                inner: Rc::clone(inner),
            }
        })
    }

    /// Returns the shared kernel state (crate-internal, for the driver).
    pub(crate) fn inner(&self) -> &Rc<RefCell<Inner>> {
        &self.inner
    }

    // ---- Time -----------------------------------------------------------------

    /// Returns the current simulation time.
    pub fn now(&self) -> SimTime {
        self.inner.borrow().now
    }

    /// Returns the number of completed delta cycles.
    pub fn delta_count(&self) -> u64 {
        self.inner.borrow().delta_count
    }

    /// Returns the simulation's frozen time resolution.
    ///
    /// Used by real-time pacing to convert sim time to wall-clock duration
    /// (`doc/systemrs-design.md` §6f).
    pub fn resolution(&self) -> systemrs_time::Resolution {
        self.inner.borrow().resolution
    }

    /// Returns `true` if `ev` fired in the current change-stamp window
    /// (`triggered()` = `trigger_stamp == change_stamp`).
    pub fn triggered(&self, ev: EventId) -> bool {
        let g = self.inner.borrow();
        g.events
            .get(ev)
            .is_some_and(|e| e.trigger_stamp == g.change_stamp)
    }

    // ---- Events ---------------------------------------------------------------

    /// Allocates a fresh, unsubscribed event at runtime and returns its id.
    ///
    /// The runtime analogue of [`crate::Sim::alloc_event`]; used by AT adapters to
    /// create per-transaction completion events mid-simulation.
    ///
    /// # Returns
    ///
    /// The new [`EventId`].
    pub fn alloc_event(&self) -> EventId {
        self.inner.borrow_mut().alloc_event()
    }

    // ---- Notification ---------------------------------------------------------

    /// Fires an immediate notification (evaluate phase only; a determinism hazard,
    /// exposed-but-discouraged per `doc/systemrs-design.md` §13).
    pub fn notify_now(&self, ev: EventId) {
        self.inner.borrow_mut().notify_immediate(ev);
    }

    /// Arms a delta notification (`notify(SC_ZERO_TIME)`).
    pub fn notify(&self, ev: EventId) {
        self.inner.borrow_mut().notify_delta(ev);
    }

    /// Arms a timed notification after a relative delay (delta if `after` is zero).
    pub fn notify_after(&self, ev: EventId, after: SimTime) {
        self.inner.borrow_mut().notify_timed(ev, after);
    }

    /// Cancels any pending notification on `ev`.
    pub fn cancel(&self, ev: EventId) {
        self.inner.borrow_mut().cancel(ev);
    }

    // ---- Channel update ------------------------------------------------------

    /// Returns the kernel-held channel state for `chan`, if registered.
    ///
    /// Typed channel handles downcast the returned `Rc<dyn UpdatableChannel>` to
    /// their concrete state type via [`crate::UpdatableChannel::as_any`].
    ///
    /// # Arguments
    ///
    /// * `chan` - The channel id.
    ///
    /// # Returns
    ///
    /// A shared handle to the channel state, or `None` if `chan` is not registered.
    pub fn channel(&self, chan: ChanId) -> Option<Rc<dyn crate::UpdatableChannel>> {
        self.inner.borrow().chans.get(chan).cloned()
    }

    /// Requests that channel `chan` be updated this delta (idempotent).
    pub fn request_update(&self, chan: ChanId) {
        let mut g = self.inner.borrow_mut();
        if g.update_pending.insert(chan, ()).is_none() {
            g.update_queue.push(chan);
        }
    }

    // ---- Thread waits (suspend) ----------------------------------------------

    /// Records a re-arm request on the running process, without suspending.
    fn arm(&self, req: WaitReq) {
        let mut g = self.inner.borrow_mut();
        let pid = g
            .running
            .expect("wait/next_trigger called outside a process");
        g.procs[pid].pending_wait = Some(req);
    }

    /// Suspends the running thread for a relative amount of time.
    ///
    /// Callable from any call depth inside an `SC_THREAD` body (the design's
    /// central property; e.g. inside `b_transport`).
    pub fn wait(&self, t: SimTime) {
        self.arm(WaitReq::Time(t));
        systemrs_runtime::suspend();
    }

    /// Suspends the running thread until `ev` fires.
    pub fn wait_event(&self, ev: EventId) {
        self.arm(WaitReq::Event(ev));
        systemrs_runtime::suspend();
    }

    /// Suspends until `ev` fires or `timeout` elapses, whichever is first.
    pub fn wait_event_timeout(&self, ev: EventId, timeout: SimTime) {
        self.arm(WaitReq::EventTimeout(ev, timeout));
        systemrs_runtime::suspend();
    }

    /// Suspends until any event in `events` fires (OR list).
    pub fn wait_any(&self, events: &[EventId]) {
        self.arm(WaitReq::Or(events.to_vec()));
        systemrs_runtime::suspend();
    }

    /// Suspends until all events in `events` have fired (AND list).
    pub fn wait_all(&self, events: &[EventId]) {
        self.arm(WaitReq::And(events.to_vec()));
        systemrs_runtime::suspend();
    }

    // ---- Runtime spawn --------------------------------------------------------

    /// Spawns a new `SC_THREAD` at runtime, made runnable in the current delta.
    ///
    /// Unlike the elaboration-time `Sim::add_thread`, this may be called from inside
    /// a running process to start a fresh stackful coroutine — e.g. an AT→LT adapter
    /// starting a per-transaction worker. The new thread is queued FIFO by spawn
    /// order and picked up by the current `crunch`'s next thread batch, exactly like
    /// a freshly-woken thread (preserving the evaluate→update→notify ordering).
    ///
    /// # Arguments
    ///
    /// * `name` - A hierarchical name for diagnostics.
    /// * `body` - The thread body; it may `Ctx::wait` from any call depth. It must be
    ///   `Send` (inherited from the coroutine backend), so it reaches kernel and model
    ///   state through `Ctx::current` and registered services — never by capturing a
    ///   `!Send` handle such as an `Rc`.
    ///
    /// # Returns
    ///
    /// The new process's [`ProcId`].
    pub fn spawn_thread<F>(&self, name: &str, body: F) -> ProcId
    where
        F: FnOnce(&Ctx) + Send + 'static,
    {
        let fiber = crate::sim::build_thread_fiber(body);
        let mut g = self.inner.borrow_mut();
        let pid = g.procs.insert(Process {
            name: name.to_owned(),
            kind: ProcKind::Thread,
            body: ProcessBody::Thread(Some(fiber)),
            static_sens: Vec::new(),
            wait: WaitState::Static,
            wait_gen: 0,
            pending_wait: None,
            wake: WakeReason::Normal,
            queued: false,
            dead: false,
        });
        g.make_runnable(pid);
        pid
    }

    // ---- Method next-trigger (no suspend) ------------------------------------

    /// Sets the running method's sensitivity for its next invocation to a relative
    /// time.
    pub fn next_trigger(&self, t: SimTime) {
        self.arm(WaitReq::Time(t));
    }

    /// Sets the running method's next-invocation sensitivity to a single event.
    pub fn next_trigger_event(&self, ev: EventId) {
        self.arm(WaitReq::Event(ev));
    }

    /// Sets the running method's next-invocation sensitivity to an OR list.
    pub fn next_trigger_any(&self, events: &[EventId]) {
        self.arm(WaitReq::Or(events.to_vec()));
    }

    // ---- Services -------------------------------------------------------------

    /// Retrieves a previously-registered service of type `T`.
    ///
    /// # Returns
    ///
    /// The shared service handle, or `None` if no service of type `T` is set.
    pub fn try_service<T: Any>(&self) -> Option<Rc<T>> {
        let g = self.inner.borrow();
        g.services
            .get(&std::any::TypeId::of::<T>())
            .cloned()
            .and_then(|rc| rc.downcast::<T>().ok())
    }

    /// Retrieves a service of type `T`, panicking if it is not registered.
    ///
    /// # Panics
    ///
    /// Panics if no service of type `T` has been registered.
    pub fn service<T: Any>(&self) -> Rc<T> {
        self.try_service::<T>()
            .expect("requested service is not registered")
    }
}
