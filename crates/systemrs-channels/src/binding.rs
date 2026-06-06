//! The two-phase binding machinery shared by [`crate::Port`] and [`crate::Export`].
//!
//! SystemC binds in two phases: `bind()` merely *records* a request, and
//! `complete_binding()` resolves it at `end_of_elaboration`, flattening
//! hierarchical port-to-port forwards depth-first and enforcing port-policy
//! cardinality (`sc_port.cpp`, `doc/systemrs-design.md` §3.5, §6d).
//!
//! The binding state lives in a kernel-owned [`PortRegistry`] (a [`Sim`] service)
//! keyed by [`ObjectId`], **not** inside the `Port`/`Export` handle — so the
//! recursive [`complete`] can reach a *parent endpoint by id*. This mirrors the TLM
//! socket registry. Resolution yields a set of `ObjectId`s (the bound interface-
//! providing objects); typed interface dispatch is layered on top by the socket
//! layer (M2-09). The recursion threads ids and re-borrows the registry per access,
//! never holding a `RefCell` borrow across the recursive call — the borrow-safe
//! discipline that prevents a double-borrow panic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use systemrs_core::{Elaborate, ObjectKind};
use systemrs_diag::ReportError;
use systemrs_kernel::{Ctx, ObjectId, Phase, Sim};

/// One recorded (unresolved) bind request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindElem {
    /// A direct bind to a leaf channel / interface-providing object.
    Channel(ObjectId),

    /// A bind that forwards through another endpoint (hierarchical port-to-port or
    /// port-to-export), flattened depth-first at completion.
    Parent(ObjectId),
}

/// The cardinality policy enforced at the end of binding completion
/// ([`Port::complete_binding`](crate::Port::complete_binding)).
///
/// Mirrors SystemC's port multiplicity policy (`sc_port.cpp:520-549`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortPolicy {
    /// At least one interface must be bound (the default for a simple port).
    OneOrMore,

    /// At least `max` interfaces (and at least one) must be bound.
    AllBound(usize),

    /// Any number of interfaces, including zero (unbounded/optional multiport).
    ZeroOrMore,
}

impl PortPolicy {
    /// Checks a resolved interface count against this policy.
    ///
    /// # Arguments
    ///
    /// * `n` - The number of resolved interfaces.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the count satisfies the policy.
    ///
    /// # Errors
    ///
    /// Returns a `SYSTEMRS/BIND` [`ReportError`] if the policy is violated.
    fn check(self, n: usize) -> Result<(), ReportError> {
        let ok = match self {
            PortPolicy::OneOrMore => n >= 1,
            PortPolicy::AllBound(max) => n >= max && n >= 1,
            PortPolicy::ZeroOrMore => true,
        };
        if ok {
            Ok(())
        } else {
            Err(systemrs_diag::error(
                "SYSTEMRS/BIND",
                &format!("port binding violates policy {self:?}: {n} interface(s) bound"),
            ))
        }
    }
}

/// The resolution state of one endpoint's binding.
pub(crate) enum BindState {
    /// No bind recorded yet.
    Unbound,

    /// Bind requests recorded, not yet resolved.
    Recorded(Vec<BindElem>),

    /// Resolved to a flat set of interface-providing object ids.
    Bound(Vec<ObjectId>),
}

/// Per-endpoint binding metadata held in the [`PortRegistry`].
pub(crate) struct PortMeta {
    /// Whether this endpoint is a port or an export (for diagnostics).
    pub(crate) kind: ObjectKind,

    /// The current binding state.
    pub(crate) bind: BindState,

    /// The cardinality policy enforced at completion.
    pub(crate) policy: PortPolicy,
}

/// The kernel-owned registry of endpoint binding state (a [`Sim`] service).
///
/// Lookups are by [`ObjectId`]; the map is never iterated to make an ordering
/// decision (completion order is driven by the object store's ordered elaborator
/// buckets), so no observable behaviour depends on `HashMap` order.
pub(crate) struct PortRegistry {
    /// Endpoint id → its binding metadata.
    pub(crate) metas: HashMap<ObjectId, PortMeta>,
}

impl PortRegistry {
    /// Creates an empty registry.
    fn new() -> Self {
        PortRegistry {
            metas: HashMap::new(),
        }
    }
}

