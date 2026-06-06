//! [`Export<IF>`]: a provided-interface endpoint that re-publishes an interface
//! upward through the hierarchy.
//!
//! Like [`crate::Port`], an `Export` is a `Copy` id handle whose binding state lives
//! in the registry ([`crate::binding`]). An export is the *provider* side of a
//! two-phase bind: it is bound to the channel (or lower export) that supplies the
//! interface, and a parent port binds *to* the export (forwarding through it). It
//! resolves via the same [`crate::binding::complete`] flatten.

use std::marker::PhantomData;

use systemrs_core::ObjectKind;
use systemrs_diag::ReportError;
use systemrs_kernel::{ObjectId, Sim};

use crate::binding::{self, BindElem, PortPolicy};

/// A provided-interface endpoint (`sc_export<IF>`).
pub struct Export<IF> {
    /// The endpoint's object id.
    id: ObjectId,

    /// Compile-time interface tag (keeps the handle `Send`/`Copy`).
    _p: PhantomData<fn() -> IF>,
}

impl<IF> Clone for Export<IF> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<IF> Copy for Export<IF> {}

impl<IF> Export<IF> {
    /// Creates an unbound export (policy [`PortPolicy::OneOrMore`]) in the current scope.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - The export's local name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new export.
    pub fn new(sim: &Sim, name: &str) -> Self {
        Self::with_policy(sim, name, PortPolicy::OneOrMore)
    }

    /// Creates an unbound export with an explicit cardinality `policy`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - The export's local name.
    /// * `policy` - The cardinality policy.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new export.
    pub fn with_policy(sim: &Sim, name: &str, policy: PortPolicy) -> Self {
        let id = binding::register_endpoint(sim, name, ObjectKind::Export, policy);
        Export {
            id,
            _p: PhantomData,
        }
    }

    /// Returns this export's object id.
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Records the channel (interface provider) this export publishes.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `channel` - The object id of the provided channel.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` error if called after the simulation has started.
    pub fn bind_channel(&self, sim: &Sim, channel: ObjectId) -> Result<(), ReportError> {
        binding::ensure_build(sim)?;
        binding::record(
            &binding::port_registry(sim),
            self.id,
            BindElem::Channel(channel),
        )
    }

    /// Records a forward to a lower export of the same interface.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `inner` - The lower export this one forwards to.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` error if called after the simulation has started.
    pub fn bind_export(&self, sim: &Sim, inner: &Export<IF>) -> Result<(), ReportError> {
        binding::ensure_build(sim)?;
        binding::record(
            &binding::port_registry(sim),
            self.id,
            BindElem::Parent(inner.id),
        )
    }

    /// Resolves this export's binding now (also driven at `end_of_elaboration`).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Returns
    ///
    /// The resolved set of interface-providing object ids.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` error if the port-policy cardinality is violated.
    pub fn complete_binding(&self, sim: &Sim) -> Result<Vec<ObjectId>, ReportError> {
        binding::complete(&binding::port_registry(sim), self.id)
    }

    /// Returns this export's resolved interface ids (empty until completed).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    ///
    /// # Returns
    ///
    /// A clone of the resolved id set.
    pub fn resolved(&self, sim: &Sim) -> Vec<ObjectId> {
        binding::resolved(&binding::port_registry(sim), self.id)
    }
}
