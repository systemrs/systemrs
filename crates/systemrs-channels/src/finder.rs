//! Event-finder seam (`sc_event_finder`) — a no-op in Milestone 2.
//!
//! SystemC resolves *static sensitivity to a port's event* (e.g. a method sensitive
//! to a clock port's posedge) at bind time, via an `sc_event_finder`. The M2
//! elaboration barrier is deliberately `EventId`-free — its exit criteria do not
//! involve sensitivity — so this is only a placeholder seam. The real selector,
//! resolving a port binding to a concrete kernel event, lands with the typed
//! channel interfaces in M3 (`doc/systemrs-design.md` §4 "Events & sensitivity").

/// A placeholder for a future event selector resolved at binding completion.
///
/// The single M2 variant resolves to nothing; the typed selectors arrive in M3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSelector {
    /// Selects no event (the only variant in Milestone 2).
    None,
}
