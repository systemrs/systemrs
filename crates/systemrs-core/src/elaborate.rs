//! The elaboration lifecycle callbacks.

use systemrs_kernel::Ctx;

/// The four-phase elaboration lifecycle, as default-empty methods.
///
/// Mirrors SystemC's `before_end_of_elaboration` / `end_of_elaboration` /
/// `start_of_simulation` / `end_of_simulation` virtual callbacks
/// (`doc/systemrs-design.md` §3.4, §6b), driven in the fixed registry order with
/// the construction-done fixpoint. A module implements only the hooks it needs.
pub trait Elaborate {
    /// Called near the end of elaboration, before binding is finalized; modules
    /// created here still receive this callback (the construction fixpoint).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle for elaboration-time wiring.
    fn before_end_of_elaboration(&mut self, ctx: &Ctx) {
        let _ = ctx;
    }

    /// Called once binding is complete; the static hierarchy is now frozen.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    fn end_of_elaboration(&mut self, ctx: &Ctx) {
        let _ = ctx;
    }

    /// Called immediately before the first delta cycle.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    fn start_of_simulation(&mut self, ctx: &Ctx) {
        let _ = ctx;
    }

    /// Called once the simulation has stopped.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The kernel handle.
    fn end_of_simulation(&mut self, ctx: &Ctx) {
        let _ = ctx;
    }
}
