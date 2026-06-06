//! [`Kernel<S>`]: a typestate front door over [`Sim`] for hierarchical models.
//!
//! The flat [`Sim`] is the engine and stays the path for simple models. `Kernel`
//! adds the design's `Building → Running` typestate (`doc/systemrs-design.md` §6a,
//! §14): construction APIs (`module`, channel/port creation via [`Kernel::sim`])
//! exist only on [`Kernel<Building>`]; [`Kernel::build`] consumes it, drives
//! elaboration eagerly, and yields a [`Kernel<Running>`] exposing only `run`/`now`/
//! `finish`. Because `Running` has no construction methods, **binding after start is
//! a compile error** — the compile-time half of the design's bind-before-start
//! guard (the runtime half being the `Phase::Build` check in the binding APIs).

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use systemrs_diag::ReportError;
use systemrs_kernel::{ObjectId, Sim};
use systemrs_time::{Resolution, SimTime};

use crate::elaborate::Elaborate;
use crate::module::{self, Builder};

/// Typestate marker: the elaboration phase (construction allowed).
pub struct Building;

/// Typestate marker: the simulation is elaborated and may run (no construction).
pub struct Running;

/// A typestate wrapper over [`Sim`]: `Kernel<Building>` builds, `Kernel<Running>` runs.
pub struct Kernel<S> {
    /// The wrapped engine.
    sim: Sim,

    /// The typestate marker.
    _state: PhantomData<S>,
}

impl Kernel<Building> {
    /// Creates a building kernel with the default time resolution.
    ///
    /// # Returns
    ///
    /// A fresh [`Kernel<Building>`].
    pub fn new() -> Self {
        Kernel {
            sim: Sim::new(),
            _state: PhantomData,
        }
    }

    /// Creates a building kernel with an explicit time resolution.
    ///
    /// # Arguments
    ///
    /// * `resolution` - The frozen time resolution.
    ///
    /// # Returns
    ///
    /// A fresh [`Kernel<Building>`].
    pub fn with_resolution(resolution: Resolution) -> Self {
        Kernel {
            sim: Sim::with_resolution(resolution),
            _state: PhantomData,
        }
    }

    /// Returns the underlying [`Sim`] for constructing top-level channels/ports
    /// (which register under the root scope).
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Creates a top-level anonymous module scope (see [`module::module`]).
    ///
    /// # Arguments
    ///
    /// * `name` - The module's local name.
    /// * `build` - The module body.
    ///
    /// # Returns
    ///
    /// The module's [`ObjectId`].
    ///
    /// # Errors
    ///
    /// Propagates construction errors from [`module::module`].
    pub fn module<F>(&self, name: &str, build: F) -> Result<ObjectId, ReportError>
    where
        F: FnOnce(&mut Builder),
    {
        module::module(&self.sim, name, build)
    }

    /// Creates a top-level module instance with lifecycle callbacks (see
    /// [`module::module_with`]).
    ///
    /// # Arguments
    ///
    /// * `name` - The module's local name.
    /// * `build` - Builds the module value.
    ///
    /// # Returns
    ///
    /// A shared handle to the module instance.
    ///
    /// # Errors
    ///
    /// Propagates construction errors from [`module::module_with`].
    pub fn module_with<M, F>(&self, name: &str, build: F) -> Result<Rc<RefCell<M>>, ReportError>
    where
        M: Elaborate + 'static,
        F: FnOnce(&mut Builder) -> M,
    {
        module::module_with(&self.sim, name, build)
    }

    /// Completes elaboration (drives the barrier eagerly) and transitions to the
    /// running state.
    ///
    /// After this, construction methods are gone from the type, so any binding /
    /// module creation is a compile error.
    ///
    /// # Returns
    ///
    /// The [`Kernel<Running>`] ready to step.
    #[must_use]
    pub fn build(self) -> Kernel<Running> {
        self.sim.elaborate();
        Kernel {
            sim: self.sim,
            _state: PhantomData,
        }
    }
}

impl Default for Kernel<Building> {
    fn default() -> Self {
        Kernel::new()
    }
}

impl Kernel<Running> {
    /// Runs the simulation until `until`.
    ///
    /// # Arguments
    ///
    /// * `until` - The time to stop at.
    pub fn run(&self, until: SimTime) {
        self.sim.run_until(until);
    }

    /// Returns the current simulation time.
    pub fn now(&self) -> SimTime {
        self.sim.now()
    }

    /// Returns the number of completed delta cycles.
    pub fn delta_count(&self) -> u64 {
        self.sim.delta_count()
    }

    /// Returns the underlying [`Sim`] (e.g. to read a channel via `sim.ctx()`).
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Ends the simulation, firing `end_of_simulation` exactly once.
    ///
    /// Consumes the kernel; the `Drop` of the wrapped [`Sim`] is the idempotent
    /// backstop if `finish` is not called.
    pub fn finish(self) {
        self.sim.end_of_sim();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use systemrs_kernel::Ctx;
    use systemrs_time::SimTime;

    use super::{Building, Kernel};
    use crate::module::Module;
    use crate::{Builder, Elaborate};

    /// A module that records its lifecycle callbacks.
    struct Probe {
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Elaborate for Probe {
        fn before_end_of_elaboration(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push("before");
        }
        fn end_of_elaboration(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push("end");
        }
        fn start_of_simulation(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push("start");
        }
    }

    impl Module for Probe {}

    /// `build()` drives the barrier eagerly; `run()` then steps without re-elaborating.
    #[test]
    fn build_drives_elaboration_eagerly_then_runs() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&log);

        let building = Kernel::<Building>::new();
        building
            .module_with("cpu", move |_b: &mut Builder| Probe { log: l })
            .expect("cpu");

        // build() completes elaboration before any run() call.
        let running = building.build();
        assert_eq!(*log.borrow(), vec!["before", "end", "start"]);

        running.run(SimTime::ZERO);
        running.finish();
    }
}
