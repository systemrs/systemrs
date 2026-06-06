//! Sockets, reconciled onto the generic two-phase binding (`doc/systemrs-design.md`
//! §6d): an [`InitiatorSocket`] *is* a forward [`Port`], a [`TargetSocket`] is the
//! interface-providing leaf its `b_transport`/`transport_dbg` closures hang off.
//!
//! `bind` is now **deferred** — it records a bind request resolved at the
//! elaboration barrier by the port's `complete_binding`, rather than wiring the
//! routing immediately. An unbound initiator therefore fails its `OneOrMore` port
//! policy at elaboration (a clean FATAL at the barrier), not at first transport.
//!
//! The closure registry (a `Sim` service) is kept as **resolved-interface storage**:
//! after binding resolves the initiator's port to the target's object id, transport
//! looks the closure up by that id. The crossed backward path (`Port<BwTransport>`,
//! the double-bind, `nb_transport_bw`) is **out of scope for Milestone 2** — only
//! the forward LT path is modelled here; the backward path lands in M4. Likewise the
//! target-side *export* hierarchy (passthrough/multi sockets) is deferred.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use systemrs_channels::{Port, PortPolicy};
use systemrs_core::{ObjectKind, store};
use systemrs_kernel::{Ctx, ObjectId, Sim};
use systemrs_time::SimTime;

use crate::gp::GenericPayload;
use crate::protocol::BaseProtocol;

/// A registered blocking-transport callback.
///
/// Stored as a shared `Rc<dyn Fn>` so `b_transport` *clones* the handle and calls
/// it without removing it from the registry — re-entrancy-safe (a second initiator
/// may legally enter the same target while the first is parked at a `wait()` inside
/// `b_transport`) and unwind-safe. Targets that need mutable state use interior
/// mutability (e.g. `RefCell`).
type BTransportFn = Rc<dyn Fn(&Ctx, &mut GenericPayload, &mut SimTime)>;

/// A registered debug-transport callback (shared, re-entrancy-safe).
type DbgFn = Rc<dyn Fn(&mut GenericPayload) -> u32>;

/// The callbacks a target socket exposes.
#[derive(Default)]
struct TargetEntry {
    /// The blocking-transport callback, if registered.
    b_transport: Option<BTransportFn>,

    /// The debug-transport callback, if registered.
    transport_dbg: Option<DbgFn>,
}

/// The kernel-owned closure store, keyed by each target socket's object id.
struct SocketRegistry {
    /// Target object id → its callbacks.
    targets: HashMap<ObjectId, TargetEntry>,
}

impl SocketRegistry {
    /// Creates an empty registry.
    fn new() -> Self {
        SocketRegistry {
            targets: HashMap::new(),
        }
    }
}

/// Returns the simulation's socket registry, creating it on first use.
fn registry(sim: &Sim) -> Rc<RefCell<SocketRegistry>> {
    let ctx = sim.ctx();
    if let Some(existing) = ctx.try_service::<RefCell<SocketRegistry>>() {
        return existing;
    }
    let registry = Rc::new(RefCell::new(SocketRegistry::new()));
    sim.register_service(Rc::clone(&registry));
    registry
}

/// Returns the socket registry from a runtime [`Ctx`].
fn registry_from_ctx(ctx: &Ctx) -> Rc<RefCell<SocketRegistry>> {
    ctx.service::<RefCell<SocketRegistry>>()
}

/// An initiator socket: the forward end of a transaction path.
///
/// A `Copy` handle wrapping a forward [`Port`], so it can be captured by an
/// `SC_THREAD` body and used to call `b_transport` from any call depth.
#[derive(Debug, Clone, Copy)]
pub struct InitiatorSocket {
    /// The forward port; resolves to the bound target's object id at elaboration.
    port: Port<BaseProtocol>,
}

impl InitiatorSocket {
    /// Creates an unbound initiator socket (forward port, `OneOrMore` policy).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (registered under the current scope).
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new socket.
    pub fn new(sim: &Sim, name: &str) -> Self {
        InitiatorSocket {
            port: Port::with_policy(sim, name, PortPolicy::OneOrMore),
        }
    }

