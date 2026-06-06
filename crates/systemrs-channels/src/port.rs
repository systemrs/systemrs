//! [`Port<IF>`]: a required-interface endpoint with two-phase deferred binding.
//!
//! A `Port` is a `Copy` id handle (`doc/systemrs-design.md` §6d); its binding state
//! lives in the kernel-owned registry ([`crate::binding`]). The `IF` type parameter
//! is a compile-time tag ensuring a port binds only to matching exports/parents;
//! resolution itself is in terms of [`ObjectId`]s. The handle stays `Send`/`Copy`
//! (its phantom is `fn() -> IF`), so it can be captured by an `SC_THREAD` body.

use std::marker::PhantomData;

use systemrs_core::ObjectKind;
use systemrs_diag::ReportError;
use systemrs_kernel::{Ctx, ObjectId, Sim};

use crate::binding::{self, BindElem, PortPolicy};
use crate::export::Export;

/// A required-interface endpoint (`sc_port<IF>`).
pub struct Port<IF> {
    /// The endpoint's object id (the key into the binding registry and hierarchy).
    id: ObjectId,

    /// Compile-time interface tag; `fn() -> IF` keeps the handle `Send`/`Copy` and
    /// covariant in `IF` without owning one.
    _p: PhantomData<fn() -> IF>,
}

impl<IF> Clone for Port<IF> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<IF> Copy for Port<IF> {}

impl<IF> core::fmt::Debug for Port<IF> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Port").field("id", &self.id).finish()
    }
}

impl<IF> Port<IF> {
    /// Creates an unbound port (policy [`PortPolicy::OneOrMore`]) in the current scope.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - The port's local name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new port.
    pub fn new(sim: &Sim, name: &str) -> Self {
        Self::with_policy(sim, name, PortPolicy::OneOrMore)
    }

    /// Creates an unbound port with an explicit cardinality `policy`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - The port's local name.
    /// * `policy` - The cardinality policy (e.g. [`PortPolicy::ZeroOrMore`] for a
    ///   multiport).
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new port.
    pub fn with_policy(sim: &Sim, name: &str, policy: PortPolicy) -> Self {
        let id = binding::register_endpoint(sim, name, ObjectKind::Port, policy);
        Port {
            id,
            _p: PhantomData,
        }
    }

    /// Returns this port's object id.
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Records a direct bind to a leaf channel (the interface provider).
    ///
    /// Two-phase: this only *records* the request; resolution happens at
    /// `complete_binding`.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `channel` - The object id of the bound channel.
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

    /// Records a hierarchical bind to a parent port (port-to-port), flattened
    /// depth-first at completion.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `parent` - The parent port this port forwards through.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` error if called after the simulation has started.
    pub fn bind_parent(&self, sim: &Sim, parent: &Port<IF>) -> Result<(), ReportError> {
        binding::ensure_build(sim)?;
        binding::record(
            &binding::port_registry(sim),
            self.id,
            BindElem::Parent(parent.id),
        )
    }

    /// Records a bind to an export of the same interface, flattened at completion.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `export` - The export this port binds to.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` error if called after the simulation has started.
    pub fn bind_export(&self, sim: &Sim, export: &Export<IF>) -> Result<(), ReportError> {
        binding::ensure_build(sim)?;
        binding::record(
            &binding::port_registry(sim),
            self.id,
            BindElem::Parent(export.id()),
        )
    }

    /// Resolves this port's binding now (also driven automatically at
    /// `end_of_elaboration`).
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

    /// Returns this port's resolved interface ids (empty until completed).
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

    /// Returns this port's resolved interface ids during simulation, from a running
    /// [`Ctx`] (the binding is resolved once at the elaboration barrier).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The running kernel handle.
    ///
    /// # Returns
    ///
    /// The resolved interface id set (empty if unbound/unresolved).
    pub fn resolved_in_ctx(&self, ctx: &Ctx) -> Vec<ObjectId> {
        binding::resolved_ctx(ctx, self.id)
    }
}
