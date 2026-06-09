//! [`IrqLine`] — a first-class level-sensitive interrupt line.
//!
//! SystemRS models interrupts ad-hoc as a bare kernel `EventId` (e.g.
//! `systemrs-examples`' DMA engine notifies a completion event). That edge-only idiom
//! is wrong for a CPU interrupt controller: RISC-V `mip` (and most real interrupt
//! pins) are **level-sensitive** — a source holds the line high until it is serviced,
//! and a hart must observe that *level* between instructions, not just an edge.
//!
//! [`IrqLine`] is the single shared contract both an in-process CPU model and a future
//! out-of-process co-simulation bridge observe identically: a driver (the peripheral)
//! [`raise`](IrqLine::raise)s and [`lower`](IrqLine::lower)s the line; a consumer (the
//! CPU) samples [`pending`](IrqLine::pending) each instruction *and* may park on
//! [`changed_event`](IrqLine::changed_event) (the WFI wait target) so an idle hart
//! wakes on assertion.
//!
//! The line is a `Copy`/`Send` handle (an index plus a kernel `EventId`) — so it can be
//! captured by an `SC_THREAD` body, whose closure must be `Send`. The levels live in a
//! kernel-owned [`IrqController`] service (the same "state in a `Sim` service, handle by
//! id" discipline the socket registry uses), keeping the synchronous core `!Send` while
//! the handle stays `Send`. Level changes are observed immediately (no delta delay), so
//! the trap-sampling logic is straightforwardly combinational.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::{Ctx, EventId, Sim};

/// The kernel-owned table of interrupt-line levels (a `Sim` service).
///
/// One `bool` per [`IrqLine`], indexed by the line's slot. Held behind a `RefCell`
/// because every line shares the single service instance.
struct IrqController {
    /// The asserted level of each allocated line, indexed by [`IrqLine::index`].
    levels: RefCell<Vec<bool>>,

    /// An aggregate event notified whenever *any* line changes level — the single
    /// wait target a consumer parks on to wake on the next interrupt from any source
    /// (see [`irq_wake_event`]).
    wake: EventId,
}

impl IrqController {
    /// Returns the simulation's IRQ controller service, creating it on first use.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Returns
    ///
    /// The shared [`IrqController`] service.
    fn get(sim: &Sim) -> Rc<IrqController> {
        let ctx = sim.ctx();
        if let Some(existing) = ctx.try_service::<IrqController>() {
            return existing;
        }
        let controller = Rc::new(IrqController {
            levels: RefCell::new(Vec::new()),
            wake: sim.alloc_event(),
        });
        sim.register_service(Rc::clone(&controller));
        controller
    }
}

/// Returns the aggregate event notified whenever *any* interrupt line changes level.
///
/// A consumer (a CPU executing wait-for-interrupt) parks on this single event to wake
/// on the next change from any source, instead of juggling one event per line. The
/// controller is created on first use, so this is valid even before any line exists.
///
/// # Arguments
///
/// * `sim` - The simulation.
///
/// # Returns
///
/// The aggregate "any line changed" event.
pub fn irq_wake_event(sim: &Sim) -> EventId {
    IrqController::get(sim).wake
}

/// A level-sensitive interrupt line shared between a driver and a consumer.
///
/// A `Copy` handle: the peripheral and the CPU hold copies of the *same* line (same
/// slot, same change event). The line is level-sensitive — it stays asserted until the
/// driver lowers it (or the consumer [`ack`](IrqLine::ack)s it) — so a held-high line
/// is observed by the consumer's level sample rather than re-firing an edge each cycle.
///
/// # Examples
///
/// A driver raises the line; a consumer parked on the change event wakes, sees it
/// pending, then acknowledges it:
///
/// ```
/// use systemrs_tlm_utils::IrqLine;
/// use systemrs_kernel::Sim;
/// use systemrs_time::SimTime;
///
/// let sim = Sim::new();
/// let line = IrqLine::new(&sim);
///
/// let driver = line;
/// sim.add_thread("source", &[], true, move |cx| {
///     cx.wait(SimTime::from_ns(5));
///     driver.raise(cx);
/// });
///
/// let consumer = line;
/// let ev = line.changed_event();
/// sim.add_thread("cpu", &[], true, move |cx| {
///     if !consumer.pending(cx) {
///         cx.wait_event(ev); // park until the line changes
///     }
///     assert!(consumer.pending(cx));
///     consumer.ack(cx); // deassert
/// });
///
/// sim.run_until(SimTime::from_ns(20));
/// assert!(!line.is_asserted(&sim));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IrqLine {
    /// This line's slot in the [`IrqController`] level table.
    index: usize,

    /// The kernel event notified on every level change; the WFI/idle wait target.
    changed: EventId,
}

