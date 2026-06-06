//! Primitive channels for SystemRS.
//!
//! These reproduce SystemC's evaluate-then-update determinism
//! (`doc/systemrs-design.md` §3.6, §6c): a `write`/`put` only *stages* a value and
//! calls `request_update`; the committed value becomes visible only after the
//! update phase, so process execution order within a delta cannot affect read
//! values. Value-changed events fire one delta later.
//!
//! Channel *handles* are `Copy`/`Send` (an id + phantom type); the state lives in
//! the kernel arena. This lets the same handle be used from both `SC_METHOD`
//! bodies and (Send-required) `SC_THREAD` bodies, refer-by-id per the design (§6a).
//!
//! From Milestone 2 this crate also hosts the generic interface/port/export binding
//! machinery ([`Port`]/[`Export`]/[`Interface`], §3.5, §6d): two-phase deferred
//! binding resolved at the elaboration barrier, with hierarchical port-to-port
//! flattening and port-policy cardinality.

// Pre-1.0: the M2 binding machinery (ports/exports/registry) is consumed by the
// elaboration driver (M2-06) and the socket reconcile (M2-09); allow until 1.0.0
// per the Rust skill, matching `systemrs-kernel`/`systemrs-core`/`systemrs-tlm2`.
#![allow(dead_code)]

mod binding;
mod clock;
mod export;
mod fifo;
mod finder;
mod interface;
mod port;
mod signal;

pub use binding::PortPolicy;
pub use clock::Clock;
pub use export::Export;
pub use fifo::Fifo;
pub use finder::EventSelector;
pub use interface::Interface;
pub use port::Port;
pub use signal::{Buffer, Signal};

#[cfg(test)]
mod tests;
