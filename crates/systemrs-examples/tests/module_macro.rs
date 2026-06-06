//! Verifies the `#[module]` proc-macro works through the `systemrs` facade.
//!
//! This crate depends *only* on the `systemrs` facade (not on `systemrs-macros`
//! directly), so a successful compile proves the macro's `::systemrs::`-qualified
//! codegen resolves with no dependency cycle.

use systemrs::module;
use systemrs::prelude::*;

/// A module declared with the `#[module]` attribute (which generates `impl Module`).
#[module]
struct Widget {
    /// A field, to confirm the struct body is preserved by the macro.
    _id: u32,
}

impl Elaborate for Widget {
    fn end_of_elaboration(&mut self, _ctx: &Ctx) {
        // A module with one trivial callback.
    }
}

/// Compiles only if `M` implements the generated `Module` marker.
fn assert_is_module<M: Module>() {}

/// The `#[module]` attribute generates the `Module` marker impl, and the type can be
/// built and elaborated through the `Kernel` front door.
#[test]
fn module_macro_generates_marker_and_elaborates() {
    assert_is_module::<Widget>();

    let kernel = Kernel::<Building>::new();
    kernel
        .module_with("widget", |_b| Widget { _id: 7 })
        .expect("widget builds");
    let running = kernel.build();
    running.run(SimTime::ZERO);
    running.finish();
}
