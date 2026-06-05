//! Sockets: the user-facing connection primitive, with the crossed bind cycle
//! neutralized by a kernel-owned registry of `Copy` ids.
//!
//! A literal `Rc` cycle between initiator and target would leak; the resolution is
//! a kernel-owned **socket registry with id handles** — a cycle of indices is not a
//! memory-management cycle (`doc/systemrs-design.md` §6d). The convenience-socket
//! ergonomics register **boxed closures**, replacing SystemC's `void*` trampoline.

use std::cell::RefCell;
use std::rc::Rc;

use slotmap::{SecondaryMap, SlotMap};
use systemrs_kernel::{Ctx, Sim};
use systemrs_time::SimTime;

use crate::gp::GenericPayload;

slotmap::new_key_type! {
    /// Identifies an initiator socket in the registry.
    pub struct InitiatorId;

    /// Identifies a target socket in the registry.
    pub struct TargetId;
}

/// A registered blocking-transport callback.
///
/// Stored as a shared `Rc<dyn Fn>` (not a taken-out `FnMut`) so that
/// `b_transport` *clones* the handle and calls it without removing it from the
/// registry. This makes the call re-entrancy-safe (a second initiator may legally
/// enter the same target while the first is parked at a `wait()` inside
/// `b_transport`) and unwind-safe (no slot is left empty if the callback panics).
/// Targets that need mutable state use interior mutability (e.g. `RefCell`).
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

/// The kernel-owned socket registry (a [`Sim`] service).
pub(crate) struct SocketRegistry {
    /// Target sockets and their callbacks.
    targets: SlotMap<TargetId, TargetEntry>,

    /// Allocated initiator socket ids.
    initiators: SlotMap<InitiatorId, ()>,

    /// Forward routing: initiator → bound target.
    bindings: SecondaryMap<InitiatorId, TargetId>,
}

impl SocketRegistry {
    /// Creates an empty registry.
    fn new() -> Self {
        SocketRegistry {
            targets: SlotMap::with_key(),
            initiators: SlotMap::with_key(),
            bindings: SecondaryMap::new(),
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
/// The handle is a `Copy`/`Send` id, so it can be captured by an `SC_THREAD` body
/// (which must be `Send`) and used to call `b_transport` from any call depth.
#[derive(Debug, Clone, Copy)]
pub struct InitiatorSocket {
    /// The registry id.
    id: InitiatorId,
}

impl InitiatorSocket {
    /// Creates an unbound initiator socket.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved for a future name registry).
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new socket.
    pub fn new(sim: &Sim, name: &str) -> Self {
        let _ = name;
        let id = registry(sim).borrow_mut().initiators.insert(());
        InitiatorSocket { id }
    }

    /// Binds this initiator to `target` (the crossed forward routing).
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `target` - The target socket to bind to.
    pub fn bind(&self, sim: &Sim, target: &TargetSocket) {
        registry(sim)
            .borrow_mut()
            .bindings
            .insert(self.id, target.id);
    }

    /// Performs a blocking transport to the bound target.
    ///
    /// The target callback may call `ctx.wait` (e.g. to model access latency) —
    /// demonstrating the design's central property: `wait()` is reachable from
    /// inside `b_transport` (`doc/systemrs-design.md` §6a, §6d).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    /// * `txn` - The transaction payload, mutated in place by the target.
    /// * `delay` - The timing annotation; the target may increase it.
    ///
    /// # Panics
    ///
    /// Panics (FATAL) if the socket is unbound or the target has no `b_transport`
    /// callback.
    pub fn b_transport(&self, ctx: &Ctx, txn: &mut GenericPayload, delay: &mut SimTime) {
        let registry = registry_from_ctx(ctx);

        // Clone the shared callback handle without holding the registry borrow
        // across the call (the callback may re-enter the kernel via wait()).
        let callback = {
            let reg = registry.borrow();
            let target_id = reg.bindings.get(self.id).copied().unwrap_or_else(|| {
                systemrs_diag::report_fatal("SYSTEMRS/TLM2", "b_transport on an unbound socket")
            });
            reg.targets[target_id].b_transport.clone()
        };
        let callback = callback.unwrap_or_else(|| {
            systemrs_diag::report_fatal("SYSTEMRS/TLM2", "target has no b_transport callback")
        });

        callback(ctx, txn, delay);
    }

    /// Performs a side-effect-free debug transport to the bound target.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle (used only to reach the registry).
    /// * `txn` - The transaction payload to service.
    ///
    /// # Returns
    ///
    /// The number of bytes serviced (0 if no debug callback is registered).
    pub fn transport_dbg(&self, ctx: &Ctx, txn: &mut GenericPayload) -> u32 {
        let registry = registry_from_ctx(ctx);
        let callback = {
            let reg = registry.borrow();
            let Some(target_id) = reg.bindings.get(self.id).copied() else {
                return 0;
            };
            reg.targets[target_id].transport_dbg.clone()
        };
        match callback {
            Some(callback) => callback(txn),
            None => 0,
        }
    }
}

/// A target socket: the receiving end of a transaction path.
#[derive(Debug, Clone, Copy)]
pub struct TargetSocket {
    /// The registry id.
    id: TargetId,
}

impl TargetSocket {
    /// Creates a target socket with no callbacks yet.
    ///
    /// # Arguments
    ///
    /// * `sim` - The simulation under construction.
    /// * `name` - A hierarchical name (reserved for a future name registry).
    ///
    /// # Returns
    ///
    /// A `Copy` handle to the new socket.
    pub fn new(sim: &Sim, name: &str) -> Self {
        let _ = name;
        let id = registry(sim)
            .borrow_mut()
            .targets
            .insert(TargetEntry::default());
        TargetSocket { id }
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
        registry(sim).borrow_mut().targets[self.id].b_transport = Some(Rc::new(callback));
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
        registry(sim).borrow_mut().targets[self.id].transport_dbg = Some(Rc::new(callback));
    }
}
