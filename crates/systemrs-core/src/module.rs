//! Module construction: the `module(name, |m| {…})` scope closure and [`Builder`].
//!
//! A module is a named interior node of the object hierarchy. Construction is
//! explicit via a scoped closure that pushes the module's scope, runs the body to
//! create children, and pops on return (`doc/systemrs-design.md` §6b) — replacing
//! SystemC's `sc_module_name` global and the hidden `sensitive <<` coupling.
//!
//! Two forms:
//!
//! - [`module`] — an anonymous scope: the closure creates children directly; the
//!   module itself has no lifecycle callbacks.
//! - [`module_with`] — a module *instance*: the closure builds a value implementing
//!   [`Elaborate`], which is registered so its four lifecycle callbacks fire at the
//!   barrier. This is what the `#[module]` proc-macro targets.
//!
//! ## Layering and ambient scope
//!
//! [`Builder`] lives in `systemrs-core` (below the channel/socket crates), so it
//! does not expose `m.signal()`/`m.socket()` helpers. Instead, channel and port
//! constructors take `m.sim()` and register under the **ambient scope** the builder
//! has pushed (e.g. `Port::new(m.sim(), "p")` lands under the current module). The
//! builder provides only the core-level child constructors: nested modules and
//! processes.

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_diag::ReportError;
use systemrs_kernel::{ObjectId, Phase, Sim};

use crate::build::{MethodBuilder, ThreadBuilder};
use crate::elaborate::Elaborate;
use crate::hierarchy::ScopeGuard;
use crate::object::{ObjectKind, store};

/// Marker trait for module types (`sc_module`).
///
/// Implemented (by hand or by the `#[module]` proc-macro) on a struct whose fields
/// are the module's sub-components and whose [`Elaborate`] impl carries its
/// lifecycle logic. Registered via [`module_with`].
pub trait Module: Elaborate {}

/// Opens a module object under the current scope and returns it with a scope guard.
fn open_scope(sim: &Sim, name: &str) -> Result<(ObjectId, ScopeGuard), ReportError> {
    if sim.phase() != Phase::Build {
        return Err(systemrs_diag::error(
            "SYSTEMRS/ELAB",
            "modules can only be created during elaboration (before the simulation starts)",
        ));
    }
    let store = store(sim);
    let parent = store.borrow().current_scope();
    let id = store.borrow_mut().insert(parent, name, ObjectKind::Module);
    let guard = ScopeGuard::enter(Rc::clone(&store), id);
    Ok((id, guard))
}

/// Creates an anonymous module scope and runs `build` to populate its children.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `name` - The module's local name (made unique among its siblings).
/// * `build` - A closure that creates the module's children via the [`Builder`].
///
/// # Returns
///
/// The module's [`ObjectId`].
///
/// # Errors
///
/// Returns a `SYSTEMRS/ELAB` error if called after the simulation has started.
pub fn module<F>(sim: &Sim, name: &str, build: F) -> Result<ObjectId, ReportError>
where
    F: FnOnce(&mut Builder),
{
    let (id, guard) = open_scope(sim, name)?;
    let mut builder = Builder { sim, scope: id };
    build(&mut builder);
    drop(guard);
    Ok(id)
}

/// Creates a module *instance* and registers it as an elaborator.
///
/// The `build` closure constructs the module value (creating its children in scope)
/// and returns it; the value is registered so its [`Elaborate`] lifecycle callbacks
/// fire at the barrier. The returned handle gives access to the instance afterwards.
///
/// # Arguments
///
/// * `sim` - The simulation under construction.
/// * `name` - The module's local name.
/// * `build` - A closure that builds the module value from the [`Builder`].
///
/// # Returns
///
/// A shared handle to the constructed module instance.
///
/// # Errors
///
/// Returns a `SYSTEMRS/ELAB` error if called after the simulation has started.
pub fn module_with<M, F>(sim: &Sim, name: &str, build: F) -> Result<Rc<RefCell<M>>, ReportError>
where
    M: Elaborate + 'static,
    F: FnOnce(&mut Builder) -> M,
{
    let (id, guard) = open_scope(sim, name)?;
    let mut builder = Builder { sim, scope: id };
    let instance = build(&mut builder);
    drop(guard);
    let rc = Rc::new(RefCell::new(instance));
    store(sim).borrow_mut().attach_elaborator(id, rc.clone());
    Ok(rc)
}

/// The handle a module body uses to create its children, with explicit scope and
/// no hidden last-process state (`doc/systemrs-design.md` §6b).
///
/// Provides the core-level child constructors (nested modules, processes); channels
/// and ports are created with [`Builder::sim`] and register under the ambient scope.
pub struct Builder<'s> {
    /// The simulation under construction.
    sim: &'s Sim,

    /// The current module's object id (the ambient scope for child constructors).
    scope: ObjectId,
}