/// Returns the simulation's port registry, creating and registering it on first use.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
///
/// # Returns
///
/// The shared [`PortRegistry`] handle.
pub(crate) fn port_registry(sim: &Sim) -> Rc<RefCell<PortRegistry>> {
    let ctx = sim.ctx();
    if let Some(existing) = ctx.try_service::<RefCell<PortRegistry>>() {
        return existing;
    }
    let reg = Rc::new(RefCell::new(PortRegistry::new()));
    sim.register_service(Rc::clone(&reg));
    reg
}

/// Returns `Ok` only during the elaboration (build) phase.
///
/// # Errors
///
/// Returns a `SYSTEMRS/BIND` [`ReportError`] if binding is attempted after the
/// simulation has started (the runtime half of the bind-before-start guard, EC4).
pub(crate) fn ensure_build(sim: &Sim) -> Result<(), ReportError> {
    if sim.phase() == Phase::Build {
        Ok(())
    } else {
        Err(systemrs_diag::error(
            "SYSTEMRS/BIND",
            "binding is only allowed during elaboration (before the simulation starts)",
        ))
    }
}

/// Registers an endpoint object and its binding metadata, returning its id.
///
/// Inserts an object of `kind` under the current hierarchy scope, attaches a
/// [`BindingElaborator`] so [`complete`] is driven at `end_of_elaboration`, and
/// seeds an [`PortMeta`] in the registry.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `name` - The endpoint's local name.
/// * `kind` - [`ObjectKind::Port`] or [`ObjectKind::Export`].
/// * `policy` - The cardinality policy.
///
/// # Returns
///
/// The new endpoint's [`ObjectId`].
pub(crate) fn register_endpoint(
    sim: &Sim,
    name: &str,
    kind: ObjectKind,
    policy: PortPolicy,
) -> ObjectId {
    let reg = port_registry(sim);
    let store = systemrs_core::store(sim);

    let parent = store.borrow().current_scope();
    let elab = Rc::new(RefCell::new(BindingElaborator {
        id: ObjectId::default(),
        kind,
        registry: Rc::clone(&reg),
    }));
    let id = store
        .borrow_mut()
        .register_elaborator(parent, kind, name, elab.clone());
    elab.borrow_mut().id = id;

    reg.borrow_mut().metas.insert(
        id,
        PortMeta {
            kind,
            bind: BindState::Unbound,
            policy,
        },
    );
    id
}

/// Records a bind request on `id` (the two-phase *record* step; no resolution).
///
/// # Arguments
///
/// * `reg` - The port registry.
/// * `id` - The endpoint being bound.
/// * `elem` - The recorded bind element.
///
/// # Errors
///
/// Returns a `SYSTEMRS/BIND` [`ReportError`] if `id` is not a registered endpoint
/// or has already been resolved.
pub(crate) fn record(
    reg: &RefCell<PortRegistry>,
    id: ObjectId,
    elem: BindElem,
) -> Result<(), ReportError> {
    let mut r = reg.borrow_mut();
    let Some(meta) = r.metas.get_mut(&id) else {
        return Err(systemrs_diag::error(
            "SYSTEMRS/BIND",
            "bind on an unregistered endpoint",
        ));
    };
    match &mut meta.bind {
        BindState::Unbound => meta.bind = BindState::Recorded(vec![elem]),
        BindState::Recorded(v) => v.push(elem),
        BindState::Bound(_) => {
            return Err(systemrs_diag::error(
                "SYSTEMRS/BIND",
                "cannot bind an endpoint after its binding is complete",
            ));
        }
    }
    Ok(())
}

