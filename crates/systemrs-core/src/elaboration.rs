//! The elaboration driver: the construction fixpoint and the lifecycle callbacks.
//!
//! Installed as the kernel's elaboration hook by [`crate::store`], [`drive`] runs
//! once at the elaboration barrier (between `install_current` and the first
//! evaluate, `doc/systemrs-design.md` §6b). It reproduces SystemC's elaboration
//! sequence over the object store's four per-bucket elaborator registries:
//!
//! 1. **`before_end_of_elaboration` construction fixpoint** — drive the not-yet-seen
//!    elaborators in each bucket (in [`BUCKET_ORDER`]), repeating until a full pass
//!    registers nothing new. Modules created inside a callback are picked up the
//!    next pass.
//! 2. **`end_of_elaboration`** in bucket order — ports and exports complete their
//!    binding here (their [`crate::Elaborate`] impl runs `complete_binding`), so a
//!    module sees resolved bindings.
//! 3. **`start_of_simulation`** in bucket order.
//!
//! `end_of_simulation` (the teardown hook) fires in the same forward bucket order.
//!
//! ## Borrow-release discipline
//!
//! Each pass takes the store borrow only long enough to *clone the `Rc` elaborators
//! out*, then releases it before invoking a callback — so a callback that creates a
//! new object (re-entering the store via `cx.module`/a port constructor) cannot
//! double-borrow it. The store is referenced only by id between calls.

use std::cell::RefCell;

use systemrs_diag::ReportError;
use systemrs_kernel::Ctx;

use crate::object::{BUCKET_ORDER, ObjectStore};

/// Drives the elaboration barrier once over the simulation's object store.
///
/// # Arguments
///
/// * `ctx` - The kernel handle for the elaborating simulation.
///
/// # Returns
///
/// `Ok(())` when elaboration completes. (Binding/cardinality failures surface as a
/// FATAL abort from within the offending endpoint's `end_of_elaboration`, so they
/// abort at the barrier rather than being threaded back here.)
///
/// # Errors
///
/// Reserved for future non-binding elaboration failures; currently always `Ok`.
// The `Result` return is dictated by the kernel `ElaborationHook` contract; binding
// failures abort via FATAL inside the offending endpoint, so today this is always
// `Ok`, but the fallible signature must be preserved for the hook.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn drive(ctx: &Ctx) -> Result<(), ReportError> {
    let Some(store) = ctx.try_service::<RefCell<ObjectStore>>() else {
        return Ok(());
    };

    // (1) before_end_of_elaboration — per-bucket construction fixpoint.
    loop {
        let mut any_new = false;
        for kind in BUCKET_ORDER {
            let batch = store.borrow_mut().take_new_before_end(kind);
            if !batch.is_empty() {
                any_new = true;
            }
            for elab in batch {
                elab.borrow_mut().before_end_of_elaboration(ctx);
            }
        }
        if !any_new {
            break;
        }
    }

    // (2) end_of_elaboration — ports/exports complete their binding here.
    for kind in BUCKET_ORDER {
        // Bind to a local so the store borrow is released before the callbacks run
        // (the borrow-release discipline; a callback may re-enter the store).
        let elabs = store.borrow().all_elaborators(kind);
        for elab in elabs {
            elab.borrow_mut().end_of_elaboration(ctx);
        }
    }

    // (3) start_of_simulation.
    for kind in BUCKET_ORDER {
        let elabs = store.borrow().all_elaborators(kind);
        for elab in elabs {
            elab.borrow_mut().start_of_simulation(ctx);
        }
    }

    Ok(())
}

