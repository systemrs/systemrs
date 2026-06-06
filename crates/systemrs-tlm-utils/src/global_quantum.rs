//! The global quantum — the temporal-decoupling time ceiling (`tlm_global_quantum`).
//!
//! Per `doc/systemrs-design.md` §6d, the global quantum lives in the runtime
//! *context*, not a true singleton: it is a [`Sim`] service, so multiple
//! simulations per process each carry their own. The keeper reaches it via the
//! kernel handle. The grid-alignment arithmetic is integer-only (SimTime has no
//! `Rem`), so it never introduces `f64` onto the time path.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::{Ctx, Sim};
use systemrs_time::SimTime;

/// The global quantum ceiling for temporal decoupling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalQuantum {
    /// The quantum length (`SimTime::ZERO` disables decoupling).
    quantum: SimTime,
}

impl GlobalQuantum {
    /// Creates a global quantum of length `quantum`.
    ///
    /// # Arguments
    ///
    /// * `quantum` - The quantum length.
    ///
    /// # Returns
    ///
    /// The [`GlobalQuantum`].
    pub fn new(quantum: SimTime) -> Self {
        GlobalQuantum { quantum }
    }

    /// Returns the quantum length.
    pub fn get(&self) -> SimTime {
        self.quantum
    }

    /// Sets the quantum length.
    ///
    /// # Arguments
    ///
    /// * `quantum` - The new quantum length.
    pub fn set(&mut self, quantum: SimTime) {
        self.quantum = quantum;
    }

    /// Returns the time from `now` to the next quantum-grid boundary.
    ///
    /// `q - (now mod q)`, integer-only (`tlm_global_quantum::compute_local_quantum`).
    /// At a grid boundary (`now mod q == 0`) this returns a **full** quantum, matching
    /// SystemC. A zero quantum returns zero (decoupling disabled).
    ///
    /// # Arguments
    ///
    /// * `now` - The current simulation time.
    ///
    /// # Returns
    ///
    /// The local quantum from `now`.
    pub fn compute_local_quantum(&self, now: SimTime) -> SimTime {
        if self.quantum.is_zero() {
            SimTime::ZERO
        } else {
            self.quantum - SimTime::from_units(now.units() % self.quantum.units())
        }
    }
}

/// Sets (or updates) the simulation's global quantum, registering the service.
///
/// Call during elaboration (like SystemC's `set_global_quantum` before `sc_start`).
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `quantum` - The quantum length.
pub fn set_global_quantum(sim: &Sim, quantum: SimTime) {
    let ctx = sim.ctx();
    if let Some(existing) = ctx.try_service::<RefCell<GlobalQuantum>>() {
        existing.borrow_mut().set(quantum);
    } else {
        sim.register_service(Rc::new(RefCell::new(GlobalQuantum::new(quantum))));
    }
}

/// Returns the global-quantum service for the running simulation, if set.
///
/// # Arguments
///
/// * `ctx` - The kernel handle.
///
/// # Returns
///
/// The shared [`GlobalQuantum`], or `None` if none was set.
pub(crate) fn global_quantum_from_ctx(ctx: &Ctx) -> Option<Rc<RefCell<GlobalQuantum>>> {
    ctx.try_service::<RefCell<GlobalQuantum>>()
}

#[cfg(test)]
mod tests {
    use super::{GlobalQuantum, global_quantum_from_ctx, set_global_quantum};
    use systemrs_kernel::Sim;
    use systemrs_time::SimTime;

    /// `compute_local_quantum` aligns to the grid (full quantum on a boundary).
    #[test]
    fn compute_local_quantum_grid_alignment() {
        let q = GlobalQuantum::new(SimTime::from_ns(100));
        assert_eq!(
            q.compute_local_quantum(SimTime::ZERO),
            SimTime::from_ns(100)
        );
        assert_eq!(
            q.compute_local_quantum(SimTime::from_ns(30)),
            SimTime::from_ns(70)
        );
        // On a grid boundary, a FULL quantum (matches SystemC).
        assert_eq!(
            q.compute_local_quantum(SimTime::from_ns(100)),
            SimTime::from_ns(100)
        );
        assert_eq!(
            q.compute_local_quantum(SimTime::from_ns(250)),
            SimTime::from_ns(50)
        );
        // A zero quantum disables decoupling.
        let zero = GlobalQuantum::new(SimTime::ZERO);
        assert_eq!(
            zero.compute_local_quantum(SimTime::from_ns(7)),
            SimTime::ZERO
        );
    }

    /// The service round-trips and coexists with other (distinct-type) services.
    #[test]
    fn service_round_trips_without_clobbering() {
        let sim = Sim::new();
        // A distinct service to prove no TypeId collision.
        sim.register_service(std::rc::Rc::new(std::cell::RefCell::new(42u16)));
        set_global_quantum(&sim, SimTime::from_ns(64));

        let gq = global_quantum_from_ctx(&sim.ctx()).expect("registered");
        assert_eq!(gq.borrow().get(), SimTime::from_ns(64));
        // Updating in place.
        set_global_quantum(&sim, SimTime::from_ns(128));
        assert_eq!(gq.borrow().get(), SimTime::from_ns(128));
        // The other service is untouched.
        let other = sim.ctx().service::<std::cell::RefCell<u16>>();
        assert_eq!(*other.borrow(), 42);
    }
}