/// Resolves `id`'s binding: flattens parent forwards depth-first, enforces the
/// port policy, and caches the resolved interface-id set.
///
/// Idempotent — re-entry returns the cached `Bound` set. A placeholder empty
/// `Bound` is installed before recursing so a binding cycle terminates (rather than
/// recursing forever) instead of panicking.
///
/// # Arguments
///
/// * `reg` - The port registry.
/// * `id` - The endpoint to resolve.
///
/// # Returns
///
/// The resolved set of interface-providing object ids (empty if `id` is a leaf
/// channel that is not itself a registered endpoint).
///
/// # Errors
///
/// Returns a `SYSTEMRS/BIND` [`ReportError`] if a port-policy cardinality
/// constraint is violated.
pub(crate) fn complete(
    reg: &RefCell<PortRegistry>,
    id: ObjectId,
) -> Result<Vec<ObjectId>, ReportError> {
    // Already resolved (or a non-endpoint leaf)? Re-borrow released immediately.
    {
        let r = reg.borrow();
        match r.metas.get(&id) {
            None => return Ok(Vec::new()),
            Some(meta) => {
                if let BindState::Bound(ids) = &meta.bind {
                    return Ok(ids.clone());
                }
            }
        }
    }

    // Take the recorded elements out, installing a placeholder `Bound` as a cycle
    // guard; the store borrow is dropped before the recursion below.
    let (elems, policy) = {
        let mut r = reg.borrow_mut();
        let Some(meta) = r.metas.get_mut(&id) else {
            return Ok(Vec::new());
        };
        let elems = match std::mem::replace(&mut meta.bind, BindState::Bound(Vec::new())) {
            BindState::Recorded(e) => e,
            BindState::Unbound => Vec::new(),
            BindState::Bound(ids) => {
                meta.bind = BindState::Bound(ids.clone());
                return Ok(ids);
            }
        };
        (elems, meta.policy)
    };

    // Resolve, flattening parent forwards depth-first (no borrow held here).
    let mut resolved = Vec::new();
    for elem in elems {
        match elem {
            BindElem::Channel(oid) => resolved.push(oid),
            BindElem::Parent(pid) => resolved.extend(complete(reg, pid)?),
        }
    }

    policy.check(resolved.len())?;

    if let Some(meta) = reg.borrow_mut().metas.get_mut(&id) {
        meta.bind = BindState::Bound(resolved.clone());
    }
    Ok(resolved)
}

/// Returns the resolved interface set for a completed endpoint (empty if unresolved
/// or unknown).
///
/// # Arguments
///
/// * `reg` - The port registry.
/// * `id` - The endpoint id.
///
/// # Returns
///
/// A clone of the endpoint's resolved `Bound` ids, or empty.
pub(crate) fn resolved(reg: &RefCell<PortRegistry>, id: ObjectId) -> Vec<ObjectId> {
    match reg.borrow().metas.get(&id) {
        Some(PortMeta {
            bind: BindState::Bound(ids),
            ..
        }) => ids.clone(),
        _ => Vec::new(),
    }
}

/// The elaborator that drives an endpoint's [`complete`] at `end_of_elaboration`.
///
/// Registered into the object store's port/export bucket; the M2-06 elaboration
/// driver invokes `end_of_elaboration` in bucket order. A binding/cardinality
/// failure here surfaces as a FATAL abort (consistent with the design's
/// "elaboration error → FATAL" rule); the fallible [`complete`] is used directly by
/// tests to assert a clean `Err`.
struct BindingElaborator {
    /// This endpoint's object id (set after registration).
    id: ObjectId,

    /// Port or export (drives `object_kind` for bucket routing).
    kind: ObjectKind,

    /// The shared port registry.
    registry: Rc<RefCell<PortRegistry>>,
}

impl Elaborate for BindingElaborator {
    fn object_kind(&self) -> ObjectKind {
        self.kind
    }

