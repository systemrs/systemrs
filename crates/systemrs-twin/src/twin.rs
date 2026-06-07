//! [`TwinBuilder`] — a small fluent front door for the digital-twin subsystems
//! (`doc/systemrs-design.md` §6f).
//!
//! Chains the common setup — seed the RNG, install a pacer — onto a `&Sim` during
//! elaboration. External-input attachment and replay have their own richer
//! constructors ([`crate::attach_external_input`], [`crate::JournalReplayer`]); this
//! builder covers the parts that are pure setup with no returned producer handles.

use std::rc::Rc;

use systemrs_kernel::Sim;
use systemrs_time::SimTime;

use crate::pacer::RealTimePacer;
use crate::rng::Rng;

/// A fluent builder for the digital-twin subsystems on a simulation.
pub struct TwinBuilder<'a> {
    /// The simulation being configured.
    sim: &'a Sim,
}

impl<'a> TwinBuilder<'a> {
    /// Starts configuring `sim`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Returns
    ///
    /// A new [`TwinBuilder`].
    pub fn new(sim: &'a Sim) -> Self {
        TwinBuilder { sim }
    }

    /// Installs a seeded [`Rng`] service and returns the handle.
    ///
    /// # Arguments
    ///
    /// * `seed` - The RNG seed (record it in the journal for replay).
    ///
    /// # Returns
    ///
    /// The shared [`Rng`] service.
    pub fn seed(&self, seed: u64) -> Rc<Rng> {
        Rng::install(self.sim, seed)
    }

    /// Installs a [`RealTimePacer`] and returns its handle (read slip via
    /// [`RealTimePacer::stats`]).
    ///
    /// # Arguments
    ///
    /// * `scale` - Wall nanoseconds per simulation nanosecond (`1.0` = real time).
    /// * `tolerance` - How far ahead of wall clock the sim may run before sleeping.
    ///
    /// # Returns
    ///
    /// The shared [`RealTimePacer`].
    pub fn pacer(&self, scale: f64, tolerance: SimTime) -> Rc<RealTimePacer> {
        let pacer = RealTimePacer::new(scale, tolerance);
        pacer.install(self.sim);
        pacer
    }
}
