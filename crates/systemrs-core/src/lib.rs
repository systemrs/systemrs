//! Module and elaboration ergonomics for SystemRS.
//!
//! The kernel ([`systemrs_kernel`]) exposes a faithful, low-level building API
//! (`add_method`/`add_thread` with explicit sensitivity slices). This crate layers
//! the idiomatic *facade* the design recommends (`doc/systemrs-design.md` §6b, §7):
//! an explicit process builder with `.sensitive_to(..)`/`.dont_initialize()`
//! replacing SystemC's hidden "last-created-process" `sensitive <<` coupling, and
//! the four lifecycle callbacks as a default-empty [`Elaborate`] trait, the object
//! hierarchy ([`ObjectStore`]), and (from M2) the elaboration barrier.

// Pre-1.0: M2 introduces hierarchy/binding scaffolding (ObjectStore methods, the
// per-bucket elaborator registries) that later milestones consume; allow until
// 1.0.0 per the Rust skill, matching `systemrs-kernel`/`systemrs-tlm2`.
#![allow(dead_code)]

mod attribute;
mod build;
mod elaborate;
mod elaboration;
mod hierarchy;
mod kernel_typestate;
mod module;
mod name;
mod object;

pub use attribute::AttributeStore;
pub use build::{Build, MethodBuilder, ThreadBuilder};
pub use elaborate::Elaborate;
pub use kernel_typestate::{Building, Kernel, Running};
pub use module::{Builder, Module, module, module_with};
pub use object::{ObjectKind, ObjectMeta, ObjectStore, store};

// Re-export the kernel surface model authors touch, so a single `use` of the
// facade (or this crate) suffices.
pub use systemrs_kernel::{Ctx, EventId, ObjectId, Sim};