    fn end_of_elaboration(&mut self, _ctx: &Ctx) {
        if let Err(e) = complete(&self.registry, self.id) {
            systemrs_diag::report_fatal("SYSTEMRS/BIND", &format!("{e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Export, Port};
    use systemrs_time::SimTime;

    /// A compile-time interface tag for the binding tests.
    struct Marker;

    /// Registers a leaf channel object under the root and returns its id.
    fn channel(sim: &Sim, name: &str) -> ObjectId {
        let store = systemrs_core::store(sim);
        let root = store.borrow().root();
        store
            .borrow_mut()
            .insert(root, name, ObjectKind::PrimChannel)
    }

    /// EC2: a direct channel bind resolves via two-phase bind + completion.
    #[test]
    fn direct_channel_bind_resolves() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let p = Port::<Marker>::new(&sim, "p");
        p.bind_channel(&sim, ch).unwrap();
        assert_eq!(p.complete_binding(&sim).unwrap(), vec![ch]);
        assert_eq!(p.resolved(&sim), vec![ch]);
    }

    /// EC3: a hierarchical port-to-port bind flattens depth-first to the channel.
    #[test]
    fn hierarchical_port_to_port_resolves() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let parent = Port::<Marker>::new(&sim, "parent");
        parent.bind_channel(&sim, ch).unwrap();
        let child = Port::<Marker>::new(&sim, "child");
        child.bind_parent(&sim, &parent).unwrap();
        assert_eq!(child.complete_binding(&sim).unwrap(), vec![ch]);
    }

    /// EC3: a port bound to an export flattens through the export to its channel.
    #[test]
    fn port_to_export_resolves() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let exp = Export::<Marker>::new(&sim, "exp");
        exp.bind_channel(&sim, ch).unwrap();
        let port = Port::<Marker>::new(&sim, "port");
        port.bind_export(&sim, &exp).unwrap();
        assert_eq!(port.complete_binding(&sim).unwrap(), vec![ch]);
    }

    /// A multiport parent splices all its interfaces, in order, into the child.
    #[test]
    fn multiport_flatten_preserves_order() {
        let sim = Sim::new();
        let ch1 = channel(&sim, "ch1");
        let ch2 = channel(&sim, "ch2");
        let parent = Port::<Marker>::new(&sim, "parent");
        parent.bind_channel(&sim, ch1).unwrap();
        parent.bind_channel(&sim, ch2).unwrap();
        let child = Port::<Marker>::new(&sim, "child");
        child.bind_parent(&sim, &parent).unwrap();
        assert_eq!(child.complete_binding(&sim).unwrap(), vec![ch1, ch2]);
    }

    /// EC7: `OneOrMore` with zero binds is a clean error, not a panic.
    #[test]
    fn one_or_more_zero_binds_errors() {
        let sim = Sim::new();
        let p = Port::<Marker>::new(&sim, "p");
        assert!(p.complete_binding(&sim).is_err());
    }

    /// EC7: `AllBound(n)` requires at least `n` interfaces.
    #[test]
    fn all_bound_enforces_minimum() {
        let sim = Sim::new();
        let ch1 = channel(&sim, "ch1");
        let p = Port::<Marker>::with_policy(&sim, "p", PortPolicy::AllBound(2));
        p.bind_channel(&sim, ch1).unwrap();
        assert!(p.complete_binding(&sim).is_err());

        let ch2 = channel(&sim, "ch2");
        let q = Port::<Marker>::with_policy(&sim, "q", PortPolicy::AllBound(2));
        q.bind_channel(&sim, ch1).unwrap();
        q.bind_channel(&sim, ch2).unwrap();
        assert_eq!(q.complete_binding(&sim).unwrap(), vec![ch1, ch2]);
    }

    /// EC7: `ZeroOrMore` accepts zero binds (an unbound optional multiport).
    #[test]
    fn zero_or_more_allows_empty() {
        let sim = Sim::new();
        let p = Port::<Marker>::with_policy(&sim, "p", PortPolicy::ZeroOrMore);
        assert_eq!(p.complete_binding(&sim).unwrap(), Vec::<ObjectId>::new());
    }

    /// A three-deep port chain completes without a `RefCell` double-borrow panic.
    #[test]
    fn deep_chain_no_double_borrow() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let gp = Port::<Marker>::new(&sim, "gp");
        gp.bind_channel(&sim, ch).unwrap();
        let p = Port::<Marker>::new(&sim, "p");
        p.bind_parent(&sim, &gp).unwrap();
        let child = Port::<Marker>::new(&sim, "child");
        child.bind_parent(&sim, &p).unwrap();
        assert_eq!(child.complete_binding(&sim).unwrap(), vec![ch]);
    }

    /// Completion is idempotent: a second call returns the cached resolved set.
    #[test]
    fn completion_is_idempotent() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let p = Port::<Marker>::new(&sim, "p");
        p.bind_channel(&sim, ch).unwrap();
        let first = p.complete_binding(&sim).unwrap();
        let second = p.complete_binding(&sim).unwrap();
        assert_eq!(first, second);
        assert_eq!(second, vec![ch]);
    }

    /// EC4 (runtime half): binding after the simulation has started is a clean error.
    #[test]
    fn bind_after_start_errors() {
        let sim = Sim::new();
        let ch = channel(&sim, "ch");
        let p = Port::<Marker>::new(&sim, "p");
        sim.run_until(SimTime::ZERO); // advances past the Build phase
        assert!(p.bind_channel(&sim, ch).is_err());
    }
}
