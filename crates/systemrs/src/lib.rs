//! # SystemRS
//!
//! A Rust, TLM-only equivalent of SystemC for transaction-level digital twins.
//!
//! SystemRS reproduces the parts of SystemC and TLM-2.0 needed to author digital
//! twins at transaction level, on a faithfully-ported single-threaded, cooperative,
//! three-phase delta-cycle scheduler — the determinism contract on which all TLM
//! behaviour rests (`doc/systemrs-design.md`). It layers idiomatic Rust on top:
//! stackful coroutines for `SC_THREAD`, an arena-and-generational-id object store,
//! sum types instead of signed-integer conventions, and `TypeId` maps instead of
//! RTTI.
//!
//! This crate is the **facade**: it re-exports the public API of the layered
//! crates. Most users `use systemrs::prelude::*;`.
//!
//! ## Layered crates
//!
//! - [`systemrs_time`] — `SimTime`, `Resolution`.
//! - [`systemrs_kernel`] — the scheduler, events, processes, [`Sim`], [`Ctx`].
//! - [`systemrs_core`] — module/elaboration ergonomics ([`Build`], [`Elaborate`]).
//! - [`systemrs_channels`] — `Signal`/`Buffer`/`Fifo`/`Clock`.
//! - [`systemrs_tlm2`] — the generic payload, transport, sockets, and a memory target.
//! - [`systemrs_diag`] — reporting.
//!
//! ## Examples
//!
//! The `systemrs-examples` crate ships two runnable models built on this facade:
//! an incrementing counter (clock + `SC_METHOD` + signal) and a basic RV32I CPU
//! hart (`SC_THREAD` + `b_transport` over a socket to a memory target).

pub use systemrs_channels as channels;
pub use systemrs_core as core;
pub use systemrs_diag as diag;
pub use systemrs_kernel as kernel;
pub use systemrs_tlm_utils as tlm_utils;
pub use systemrs_tlm2 as tlm2;

// The `#[module]` attribute macro (the facade is the only crate that may re-export
// it without forming a dependency cycle; the macro emits `::systemrs::`-paths).
pub use systemrs_macros::module;

// Flat re-exports of the most-used items.
pub use systemrs_channels::{Buffer, Clock, Export, Fifo, Interface, Port, PortPolicy, Signal};
pub use systemrs_core::{
    AttributeStore, Build, Builder, Building, Elaborate, Kernel, Module, ObjectId, ObjectKind,
    ObjectMeta, ObjectStore, Running, module, module_with, store,
};
pub use systemrs_kernel::{ChanId, Ctx, EventId, ProcId, Sim};
pub use systemrs_time::{Resolution, SimTime};
pub use systemrs_tlm_utils::{
    AtToLtAdapter, GlobalQuantum, LtToAtAdapter, PeqWithGet, PhaseQueue, QuantumKeeper, TxnId,
    next_phase, set_global_quantum,
};
pub use systemrs_tlm2::{
    BwBaseProtocol, ByteEnable, Command, GenericPayload, InitiatorSocket, Memory, Phase,
    ResponseStatus, TargetSocket, TlmSync, Txn, TxnPool,
};

pub mod prelude;
