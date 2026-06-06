//! Passthrough target sockets (typestate stub).
//!
//! [`PassthroughTargetSocket`] is a **distinct type** so a hierarchical passthrough
//! binding is type-checked rather than a runtime convention
//! (`doc/systemrs-design.md` §6d). Milestone 4 ships only the type distinction; the
//! forwarding/fan-out implementation is deferred beyond M4 (the exit criteria do not
//! exercise it).

use systemrs_kernel::Sim;
use systemrs_tlm2::TargetSocket;

/// A passthrough target socket that forwards to a downstream target (forwarding
/// deferred to M5).
#[derive(Debug, Clone, Copy)]
pub struct PassthroughTargetSocket {
    /// The wrapped target socket.
    inner: TargetSocket,
}

impl PassthroughTargetSocket {
    /// Creates a passthrough target socket.
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
        PassthroughTargetSocket {
            inner: TargetSocket::new(sim, name),
        }
    }

    /// Returns the wrapped [`TargetSocket`].
    pub fn inner(&self) -> TargetSocket {
        self.inner
    }
}
