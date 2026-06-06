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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use systemrs_channels::{Export, Port, PortPolicy};
use systemrs_core::{ObjectKind, store};
use systemrs_kernel::{Ctx, ObjectId, Sim};
use systemrs_time::SimTime;

use crate::gp::GenericPayload;
use crate::mm::Txn;
use crate::phase::{Phase, TlmSync};
use crate::protocol::{BaseProtocol, BwBaseProtocol, Dmi};

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

/// A registered non-blocking transport callback (forward *or* backward).
///
/// Takes `&Txn` (`&Rc<RefCell<GenericPayload>>`) — the AT aliasing rule — not a
/// `&mut GenericPayload`, because the parked transaction is shared across phases.
type NbFn = Rc<dyn Fn(&Ctx, &Txn, Phase, &mut SimTime) -> TlmSync>;

/// A registered DMI-grant callback: populate `dmi`, return `true` if granted.
type DmiFn = Rc<dyn Fn(&Txn, &mut Dmi) -> bool>;

/// A registered DMI-invalidation callback (backward, target → initiator).
type InvalidateFn = Rc<dyn Fn(&Ctx, u64, u64)>;

/// The callbacks a target socket exposes.
#[derive(Default)]
struct TargetEntry {
    /// The blocking-transport (LT) callback, if registered.
    b_transport: Option<BTransportFn>,

    /// The forward non-blocking (AT) callback, if registered.
    nb_transport_fw: Option<NbFn>,

    /// The DMI-grant callback, if registered.
    get_direct_mem_ptr: Option<DmiFn>,

    /// The debug-transport callback, if registered.
    transport_dbg: Option<DbgFn>,
}

/// The callbacks an initiator socket exposes on the backward (AT) path.
#[derive(Default)]
struct InitiatorEntry {
    /// The backward non-blocking (AT) callback, if registered.
    nb_transport_bw: Option<NbFn>,

    /// The DMI-invalidation callback, if registered.
    invalidate_direct_mem_ptr: Option<InvalidateFn>,
}

/// The kernel-owned closure store.
struct SocketRegistry {
    /// Target object id → its forward callbacks.
    targets: HashMap<ObjectId, TargetEntry>,

    /// Initiator `bw_export` object id → its backward callback. Keyed by the
    /// `bw_export` id because an [`InitiatorSocket`] has no own id; that id is the
    /// only one a target can resolve via its `bw_port`.
    initiators: HashMap<ObjectId, InitiatorEntry>,

    /// `true` while an `invalidate_direct_mem_ptr` is in progress; the re-entrancy
    /// guard that forbids `get_direct_mem_ptr` from within it (§3.9 HARD RULE).
    dmi_invalidating: Cell<bool>,
}

impl SocketRegistry {
    /// Creates an empty registry.
    fn new() -> Self {
        SocketRegistry {
            targets: HashMap::new(),
            initiators: HashMap::new(),
            dmi_invalidating: Cell::new(false),
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

    /// The backward export; its object id keys this initiator's `nb_transport_bw`
    /// callback and is what a bound target's `bw_port` resolves to (`ZeroOrMore`, so
    /// a pure-LT model that never uses the AT path still elaborates).
    bw_export: Export<BwBaseProtocol>,
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
            bw_export: Export::with_policy(sim, &format!("{name}.bw"), PortPolicy::ZeroOrMore),
        }
    }

    /// Returns the backward-export object id (the key for this initiator's
    /// `nb_transport_bw` callback).
    pub fn bw_id(&self) -> ObjectId {
        self.bw_export.id()
    }

    /// Binds this initiator to `target` (deferred; resolved at the barrier).
    ///
    /// Records the crossed double-bind: the forward path (`self.port` → `target`) and
    /// the backward path (`target.bw_port` → this initiator's `bw_export`), so the AT
    /// `nb_transport_bw` can route from the target back to this initiator.
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
        // Backward (target → initiator) crossed bind: the target's bw_port resolves
        // to this initiator's bw_export id.
        if let Err(e) = target.bw_port.bind_channel(sim, self.bw_export.id()) {
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

    /// Forward non-blocking transport to the bound target (the AT request path).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The shared transaction handle.
    /// * `phase` - The current phase.
    /// * `delay` - The timing annotation.
    ///
    /// # Returns
    ///
    /// The target's [`TlmSync`] (or [`TlmSync::Accepted`] if it has no nb callback).
    pub fn nb_transport_fw(
        &self,
        ctx: &Ctx,
        txn: &Txn,
        phase: Phase,
        delay: &mut SimTime,
    ) -> TlmSync {
        let target = self.resolve_target(ctx);
        let callback = registry_from_ctx(ctx)
            .borrow()
            .targets
            .get(&target)
            .and_then(|t| t.nb_transport_fw.clone());
        match callback {
            Some(callback) => callback(ctx, txn, phase, delay),
            None => TlmSync::Accepted,
        }
    }

    /// Registers this initiator's backward (response-path) non-blocking callback.
    ///
    /// The target reaches it by resolving its `bw_port` to this initiator's
    /// `bw_export` id (the registry key).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `nb_transport_bw` implementation.
    pub fn register_nb_transport_bw<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &Txn, Phase, &mut SimTime) -> TlmSync + 'static,
    {
        let reg = registry(sim);
        let mut reg = reg.borrow_mut();
        let entry = reg.initiators.entry(self.bw_export.id()).or_default();
        debug_assert!(
            entry.nb_transport_bw.is_none(),
            "nb_transport_bw already registered on this initiator socket"
        );
        entry.nb_transport_bw = Some(Rc::new(callback));
    }

