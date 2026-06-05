//! Module and elaboration ergonomics for SystemRS.
//!
//! The kernel ([`systemrs_kernel`]) exposes a faithful, low-level building API
//! (`add_method`/`add_thread` with explicit sensitivity slices). This crate layers
//! the idiomatic *facade* the design recommends (`doc/systemrs-design.md` §6b, §7):
//! an explicit process builder with `.sensitive_to(..)`/`.dont_initialize()`
//! replacing SystemC's hidden "last-created-process" `sensitive <<` coupling, and
//! the four lifecycle callbacks as a default-empty [`Elaborate`] trait.

mod build;
mod elaborate;

pub use build::{Build, MethodBuilder, ThreadBuilder};
pub use elaborate::Elaborate;

// Re-export the kernel surface model authors touch, so a single `use` of the
// facade (or this crate) suffices.
pub use systemrs_kernel::{Ctx, EventId, Sim};
