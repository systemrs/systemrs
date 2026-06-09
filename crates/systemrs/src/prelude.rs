//! The SystemRS prelude: the common types and traits for authoring models.
//!
//! ```
//! use systemrs::prelude::*;
//!
//! let sim = Sim::new();
//! let clk = Clock::new(&sim, "clk", SimTime::from_ns(10));
//! sim.method("tick")
//!     .sensitive_to(clk.posedge_event())
//!     .dont_initialize()
//!     .finish(|_cx| { /* one tick */ });
//! sim.run_until(SimTime::from_ns(100));
//! ```

pub use systemrs_channels::{Buffer, Clock, Export, Fifo, Interface, Port, PortPolicy, Signal};
pub use systemrs_core::{
    AttributeStore, Build, Builder, Building, Elaborate, Kernel, Module, ObjectId, ObjectKind,
    ObjectMeta, ObjectStore, Running, module, module_with, store,
};
pub use systemrs_diag::{ReportHandler, Severity, Verbosity};
pub use systemrs_kernel::{ChanId, Ctx, EventId, ProcId, Sim, Stage};
pub use systemrs_macros::module as module_macro;
pub use systemrs_pdes::{
    BoundaryLink, LinkReceiver, LinkSender, LocalHost, Orchestrator, Region, RegionId,
    global_quantum_boundary,
};
pub use systemrs_time::{Resolution, SimTime};
pub use systemrs_tlm_utils::{
    AtToLtAdapter, GlobalQuantum, LtToAtAdapter, PeqWithGet, PhaseQueue, QuantumKeeper,
    set_global_quantum,
};
pub use systemrs_tlm1::{AnalysisFifo, AnalysisPort, AnalysisWrite};
pub use systemrs_tlm2::{
    ByteEnable, Command, GenericPayload, InitiatorSocket, Memory, Phase, ResponseStatus,
    TargetSocket, TlmSync, Txn, TxnPool,
};
pub use systemrs_trace::{MemorySink, TraceEvent, TraceSink, Tracer, WriterSink};
pub use systemrs_twin::{
    ChannelInputSender, JournalReplayer, RealTimePacer, Rng, StopSignal, TwinBuilder,
    attach_external_input, channel_input, journal_input,
};
