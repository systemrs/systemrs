//! `Signal<T>` (skip-if-unchanged) and `Buffer<T>` (always-fire) value channels.

use std::any::Any;
use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;

use systemrs_kernel::{ChanId, Ctx, EventId, Sim, UpdatableChannel};

/// Kernel-held state for a [`Signal`]/[`Buffer`]: a double-buffered value plus its
/// value-changed event.
pub(crate) struct SignalState<T: Copy> {
    /// The committed value (visible to `read`).
    cur: Cell<T>,

    /// The staged value (set by `write`, committed at update).
    new: Cell<T>,

    /// Whether a write is staged for this delta.
    pending: Cell<bool>,

    /// `true` for buffer semantics (fire on every write), `false` to skip
    /// unchanged writes (signal semantics) — the observable distinction of §3.6.
    always_fire: bool,

    /// The value-changed event, fired one delta after a committing write.
    value_changed: EventId,
}

impl<T: Copy + PartialEq + 'static> UpdatableChannel for SignalState<T> {
    fn update(&self, ctx: &Ctx) {
        if !self.pending.get() {
            return;
        }
        self.pending.set(false);
        let old = self.cur.get();
        let new = self.new.get();
        self.cur.set(new);
        if self.always_fire || old != new {
            ctx.notify(self.value_changed); // fires next delta
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A deterministic double-buffered value channel (`sc_signal`).
///
/// `write` stages a value; `read` returns the value committed at the previous
/// update. The value-changed event fires the delta after a committing write, and
/// only when the value actually changed.
///
/// # Examples
///
/// A write in one delta is visible to a reader in the next:
///
/// ```
/// use systemrs_channels::Signal;
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
///
/// let sim = Sim::new();
/// let sig: Signal<u32> = Signal::new(&sim, "s", 0);
/// sim.add_thread("driver", &[], true, move |cx| {
///     assert_eq!(sig.read(cx), 0); // the initial value
///     sig.write(cx, 7);
///     assert_eq!(sig.read(cx), 0); // not yet — still the old committed value
///     cx.wait(SimTime::from_ns(1)); // cross an update boundary
///     assert_eq!(sig.read(cx), 7); // now committed
/// });
/// sim.run_until(SimTime::from_ns(10));
/// ```
#[derive(Clone, Copy)]
pub struct Signal<T: Copy> {
    /// The channel id in the kernel arena.
    id: ChanId,

    /// The value-changed event id.
    value_changed: EventId,

    /// Carries `T` without owning it (the state lives in the arena).
    _t: PhantomData<T>,
}

impl<T: Copy + PartialEq + 'static> Signal<T> {
    /// Creates a signal initialized to `init`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved for a future name registry).
    /// * `init` - The initial committed value.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new signal.
    pub fn new(sim: &Sim, name: &str, init: T) -> Self {
        let _ = name;
        let (id, value_changed) = register(sim, init, false);
        Signal {
            id,
            value_changed,
            _t: PhantomData,
        }
    }

    /// Returns the committed value.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle (e.g. `sim.ctx()` or a process's `cx`).
    pub fn read(&self, ctx: &Ctx) -> T {
        with_state::<T, _, _>(ctx, self.id, |st| st.cur.get())
    }

    /// Stages `value` for commit at the update phase and requests an update.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    /// * `value` - The value to stage.
    pub fn write(&self, ctx: &Ctx, value: T) {
        with_state::<T, _, _>(ctx, self.id, |st| {
            st.new.set(value);
            st.pending.set(true);
        });
        ctx.request_update(self.id);
    }

    /// Returns the value-changed event (for static sensitivity).
    pub fn value_changed_event(&self) -> EventId {
        self.value_changed
    }

    /// Returns the channel id.
    pub fn id(&self) -> ChanId {
        self.id
    }
}

/// A value channel that fires its event on *every* write (`sc_buffer`), even when
/// the value is unchanged — the observable distinction from [`Signal`] (§3.6).
#[derive(Clone, Copy)]
pub struct Buffer<T: Copy> {
    /// The underlying state handle (with `always_fire = true`).
    inner: Signal<T>,
}

impl<T: Copy + PartialEq + 'static> Buffer<T> {
    /// Creates a buffer initialized to `init`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved for a future name registry).
    /// * `init` - The initial committed value.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new buffer.
    pub fn new(sim: &Sim, name: &str, init: T) -> Self {
        let _ = name;
        let (id, value_changed) = register(sim, init, true);
        Buffer {
            inner: Signal {
                id,
                value_changed,
                _t: PhantomData,
            },
        }
    }

    /// Returns the committed value.
    pub fn read(&self, ctx: &Ctx) -> T {
        self.inner.read(ctx)
    }

    /// Stages `value` and requests an update (the event fires regardless of change).
    pub fn write(&self, ctx: &Ctx, value: T) {
        self.inner.write(ctx, value);
    }

    /// Returns the value-changed event.
    pub fn value_changed_event(&self) -> EventId {
        self.inner.value_changed_event()
    }
}

/// Registers a fresh signal/buffer state and returns its `(ChanId, value_changed)`.
fn register<T: Copy + PartialEq + 'static>(
    sim: &Sim,
    init: T,
    always_fire: bool,
) -> (ChanId, EventId) {
    let value_changed = sim.alloc_event();
    let state = Rc::new(SignalState {
        cur: Cell::new(init),
        new: Cell::new(init),
        pending: Cell::new(false),
        always_fire,
        value_changed,
    });
    let id = sim.register_channel(state);
    (id, value_changed)
}

/// Downcasts the kernel-held channel state for `id` to `SignalState<T>` and runs
/// `f` against it.
fn with_state<T, F, R>(ctx: &Ctx, id: ChanId, f: F) -> R
where
    T: Copy + PartialEq + 'static,
    F: FnOnce(&SignalState<T>) -> R,
{
    let rc = ctx.channel(id).expect("signal channel is registered");
    let st = rc
        .as_any()
        .downcast_ref::<SignalState<T>>()
        .expect("channel id refers to a Signal of this value type");
    f(st)
}
