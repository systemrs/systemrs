//! [`ScopeGuard`]: RAII push/pop of the hierarchy scope.
//!
//! Replaces SystemC's `sc_module_name` LIFO global (`doc/systemrs-design.md` §6b).
//! While a module body runs, the module's [`ObjectId`] is the current scope, so
//! child objects (sub-modules, ports, processes, channels) register under it. The
//! guard pops on drop — including when the module body panics — and asserts the
//! popped scope matches, catching corruption (`sc_object.cpp` scope checks).

use std::cell::RefCell;
use std::rc::Rc;

use systemrs_kernel::ObjectId;

use crate::object::ObjectStore;

/// An RAII guard that pushes a hierarchy scope on creation and pops it on drop.
pub(crate) struct ScopeGuard {
    /// The shared object store whose scope stack is managed.
    store: Rc<RefCell<ObjectStore>>,

    /// The scope this guard pushed (verified on pop).
    id: ObjectId,
}

impl ScopeGuard {
    /// Pushes `id` as the current scope and returns the guard.
    ///
    /// # Arguments
    ///
    /// * `store` - The shared object store.
    /// * `id` - The scope (module object) being entered.
    ///
    /// # Returns
    ///
    /// A guard that pops `id` when dropped.
    pub(crate) fn enter(store: Rc<RefCell<ObjectStore>>, id: ObjectId) -> Self {
        store.borrow_mut().push_scope(id);
        ScopeGuard { store, id }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let popped = self.store.borrow_mut().pop_scope();
        if popped != Some(self.id) {
            systemrs_diag::report_fatal(
                "SYSTEMRS/SCOPE",
                "hierarchy scope stack corrupted: popped scope does not match the guard",
            );
        }
    }
}
