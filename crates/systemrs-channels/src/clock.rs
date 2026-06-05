//! `Clock`: a self-scheduling periodic boolean signal with edge events.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use systemrs_kernel::{ChanId, Ctx, EventId, Sim, UpdatableChannel};
use systemrs_time::SimTime;

/// Kernel-held state for a [`Clock`]: a double-buffered level plus edge events.
struct ClockState {
    /// The committed level.
    cur: Cell<bool>,

    /// The staged level (committed at update).
    new: Cell<bool>,

    /// Whether a toggle is staged this delta.
    pending: Cell<bool>,

    /// Fired on any level change.
    value_changed: EventId,

    /// Fired on a 0→1 transition.
    posedge: EventId,

    /// Fired on a 1→0 transition.
    negedge: EventId,
}

impl UpdatableChannel for ClockState {
    fn update(&self, ctx: &Ctx) {
        if !self.pending.get() {
            return;
        }
        self.pending.set(false);
        let old = self.cur.get();
        let new = self.new.get();
        self.cur.set(new);
        if old != new {
            ctx.notify(self.value_changed);
            if new {
                ctx.notify(self.posedge);
            } else {
                ctx.notify(self.negedge);
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A periodic clock driven by a self-retriggering `SC_METHOD`.
///
/// Implemented exactly as the design recommends (`doc/systemrs-design.md` §6c): a
/// process toggles a boolean and schedules the next edge via `next_trigger`, rather
/// than being special-cased in the kernel. Starting low, the first edge is a
/// posedge at time zero, then edges alternate every half-period.
#[derive(Clone, Copy)]
pub struct Clock {
    /// The channel id.
    id: ChanId,

    /// The value-changed event id.
    value_changed: EventId,

    /// The posedge event id.
    posedge: EventId,

    /// The negedge event id.
    negedge: EventId,
}

impl Clock {
    /// Creates a clock of the given period and starts its generator process.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (used to name the generator process).
    /// * `period` - The full clock period; edges occur every half-period.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new clock.
    pub fn new(sim: &Sim, name: &str, period: SimTime) -> Self {
        let value_changed = sim.alloc_event();
        let posedge = sim.alloc_event();
        let negedge = sim.alloc_event();
        let state = Rc::new(ClockState {
            cur: Cell::new(false),
            new: Cell::new(false),
            pending: Cell::new(false),
            value_changed,
            posedge,
            negedge,
        });
        let id = sim.register_channel(Rc::clone(&state) as Rc<dyn UpdatableChannel>);

        let half = period.scaled(0.5);
        let gen_state = Rc::clone(&state);
        let mut level = false;
        sim.add_method(&format!("{name}.gen"), &[], true, move |ctx| {
            level = !level;
            gen_state.new.set(level);
            gen_state.pending.set(true);
            ctx.request_update(id);
            ctx.next_trigger(half);
        });

        Clock {
            id,
            value_changed,
            posedge,
            negedge,
        }
    }

    /// Returns the committed level.
    ///
    /// # Arguments
    ///
    /// * `ctx` - A kernel handle.
    pub fn read(&self, ctx: &Ctx) -> bool {
        let rc = ctx.channel(self.id).expect("clock channel is registered");
        rc.as_any()
            .downcast_ref::<ClockState>()
            .expect("channel id refers to a Clock")
            .cur
            .get()
    }

    /// Returns the posedge event (0→1).
    pub fn posedge_event(&self) -> EventId {
        self.posedge
    }

    /// Returns the negedge event (1→0).
    pub fn negedge_event(&self) -> EventId {
        self.negedge
    }

    /// Returns the value-changed event.
    pub fn value_changed_event(&self) -> EventId {
        self.value_changed
    }
}