    /// Binds this initiator to `target` (deferred; resolved at the barrier).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to bind to.
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if called after the simulation has started.
    pub fn bind(&self, sim: &Sim, target: &TargetSocket) {
        if let Err(e) = self.port.bind_channel(sim, target.id) {
            systemrs_diag::report_fatal("SYSTEMRS/TLM2", &format!("{e}"));
        }
    }

    /// Resolves the bound target's object id from a running [`Ctx`].
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if the socket is unbound or its binding did not resolve.
    fn resolve_target(self, ctx: &Ctx) -> ObjectId {
        *self.port.resolved_in_ctx(ctx).first().unwrap_or_else(|| {
            systemrs_diag::report_fatal(
                "SYSTEMRS/TLM2",
                "b_transport on an unbound or unresolved socket",
            )
        })
    }

    /// Performs a blocking transport to the bound target.
    ///
    /// The target callback may call `ctx.wait` (e.g. to model access latency),
    /// demonstrating that `wait()` is reachable from inside `b_transport`
    /// (`doc/systemrs-design.md` §6a, §6d).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction payload, mutated in place by the target.
    /// * `delay` - The timing annotation; the target may increase it.
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if the socket is unresolved or the target has no `b_transport`.
    pub fn b_transport(&self, ctx: &Ctx, txn: &mut GenericPayload, delay: &mut SimTime) {
        let target = self.resolve_target(ctx);
        let callback = registry_from_ctx(ctx)
            .borrow()
            .targets
            .get(&target)
            .and_then(|t| t.b_transport.clone());
        let callback = callback.unwrap_or_else(|| {
            systemrs_diag::report_fatal("SYSTEMRS/TLM2", "target has no b_transport callback")
        });
        callback(ctx, txn, delay);
    }

    /// Performs a side-effect-free debug transport to the bound target.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction payload to service.
    ///
    /// # Returns
    ///
    /// The number of bytes serviced (0 if unbound or no debug callback registered).
    pub fn transport_dbg(&self, ctx: &Ctx, txn: &mut GenericPayload) -> u32 {
        let Some(&target) = self.port.resolved_in_ctx(ctx).first() else {
            return 0;
        };
        let callback = registry_from_ctx(ctx)
            .borrow()
            .targets
            .get(&target)
            .and_then(|t| t.transport_dbg.clone());
        match callback {
            Some(callback) => callback(txn),
            None => 0,
        }
    }
}

/// A target socket: the receiving end of a transaction path (the interface leaf).
#[derive(Debug, Clone, Copy)]
pub struct TargetSocket {
    /// The target's object id; the key for its registered callbacks and the
    /// interface id an initiator's port resolves to.
    id: ObjectId,
}

impl TargetSocket {
    /// Creates a target socket with no callbacks yet, registered under the current
    /// scope.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name.
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new socket.
    pub fn new(sim: &Sim, name: &str) -> Self {
        let store = store(sim);
        let parent = store.borrow().current_scope();
        let id = store.borrow_mut().insert(parent, name, ObjectKind::Socket);
        registry(sim)
            .borrow_mut()
            .targets
            .insert(id, TargetEntry::default());
        TargetSocket { id }
    }

    /// Returns this target socket's object id (the interface id initiators resolve to).
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// Registers the blocking-transport callback (a convenience-socket binding).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `b_transport` implementation. It receives the kernel
    ///   handle and may `wait`.
    pub fn register_b_transport<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &mut GenericPayload, &mut SimTime) + 'static,
    {
        registry(sim)
            .borrow_mut()
            .targets
            .entry(self.id)
            .or_default()
            .b_transport = Some(Rc::new(callback));
    }

    /// Registers the debug-transport callback.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `transport_dbg` implementation (no `Ctx`, wait-free).
    pub fn register_transport_dbg<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&mut GenericPayload) -> u32 + 'static,
    {
        registry(sim)
            .borrow_mut()
            .targets
            .entry(self.id)
            .or_default()
            .transport_dbg = Some(Rc::new(callback));
    }
}