    /// Requests a DMI grant for `txn`'s region from the bound target.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction describing the region.
    ///
    /// # Returns
    ///
    /// The granted [`Dmi`] descriptor, or `None` if the target denies DMI or has no
    /// DMI callback.
    ///
    /// # Panics
    ///
    /// Aborts (FATAL) if called re-entrantly from inside an
    /// `invalidate_direct_mem_ptr` (the §3.9 HARD RULE).
    pub fn get_direct_mem_ptr(&self, ctx: &Ctx, txn: &Txn) -> Option<Dmi> {
        let reg = registry_from_ctx(ctx);
        if reg.borrow().dmi_invalidating.get() {
            systemrs_diag::report_fatal(
                "SYSTEMRS/TLM2",
                "get_direct_mem_ptr called inside invalidate_direct_mem_ptr",
            );
        }
        let target = self.resolve_target(ctx);
        let callback = reg
            .borrow()
            .targets
            .get(&target)
            .and_then(|t| t.get_direct_mem_ptr.clone())?;
        let mut dmi = Dmi::default();
        callback(txn, &mut dmi).then_some(dmi)
    }

    /// Registers this initiator's DMI-invalidation (backward) callback.
    ///
    /// HARD RULE: the callback must not call `get_direct_mem_ptr` (enforced at
    /// runtime by the registry re-entrancy guard).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `invalidate_direct_mem_ptr` implementation.
    pub fn register_invalidate_direct_mem_ptr<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, u64, u64) + 'static,
    {
        let reg = registry(sim);
        let mut reg = reg.borrow_mut();
        let entry = reg.initiators.entry(self.bw_export.id()).or_default();
        debug_assert!(
            entry.invalidate_direct_mem_ptr.is_none(),
            "invalidate_direct_mem_ptr already registered on this initiator socket"
        );
        entry.invalidate_direct_mem_ptr = Some(Rc::new(callback));
    }
}

/// A target socket: the receiving end of a transaction path (the interface leaf).
#[derive(Debug, Clone, Copy)]
pub struct TargetSocket {
    /// The target's object id; the key for its registered callbacks and the
    /// interface id an initiator's port resolves to.
    id: ObjectId,

    /// The backward port (the response call path target → initiator); resolves to
    /// the bound initiator's `bw_export` id. `ZeroOrMore`, so an unbound (pure-LT)
    /// target still elaborates.
    bw_port: Port<BwBaseProtocol>,
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
        let bw_port = Port::with_policy(sim, &format!("{name}.bw"), PortPolicy::ZeroOrMore);
        TargetSocket { id, bw_port }
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

    /// Registers this target's forward non-blocking (AT request) callback.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - The `nb_transport_fw` implementation.
    pub fn register_nb_transport_fw<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Ctx, &Txn, Phase, &mut SimTime) -> TlmSync + 'static,
    {
        let reg = registry(sim);
        let mut reg = reg.borrow_mut();
        let entry = reg.targets.entry(self.id).or_default();
        debug_assert!(
            entry.nb_transport_fw.is_none(),
            "nb_transport_fw already registered on this target socket"
        );
        entry.nb_transport_fw = Some(Rc::new(callback));
    }

    /// Backward non-blocking transport to the bound initiator (the AT response path).
    ///
    /// Resolves `self.bw_port` to the bound initiator's `bw_export` id and calls its
    /// registered `nb_transport_bw` (or returns [`TlmSync::Accepted`] if none).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The shared transaction handle.
    /// * `phase` - The current phase.
    /// * `delay` - The timing annotation.
    ///
    /// # Returns
    ///
    /// The initiator's [`TlmSync`].
    pub fn nb_transport_bw(
        &self,
        ctx: &Ctx,
        txn: &Txn,
        phase: Phase,
        delay: &mut SimTime,
    ) -> TlmSync {
        let Some(&initiator) = self.bw_port.resolved_in_ctx(ctx).first() else {
            return TlmSync::Accepted;
        };
        let callback = registry_from_ctx(ctx)
            .borrow()
            .initiators
            .get(&initiator)
            .and_then(|i| i.nb_transport_bw.clone());
        match callback {
            Some(callback) => callback(ctx, txn, phase, delay),
            None => TlmSync::Accepted,
        }
    }

    /// Registers this target's DMI-grant callback.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `callback` - Populates the [`Dmi`] and returns `true` if DMI is granted.
    pub fn register_get_direct_mem_ptr<F>(&self, sim: &Sim, callback: F)
    where
        F: Fn(&Txn, &mut Dmi) -> bool + 'static,
    {
        let reg = registry(sim);
        let mut reg = reg.borrow_mut();
        let entry = reg.targets.entry(self.id).or_default();
        debug_assert!(
            entry.get_direct_mem_ptr.is_none(),
            "get_direct_mem_ptr already registered on this target socket"
        );
        entry.get_direct_mem_ptr = Some(Rc::new(callback));
    }

    /// Invalidates a previously-granted DMI region on the bound initiator.
    ///
    /// Sets the registry re-entrancy guard while the initiator's invalidation
    /// callback runs, so a `get_direct_mem_ptr` from within it aborts (§3.9).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `start` - The inclusive start address to invalidate.
    /// * `end` - The inclusive end address to invalidate.
    pub fn invalidate_direct_mem_ptr(&self, ctx: &Ctx, start: u64, end: u64) {
        let Some(&initiator) = self.bw_port.resolved_in_ctx(ctx).first() else {
            return;
        };
        let reg = registry_from_ctx(ctx);
        let callback = reg
            .borrow()
            .initiators
            .get(&initiator)
            .and_then(|i| i.invalidate_direct_mem_ptr.clone());
        if let Some(callback) = callback {
            reg.borrow().dmi_invalidating.set(true);
            callback(ctx, start, end);
            reg.borrow().dmi_invalidating.set(false);
        }
    }
}
