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
//! - [`systemrs_tlm1`] — the analysis sublayer (`AnalysisPort`/`AnalysisFifo`).
//! - [`systemrs_tlm2`] — the generic payload, transport, sockets, and a memory target.
//! - [`systemrs_tlm_utils`] — quantum keeper, PEQs, LT↔AT adapters, convenience sockets.
//! - [`systemrs_trace`] — stage-callback tracing and an off-thread telemetry writer.
//! - [`systemrs_twin`] — the digital-twin layer (pacing, external input, replay).
//! - [`systemrs_diag`] — reporting.
//!
//! ## Quickstart
//!
//! A clock-driven counter — a `Clock`, an `SC_METHOD` sensitive to its rising edge,
//! and a `Signal` carrying the count:
//!
//! ```
//! use systemrs::prelude::*;
//!
//! let sim = Sim::new();
//! let count: Signal<u32> = Signal::new(&sim, "count", 0);
//! let clock = Clock::new(&sim, "clk", SimTime::from_ns(10));
//!
//! let mut n = 0u32;
//! sim.method("counter")
//!     .sensitive_to(clock.posedge_event())
//!     .dont_initialize()
//!     .finish(move |cx| {
//!         n += 1;
//!         count.write(cx, n);
//!     });
//!
//! sim.run_until(SimTime::from_ns(45)); // posedges at 0,10,20,30,40 ns
//! assert_eq!(count.read(&sim.ctx()), 5);
//! ```
//!
//! The `systemrs-examples` crate ships fuller reference models: an enable-gated
//! counter, an RV32I CPU hart over `b_transport`, a fixed-point guitar reverb, a
//! DMA engine over the AT protocol, and a real-time sensor twin.

pub use systemrs_channels as channels;
pub use systemrs_core as core;
pub use systemrs_diag as diag;
pub use systemrs_kernel as kernel;
pub use systemrs_pdes as pdes;
pub use systemrs_tlm_utils as tlm_utils;
pub use systemrs_tlm1 as tlm1;
pub use systemrs_tlm2 as tlm2;
pub use systemrs_trace as trace;
pub use systemrs_twin as twin;

// The `#[module]` attribute macro (the facade is the only crate that may re-export
// it without forming a dependency cycle; the macro emits `::systemrs::`-paths).
pub use systemrs_macros::module;

// Flat re-exports of the most-used items.
pub use systemrs_channels::{Buffer, Clock, Export, Fifo, Interface, Port, PortPolicy, Signal};
pub use systemrs_core::{
    AttributeStore, Build, Builder, Building, Elaborate, Kernel, Module, ObjectId, ObjectKind,
    ObjectMeta, ObjectStore, Running, module, module_with, store,
};
pub use systemrs_diag::{ActionFlags, ReportHandler, Severity, Verbosity};
pub use systemrs_kernel::{ChanId, Ctx, EventId, KernelSnapshot, ProcId, Sim, Stage};
pub use systemrs_pdes::{
    BoundaryLink, LinkReceiver, LinkSender, LocalHost, LocalLink, Orchestrator,
    OrchestratorBuilder, PdesError, Region, RegionId, assert_traces_match, global_quantum_boundary,
};
pub use systemrs_time::{Resolution, SimTime};
pub use systemrs_tlm_utils::{
    AtMemory, AtToLtAdapter, GlobalQuantum, LtToAtAdapter, MultiTargetSocket,
    PassthroughTargetSocket, PeqWithGet, PhaseQueue, QuantumKeeper, SimpleInitiatorSocket,
    SimpleTargetSocket, TxnId, next_phase, set_global_quantum,
};
pub use systemrs_tlm1::{AnalysisFifo, AnalysisPort, AnalysisTriple, AnalysisWrite};
pub use systemrs_tlm2::{
    BwBaseProtocol, ByteEnable, Command, Dmi, DmiAccess, GenericPayload, InitiatorSocket, Memory,
    Phase, ResponseStatus, TargetSocket, TlmSync, Txn, TxnPool,
};
pub use systemrs_trace::{
    MemorySink, TraceCommand, TraceEvent, TraceResponse, TraceSink, Tracer, TxnRecord, WriterSink,
};
pub use systemrs_twin::{
    ChannelInput, ChannelInputSender, ExternalInput, InjectionKind, InjectionRecord, Journal,
    JournalRecorder, JournalReplayer, PacerStats, RealTimePacer, Rng, StopSignal, TwinBuilder,
    attach_external_input, channel_input, journal_input,
};

pub mod prelude;
