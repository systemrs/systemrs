//! Scheduler phase and run-stage definitions.

/// The current phase of the delta cycle.
///
/// The strict three-phase order EVALUATE → UPDATE → DELTA-NOTIFY is the
/// determinism guarantee (`doc/systemrs-design.md` §6a). [`Phase::Build`] is the
/// pre-simulation elaboration phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Elaboration: building the static hierarchy. No process runs.
    Build,

    /// Evaluate: run all runnable methods then threads to completion.
    Evaluate,

    /// Update: apply pending primitive-channel updates.
    Update,

    /// Delta-notify: fire delta-notified events, queueing next-delta processes.
    Notify,
}

/// A simulation stage at which trace/sample callbacks may fire.
///
/// SystemRS keeps only the two stages tracing needs (`doc/systemrs-design.md` §4,
/// "Stage/phase callbacks — SIMPLIFY"): sampling occurs *after* the update phase
/// commits new values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Before a timestep's time advance commits (`SC_PRE_TIMESTEP`).
    PreTimestep,

    /// After the update phase commits, for delta tracing (`SC_POST_UPDATE`).
    PostUpdate,
}

/// The starvation policy governing time advance when no events are pending.
///
/// Mirrors SystemC's `SC_RUN_TO_TIME` vs `SC_EXIT_ON_STARVATION`
/// (`doc/systemrs-design.md` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Starvation {
    /// Advance time to the requested end even with no events (default `sc_start`).
    RunToTime,

    /// Stop as soon as the runnable set and event queues are empty.
    ExitOnStarvation,
}
