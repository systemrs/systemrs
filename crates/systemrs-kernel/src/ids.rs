//! Generational arena identifiers.
//!
//! Per `doc/systemrs-design.md` §6a, every kernel-owned entity lives in a
//! `SlotMap`-backed arena keyed by a `Copy` generational id. Components refer to
//! one another *by id, never by reference* — dissolving SystemC's raw-pointer
//! object graph and sidestepping the borrow checker. A "dead" entity is a stale
//! generation, detected automatically by the slotmap.

slotmap::new_key_type! {
    /// Identifies a process (`SC_METHOD` or `SC_THREAD`) in the process arena.
    pub struct ProcId;

    /// Identifies an [`crate::Event`] in the event arena.
    pub struct EventId;

    /// Identifies an updatable primitive channel in the channel arena.
    pub struct ChanId;

    /// Identifies an object in the elaboration object hierarchy (modules, ports,
    /// exports, sockets, …). The store body lives in `systemrs-core`; the `Copy`
    /// key lives here so lower layers can refer to objects by id without an upward
    /// crate dependency (`doc/systemrs-design.md` §6b, §10.3).
    pub struct ObjectId;
}
