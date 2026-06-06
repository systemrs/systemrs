//! Multi-target sockets (typestate stub).
//!
//! [`MultiTargetSocket`] is a **distinct type** from
//! [`systemrs_tlm2::TargetSocket`], so binding a plain initiator into a multi socket
//! (or vice versa) is a *compile* error rather than a runtime convention
//! (`doc/systemrs-design.md` §6d). Milestone 4 ships only the type distinction; the
//! full id-routed fan-out (one target fronting many initiators) is deferred beyond
//! M4 — the exit criteria do not exercise it.
//!
//! ## A multi-into-non-multi bind does not compile
//!
//! ```compile_fail
//! use systemrs::prelude::*;
//! use systemrs::tlm_utils::MultiTargetSocket;
//!
//! let sim = Sim::new();
//! let multi = MultiTargetSocket::new(&sim, "m");
//! let isock = InitiatorSocket::new(&sim, "i");
//! isock.bind(&sim, &multi); // ERROR: expected &TargetSocket, found &MultiTargetSocket
//! ```

use systemrs_kernel::Sim;
use systemrs_tlm2::TargetSocket;

/// A multi-target socket able to front several initiators (fan-out deferred to M5).
#[derive(Debug, Clone, Copy)]
pub struct MultiTargetSocket {
    /// The wrapped target socket.
    inner: TargetSocket,
}

impl MultiTargetSocket {
    /// Creates a multi-target socket.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle.
    pub fn new(sim: &Sim, name: &str) -> Self {
        MultiTargetSocket {
            inner: TargetSocket::new(sim, name),
        }
    }

    /// Returns the wrapped [`TargetSocket`].
    pub fn inner(&self) -> TargetSocket {
        self.inner
    }
}