impl IrqLine {
    /// Creates a new, deasserted interrupt line.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction (allocates a level slot and the
    ///   change event).
    ///
    /// # Returns
    ///
    /// A fresh [`IrqLine`] at level `false`.
    pub fn new(sim: &Sim) -> Self {
        let controller = IrqController::get(sim);
        let index = {
            let mut levels = controller.levels.borrow_mut();
            levels.push(false);
            levels.len() - 1
        };
        IrqLine {
            index,
            changed: sim.alloc_event(),
        }
    }

    /// Drives the line to `level`, notifying the change event on a transition.
    ///
    /// A no-op (no notification) when the level is unchanged, so repeatedly holding a
    /// line high does not flood the scheduler with edges.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    /// * `level` - The new asserted level.
    pub fn set_level(&self, cx: &Ctx, level: bool) {
        let controller = cx.service::<IrqController>();
        let changed = {
            let mut levels = controller.levels.borrow_mut();
            if levels[self.index] == level {
                false
            } else {
                levels[self.index] = level;
                true
            }
        };
        if changed {
            cx.notify(self.changed);
            cx.notify(controller.wake);
        }
    }

    /// Asserts the line (level → `true`), waking any waiter on a rising edge.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn raise(&self, cx: &Ctx) {
        self.set_level(cx, true);
    }

    /// Deasserts the line (level → `false`).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn lower(&self, cx: &Ctx) {
        self.set_level(cx, false);
    }

    /// Consumer-side acknowledgment: deasserts the line.
    ///
    /// Semantically identical to [`lower`](IrqLine::lower); named for the consumer
    /// (CPU) side that clears an interrupt it has serviced (a software-interrupt or
    /// edge-latched source). Level-driven sources (a timer holding `MTIP` until its
    /// compare register is rewritten) should instead [`lower`](IrqLine::lower) from the
    /// driver when the cause clears.
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    pub fn ack(&self, cx: &Ctx) {
        self.set_level(cx, false);
    }

    /// Returns the current asserted level (`true` = an interrupt is pending).
    ///
    /// # Arguments
    ///
    /// * `cx` - The kernel handle.
    ///
    /// # Returns
    ///
    /// `true` if the line is currently asserted.
    pub fn pending(&self, cx: &Ctx) -> bool {
        cx.service::<IrqController>().levels.borrow()[self.index]
    }

    /// Returns the kernel event notified on every level change.
    ///
    /// A consumer executing a wait-for-interrupt parks on this event; it is notified on
    /// both rising and falling edges (a falling edge yields a harmless spurious wake the
    /// consumer dismisses via [`pending`](IrqLine::pending)).
    pub fn changed_event(&self) -> EventId {
        self.changed
    }

    /// Backdoor: reads the line's level outside a process (for testbench inspection).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation.
    ///
    /// # Returns
    ///
    /// `true` if the line is currently asserted.
    pub fn is_asserted(&self, sim: &Sim) -> bool {
        IrqController::get(sim).levels.borrow()[self.index]
    }
}

#[cfg(test)]
mod tests {
    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    use super::IrqLine;

    /// A raised line is observed as pending immediately (same delta), and ack deasserts
    /// it; two lines are independent.
    #[test]
    fn raise_then_ack_levels() {
        let sim = Sim::new();
        let a = IrqLine::new(&sim);
        let b = IrqLine::new(&sim);

        sim.add_method("t", &[], true, move |cx| {
            assert!(!a.pending(cx));
            a.raise(cx);
            assert!(a.pending(cx)); // immediate, no delta delay
            assert!(!b.pending(cx)); // independent line
            a.ack(cx);
            assert!(!a.pending(cx));
        });

        sim.run_until(SimTime::from_ns(10));
        assert!(!a.is_asserted(&sim));
    }

    /// A hart parked in WFI on the change event wakes when the line is raised.
    #[test]
    fn wfi_wakes_on_assert() {
        let sim = Sim::new();
        let line = IrqLine::new(&sim);
        // A second line is the `Copy`/`Send` "woke" flag (no `Rc` into the thread body).
        let woke = IrqLine::new(&sim);

        let driver = line;
        sim.add_thread("source", &[], true, move |cx| {
            cx.wait(SimTime::from_ns(5));
            driver.raise(cx);
        });

        let consumer = line;
        let ev = line.changed_event();
        sim.add_thread("hart", &[], true, move |cx| {
            if !consumer.pending(cx) {
                cx.wait_event(ev);
            }
            assert!(consumer.pending(cx));
            woke.raise(cx);
        });

        sim.run_until(SimTime::from_ns(20));
        assert!(woke.is_asserted(&sim));
    }

    /// Holding a line high (repeated raises) is idempotent and stays pending.
    #[test]
    fn repeated_raise_is_idempotent() {
        let sim = Sim::new();
        let line = IrqLine::new(&sim);
        sim.add_method("t", &[], true, move |cx| {
            line.raise(cx);
            line.raise(cx);
            line.set_level(cx, true);
            assert!(line.pending(cx));
        });
        sim.run_until(SimTime::from_ns(10));
        assert!(line.is_asserted(&sim));
    }
}