impl<'s> Builder<'s> {
    /// Returns the simulation handle, for constructing channels/ports that register
    /// under the ambient scope (e.g. `Port::new(m.sim(), "p")`).
    pub fn sim(&self) -> &'s Sim {
        self.sim
    }

    /// Returns this module's object id.
    pub fn scope(&self) -> ObjectId {
        self.scope
    }

    /// Creates a nested anonymous sub-module under this module.
    ///
    /// # Arguments
    ///
    /// * `name` - The sub-module's local name.
    /// * `build` - The sub-module body.
    ///
    /// # Returns
    ///
    /// The sub-module's [`ObjectId`].
    ///
    /// # Errors
    ///
    /// Propagates any error from [`module`].
    pub fn module<F>(&self, name: &str, build: F) -> Result<ObjectId, ReportError>
    where
        F: FnOnce(&mut Builder),
    {
        module(self.sim, name, build)
    }

    /// Creates a nested module instance (with lifecycle callbacks) under this module.
    ///
    /// # Arguments
    ///
    /// * `name` - The sub-module's local name.
    /// * `build` - Builds the sub-module value.
    ///
    /// # Returns
    ///
    /// A shared handle to the sub-module instance.
    ///
    /// # Errors
    ///
    /// Propagates any error from [`module_with`].
    pub fn module_with<M, F>(&self, name: &str, build: F) -> Result<Rc<RefCell<M>>, ReportError>
    where
        M: Elaborate + 'static,
        F: FnOnce(&mut Builder) -> M,
    {
        module_with(self.sim, name, build)
    }

    /// Begins an `SC_METHOD` scoped to this module (hierarchically named).
    ///
    /// # Arguments
    ///
    /// * `name` - The method's local name (qualified with the module path).
    ///
    /// # Returns
    ///
    /// A [`MethodBuilder`] to add sensitivity and finalize.
    pub fn method(&self, name: &str) -> MethodBuilder<'s> {
        MethodBuilder::for_name(self.sim, self.register_process(name))
    }

    /// Begins an `SC_THREAD` scoped to this module (hierarchically named).
    ///
    /// The thread body inherits the kernel's `Send` requirement, so it must capture
    /// only `Send` state — `Copy` handles (ids, sockets) or `Arc`, never `Rc`.
    ///
    /// # Arguments
    ///
    /// * `name` - The thread's local name (qualified with the module path).
    ///
    /// # Returns
    ///
    /// A [`ThreadBuilder`] to add sensitivity and finalize.
    pub fn thread(&self, name: &str) -> ThreadBuilder<'s> {
        ThreadBuilder::for_name(self.sim, self.register_process(name))
    }

    /// Registers a `Process` object under this module's scope and returns its full,
    /// dot-joined hierarchical name (used as the kernel process name).
    fn register_process(&self, name: &str) -> String {
        let store = store(self.sim);
        let id = store
            .borrow_mut()
            .insert(self.scope, name, ObjectKind::Process);
        store.borrow().full_name(id).to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use systemrs_kernel::{Ctx, Sim};
    use systemrs_time::SimTime;

    use super::{Builder, Module, module, module_with};
    use crate::object::{ObjectKind, store};
    use crate::Elaborate;

    /// EC1: nested modules and their child processes get unique dot-joined names.
    #[test]
    fn nested_modules_have_dot_joined_names() {
        let sim = Sim::new();
        let cpu_id = Cell::new(None);
        let top = module(&sim, "top", |t: &mut Builder| {
            let c = t
                .module("cpu", |c: &mut Builder| {
                    c.method("tick").dont_initialize().finish(|_| {});
                })
                .expect("cpu");
            cpu_id.set(Some(c));
        })
        .expect("top");

        let store = store(&sim);
        let s = store.borrow();
        assert_eq!(s.full_name(top), "top");
        let cpu = cpu_id.get().expect("cpu id");
        assert_eq!(s.full_name(cpu), "top.cpu");
        let kids = s.children(cpu);
        assert_eq!(kids.len(), 1);
        assert_eq!(s.full_name(kids[0]), "top.cpu.tick");
        assert_eq!(s.kind(kids[0]), Some(ObjectKind::Process));
    }

    /// A duplicate sibling module name is auto-renamed.
    #[test]
    fn duplicate_module_names_are_unique() {
        let sim = Sim::new();
        let a = module(&sim, "m", |_| {}).expect("a");
        let b = module(&sim, "m", |_| {}).expect("b");
        let store = store(&sim);
        let s = store.borrow();
        assert_eq!(s.full_name(a), "m");
        assert_eq!(s.full_name(b), "m_0");
    }

    /// A module instance built with `module_with` has its lifecycle callbacks driven.
    #[test]
    fn module_with_drives_callbacks() {
        /// A minimal module that records its lifecycle callbacks.
        struct Probe {
            log: Rc<RefCell<Vec<&'static str>>>,
        }

        impl Elaborate for Probe {
            fn before_end_of_elaboration(&mut self, _ctx: &Ctx) {
                self.log.borrow_mut().push("before");
            }
            fn end_of_elaboration(&mut self, _ctx: &Ctx) {
                self.log.borrow_mut().push("end");
            }
            fn start_of_simulation(&mut self, _ctx: &Ctx) {
                self.log.borrow_mut().push("start");
            }
        }

        impl Module for Probe {}

        let sim = Sim::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let _cpu = module_with(&sim, "cpu", |_b| Probe {
            log: Rc::clone(&log),
        })
        .expect("cpu");

        sim.run_until(SimTime::ZERO);
        assert_eq!(*log.borrow(), vec!["before", "end", "start"]);
    }

    /// Creating a module after the simulation has started is a clean error.
    #[test]
    fn module_after_start_errors() {
        let sim = Sim::new();
        sim.run_until(SimTime::ZERO);
        assert!(module(&sim, "late", |_| {}).is_err());
    }
}
