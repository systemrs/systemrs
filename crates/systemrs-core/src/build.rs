//! Ergonomic process builders with explicit sensitivity.

use systemrs_kernel::{Ctx, EventId, ProcId, Sim};

/// Extension trait adding ergonomic process builders to [`Sim`].
///
/// Replaces the kernel's positional `add_method`/`add_thread` with a fluent
/// builder, so model code reads like SystemC's `SC_METHOD` + `sensitive << ev`
/// without the hidden last-process coupling (`doc/systemrs-design.md` §6b).
pub trait Build {
    /// Begins building an `SC_METHOD` named `name`.
    ///
    /// # Arguments
    ///
    /// * `name` - A hierarchical name for diagnostics.
    ///
    /// # Returns
    ///
    /// A [`MethodBuilder`] to add sensitivity and finalize with a body.
    fn method<'s>(&'s self, name: &str) -> MethodBuilder<'s>;

    /// Begins building an `SC_THREAD` named `name`.
    ///
    /// # Arguments
    ///
    /// * `name` - A hierarchical name for diagnostics.
    ///
    /// # Returns
    ///
    /// A [`ThreadBuilder`] to add sensitivity and finalize with a body.
    fn thread<'s>(&'s self, name: &str) -> ThreadBuilder<'s>;
}

impl Build for Sim {
    fn method<'s>(&'s self, name: &str) -> MethodBuilder<'s> {
        MethodBuilder {
            sim: self,
            name: name.to_owned(),
            sens: Vec::new(),
            initialize: true,
        }
    }

    fn thread<'s>(&'s self, name: &str) -> ThreadBuilder<'s> {
        ThreadBuilder {
            sim: self,
            name: name.to_owned(),
            sens: Vec::new(),
            initialize: true,
        }
    }
}

/// A builder for an `SC_METHOD`: accumulate sensitivity, then `finish` with a body.
pub struct MethodBuilder<'s> {
    /// The simulation under construction.
    sim: &'s Sim,

    /// The method's name.
    name: String,

    /// The events the method is statically sensitive to.
    sens: Vec<EventId>,

    /// Whether the method runs once at start of simulation.
    initialize: bool,
}

impl MethodBuilder<'_> {
    /// Adds `event` to the method's static sensitivity.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to become sensitive to.
    #[must_use]
    pub fn sensitive_to(mut self, event: EventId) -> Self {
        self.sens.push(event);
        self
    }

    /// Suppresses the initial run at start of simulation (`dont_initialize`).
    #[must_use]
    pub fn dont_initialize(mut self) -> Self {
        self.initialize = false;
        self
    }

    /// Finalizes the method with its run-to-completion body.
    ///
    /// # Arguments
    ///
    /// * `body` - The callback, run on each trigger.
    ///
    /// # Returns
    ///
    /// The new process's [`ProcId`].
    pub fn finish<F>(self, body: F) -> ProcId
    where
        F: FnMut(&Ctx) + 'static,
    {
        self.sim
            .add_method(&self.name, &self.sens, self.initialize, body)
    }
}

/// A builder for an `SC_THREAD`: accumulate sensitivity, then `finish` with a body.
pub struct ThreadBuilder<'s> {
    /// The simulation under construction.
    sim: &'s Sim,

    /// The thread's name.
    name: String,

    /// The events the thread is statically sensitive to.
    sens: Vec<EventId>,

    /// Whether the thread starts at start of simulation.
    initialize: bool,
}

impl ThreadBuilder<'_> {
    /// Adds `event` to the thread's static sensitivity.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to become sensitive to.
    #[must_use]
    pub fn sensitive_to(mut self, event: EventId) -> Self {
        self.sens.push(event);
        self
    }

    /// Suppresses the initial start at start of simulation (`dont_initialize`).
    #[must_use]
    pub fn dont_initialize(mut self) -> Self {
        self.initialize = false;
        self
    }

    /// Finalizes the thread with its (stackful) body.
    ///
    /// # Arguments
    ///
    /// * `body` - The thread body; may call `Ctx::wait` from any depth. Must be
    ///   `Send` (corosensei's requirement).
    ///
    /// # Returns
    ///
    /// The new process's [`ProcId`].
    pub fn finish<F>(self, body: F) -> ProcId
    where
        F: FnOnce(&Ctx) + Send + 'static,
    {
        self.sim
            .add_thread(&self.name, &self.sens, self.initialize, body)
    }
}