/// Fires `end_of_simulation` on every elaborator, in forward bucket order.
///
/// Installed as the kernel's end-of-simulation hook; invoked once at teardown.
///
/// # Arguments
///
/// * `ctx` - The kernel handle for the simulation being torn down.
pub(crate) fn end_of_simulation(ctx: &Ctx) {
    let Some(store) = ctx.try_service::<RefCell<ObjectStore>>() else {
        return;
    };
    for kind in BUCKET_ORDER {
        let elabs = store.borrow().all_elaborators(kind);
        for elab in elabs {
            elab.borrow_mut().end_of_simulation(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use systemrs_kernel::{Ctx, Sim};
    use systemrs_time::SimTime;

    use crate::object::{ObjectKind, ObjectStore};
    use crate::{Elaborate, store};

    /// A shared list of `"tag:phase"` strings recording callback order.
    type Log = Rc<RefCell<Vec<String>>>;

    /// An `Elaborate` probe that logs each callback and may spawn one child on its
    /// first `before_end_of_elaboration` (to exercise the construction fixpoint and
    /// store re-entrancy).
    struct Probe {
        tag: String,
        kind: ObjectKind,
        log: Log,
        spawn: Option<(String, ObjectKind)>,
        spawned: bool,
    }

    impl Probe {
        fn new(tag: &str, kind: ObjectKind, log: &Log) -> Self {
            Probe {
                tag: tag.to_owned(),
                kind,
                log: Rc::clone(log),
                spawn: None,
                spawned: false,
            }
        }

        fn spawning(tag: &str, kind: ObjectKind, child: (&str, ObjectKind), log: &Log) -> Self {
            let mut p = Probe::new(tag, kind, log);
            p.spawn = Some((child.0.to_owned(), child.1));
            p
        }
    }

    impl Elaborate for Probe {
        fn object_kind(&self) -> ObjectKind {
            self.kind
        }

        fn before_end_of_elaboration(&mut self, ctx: &Ctx) {
            self.log.borrow_mut().push(format!("{}:before", self.tag));
            if self.spawned {
                return;
            }
            if let Some((ctag, ckind)) = self.spawn.clone() {
                self.spawned = true;
                // Re-enter the store from inside a callback (the driver must hold no
                // borrow here, else this double-borrows and panics).
                if let Some(store) = ctx.try_service::<RefCell<ObjectStore>>() {
                    let root = store.borrow().root();
                    let child = Probe::new(&ctag, ckind, &self.log);
                    store.borrow_mut().register_elaborator(
                        root,
                        ckind,
                        &ctag,
                        Rc::new(RefCell::new(child)),
                    );
                }
            }
        }

        fn end_of_elaboration(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push(format!("{}:end", self.tag));
        }

        fn start_of_simulation(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push(format!("{}:start", self.tag));
        }

        fn end_of_simulation(&mut self, _ctx: &Ctx) {
            self.log.borrow_mut().push(format!("{}:eos", self.tag));
        }
    }

    fn register(sim: &Sim, p: Probe) {
        let kind = p.kind;
        let tag = p.tag.clone();
        let s = store(sim);
        let root = s.borrow().root();
        s.borrow_mut()
            .register_elaborator(root, kind, &tag, Rc::new(RefCell::new(p)));
    }

    /// Callbacks fire in fixed bucket order (port → module) across all phases, and
    /// the whole barrier runs exactly once across stepped `run_until` calls.
    #[test]
    fn callbacks_run_in_bucket_order_once() {
        let sim = Sim::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        // Register module before port to prove ordering is by bucket, not insertion.
        register(&sim, Probe::new("m", ObjectKind::Module, &log));
        register(&sim, Probe::new("p", ObjectKind::Port, &log));

        sim.run_until(SimTime::ZERO);
        sim.run_until(SimTime::from_ns(10)); // second call must not re-elaborate

        assert_eq!(
            *log.borrow(),
            vec![
                "p:before", "m:before", // before_end_of_elaboration, port then module
                "p:end", "m:end", // end_of_elaboration
                "p:start", "m:start", // start_of_simulation
            ]
        );
    }

    /// A module created inside `before_end_of_elaboration` still receives its own
    /// `before_end_of_elaboration` (the construction fixpoint) — and re-entering the
    /// store from the callback does not panic (the borrow-release discipline).
    #[test]
    fn construction_fixpoint_and_reentrancy() {
        let sim = Sim::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        register(
            &sim,
            Probe::spawning(
                "root",
                ObjectKind::Module,
                ("child", ObjectKind::Module),
                &log,
            ),
        );

        sim.run_until(SimTime::ZERO);

        let entries = log.borrow();
        assert!(entries.contains(&"root:before".to_string()));
        assert!(entries.contains(&"child:before".to_string()));
        assert!(entries.contains(&"child:end".to_string()));
        assert!(entries.contains(&"child:start".to_string()));
    }

    /// A spawned port and module from one callback land in their own buckets and
    /// both receive `before_end_of_elaboration` (mixed-kind fixpoint).
    #[test]
    fn mixed_kind_fixpoint() {
        let sim = Sim::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        register(
            &sim,
            Probe::spawning("m", ObjectKind::Module, ("cp", ObjectKind::Port), &log),
        );
        sim.run_until(SimTime::ZERO);
        let store = store(&sim);
        assert_eq!(store.borrow().bucket_len(ObjectKind::Port), 1);
        assert!(log.borrow().contains(&"cp:before".to_string()));
    }

    /// `end_of_simulation` fires exactly once (via the teardown latch).
    #[test]
    fn end_of_simulation_fires_once() {
        let sim = Sim::new();
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        register(&sim, Probe::new("m", ObjectKind::Module, &log));
        sim.run_until(SimTime::ZERO);
        sim.end_of_sim();
        sim.end_of_sim(); // idempotent
        assert_eq!(log.borrow().iter().filter(|s| *s == "m:eos").count(), 1);
    }
}
